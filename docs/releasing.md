# Release checks

Run commands from the document-graphics repository root. Cargo discovers local
configuration from the working directory; using `--manifest-path` from a
consumer checkout can apply that consumer's overrides to the release build.

## Review and validation

Review public API changes, source/raster limits, process cleanup and platform
guarantees. Backends should share rendering mechanisms while leaving application
scheduling and cache policies with consumers. Preserve license notices.

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo +1.90.0 check --all-targets --locked
cargo package --list --locked
cargo publish --dry-run --locked
```

The dry run packages and verifies without uploading. Use a clean checkout and
review the archive's file list. Cargo.lock is tracked for reproducible repository
checks; consumers resolve transitive dependencies using their own lockfiles.
The package includes library sources, tests, README, license and Cargo metadata.

For renderer changes, check both consumers with local dependency overrides:

- In mdviewer: run Rust tests and `tests/terminal_smoke.py` against a built
  binary; inspect graphics in a compatible terminal at normal and narrow widths.
- In Plastext: run the platform Mermaid worker and UI integration tests; inspect
  diagrams in both appearances, zoom, split panes, resizing and Source/Preview.
- Preserve readable source on failure. Check that navigation and closing views
  cancel obsolete work without leaving child processes behind.

Compare image output and feature costs when behavior changes. Keep generated
images and raw timing samples in ignored `target/` directories. A rename or
repository move alone should preserve renderer output.

## Manual publication

After review and consumer validation, publish from this repository:

```sh
cargo publish --locked
```

There is no automatic publishing workflow.

In **each consumer**, remove only this crate's local override, update its version
pin when needed, and resolve the registry package:

```sh
cargo update -p document-graphics --precise 0.1.0
cargo check --locked
```

Review the registry source and checksum in each Cargo.lock, rerun consumer
integration checks, and verify a clean checkout without local overrides. Commit
the updated lockfiles and remove initial-release setup notices. Keep the shared
library's version independent of application versions.
