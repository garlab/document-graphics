//! Render a complete picture from an unsaved LaTeX snapshot in a disposable native worker.
use crate::{MAX_EDGE, MAX_PIXELS, Raster, RasterOptions, process::RequestGuard};
use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

// Includes the reader's scoped macro definitions as well as the picture itself.
pub const MAX_SOURCE: usize = 512 * 1024;
const MAX_ERROR: usize = 1024;

/// Dispatch this private argument to [`worker_main`] before application startup.
pub const WORKER_ARGUMENT: &str = "--tikz-worker";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DarkPalette {
    pub ink: [u8; 3],
    pub surface: [u8; 3],
}

/// Render in a disposable child of the current executable. Bounds and scale apply
/// to reading dimensions (150% of print size). Errors leave source available to the host.
pub fn render(
    source: &str,
    dark: Option<DarkPalette>,
    fitting: RasterOptions,
    timeout: Duration,
    cancel: &AtomicBool,
) -> io::Result<Raster> {
    if !cfg!(target_os = "macos") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Isolated TikZ preview requires macOS",
        ));
    }
    if !fitting.valid() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid TikZ raster options",
        ));
    }
    if source.len() > MAX_SOURCE {
        return Err(io::Error::other("TikZ input exceeds the preview limit"));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid TikZ timeout"))?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg(WORKER_ARGUMENT)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    thread::scope(|scope| {
        let (reply, receive) = mpsc::channel();
        scope.spawn(move || {
            let header = match dark {
                Some(DarkPalette {
                    ink: [r, g, b],
                    surface: [sr, sg, sb],
                }) => [1, r, g, b, sr, sg, sb],
                None => [0; 7],
            };
            let written = input
                .write_all(&header)
                .and_then(|()| input.write_all(&fitting.bounds.0.to_le_bytes()))
                .and_then(|()| input.write_all(&fitting.bounds.1.to_le_bytes()))
                .and_then(|()| input.write_all(&fitting.scale.to_le_bytes()))
                .and_then(|()| input.write_all(source.as_bytes()));
            drop(input);
            let result = written.and_then(|()| read_raster(output, fitting.bounds));
            let _ = reply.send(result);
        });
        let mut guard = RequestGuard::new(cancel, deadline, 256 * 1024 * 1024);
        let answer = loop {
            match guard.check(child.id(), false) {
                #[cfg(target_os = "macos")]
                Err(crate::process::RequestError::MemoryUnavailable) => {}
                Err(error) => break Err(crate::worker::request_error(error)),
                Ok(()) => {}
            }
            match receive.recv_timeout(guard.remaining().min(Duration::from_millis(10))) {
                Ok(result) => {
                    break guard
                        .check_completion()
                        .map_err(crate::worker::request_error)
                        .and(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break Err(io::Error::other("TikZ renderer stopped")),
            }
        };
        // Close both pipes even when cancellation interrupts a blocked read or write.
        crate::process::kill_and_reap(&mut child);
        answer
    })
}

fn read_raster(mut output: impl Read, bounds: (u32, u32)) -> io::Result<Raster> {
    let mut number = || {
        let mut bytes = [0; 4];
        output.read_exact(&mut bytes)?;
        Ok::<_, io::Error>(u32::from_le_bytes(bytes))
    };
    let width = number()?;
    if width == 0 {
        let mut message = String::new();
        output.take(MAX_ERROR as u64).read_to_string(&mut message)?;
        return Err(io::Error::new(io::ErrorKind::InvalidData, message));
    }
    let height = number()?;
    let original_width = number()?;
    let original_height = number()?;
    if [width, height, original_width, original_height]
        .iter()
        .any(|n| !(1..=MAX_EDGE).contains(n))
        || width > bounds.0
        || height > bounds.1
        || u64::from(width) * u64::from(height) > MAX_PIXELS as u64
    {
        return Err(io::Error::other("Invalid TikZ preview dimensions"));
    }
    let mut rgba = vec![0; width as usize * height as usize * 4];
    output.read_exact(&mut rgba)?;
    Ok(Raster {
        width,
        height,
        original_width,
        original_height,
        rgba,
    })
}

