# Mermaid backend

This is a narrowly patched copy of [mermaid-rs-renderer 0.3.1](https://crates.io/crates/mermaid-rs-renderer/0.3.1), maintained for document-graphics. The upstream MIT license and attribution are preserved in LICENSE.

The patch also preserves all ER key badges and fixes chart label contrast and multiline axis labels. The added `set_font_database` API lets a caller supply embedded fonts. Once configured, text measurement uses only that database: no system-font discovery or filesystem font cache. Existing callers retain upstream behavior until they opt in. Set the database before rendering; configuration is process-wide.

The source changes are recorded in `https://github.com/garlab/document-graphics/blob/main/patches/mermaid-backend.patch` for upstream review. Keep this delta small and remove the fork when an upstream release exposes the required API. The package has its own name so a registry release can be used without consumer-level Cargo patches.
