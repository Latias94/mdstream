# Changelog

This project follows a pragmatic changelog format during early development.
Version numbers follow SemVer, but the public API is expected to change rapidly until `1.0`.

## Unreleased

### Added

- Added a `builder_extensions` example showing how to configure boundary
  plugins and pending transformers with `MdStreamBuilder`.

## 0.3.0 - 2026-07-07

This release focuses on a cleaner public API, safer streaming edge cases, and
stronger release verification.

### Breaking Changes

- Low-level `mdstream::pending` and `mdstream::syntax` module paths are now
  internal. Import `TerminatorOptions`, `terminate_markdown`, and syntax helpers
  from the crate root instead.

### Added

- Added `MdStreamBuilder` for setup-heavy streams that register boundary plugins
  or pending transformers before runtime.
- Added benchmark, property-test, and fuzzing coverage for streaming hot paths
  and chunk-boundary robustness.

### Fixed

- Fixed custom tag analysis for non-standalone closing tags such as
  `</tag> trailing`.
- Fixed custom tag analysis so code-indented tag-like lines are not treated as
  application tags.
- Fixed incomplete table delimiter handling across streaming chunk boundaries.
- Removed avoidable production panic paths, including sync pulldown
  scratch-buffer recovery.

### Dependency Updates

- Upgraded direct dependency requirements: `pulldown-cmark` 0.13.4,
  `tokio` 1.52.3, `ratatui` 0.30.2, `crossterm` 0.29.0, and
  `unicode-width` 0.2.2.
- Raised `mdstream-tokio` MSRV to Rust 1.88.0. `mdstream` remains Rust 1.85+.

### Changed

- Centralized Streamdown-compatible defaults plus internal container/reference
  handling so plugins, analyzers, core invalidation, and the pulldown adapter
  share one interpretation.
- Expanded CI and release checks for nextest, doc tests, examples,
  benchmark/fuzz compilation, packaging, and split MSRV validation.

## 0.2.0

Highlights:
- Bugfix: code fence opening line no longer closes the fence immediately (thanks @omgpointless, #1).
- New: opt-in `sync` feature to require `Send + Sync` for `PendingTransformer` and `BoundaryPlugin`.
- New: `mdstream-tokio` crate (newline/time-window delta coalescing + an actor helper for owned `Update`s).
- New: `agent_tui` example (`cargo run -p mdstream-tokio --example agent_tui`) showing a Codex/Gemini-CLI style streaming UI:
  channel-fed updates, follow-tail, and pending code fence truncation to reduce flicker.
- Performance: borrowed update API (`append_ref` / `finalize_ref`) and faster pending code fence display updates for large blocks.
- Highlight: improved streaming smoothness for large code fences (fewer allocations + safer pending display).

## 0.1.0

Initial experimental release.

Highlights:
- Streaming-first block splitter: stable committed blocks + a single pending block.
- Pending terminator (Streamdown/remend-inspired) to reduce flicker from incomplete Markdown.
- Render-agnostic core designed for UI integrations (egui, gpui/Zed, TUI, etc.).
- Optional `pulldown-cmark` adapter (`pulldown` feature) with best-effort invalidation support.
- Custom boundary plugins (tag containers, `:::` containers) inspired by Streamdown + Incremark.
- Memory guardrail: optional buffer compaction via `Options.max_buffer_bytes`.
