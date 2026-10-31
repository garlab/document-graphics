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
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo +1.90.0 check --all-targets --all-features --locked
cargo package --list --locked
cargo publish --dry-run --locked
```

The dry run packages and verifies without uploading. Use a clean checkout and
review the archive's file list. Cargo.lock is tracked for reproducible repository
checks; consumers resolve transitive dependencies using their own lockfiles.
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

## Publication

The [release workflow](../.github/workflows/release.yml) publishes version tags
through crates.io's Trusted Publishing; the repository stores no registry token.
After review and consumer validation:

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
