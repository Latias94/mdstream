# Architecture

This document defines the architecture and contracts of `mdstream`.

## Why `mdstream`

In LLM streaming scenarios, the naive approach is:

1. Append chunk to the accumulated Markdown string.
2. Re-parse the full string.
3. Re-render the full UI tree.

This produces the familiar **O(n²)** behavior and causes visible flicker.

`mdstream` fixes this by modeling the stream as:

- `committed` blocks: stable, never change
- `pending` block: the only piece that can change between ticks

Downstream UIs can:

- append new committed blocks to their view
- update only the last pending block

## Core Principles

1. **Committed blocks are immutable**: once committed, a block's `raw` text never changes.
2. **Incremental updates are local**: each `append()` call must be ~O(len(chunk)) and should not rescan the full history.
3. **Render-agnostic output**: the library outputs blocks and optional metadata; it does not render.
4. **Streaming-friendly incomplete handling**: the pending block can be transformed into a `display` string so downstream parsers don’t choke on incomplete syntax.

## Public Types (conceptual)

### BlockId

- A monotonically increasing identifier for stable caching in UIs.

### BlockStatus

- `Committed`
- `Pending`

### BlockKind (hint)

Block kind is a **best-effort hint** for UIs and adapters; it is not a strict grammar guarantee.

Typical variants:

- `Paragraph`, `Heading`, `List`, `BlockQuote`, `CodeFence`, `HtmlBlock`, `Table`, `ThematicBreak`
- `MathBlock` (`$$ ... $$`)
- `FootnoteDefinition`
- `Unknown`

### Block

- `id: BlockId`
- `status: BlockStatus`
- `kind: BlockKind`
- `raw: String` (always present)
- `display: Option<String>` (only for `Pending`, optional)

### Update

- `committed: Vec<Block>`: new stable blocks emitted in this update
- `pending: Option<Block>`: the current pending block (if any)
- `invalidated: Vec<BlockId>`: optional list of previously committed blocks that should be re-parsed by adapters (see below)

`invalidated` exists to support cross-block semantics without breaking the “committed text is immutable” rule.

## State Machine Overview

Internally we maintain:

- `buffer`: accumulated text (optionally capped)
- `line index`: incremental line splitting to avoid re-splitting the whole buffer
- `context`: line-scoped context (code fence state, container state, list/blockquote depth, etc.)
- `pending_start`: where the current pending block begins
- `next_block_id`

### Stable boundary detection

The stable boundary detector scans only new lines and advances a “stable boundary” when the previous block can no longer change.

Key contexts (inspired by Incremark):

- fenced code blocks: keep pending until closing fence arrives
- containers (optional): keep pending until container end marker arrives
- footnote definitions: handle continuation indentation
- block quotes & lists: conservative boundary rules to avoid splitting nested structures

### Streaming transforms (pending pipeline)

The pending pipeline runs only on the pending block and produces `display`.

Default design is inspired by Streamdown `remend` but implemented in Rust:

- only scans a tail window (eg 16KiB) to keep per-tick cost bounded
- never modifies committed text

Optional transformer:

- fenced JSON repair using `jsonrepair` (opt-in, only inside ```json fences)

## Cross-block Semantics Strategy

Some Markdown constructs are inherently document-scoped:

- footnote references/definitions
- reference-style link definitions (`[id]: url`)

`mdstream` supports two strategies:

1. **SingleBlock**: if footnotes are detected, treat the whole document as a single block (Streamdown-like).
2. **Invalidate**: keep blocks, but when a new definition arrives, emit `invalidated` IDs so adapters can selectively re-parse (Incremark-like).

The default can prioritize streaming stability (SingleBlock for footnotes) while still allowing advanced consumers to opt into invalidation.

## Invariants

- `committed` blocks are append-only, stable, and never re-emitted with changed text.
- At most one `pending` block exists at a time.
- `append()` does not allocate proportional to total history (no full re-parse).

