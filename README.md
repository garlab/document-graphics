# document-graphics

Bounded native Mermaid, SVG and optional SGF Go-board rendering for terminal and GUI applications. The
crate returns straight RGBA pixels and logical dimensions. It has no UI framework,
Markdown document model, application cache, broker or background service.

Mermaid supports an ASCII subset: flowcharts, sequence, class and state diagrams,
and ER relationships. Unsupported syntax returns `Ok(None)` through the worker;
callers can retain the original source. SVG rendering uses bundled KaTeX fonts
and rejects external resources, filters, reuse and foreign content.

## Scope

The library renders the graphical content of documents independently of their
layout or user interface. Today it supports Mermaid diagrams, a restricted
SVG subset and Go positions from SGF records (`sgf` feature). Image decoding,
other diagram formats and music notation are possible future modules; they are
not implemented here yet.
Applications own document layout, interaction, scheduling and cache budgets.
Equation layout remains in `katex-native`.

New backends should remain optional and initialize only when used. Their own
input limits and platform isolation requirements must be explicit. Future APIs
may retain a bounded parsed model and render a selected fragment, position, page
or tile without retaining an image for every item.

## Integration

Requires Rust 1.90 or newer.

```toml
[dependencies]
document-graphics = "0.1"
```

Handle the private worker argument before initializing the application:

```no_run
use document_graphics::{RasterOptions, worker::{self, Worker}};
use std::{sync::atomic::AtomicBool, time::Duration};

fn main() -> std::io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--preview-worker") {
        return worker::worker_main();
    }
    // Run blocking rendering on a background thread in interactive applications.
    let mut worker = Worker::spawn(&std::env::current_exe()?, "--preview-worker")?;
    let cancelled = AtomicBool::new(false);
    let _image = worker.render(
        "flowchart LR; Read-->Preview", false,
        RasterOptions::scaled(2.), Duration::from_secs(2), &cancelled,
    )?;
    // Move image.rgba into the application's image type; no pixel copy is needed.
    Ok(())
}
```

`RasterOptions::fit((width, height))` fits within physical bounds without upscaling.
`RasterOptions::scaled(2.)` requests Retina output while preserving logical SVG
size. Applications can set both fields explicitly. Source and raster limits apply
to every mode: 16 KiB / 256 lines of Mermaid, 128 nodes, 256 edges, 256 KiB SVG,
one million output pixels, and an 8,192-pixel edge. Additional nesting, geometry,
annotation and label limits are enforced before expensive work.

Use `svg::rasterize(source, options)` for bounded local SVG. It accepts no file
paths and does not fetch resources. Only the internal Mermaid route permits
bounded arrow markers. Mermaid parsing and layout are private to the worker;
there is no public in-process Mermaid entry point.

## SGF Go records

Enable the optional `sgf` feature (available from 0.1.1). It adds no dependencies,
network access or runtime installation requirements. Parse a bounded collection,
reconstruct the selected board, and render it off the application's UI thread:

```rust
# #[cfg(feature = "sgf")] {
use document_graphics::{sgf::{Collection, RenderOptions, rasterize}, RasterOptions};
use std::sync::{Arc, atomic::AtomicBool};
let cancel = AtomicBool::new(false);
let source: Arc<[u8]> = Arc::from(&b"(;FF[4]GM[1]SZ[9];B[dd];W[ee])"[..]);
let collection = Collection::parse(source, &cancel)?;
let position = collection.position(2, &cancel)?;
let image = rasterize(&position, RenderOptions::default(), RasterOptions::scaled(2.), &cancel)?;
# }
# Ok::<(), Box<dyn std::error::Error>>(())
```

The compact index retains source ranges and tree links, including unknown
properties; it does not store a board or bitmap for every node. `Position::advance`
replays a direct child incrementally. Applications choose which positions to cache.
Captures, passes, setup stones, handicap, move numbers, common board marks,
labels, arrows and lines are supported. Replay follows SGF positions without
checking competitive legality, ko or turn order. Comments and game information
are exposed as text for the caller's UI, not embedded in the diagram.

Limits: 8 MiB source, 100,000 nodes, 1,024 games, 256 levels of variation nesting,
300,000 properties (128 per node), 500,000 values, 128 KiB per value, boards up to
52 × 52, 512 marks and 512 lines per position, and 32 characters per board label.
Position reconstruction has a 20-million-step work limit. SVG and raster limits
also apply, so dense annotated boards can exceed the diagram budget. UTF-8 and
ISO-8859-1 are accepted; missing `CA` uses UTF-8 when valid, otherwise ISO-8859-1.
Other games/encodings return an explicit error. `VW` cropping and legacy markup
are retained in source but not interpreted. Board labels use bundled fonts.

Unlike Mermaid's layout engine, this bounded, iterative parser and replay loop
run in-process with cooperative cancellation, followed by the restricted SVG
renderer. They are not isolated by the Mermaid worker's OS sandbox. Callers must
schedule them off-thread, coalesce requests and retain readable source on failure.
There is no hard wall-clock deadline on this in-process API.

The repository's `sgf_probe` example reads an SGF directory, checks every node and
renders roots and leaves without modifying any input:

```sh
cargo run --release --features sgf --example sgf_probe -- path/to/sgf-directory
```

## Worker lifecycle

Applications choose cache sizes, scheduling, idle retirement and total invocation
budgets. Each worker handles at most `worker::MAX_REQUESTS` (32) requests; check
`requests_completed()` and replace the worker when needed. Rejected syntax does
not break the worker. An I/O error, cancellation or deadline stops and reaps it
before returning; an errored handle must be replaced. Dropping a healthy handle
also stops and reaps its child. Process creation itself is outside the per-render
deadline.

The child receives source, theme and raster options over bounded private pipes;
its inherited environment is cleared. Unix workers disable core dumps and have
an eight-second cumulative CPU backstop. On macOS the worker's sandbox profile
allows everything by default, then denies file-data reads, writes, networking and
child processes; other operations, such as Mach service lookups, remain allowed.
The parent samples resident memory against 256 MiB. Other platforms retain input/output bounds and deadlines,
but do not yet have equivalent OS sandbox and resident-memory enforcement.

The library owns these mechanisms. A terminal app can retain a five-second total
budget; a desktop app can reuse a worker through a bounded queue and retire it on
idle. Neither policy is imposed on the other application.

## Development

This is a standalone library repository, shared by mdviewer and Plastext.
Dependencies come from crates.io; no sibling checkout is required to build or
test the library.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo +1.90.0 check --all-targets --all-features --locked
```

Shared renderer, SVG and protocol tests belong here. Application integration and
policy tests stay with the consumers. On macOS, worker tests must run outside an
enclosing sandbox so the child can install its own sandbox.

See [release checks](https://github.com/garlab/document-graphics/blob/main/docs/releasing.md)
for testing changes in the consumers, packaging and consumer validation.

MIT; see [LICENSE](https://github.com/garlab/document-graphics/blob/main/LICENSE).
`katex-native` includes KaTeX fonts under OFL-1.1.
