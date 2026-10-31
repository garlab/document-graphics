# Mermaid backend delta

`crates/mermaid-renderer` contains the source of [mermaid-rs-renderer 0.3.1](https://crates.io/crates/mermaid-rs-renderer/0.3.1) with the upstream MIT license. The upstream crate archive SHA-256 is `6cb5b469222ce6b9896edf0a969cf68464f5a028ef394fd6643f00bd943df117`.

[mermaid-backend.patch](mermaid-backend.patch) records every source difference. The custom Cargo manifest gives the fork the registry name `document-graphics-mermaid`; CLI/PNG remain off in the consumer. No consumer-level patch is required.

Changes:

- Add `set_font_database` to use caller-supplied embedded fonts, clear stale metrics, and disable system-font discovery plus filesystem cache reads/writes. Existing upstream callers keep their default behavior until opting in.
- Render every ER key badge instead of dropping keys after the second.
- Keep timeline text and journey scores readable against their fixed pastel fills; use the theme's text color for quadrant axes and retain/escape all lines of vertical axis labels.
- Apply rustfmt to the copied source so workspace formatting checks stay reproducible.

The patch is prepared for upstream review; it has not been submitted upstream. Keep behavior changes isolated and remove the fork once an upstream release provides equivalent APIs and fixes. Shared-library tests exercise the new behavior, including the real macOS sandbox (where filesystem reads are denied), glyph fallback and record preservation. Clippy's `--no-deps` option checks maintained adapter code without promoting unrelated inherited backend lint warnings to errors.

To update, obtain the reviewed upstream archive, preserve its license, reapply the intentional changes, and regenerate the patch against that exact archive. Review all generated SVG/pixel changes and repeat consumer and package checks in [the release guide](../docs/releasing.md).
