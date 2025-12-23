# Extensions

This document describes how consumers can extend `mdstream` to support custom streaming behaviors and non-standard Markdown constructs.

## Extension Points

### 1) BoundaryPlugin

Purpose: participate in line-scoped context updates and stable boundary detection.

Use cases:

- custom containers (eg `:::warning`)
- application-specific blocks (eg `<thinking>...</thinking>`)
- language model tags

Guidelines:

- must be conservative: avoid committing too early
- must not mutate committed text

Status: implemented (MVP-level).

`mdstream` provides:

- `BoundaryPlugin` trait
- `MdStream::push_boundary_plugin(...)` and `MdStream::with_boundary_plugin(...)`
- `FenceBoundaryPlugin` as a small reference implementation (e.g. `:::warning ... :::`)
- `TagBoundaryPlugin` as another built-in example (e.g. `<thinking> ... </thinking>`)
- `ContainerBoundaryPlugin` for Incremark-compatible `::: name attr` containers (with nesting)

Notes on `:::` containers:

- If you want **Incremark parity** (`::: name attr` with nesting depth), use `ContainerBoundaryPlugin`.
  - It requires **whitespace after the marker** (so `:::warning` does not match).
  - It supports longer markers (e.g. `:::::`) and nested containers (each end closes one level).
- If you want a **fence-like block** that also matches `:::warning` (no whitespace), use
  `FenceBoundaryPlugin` instead.

Minimal example:

```rust
use mdstream::{ContainerBoundaryPlugin, FenceBoundaryPlugin, MdStream, Options, TagBoundaryPlugin};

let mut s = MdStream::new(Options::default());
s.push_boundary_plugin(FenceBoundaryPlugin::triple_colon());
s.push_boundary_plugin(TagBoundaryPlugin::thinking());
s.push_boundary_plugin(ContainerBoundaryPlugin::default());
```

### 2) PendingTransformer

Purpose: transform the pending block into a safer `display` string for downstream parsers/renderers.

Examples:

- remend-like termination for incomplete Markdown
- fenced JSON repair via `jsonrepair` (opt-in)
- custom placeholder replacement

Guidelines:

- operate on a tail window to keep cost bounded
- never change committed blocks

Status: implemented (MVP-level).

`mdstream` provides:

- `PendingTransformer` trait
- `MdStream::push_pending_transformer(...)` and `MdStream::with_pending_transformer(...)`
- Built-in transformers for Streamdown-compatible behavior:
  - `IncompleteLinkPlaceholderTransformer`
  - `IncompleteImageDropTransformer`

Minimal example:

```rust
use mdstream::{FnPendingTransformer, MdStream, Options};

let mut s = MdStream::new(Options::default());
// Append a marker so downstream parsers never see an empty string.
s.push_pending_transformer(FnPendingTransformer(|input| {
    if input.display.is_empty() { Some("<empty>".to_string()) } else { None }
}));
```

### 3) BlockAnalyzer

Purpose: extract metadata from blocks without changing text.

Examples:

- code fence info string extraction (`mermaid`, `json`, `python`, etc.)
- heuristics for “this block is likely incomplete”

Status: implemented (MVP-level).

`mdstream` provides:

- `BlockAnalyzer` trait
- `AnalyzedStream<A>` wrapper to run an analyzer on each `append()`/`finalize()`
- `CodeFenceAnalyzer` built-in analyzer that classifies code fences (e.g. `mermaid`, `json`)
- `MathAnalyzer` built-in analyzer that reports whether a `$$` math block is balanced
- `BlockHintAnalyzer` built-in analyzer that provides a small `likely_incomplete` hint for pending blocks

Minimal example:

```rust
use mdstream::{AnalyzedStream, CodeFenceAnalyzer, Options};

let mut s = AnalyzedStream::new(Options::default(), CodeFenceAnalyzer::default());
let u = s.append("```mermaid\ngraph TD;\nA-->B;\n");
assert!(u.pending_meta.is_some());
```

## Mermaid and Code Blocks

`mdstream` does not render Mermaid, but it should support it by:

- ensuring code fences are never split while unclosed (pending until closed)
- exposing the fence info string so UIs can dispatch to Mermaid renderers
- providing lightweight helpers:
  - `Block::code_fence_header()`
  - `Block::code_fence_language()`

## Philosophy

Extensions should not compromise the primary invariants:

- immutable committed blocks
- bounded per-chunk cost
- render-agnostic output
