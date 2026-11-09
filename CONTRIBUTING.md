# Contributing

Issues and pull requests are welcome. document-graphics is a standalone
rendering library shared by [gloss](https://github.com/garlab/gloss) and
[Plastext](https://github.com/garlab/plastext); changes are validated in both.

## Principles

- Stay a UI-independent rendering library. Applications own document layout,
  interaction, queues, cache budgets and worker idle policies.
- New backends are optional features that initialize only when used, with
  explicit input limits and platform isolation requirements. Future APIs may
  keep a bounded parsed model and render a selected fragment, position, page or
  tile without retaining an image for every item.
- Preserve resource limits, source fallbacks, cancellation and process
  cleanup. Expensive work must stay off consumer UI threads.
- The README describes implemented behavior and platform limits only; possible
  future formats must not appear as supported.
- Keep everything in the repository suitable for publication: generic examples,
  relative paths, no credentials or machine-specific details, and the existing
  license notices. Raw benchmarks, generated images and local dependency
  overrides stay out of the repository.

## Development

Rust 1.90 or newer.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --no-deps --locked -- -D warnings
cargo test --all-features --locked
cargo +1.90.0 check --all-targets --all-features --locked
```

Image features also need their minimal combinations:

```sh
cargo test --no-default-features --lib --locked
cargo test --no-default-features --features images --lib --locked
cargo test --no-default-features --features image-worker --lib --test images --locked
```

Renderer, SVG and protocol tests belong here; application integration and
policy tests stay with the consumers. On macOS, worker tests must run outside an
enclosing sandbox so the child can install its own. The `sgf_probe` and
`chess_probe` examples check a directory of records without modifying it.

## Mermaid backend

Mermaid parsing and layout come from
[`document-graphics-mermaid`](https://github.com/garlab/document-graphics-mermaid),
a maintained fork published separately and pinned to an exact version with its
default features disabled: this library owns SVG rasterization and needs
neither the backend's CLI nor its PNG renderer. Change the backend in its
repository and publish it first, then update the pin here. For local work on
both, add an override to an untracked `.cargo/config.toml`:

```toml
[patch.crates-io]
document-graphics-mermaid = { path = "../document-graphics-mermaid" }
```

## Workers

The Mermaid and image workers share private cancellation and deadline guards,
macOS memory sampling every 20 ms, and child termination and reaping. Their wire
protocols, I/O loops, error reporting and sandbox profiles stay separate.

## Releases and consumer checks

[docs/releasing.md](docs/releasing.md) covers local overrides in the consumers,
the checks to run in gloss and Plastext before a release, package review,
publication through the tag-triggered workflow, and the notes of past releases.
[docs/image-extraction.md](docs/image-extraction.md) records the image
decoding measurements.