/// Child entry point for the isolated native renderer. Each process handles one figure.
#[cfg(target_os = "macos")]
pub fn worker_main() -> io::Result<()> {
    crate::worker::restrict_worker()?;
    // SAFETY: backstop for this dedicated child if its parent disappears.
    unsafe {
        libc::alarm(8);
    }

    let mut input = io::stdin().lock();
    let mut header = [0; 7];
    input.read_exact(&mut header)?;
    let dark = match header {
        [0, ..] => None,
        [1, r, g, b, sr, sg, sb] => Some(DarkPalette {
            ink: [r, g, b],
            surface: [sr, sg, sb],
        }),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unknown TikZ appearance",
            ));
        }
    };
    let mut width = [0; 4];
    let mut height = [0; 4];
    let mut scale = [0; 8];
    input.read_exact(&mut width)?;
    input.read_exact(&mut height)?;
    input.read_exact(&mut scale)?;
    let fitting = RasterOptions {
        bounds: (u32::from_le_bytes(width), u32::from_le_bytes(height)),
        scale: f64::from_le_bytes(scale),
    };
    if !fitting.valid() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid TikZ raster options",
        ));
    }
    let mut source = String::new();
    input
        .take(MAX_SOURCE as u64 + 1)
        .read_to_string(&mut source)?;
    let mut output = io::stdout().lock();
    match rasterize(&source, dark, fitting) {
        Ok(image) => {
            for n in [
                image.width,
                image.height,
                image.original_width,
                image.original_height,
            ] {
                output.write_all(&n.to_le_bytes())?;
            }
            output.write_all(&image.rgba)
        }
        Err(error) => {
            output.write_all(&0u32.to_le_bytes())?;
            let mut end = error.len().min(MAX_ERROR);
            while !error.is_char_boundary(end) {
                end -= 1;
            }
            output.write_all(&error.as_bytes()[..end])
        }
    }
}

#[cfg(target_os = "macos")]
fn rasterize(
    source: &str,
    dark: Option<DarkPalette>,
    fitting: RasterOptions,
) -> Result<Raster, String> {
    use resvg::{tiny_skia, usvg};
    if source.len() > MAX_SOURCE {
        return Err("TikZ input exceeds the preview limit".into());
    }
    let figure = tikra::convert(source).map_err(|e| e.to_string())?;
    if let Some(warning) = figure.warnings.first() {
        return Err(format!("TikZ: {warning}"));
    }
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, _, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&figure.svg, &options).map_err(|e| e.to_string())?;
    let native = tree.size().to_int_size();
    let longest = native.width().max(native.height());
    if longest > MAX_EDGE {
        return Err("TikZ picture exceeds the preview dimensions".into());
    }
    // Enlarge print-sized labels alongside reflowed body text. The view fits
    // these logical dimensions to the pane, independently of raster density.
    let reading_scale = 1.5_f64.min(f64::from(MAX_EDGE) / f64::from(longest));
    let w = (f64::from(native.width()) * reading_scale).round() as u32;
    let h = (f64::from(native.height()) * reading_scale).round() as u32;
    let scale = fitting
        .scale
        .min((MAX_PIXELS as f64 / (f64::from(w) * f64::from(h))).sqrt())
        .min(f64::from(fitting.bounds.0) / f64::from(w))
        .min(f64::from(fitting.bounds.1) / f64::from(h));
    let width = ((f64::from(w) * scale) as u32).max(1);
    let height = ((f64::from(h) * scale) as u32).max(1);
    let mut pixels = tiny_skia::Pixmap::new(width, height).ok_or("Invalid TikZ preview size")?;
    if dark.is_none() {
        pixels.fill(tiny_skia::Color::WHITE);
    }
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width(),
            height as f32 / tree.size().height(),
        ),
        &mut pixels.as_mut(),
    );
    let rgba = if let Some(palette) = dark {
        pixels
            .pixels()
            .iter()
            .flat_map(|pixel| {
                if pixel.alpha() == 0 {
                    return [0; 4];
                }
                let color = pixel.demultiply();
                let [r, g, b] = palette.color([color.red(), color.green(), color.blue()]);
                [r, g, b, color.alpha()]
            })
            .collect()
    } else {
        pixels.take()
    };
    Ok(Raster {
        width,
        height,
        original_width: w,
        original_height: h,
        rgba,
    })
}

#[cfg(target_os = "macos")]
impl DarkPalette {
    fn color(self, rgb: [u8; 3]) -> [u8; 3] {
        // Every RGB color contains some black, some white, and a saturated hue.
        // Substitute the theme colors for its black and white components. This
        // maps gray labels between ink and surface and keeps distinct hues.
        let [r, g, b] = rgb;
        let white = r.min(g).min(b);
        let black = 255 - r.max(g).max(b);
        std::array::from_fn(|i| {
            let neutral = u16::from(black) * u16::from(self.ink[i])
                + u16::from(white) * u16::from(self.surface[i]);
            rgb[i] - white + ((neutral + 127) / 255) as u8
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn worker_main() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Isolated TikZ preview requires macOS",
    ))
}
