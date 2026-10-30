# Repository conventions

- Treat all tracked content as public. Use generic examples and relative paths;
  exclude credentials, machine-specific paths and private environment details.
  Preserve the public copyright attribution and license notices.
- Keep this repository a UI-independent rendering library. Applications own
  document layout, interaction, queues, cache budgets and worker idle policies.
- Describe implemented behavior and platform limits accurately in README.md.
  Possible future formats must not appear as supported capabilities.
- Preserve resource limits, source fallbacks, cancellation and process cleanup.
  Expensive backend work must stay off consumer UI threads.
- Keep raw benchmarks, generated images and local dependency overrides ignored.
  Record only concise, reproducible performance summaries when needed.
- Run formatting, Clippy and relevant tests after code changes. Check the
  declared minimum Rust version and the packaged crate before a release.
- Keep renderer/protocol tests here and consumer-specific integration tests in
  their applications. See docs/releasing.md for release validation.
