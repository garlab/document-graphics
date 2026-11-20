//! Exercise the native figure worker and check the placement of missing children.
use document_graphics::Raster;
use document_graphics::{RasterOptions, tikz};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn render(
    source: &str,
    dark: Option<tikz::DarkPalette>,
    cancel: &AtomicBool,
) -> std::io::Result<Raster> {
    tikz::render(
        source,
        dark,
        RasterOptions::scaled(2.),
        Duration::from_secs(5),
        cancel,
    )
}

fn main() {
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some(tikz::WORKER_ARGUMENT) {
        tikz::worker_main().unwrap();
        return;
    }
    let cancel = AtomicBool::new(false);
    if !cfg!(target_os = "macos") {
        assert_eq!(
            render("", None, &cancel).unwrap_err().kind(),
            std::io::ErrorKind::Unsupported
        );
        return;
    }
    for missing_first in [false, true] {
        let children = if missing_first {
            r"child[missing] child {node {\valueof{16}}}"
        } else {
            r"child {node {\valueof{16}}} child[missing]"
        };
        let source = format!(
            r"\newcommand{{\valueof}}[1]{{$\mathbf{{#1}}$}}
            \begin{{tikzpicture}}[level distance=12mm, level 1/.style={{sibling distance=40mm}}]
            \node {{$8$}} {children};
            \end{{tikzpicture}}"
        );
        let image = render(&source, None, &cancel).unwrap();
        assert!(image.original_width > 60 && image.original_height > 40);
        let top = ink_center(&image, 0..image.height / 4);
        let bottom = ink_center(&image, 3 * image.height / 4..image.height);
        if missing_first {
            assert!(top < 0.3 && bottom > 0.7, "{top}, {bottom}");
        } else {
            assert!(top > 0.7 && bottom < 0.3, "{top}, {bottom}");
        }
    }
    check_appearance(&cancel);
    // A one-inch-wide figure is 96 logical pixels at its natural size.
    let inch = r"\documentclass[border=0pt]{standalone}\begin{document}
        \begin{tikzpicture}\fill (0,0) rectangle (1in,0.5in);\end{tikzpicture}
        \end{document}";
    let image = render(inch, None, &cancel).unwrap();
    assert_eq!((image.original_width, image.original_height), (144, 72));
    assert_eq!((image.width, image.height), (288, 144));
    let fitted = tikz::render(
        inch,
        None,
        RasterOptions::fit((60, 100)),
        Duration::from_secs(5),
        &cancel,
    )
    .unwrap();
    assert_eq!((fitted.width, fitted.height), (60, 30));
    assert_eq!((fitted.original_width, fitted.original_height), (144, 72));
    assert!(render("no picture", None, &cancel).is_err());
    assert_eq!(
        render("cancelled", None, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
}

fn check_appearance(cancel: &AtomicBool) {
    let source = r"\begin{tikzpicture}
        \node at (0,0) {$8$};
        \fill[black!50] (1,0) rectangle (2,1);
        \fill[red] (3,0) rectangle (4,1);
        \fill[white] (5,0) rectangle (6,1);
        \end{tikzpicture}";
    let palette = tikz::DarkPalette {
        ink: [234, 230, 222],
        surface: [30, 36, 42],
    };
    let light = render(source, None, cancel).unwrap();
    let dark = render(source, Some(palette), cancel).unwrap();
    assert_eq!((light.width, light.height), (dark.width, dark.height));
    assert!(light.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    let pixels = || dark.rgba.as_chunks::<4>().0.iter();
    assert!(pixels().any(|p| p[3] == 0), "unpainted canvas");
    assert!(
        pixels().any(|p| p[..3] == palette.ink && (1..255).contains(&p[3])),
        "antialiased label"
    );
    assert!(
        pixels().any(|p| *p == [255, 0, 0, 255]),
        "saturated red fill"
    );
    assert!(
        pixels().any(|p| p[..3] == palette.surface && p[3] == 255),
        "explicit white fill"
    );
    assert!(
        pixels().any(|p| p[3] == 255 && p[..3].iter().all(|c| c.abs_diff(132) <= 2)),
        "mid-gray fill"
    );
}

fn ink_center(image: &Raster, rows: std::ops::Range<u32>) -> f64 {
    let mut sum = 0;
    let mut count = 0;
    for y in rows {
        for x in 0..image.width {
            if image.rgba[((y * image.width + x) * 4) as usize] < 128 {
                sum += u64::from(x);
                count += 1;
            }
        }
    }
    assert!(count > 0);
    sum as f64 / (f64::from(count) * f64::from(image.width))
}
