//! Read-only corpus check: cargo run --release --features sgf --example sgf_probe -- DIRECTORY
#[cfg(feature = "sgf")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use document_graphics::{RasterOptions, sgf};
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("expected an SGF directory")?;
    let cancel = AtomicBool::new(false);
    let (mut files, mut games, mut nodes, mut images, mut rejected) = (0, 0, 0, 0, 0);
    let (mut parse_max, mut step_max, mut raster_max) =
        (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("sgf"))
        {
            continue;
        }
        if std::fs::metadata(&path)?.len() > sgf::MAX_SOURCE as u64 {
            return Err("source limit exceeded".into());
        }
        let bytes: Arc<[u8]> = std::fs::read(path)?.into();
        let start = Instant::now();
        files += 1;
        let doc = match sgf::Collection::parse(bytes, &cancel) {
            Ok(doc) => doc,
            Err(error) => {
                rejected += 1;
                eprintln!("Input {files}: {error}");
                continue;
            }
        };
        parse_max = parse_max.max(start.elapsed());
        games += doc.games().len();
        nodes += doc.nodes().len();
        let mut previous: Option<sgf::Position> = None;
        for (id, node) in doc.nodes().iter().enumerate() {
            let start = Instant::now();
            let position = if let Some(p) = &previous
                && node.parent == Some(p.node())
            {
                p.advance(&doc, id, &cancel)?
            } else {
                doc.position(id, &cancel)?
            };
            step_max = step_max.max(start.elapsed());
            if node.parent.is_none() || node.first_child.is_none() {
                let start = Instant::now();
                let raster = sgf::rasterize(
                    &position,
                    sgf::RenderOptions::default(),
                    RasterOptions {
                        bounds: (1000, 1000),
                        scale: 4.,
                    },
                    &cancel,
                )?;
                assert!(raster.rgba.len() <= 4_000_000);
                raster_max = raster_max.max(start.elapsed());
                images += 1;
                if files == 1
                    && node.first_child.is_none()
                    && let Some(output) = std::env::args_os().nth(2)
                {
                    let output = std::path::PathBuf::from(output);
                    std::fs::write(output.with_extension("rgba"), &raster.rgba)?;
                    std::fs::write(
                        output.with_extension("size"),
                        format!("{} {}", raster.width, raster.height),
                    )?;
                }
            }
            previous = Some(position);
        }
    }
    println!(
        "files={files} games={games} nodes={nodes} rendered_boards={images} rejected={rejected}"
    );
    println!(
        "max_parse_ms={:.3} max_position_ms={:.3} max_raster_ms={:.3}",
        parse_max.as_secs_f64() * 1000.,
        step_max.as_secs_f64() * 1000.,
        raster_max.as_secs_f64() * 1000.
    );
    Ok(())
}
#[cfg(not(feature = "sgf"))]
fn main() {
    eprintln!("Enable the sgf feature to run this example.");
}
