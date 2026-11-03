# document-graphics

Bounded Mermaid/SVG rendering, optional image decoding, and Go/chess boards for terminal and GUI applications. The
crate returns straight RGBA pixels and logical dimensions. It has no UI framework,
Markdown document model, application cache, broker or background service.

Mermaid supports flowcharts, sequence, class and state diagrams, ER attribute
tables, pie charts, mind maps, timelines without sections, user journeys and
quadrant charts. Bundled fonts cover Latin (including accents), Greek and
Cyrillic labels. Unsupported syntax and unavailable glyphs return `Ok(None)`
through the worker so callers can retain the original source. The strict document SVG renderer uses
bundled KaTeX and Noto Sans fonts and rejects external resources, filters, reuse
and foreign content.

## Scope

The library renders the graphical content of documents independently of their
layout or user interface. Today it supports Mermaid diagrams, a restricted
SVG subset, Go positions from SGF records (`sgf` feature), and standard chess
from PGN/FEN records (`chess` feature), and bounded image decoding (`images`,
`image-worker` and `image-native` features, unreleased 0.1.5). Other diagram
formats and music notation are possible future modules, not implemented here.
Applications own document layout, interaction, scheduling and cache budgets.
Equation layout remains in `katex-native`.

New backends should remain optional and initialize only when used. Their own
input limits and platform isolation requirements must be explicit. Future APIs
may retain a bounded parsed model and render a selected fragment, position, page
or tile without retaining an image for every item.

## Integration

