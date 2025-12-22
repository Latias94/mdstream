# Extensions

This document describes how consumers can extend `mdstream` to support custom streaming behaviors and non-standard Markdown constructs.

## Extension Points (proposed)

### 1) BoundaryPlugin

Purpose: participate in line-scoped context updates and stable boundary detection.

Use cases:

- custom containers (eg `:::warning`)
- application-specific blocks (eg `<thinking>...</thinking>`)
- language model tags

Guidelines:

- must be conservative: avoid committing too early
- must not mutate committed text

### 2) PendingTransformer

Purpose: transform the pending block into a safer `display` string for downstream parsers/renderers.

Examples:

- remend-like termination for incomplete Markdown
- fenced JSON repair via `jsonrepair` (opt-in)
- custom placeholder replacement

Guidelines:

- operate on a tail window to keep cost bounded
- never change committed blocks

### 3) BlockAnalyzer

Purpose: extract metadata from blocks without changing text.

Examples:

- code fence info string extraction (`mermaid`, `json`, `python`, etc.)
- heuristics for “this block is likely incomplete”

## Mermaid and Code Blocks

`mdstream` does not render Mermaid, but it should support it by:

- ensuring code fences are never split while unclosed (pending until closed)
- exposing the fence info string so UIs can dispatch to Mermaid renderers

## Philosophy

Extensions should not compromise the primary invariants:

- immutable committed blocks
- bounded per-chunk cost
- render-agnostic output

