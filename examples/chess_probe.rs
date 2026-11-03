//! Read-only corpus check: cargo run --release --features chess --example chess_probe -- DIRECTORY
#[cfg(feature = "chess")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use document_graphics::{RasterOptions, chess};
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("expected an chess directory")?;
    let cancel = AtomicBool::new(false);
    let (mut files, mut games, mut nodes, mut images, mut rejected) = (0, 0, 0, 0, 0);
    let (mut parse_max, mut step_max, mut raster_max) =
        (Duration::ZERO, Duration::ZERO, Duration::ZERO);
    let mut entries = std::fs::read_dir(directory)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    for path in entries {
        let format = match path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("pgn") => chess::Format::Pgn,
            Some("fen") => chess::Format::Fen,
            _ => continue,
        };
        if std::fs::metadata(&path)?.len() > chess::MAX_SOURCE as u64 {
            return Err("source limit exceeded".into());
        }
        let bytes: Arc<[u8]> = std::fs::read(path)?.into();
        let start = Instant::now();
        files += 1;
        let doc = match chess::Collection::parse(bytes, format, &cancel) {
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
        let mut previous: Option<chess::Position> = None;
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
                let raster = chess::rasterize(
                    &position,
                    chess::RenderOptions::default(),
                    RasterOptions {
                        bounds: (1000, 1000),
                        scale: 4.,
                    },
                    &cancel,
                )?;
                assert!(raster.rgba.len() <= 4_000_000);
                raster_max = raster_max.max(start.elapsed());
                images += 1;
                if node.first_child.is_none()
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
#[cfg(not(feature = "chess"))]
fn main() {
    eprintln!("Enable the chess feature to run this example.");
}
