# mdstream

`mdstream` is a **streaming-first Markdown middleware** for Rust.

It targets LLM token-by-token / chunk-by-chunk output and helps downstream UIs (egui, gpui/Zed, TUI, etc.) avoid the classic **O(n²)** re-parse + re-render pattern that causes latency and flicker.

## When to use

Use `mdstream` when you:

- Receive Markdown incrementally (LLM streaming) and want to avoid re-parsing the full document every tick.
- Need stable cache keys for UI rendering (blocks are immutable once committed).
- Want a render-agnostic “middleware” that can feed any renderer (Rust UI frameworks, terminal output, etc.).

You probably **don’t** need `mdstream` if you only parse static Markdown once, or if you already have a renderer that handles incremental updates internally.

## API at a glance

- `MdStream`: streaming block splitter (`append` / `finalize`) that produces `Update`.
- `Update`: `committed + pending` plus signals like `reset` and `invalidated`.
- `Block`: carries `id`, `kind`, `raw`, and optional `display` (pending-only).
- `DocumentState`: a UI-friendly container to apply `Update` safely (recommended).
- Optional adapter: `PulldownAdapter` behind the `pulldown` feature.

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
- Some scope-driven transitions require a full reset (e.g. switching into single-block footnote mode):
  - `append()` may return `Update { reset: true, .. }` to tell consumers to drop cached blocks.
- A **pending pipeline** can optionally produce a `display` view for the pending block:
  - Markdown terminator (remend-like) for incomplete constructs near the tail.
  - Custom transforms via `PendingTransformer` (eg placeholders, sanitizers).

## Installation

```toml
[dependencies]
mdstream = "0.1.0"
```

## Quick Start (UI Integration)

Recommended: keep UI state in `DocumentState` and apply each `Update` to it. This makes `reset`
handling hard to get wrong.

```rust
use mdstream::{DocumentState, MdStream, Options};

let mut stream = MdStream::new(Options::default());
let mut state = DocumentState::new();

let u = stream.append("# Title\n\nHello **wor");
let applied = state.apply(u);

if applied.reset {
    // Drop any external caches derived from old blocks.
}
// If you enable invalidation (see below), `applied.invalidated` tells you which committed blocks to refresh.

for b in state.committed() {
    // Render stable blocks once.
    let text = b.display_or_raw();
    let _ = text;
}
if let Some(p) = state.pending() {
    // Render/update the pending block each tick.
    let text = p.display_or_raw();
    let _ = text;
}
```

If you prefer to manage your own `(Vec<Block>, Option<Block>)`, you can apply updates with
`Update::apply_to`.

## Examples

```sh
cargo run --example minimal
cargo run --example footnotes_reset
cargo run --example tui_like
cargo run --features pulldown --example pulldown_incremental
```

## Optional: Reference Definitions Invalidation (Best-effort)

Markdown reference-style links/images can be defined *after* they are used:

- usage: `See [docs][ref].` or `See [ref].`
- definition (often later): `[ref]: https://example.com`

In streaming UIs that parse/render **each committed block independently**, late-arriving reference
definitions can require re-parsing earlier blocks so they turn into real links.

`mdstream` provides an **opt-in** invalidation signal for this:

- Enable: `opts.reference_definitions = ReferenceDefinitionsMode::Invalidate`
- When a reference definition is **committed**, `Update.invalidated` contains the `BlockId`s of
  previously committed blocks that likely used the label.
- Consumers/adapters can re-parse only those blocks instead of re-parsing the entire document.

This is intentionally **best-effort** (optimized for LLM streaming), not a full CommonMark/GFM
reference definition implementation:

- Only single-line definitions are recognized (`^[ ]{0,3}[label]: ...`), footnotes (`[^x]:`) are excluded.
- Label matching is normalized (trim, collapse whitespace, case-insensitive).
- Usage extraction over-approximates: false positives may cause extra invalidations; the goal is to
  avoid missing invalidations.
- Definitions inside fenced code blocks do not trigger invalidations.

Example:

```rust
use mdstream::{MdStream, Options, ReferenceDefinitionsMode};

let mut opts = Options::default();
opts.reference_definitions = ReferenceDefinitionsMode::Invalidate;

let mut s = MdStream::new(opts);
let u1 = s.append("See [ref].\n\n");
assert!(u1.committed.is_empty());

let u2 = s.append("[ref]: https://example.com\n\nNext\n");
assert!(u2.invalidated.contains(&mdstream::BlockId(1)));
```

## Optional: `pulldown-cmark` Adapter (`pulldown` feature)

`mdstream` is render-agnostic. If you want to reuse the Rust ecosystem around `pulldown-cmark`
(egui, gpui/Zed, TUI renderers), enable the adapter feature:

```toml
[dependencies]
mdstream = { version = "0.1.0", features = ["pulldown"] }
```

When `reference_definitions` invalidation is enabled, the adapter can re-parse only the invalidated
blocks:

```rust
use mdstream::adapters::pulldown::{PulldownAdapter, PulldownAdapterOptions};
use mdstream::{MdStream, Options, ReferenceDefinitionsMode};

let mut opts = Options::default();
opts.reference_definitions = ReferenceDefinitionsMode::Invalidate;

let mut stream = MdStream::new(opts);
let mut adapter = PulldownAdapter::new(PulldownAdapterOptions::default());

stream.append("See [ref].\n\n");
let u1 = stream.append("[ref]: https://example.com\n");
adapter.apply_update(&u1);

stream.append("\n");
let u2 = stream.append("Next\n");
adapter.apply_update(&u2);

// `u2.invalidated` tells you which committed blocks should be re-rendered.
```

## Release notes

- Release checklist: `RELEASE_CHECKLIST.md`
- Note: This project may prune `docs/` during releases; user-facing guidance lives in this README.

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
