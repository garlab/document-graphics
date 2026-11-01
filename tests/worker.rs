//! Exercise the same isolated worker entry point as the app, without a window.
use document_graphics::{
    RasterOptions,
    worker::{self, Worker},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--mermaid-worker")) {
        worker::worker_main().unwrap();
        return;
    }
    if std::env::args().any(|arg| arg == "--large") {
        large_flowchart();
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let mut worker = Worker::spawn(&std::env::current_exe().unwrap(), "--mermaid-worker").unwrap();
    // Each diagram's logical size, in both themes. A change means the renderer,
    // resvg or the fonts moved the layout: review the images, then update it.
    let cases = [
        (
            "flowchart",
            "flowchart LR\n A[Read Markdown] --> B{Diagram?}\n B -->|Yes| C[Preview]\n B -->|No| D[Text]\n C --> E[Read]\n D --> E\n",
            Some((705, 181)),
        ),
        (
            "sequence",
            "sequenceDiagram\n participant R as Reader\n participant V as Viewer\n R->>V: Open file\n activate V\n V-->>R: Rendered preview\n deactivate V\n",
            Some((450, 287)),
        ),
        (
            "class",
            "classDiagram\n class Document {\n +String source\n +render()\n }\n Document --> Renderer\n",
            Some((201, 304)),
        ),
        (
            "state",
            "stateDiagram-v2\n [*] --> Reading\n Reading --> Rendering: diagram\n Rendering --> Reading: next block\n Reading --> [*]\n",
            Some((214, 198)),
        ),
        (
            "er",
            "erDiagram\n DOCUMENT ||--o{ BLOCK : contains\n",
            Some((196, 197)),
        ),
        (
            "subgraph",
            "flowchart LR\n subgraph Input\n A[File] --> B[Parser]\n end\n subgraph Output\n C[Diagram] --> D[Preview]\n end\n B --> C\n",
            Some((779, 147)),
        ),
        (
            "unicode",
            "flowchart LR\n A[Café déjà vu] --> B[Ελληνικά Русский]",
            Some((445, 78)),
        ),
        (
            "attributes",
            "erDiagram\n BOOK {\n int identifier PK,FK,UK\n varchar title \"display name\"\n decimal price\n }\n BOOK ||--o{ PAGE : contains\n PAGE {\n int identifier PK\n int book_id FK\n }",
            Some((411, 373)),
        ),
        (
            "pie",
            "pie showData\n title Café\n \"Thé\": 3\n \"Café\": 7",
            Some((454, 360)),
        ),
        (
            "mindmap",
            "mindmap\n root((Reading))\n   Notes\n     Ideas\n   Diagrams",
            Some((543, 103)),
        ),
        (
            "timeline",
            "timeline\n title History\n 2020 : Launch : Learn\n 2021 : Grow",
            Some((340, 280)),
        ),
        (
            "journey",
            "journey\n title Reading\n section Start\n Open file: 5: Reader\n Read: 4: Reader, Editor",
            Some((309, 247)),
        ),
        (
            "quadrant",
            "quadrantChart\n title Work\n x-axis Low effort --> High effort\n y-axis Low value --> High value\n Task A: [0.2, 0.8]\n Task B: [0.8, 0.3]",
            Some((552, 511)),
        ),
    ];
    let mut times = Vec::new();
    for (name, source, expected) in cases {
        for light in [false, true] {
            let start = Instant::now();
            let p = worker
                .render(
                    source,
                    light,
                    RasterOptions::scaled(2.),
                    std::time::Duration::from_secs(2),
                    &cancel,
                )
                .unwrap()
                .expect(name);
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            times.push(elapsed);
            if let Some((width, height)) = expected {
                assert_eq!(
                    (p.original_width, p.original_height),
                    (width, height),
                    "{name}"
                );
                if u64::from(width) * u64::from(height) * 4 <= document_graphics::MAX_PIXELS as u64
                {
                    assert_eq!((p.width, p.height), (2 * width, 2 * height), "{name}");
                }
            }
            assert!(p.width as usize * p.height as usize <= document_graphics::MAX_PIXELS);
            assert_eq!(p.rgba.len(), (p.width * p.height * 4) as usize);
            assert!(p.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
            println!(
                "{name}, light={light}: {:.2} ms, {}x{} logical, {} raster bytes",
                elapsed,
                p.original_width,
                p.original_height,
                p.rgba.len()
            );
        }
    }
    assert!(
        worker
            .render(
                "gantt\nTask: 2025-01-01, 1d\n",
                false,
                RasterOptions::fit((500, 300)),
                std::time::Duration::from_secs(2),
                &cancel
            )
            .unwrap()
            .is_none()
    );
    assert!(
        worker
            .render(
                "flowchart LR\n A[中文]-->B\n",
                false,
                RasterOptions::fit((500, 300)),
                std::time::Duration::from_secs(2),
                &cancel
            )
            .unwrap()
            .is_none()
    );
    // The renderer stays available after failure and after worker recycling.
    for _ in 0..40 {
        if worker.requests_completed() >= worker::MAX_REQUESTS {
            worker = Worker::spawn(&std::env::current_exe().unwrap(), "--mermaid-worker").unwrap();
        }
        assert!(
            worker
                .render(
                    "flowchart LR; A-->B",
                    false,
                    RasterOptions::fit((500, 300)),
                    std::time::Duration::from_secs(2),
                    &cancel
                )
                .unwrap()
                .is_some()
        );
    }
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        worker
            .render(
                "flowchart LR; A-->B",
                false,
                RasterOptions::fit((500, 300)),
                std::time::Duration::from_secs(2),
                &cancel
            )
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    println!(
        "Mermaid integration passed ({} fixture renders; cold {:.2} ms, warm max {:.2} ms).",
        times.len(),
        times[0],
        times[1..].iter().copied().fold(0., f64::max)
    );
}

// Opt-in because this exercises an interactive deadline, not just correctness.
// Run in an optimized profile; see docs/releasing.md for the command.
fn large_flowchart() {
    let mut source = String::from("flowchart TD\nN0 --> N1\n");
    for n in 1..256 {
        source.push_str(&format!("N{n} --> N{}\n", n * 2));
        if n % 2 == 0 {
            source.push_str(&format!("N{n} --> N{}\n", n + 1));
        }
    }
    source.push_str("classDef marked fill:#ffcc00,stroke:#112233\nclass N0,N2,N510 marked\n");
    for light in [false, true] {
        let start = Instant::now();
        let mut worker =
            Worker::spawn(&std::env::current_exe().unwrap(), "--mermaid-worker").unwrap();
        let image = worker
            .render(
                &source,
                light,
                RasterOptions::scaled(2.),
                std::time::Duration::from_secs(2),
                &AtomicBool::new(false),
            )
            .unwrap()
            .expect("384-node flowchart");
        assert!(image.original_width > 8192);
        assert!(image.width as usize * image.height as usize <= document_graphics::MAX_PIXELS);
        assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
        assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        println!(
            "large flowchart, light={light}: cold {:.2} ms, {}x{} logical, {} raster bytes",
            start.elapsed().as_secs_f64() * 1000.,
            image.original_width,
            image.original_height,
            image.rgba.len()
        );
    }
}
