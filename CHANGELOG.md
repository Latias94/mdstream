# Changelog

This project follows a pragmatic changelog format during early development.
Version numbers follow SemVer, but the public API is expected to change rapidly until `1.0`.

## Unreleased

- New: added `MdStreamBuilder` as a setup-time API for composing options,
  boundary plugins, and pending transformers before building an `MdStream`.
- Changed: split the stream append/finalize transaction engine out of the
  public `MdStream` facade and centralized Streamdown-compatible defaults
  through the builder path.
- Changed: centralized tag, fence-container, directive-container, and reference
  definition semantics behind crate-private modules shared by boundary plugins,
  analyzers, core invalidation, and the pulldown adapter.
- Fixed: custom tag block analysis now treats only standalone matching closing
  tag lines as closed, so trailing text after `</tag>` remains pending/open.
- Changed: made low-level `pending` and `syntax` modules internal; import
  `TerminatorOptions`, `terminate_markdown`, and syntax helpers from the crate
  root instead.
- New: added a Criterion benchmark harness and performance guide for core
  streaming hot paths.
- Changed: expanded CI and release checks around nextest, doc tests, examples,
  benchmark/fuzz compilation, packaging, and split MSRV validation.
- New: added property tests for generated Markdown-ish chunk boundaries and a
  standalone fuzz package for stream chunking and pending terminator hardening.
- Fixed: delayed incomplete table-delimiter candidates until newline so streaming
  chunk boundaries cannot split paragraphs on transient `--` prefixes.
- Fixed: removed avoidable production panic paths in committed block emission and
  sync pulldown scratch-buffer locking.
- Changed: clarified README, usage, architecture, compatibility, performance,
  fuzzing, and release-checklist guidance for the hardened workflow.
- Changed: upgraded direct dependency requirements to current releases:
  `pulldown-cmark` 0.13.4, `tokio` 1.52.3, `ratatui` 0.30.2,
  `crossterm` 0.29.0, and `unicode-width` 0.2.2.
- Changed: raised `mdstream-tokio` MSRV to Rust 1.88.0 to match
  `ratatui` 0.30.2.

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
