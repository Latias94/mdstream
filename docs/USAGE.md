# Usage

This document shows the recommended integration pattern for streaming UIs.

## The Core Pattern

Treat the incoming stream as:

- `committed`: stable blocks (append-only, never change)
- `pending`: the only block that can change per tick

UIs should:

1. Append new `committed` blocks to their view/model.
2. Replace/update the last rendered `pending` block (if present).

## Basic Example

```rust
use mdstream::{MdStream, Options};

let mut s = MdStream::new(Options::default());

// streaming tick
let u = s.append("Hello **wor");
for b in u.committed {
    // render once
}
if let Some(p) = u.pending {
    // render/update pending (use p.display if you feed it into another Markdown parser)
}
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
cargo run --example tui_like
```

## Streamdown Defaults

If you want Streamdown-compatible behavior for incomplete links/images via pending transformers:

```rust
use mdstream::MdStream;

let mut s = MdStream::streamdown_defaults();
```
