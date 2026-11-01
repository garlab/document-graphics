# Release checks

Run commands from the document-graphics repository root. Cargo discovers local
configuration from the working directory; using `--manifest-path` from a
consumer checkout can apply that consumer's overrides to the release build.

## Local dependency overrides

To build a consumer against a local checkout of this library, add this entry to
an untracked `.cargo/config.toml` in **each consumer**, adapting the relative path
to the checkout layout:

```toml
[patch.crates-io]
document-graphics = { path = "../document-graphics" }
```

Run Cargo from that consumer's root. Keep the library checkout outside its
`target/` directory so cleaning build artifacts cannot remove source. Published
consumers use a versioned registry dependency; local overrides are temporary.

## Review and validation

Review public API changes, source/raster limits, process cleanup and platform
guarantees. Backends should share rendering mechanisms while leaving application
scheduling and cache policies with consumers. Preserve license notices.

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --no-deps --locked -- -D warnings
cargo test --all-features --locked
cargo +1.90.0 check --all-targets --all-features --locked
cargo package --list --locked
cargo package --locked
```

The package command packages and verifies this library against its published
dependencies without uploading. Use a clean checkout without backend overrides
and review the archive's file list. Cargo.lock is tracked for reproducible
repository checks; consumers resolve transitive dependencies using their own lockfiles.
The package includes library sources, tests, README, license and Cargo metadata.

For renderer changes, check both consumers with
[local dependency overrides](#local-dependency-overrides):

- In mdviewer: run Rust tests and `tests/terminal_smoke.py` against a built
  binary; inspect graphics in a compatible terminal at normal and narrow widths.
- In Plastext: run the platform Mermaid worker and UI integration tests; inspect
  diagrams in both appearances, zoom, split panes, resizing and Source/Preview.
- Preserve readable source on failure. Check that navigation and closing views
  cancel obsolete work without leaving child processes behind.

Compare image output and feature costs when behavior changes. Keep generated
images and raw timing samples in ignored `target/` directories. A rename or
repository move alone should preserve renderer output.

### Backend 0.1.0 migration measurements

The 2026-10-31 migration replaced the patched, vendored `mermaid-rs-renderer`
0.3.1 with the published `document-graphics-mermaid` 0.1.0. Library tests
(including SGF and the macOS worker sandbox), consumer integration tests,
terminal checks, Clippy, Rust 1.90 and package verification passed.

The 13-diagram terminal fixture retained its text, dimensions and placement at
24 and 100 columns in both themes. Reviewed pixel changes were confined to
arrowheads, the final-state symbol and timeline/journey text colors.

Performance was measured with mdviewer's `bench/run.py`: release builds, one
warm-up and three measured processes per case, a drained 100-column PTY with
10 × 20-pixel cells, Apple M2 Pro / 16 GiB, macOS 26.4.1 and Rust 1.99.0.
These times include startup and worker execution, but exclude emulator painting.
Inputs and raw results remain outside the repositories. `mermaid-basic.md` used
the current 13-diagram fixture; the other inputs came from `bench/generate.py`.

| Input | Vendored backend | Registry backend |
| --- | ---: | ---: |
| 10 MiB mixed Markdown | 1,117.1 ms | 1,098.9 ms |
| 100 repeated diagrams | 25.4 ms | 25.2 ms |
| 40 unique diagrams (32 rendered within the budget) | 28.0 ms | 28.4 ms |
| Dense flowchart (32 nodes, 60 edges) | 426.2 ms | 586.5 ms |
| 13-diagram fixture | 66.8 ms | 70.8 ms |

The dense-flowchart slowdown (about 38%) was recorded with backend 0.1.0;
the 0.1.1 follow-up below recovers it. Retain this case when evaluating renderer
updates. A separate run with process sampling confirmed the slowdown. Its sampled
combined CLI/worker peak RSS was
13.4 → 13.5 MiB, while the 13-diagram fixture was 19.2 → 18.5 MiB. Sampling can
miss short peaks, so these are observations rather than memory bounds. The
mixed-document peak RSS was 58.0 → 58.1 MiB. The mdviewer release binary grew by
212 KiB (15.79 → 16.00 MiB); enabled runtime dependencies did not increase.

### Backend 0.1.1 performance check

The 2026-11-01 update pins the published backend 0.1.1. Its routing optimization
reuses shape polygons and skips intersection sampling for disjoint segments.
Both lockfiles changed only the backend version and checksum from the preceding
local integration state; mdviewer still used its local document-graphics override.

The same machine, release profile, corpora and PTY methodology above gave these
medians, with one warm-up and three fresh measured processes per case. Both
backend versions came from crates.io; builds and tests finished before timing.

| Input | Backend 0.1.0 | Backend 0.1.1 |
| --- | ---: | ---: |
| 10 MiB mixed Markdown | 1,097.4 ms | 1,078.2 ms |
| 100 repeated diagrams | 23.9 ms | 24.5 ms |
| 40 unique diagrams (32 rendered within the budget) | 28.2 ms | 27.5 ms |
| Dense flowchart (32 nodes, 60 edges) | 593.9 ms | 410.3 ms |
| 13-diagram fixture | 69.1 ms | 46.1 ms |

The dense case improves by about 31%, and the 13-diagram fixture by about 33%.
The mixed-document process peak RSS was 58.20 → 58.17 MiB, and the dense case
12.36 → 12.02 MiB. These are `wait4` process statistics, not concurrent
CLI/worker totals. The release binary remained about 16.00 MiB (16 bytes smaller).
Inputs and raw results remain outside the repositories; earlier baselines are
unchanged.

All 13 diagrams retained byte-identical terminal output at 24 and 100 columns in
both themes, including pixels and placement. Library and mdviewer Rust tests,
26 shared-worker fixture renders, all 43 terminal checks, formatting, Clippy and
Rust 1.90 checks passed. Timing excludes emulator painting.

## Publication

The [release workflow](../.github/workflows/release.yml) publishes version tags
through crates.io's Trusted Publishing; the repository stores no registry token.
After review and consumer validation:

The Mermaid backend is maintained and published separately as
[`document-graphics-mermaid`](https://crates.io/crates/document-graphics-mermaid).
When changing it, follow the fork's
[release guide](https://github.com/garlab/mermaid-rs-renderer/blob/fork-release/docs/release.md)
and publish the reviewed backend version first. Pin that version in this
library's `Cargo.toml`, keep `default-features = false`, and refresh `Cargo.lock`.
Confirm the backend's registry source and checksum in the lockfile, then repeat
the library and consumer checks above. Backend upgrades may change layout even
when the adapter API is unchanged; review any changed fixture dimensions or
rendered output and compare performance and memory use.

Consumers need only the document-graphics override for local integration tests;
the backend resolves from crates.io. Release document-graphics through the
existing workflow:

1. Set the new version in `Cargo.toml`, run `cargo check` so `Cargo.lock` follows,
   commit, and push `main`.
2. Tag that commit and push the tag:

   ```sh
   git tag -a vX.Y.Z -m release
   git push origin vX.Y.Z
   ```

The workflow checks that the tag matches `Cargo.toml` and is on `main`, runs the
tests and a dry-run publish, then publishes. Push `main` before the tag: if the
tag's commit isn't on `main` yet, push it and re-run the workflow. A version
already on crates.io is skipped, so re-running is safe. 0.1.0 was published by
hand, since Trusted Publishing can only be set up for a crate that exists.

In **each consumer**, remove only this crate's local override, update its version
pin when needed, and resolve the registry package:

```sh
cargo update -p document-graphics --precise X.Y.Z
cargo check --locked
```

Review the registry source and checksum in each Cargo.lock, rerun consumer
integration checks, and verify a clean checkout without local overrides. Commit
the updated lockfiles and remove initial-release setup notices. Keep the shared
library's version independent of application versions.