Requires Rust 1.90 or newer. Version 0.1.5 is prepared locally, pending release;
use a [local override](https://github.com/garlab/document-graphics/blob/main/docs/releasing.md#local-dependency-overrides) until publication.

```toml
[dependencies]
document-graphics = "0.1.5"
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
size. Applications can set both fields explicitly. Mermaid input is limited to
16 KiB / 1,024 lines, 512 nodes and 1,024 edges. Its generated SVG is limited to
1 MiB, 16,384 XML nodes, 256 KiB of geometry, a 32,768-unit logical edge and
64 million logical square units. These logical dimensions do not allocate a
full-size bitmap: output remains capped at one million pixels and an
8,192-pixel edge. Local SVG retains its 256 KiB source, 4,096 XML nodes,
64 KiB of geometry, 8,192-unit logical edge and four-million-unit area limits.
Additional nesting, annotation and label limits apply before expensive work. Pie charts allow
64 slices, mind maps 128 nodes/depth 32, timelines 64 periods/128 events, journeys
32 actors with scores 1–5, and quadrant charts 128 points with coordinates 0–1.
Unsupported timeline sections retain source. Nonfinite chart numbers are rejected.

Use `svg::rasterize(source, options)` for bounded local SVG. It accepts no file
paths and does not fetch resources. Only the internal Mermaid route permits
bounded arrow markers. Mermaid parsing and layout are private to the worker;
there is no public in-process Mermaid entry point.

The expanded Mermaid limits are available from 0.1.2, using the published
`document-graphics-mermaid` backend. They retain the existing worker memory
and raster budgets.

Version 0.1.3 pins `document-graphics-mermaid` 0.1.3 for faster flowchart route
comparisons and improved quadrant-axis label contrast. The public API, worker
protocol, feature defaults and resource limits are unchanged.

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

## Chess records

Enable the optional `chess` feature for standard-chess PGN collections and FEN
positions. It adds the MIT-licensed `cozy-chess` move generator and its types
crate. There is no engine, network access, runtime installation or system-font
requirement. Chess piece silhouettes are original vector paths under this
crate's MIT license.

```rust
# #[cfg(feature = "chess")] {
use document_graphics::{chess::{Collection, Format, RenderOptions, rasterize}, RasterOptions};
use std::sync::{Arc, atomic::AtomicBool};
let cancel = AtomicBool::new(false);
let source: Arc<[u8]> = Arc::from(&b"1.e4 e5 (1...c5 {Sicilian}) 2.Nf3 *"[..]);
let collection = Collection::parse(source, Format::Pgn, &cancel)?;
let position = collection.position(2, &cancel)?;
let image = rasterize(&position, RenderOptions::default(), RasterOptions::scaled(2.), &cancel)?;
# }
# Ok::<(), Box<dyn std::error::Error>>(())
```

SAN moves, castling, en passant, promotions, disambiguation, nested variations,
comments, annotation glyphs, tags, results and FEN starting positions are
supported. `Format::Fen` accepts one standard six-field FEN per nonempty line.
UTF-8 text falls back to Latin-1 for older PGN annotations. Chess variants are
rejected; engine annotations and drawing extensions remain comment text.

Parsing builds a compact source/tree index, with one initial board per game.
It does not validate every move eagerly. `Collection::position` replays the
requested line with legal moves; `Position::advance` handles a direct child.
Illegal or ambiguous moves return an error rather than a guessed position.
Applications own navigation, comments, cache policy and draw adjudication.
`RenderOptions::flipped` changes perspective; the last move is highlighted.

Limits: 8 MiB source, 100,000 nodes, 1,024 games, 128 variation levels, 16,384
half-moves per line, 128 tags per game, 64 KiB per tag/comment budget per node,
128 annotations per node and 300,000 annotations total. Tokens are limited to
32 bytes. Output retains the existing one-million-pixel raster cap.

This bounded, iterative parser and replay run in-process with cooperative
cancellation, followed by restricted SVG rendering. They do not use the
Mermaid worker's OS sandbox or hard deadline. Schedule them off the UI thread,
coalesce obsolete requests and preserve readable source on failure.

The read-only corpus probe checks all positions and renders roots/leaves:

```sh
cargo run --release --features chess --example chess_probe -- path/to/chess-records
```

## Image decoding (unreleased 0.1.5)

All image features are disabled by default. They create no background threads,
queues or caches until the consumer calls them.

| Feature | API and behavior |
| --- | --- |
| `images` | `images::decode` accepts PNG/JPEG bytes and returns a fitted, orientation-corrected `Raster`. Portable Rust codecs; no file access or system fonts. |
| `image-worker` | Adds `images::worker::{Worker, worker_main}`, metadata, regions, streaming PNG tiles and broader SVG in a disposable Unix helper. Portable raster codecs provide still images. |
| `image-native` | Includes `image-worker`; on macOS enables ImageIO/Core Graphics, color profiles, EXIF orientation, native codecs and GIF/WebP animation. On other Unix systems it uses the portable worker. |

For small PNG/JPEG thumbnails:

```no_run
# #[cfg(feature = "images")] {
use document_graphics::images::{decode, Format, Limits};
use std::sync::atomic::AtomicBool;
let bytes = std::fs::read("image.png")?; // The application bounds file access.
let raster = decode(&bytes, Format::Png, (800, 600), Limits::default(), &AtomicBool::new(false));
# }
# Ok::<(), Box<dyn std::error::Error>>(())
```

The default limits are 8 MiB encoded, 8,192 pixels per source edge, four million
source pixels, 32 MiB decoder allocation and one million output pixels. Callers
may lower or raise them within the documented hard ceilings. Decoding runs
in-process; cancellation is checked around codec calls, not inside them. These
allocation budgets are not a process RSS ceiling. Run decoding off UI threads.

For isolated decoding, route an application-private argument to
`images::worker::worker_main()` before UI initialization. Off-thread, call
`Worker::spawn(executable, argument, &stable_file)`, then `info(cancel)` and
`render(regions, cancel)`. The caller supplies a stable regular file (typically
an immutable snapshot); the library duplicates its descriptor, passes it to the
child, and never resolves a document path. `Info::overview(frame)` requests a
bounded thumbnail. Regions use physical output pixels and fixed-point scale
(`SCALE_UNIT` means 1:1). RGBA buffers have straight alpha. Metadata includes
dimensions, format, color profile/model, frame count and total plays;
rendered frames include a bounded delay. Animation scheduling remains app-owned.

The worker accepts 256 MiB files, 100,000 pixels per source edge, two billion
source pixels, and 10,000 frames. SVG source is limited to 32 MiB. Each request
allows 16 regions, at most 1,600 pixels per edge and 4,194,304 total output pixels.
Streaming PNG handles Adam7 without allocating the full decoded image. Other
portable codecs use a 16-megapixel limit and 128 MiB decoder allocation budget;
they provide still images without the native orientation/color pipeline.
The portable worker detects PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, QOI and PNM
from file contents; TGA recognition requires the native codec.
AVIF, HEIC and other native formats depend on installed macOS codecs. There are
no codec downloads, external tools or network requests.

Each request has a ten-second deadline. Cancellation, protocol errors, decoding
errors and deadlines stop and reap the child before returning; replace a failed
handle. Dropping a handle also reaps it. Process creation is outside the request
deadline. Children clear inherited environment variables, disable core dumps and
use a twelve-second per-operation alarm. macOS samples worker RSS against
256 MiB and fails closed if its sandbox cannot be installed. That sandbox denies
networking, file writes and child execution; codec/font reads remain allowed.
Other Unix systems use a 512 MiB address-space limit, without an equivalent OS
sandbox. Windows supports `images::decode`, but not this worker yet.

The two SVG APIs deliberately retain different policies:

- `svg::rasterize` is the strict, in-process document subset with bundled fonts,
  small source/complexity limits, and no filters, reuse or external content.
- The image worker accepts broader resvg SVG, including local `<use>` and system
  text fonts. It disables both external and data-image references. This decoder
  is private and can only be reached through the worker API, with its resource
  guards and platform restrictions.

Applications retain source/path policy, snapshot storage, queues, cache budgets,
zoom, fitting, playback and idle retirement. Enabling a feature does not replace
those policies or silently broaden the strict SVG entry point.

## Mermaid worker lifecycle

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
The Mermaid backend comes from the published
[`document-graphics-mermaid`](https://crates.io/crates/document-graphics-mermaid)
crate, maintained in a [separate fork](https://github.com/garlab/mermaid-rs-renderer).
It provides caller-supplied font metrics, complete ER key badges, chart-label
contrast fixes and bounded routing policies for large flowcharts. Its default
features stay disabled: this library owns SVG rasterization and does not need
the backend's CLI or PNG renderer.
All dependencies come from crates.io; no sibling checkout is required.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --no-deps --locked -- -D warnings
cargo test --all-features --locked
cargo +1.90.0 check --all-targets --all-features --locked
```

Shared renderer, SVG and protocol tests belong here. Application integration and
policy tests stay with the consumers. On macOS, worker tests must run outside an
enclosing sandbox so the child can install its own sandbox.

See [release checks](https://github.com/garlab/document-graphics/blob/main/docs/releasing.md)
for testing changes in the consumers, packaging and consumer validation.

MIT; see [LICENSE](https://github.com/garlab/document-graphics/blob/main/LICENSE).
`katex-native` includes KaTeX fonts under OFL-1.1. Noto Sans assets and their
license/provenance are in [fonts](https://github.com/garlab/document-graphics/blob/main/fonts/README.md).
