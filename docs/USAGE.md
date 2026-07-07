# Usage

This document shows the recommended integration pattern for streaming UIs.

## The Core Pattern

Treat the incoming stream as:

- `committed`: stable blocks (append-only, never change)
- `pending`: the only block that can change per tick

UIs should:

1. Append new `committed` blocks to their view/model.
2. Replace/update the last rendered `pending` block (if present).
3. If `Update.reset` is true, drop cached blocks and rebuild from the new state.

## Basic Example

```rust
use mdstream::{MdStream, Options};

let mut s = MdStream::new(Options::default());

// streaming tick
let u = s.append("Hello **wor");
if u.reset {
    // Drop all previously cached blocks and restart rendering.
}
for b in u.committed {
    // render once
}
if let Some(p) = u.pending {
    // render/update pending (use p.display if you feed it into another Markdown parser)
}
```

## Setup-time Builder

Use `MdStreamBuilder` when a stream needs several extension points configured before runtime:

```rust
use mdstream::{
    ContainerBoundaryPlugin, IncompleteLinkPlaceholderTransformer, MdStream, Options,
};

let mut s = MdStream::builder(Options::default())
    .boundary_plugin(ContainerBoundaryPlugin::default())
    .pending_transformer(IncompleteLinkPlaceholderTransformer::default())
    .build();

let u = s.append("::: note\nHello [docs](");
let _ = u;
```

`MdStream::new`, `MdStream::streamdown_defaults`, `push_*`, and `with_*` remain available. The
builder only makes setup-heavy construction easier to read.

## Borrowed updates (`append_ref`)

If your UI owns the stream (common for TUIs), `append_ref` avoids cloning the pending tail on every
tick:

```rust
use mdstream::{MdStream, Options};

let mut s = MdStream::new(Options::default());

let u = s.append_ref("```rs\nfn main() {\n");
assert!(u.pending.is_some());

// Render only what changed:
for b in u.committed {
    let _ = b.id;
}
if let Some(p) = u.pending {
    let text = p.display_or_raw();
    let _ = text;
}
```

Notes:

- `UpdateRef` borrows from the stream. It is not suitable for sending across threads/tasks.
- If needed, use `UpdateRef::to_owned()` (allocating) or `append()` (owned update).

Use `append_ref` when the same thread owns `MdStream` and renders immediately from the returned
view. Use `append` when the update must be stored independently, sent across a channel, or owned by
another task. `UpdateRef::to_owned()` is the explicit bridge between those two modes.

| Situation | Recommended API |
| --- | --- |
| UI thread owns `MdStream` and renders each tick | `append_ref` / `finalize_ref` |
| Worker sends updates to another task | `append` / `finalize` |
| Borrowed hot path occasionally needs ownership | `UpdateRef::to_owned()` |
| Tests compare public behavior | Compare `append()` with `append_ref().to_owned()` |

## `DocumentState` (UI State Helper)

If you keep UI state as `(Vec<Block>, Option<Block>)`, you can use `Update::apply_to`. If you want a
dedicated container, use `DocumentState`:

```rust
use mdstream::{DocumentState, MdStream, Options};

let mut s = MdStream::new(Options::default());
let mut state = DocumentState::new();

let u = s.append("Hello **wor");
let applied = state.apply(u);
if applied.reset {
    // Drop any external caches derived from old blocks.
}
```

If your UI keeps state as `(Vec<Block>, Option<Block>)`, you can use `Update::apply_to` to avoid
getting `reset` handling wrong:

```rust
use mdstream::{MdStream, Options};

let mut s = MdStream::new(Options::default());
let mut committed = Vec::new();
let mut pending = None;

let u = s.append("Hello **wor");
u.apply_to(&mut committed, &mut pending);
```

## Analyzer Example (Metadata and Hints)

If you want block metadata (e.g. code fence language) and streaming hints (e.g. likely incomplete),
wrap the stream in `AnalyzedStream`.

```rust
use mdstream::{AnalyzedStream, BlockHintAnalyzer, CodeFenceAnalyzer, Options};

let analyzer = (CodeFenceAnalyzer::default(), BlockHintAnalyzer::default());
let mut s = AnalyzedStream::new(Options::default(), analyzer);

let u = s.append("```mermaid\ngraph TD;\nA-->B;\n");

for m in &u.committed_meta {
    // m.id is a stable cache key; m.meta contains analyzer output
}
if let Some(pm) = &u.pending_meta {
    // pending meta can change every tick, just like pending text
}
```

## Demo

Run the zero-dependency demo:

```sh
cargo run -p mdstream --example tui_like
```

## Validation workflow

For normal development, use nextest for integration and unit tests:

```sh
cargo nextest run --workspace --all-features
```

Doc tests, examples, benchmarks, and fuzz targets are separate gates:

```sh
cargo test --workspace --all-features --doc
cargo check -p mdstream --examples
cargo check -p mdstream --features pulldown --examples
cargo check -p mdstream-tokio --examples
cargo check -p mdstream --benches
cargo check --manifest-path fuzz/Cargo.toml --bins
cargo package -p mdstream
cargo package -p mdstream-tokio
```

## Streamdown Defaults

If you want Streamdown-compatible behavior for incomplete links/images via pending transformers:

```rust
use mdstream::MdStream;

let mut s = MdStream::streamdown_defaults();
```
