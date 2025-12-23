# mdstream

`mdstream` is a **streaming-first Markdown middleware** for Rust.

It targets LLM token-by-token / chunk-by-chunk output and helps downstream UIs (egui, gpui/Zed, TUI, etc.) avoid the classic **O(n²)** re-parse + re-render pattern that causes latency and flicker.

## Goals

- **Fix the O(n²) problem**: only the *new* part should be processed on each chunk.
- **Render-agnostic**: no rendering; provide stable, incremental building blocks that any UI can consume.
- **Handle the dirty work**: deal with chunk boundaries and incomplete Markdown so downstream parsers/renderers behave predictably.
- **Match capabilities of Streamdown + Incremark** in streaming Markdown handling and edge-case coverage.

## Non-goals

- Not a Markdown renderer.
- Not a full CommonMark/GFM conformance test suite (we prioritize streaming stability and practical compatibility).
- Not a hard dependency on a specific parser (pulldown-cmark integration is optional).

## Core Model (high level)

- The input stream is represented as a sequence of **blocks**:
  - **Committed blocks**: stable, never change again (safe for UI to cache by `BlockId`).
  - A single **pending block**: may change while streaming (UI updates only this block).
- A **pending pipeline** can optionally produce a `display` view for the pending block:
  - Markdown terminator (remend-like) for incomplete constructs near the tail.
  - Optional fenced-JSON repair via `jsonrepair` (explicit opt-in).

## Documentation

- Architecture: `docs/ARCHITECTURE.md`
- MVP definition & acceptance tests: `docs/MVP.md`
- Roadmap: `docs/ROADMAP.md`
- Usage (integration patterns): `docs/USAGE.md`
- Compatibility & edge cases: `docs/COMPATIBILITY.md`
- Adapters (pulldown-cmark, etc.): `docs/ADAPTERS.md`
- Extension points: `docs/EXTENSIONS.md`

## Status

Initial MVP implementation is in progress:

- `MdStream` core state machine (blocks: committed + pending)
- Pending terminator (Streamdown/remend-inspired)
- Streaming boundary tests (Streamdown/Incremark-inspired)
- Reference-style link definitions invalidation (opt-in, for adapters)
- Optional `pulldown-cmark` adapter via the `pulldown` feature

Try the demo:

`cargo run --example tui_like`

Try the `pulldown-cmark` incremental demo:

`cargo run --features pulldown --example pulldown_incremental`
