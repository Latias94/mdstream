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
- newline normalization: accept `\n`, `\r\n`, and legacy `\r` and normalize to `\n` (including CRLF split across chunk boundaries)
- `line index`: incremental line splitting to avoid re-splitting the whole buffer
- `context`: line-scoped context (code fence state, container state, list/blockquote depth, etc.)
- `pending_start`: where the current pending block begins
- `next_block_id`

### Internal module map

The public entry point is still `MdStream`, but the implementation is split by responsibility:

- `stream.rs`: public facade, setup helpers, pending snapshots, and reset coordination.
- `stream/builder.rs`: setup-time `MdStreamBuilder` for composing options and extensions.
- `stream/engine.rs`: append/finalize transaction ordering, `Update`/`UpdateRef` assembly,
  reset effects, and buffer compaction handoff.
- `stream/input.rs`: `LineBuffer`, newline normalization, line indexing, and compaction-safe buffer rebuilding.
- `stream/block_machine.rs`: committed block cursors, pending block start, `BlockId` allocation, and mode ownership.
- `stream/boundary_detector.rs`: stable boundary decisions for block starts, continuations, and custom plugins.
- `stream/mode.rs`: block-mode state and `BlockKind` mapping.
- `stream/machine.rs`: line-level mode transitions and committed block emission.
- `syntax/containers.rs`: crate-private tag, fence-container, and directive-container syntax facts
  shared by boundary plugins and analyzers.
- `pending/pipeline.rs`: pending display cache, code-fence suffix fast path, terminator calls, and pending transformer chain.
- `pending/repair/*`: pending Markdown repair helpers for links, inline spans, emphasis, setext headings, and terminator context.
- `reference.rs`: reference-label normalization, definition scanning, usage indexing, and pulldown
  definition prelude state.
- `semantics/mod.rs`: document-scoped effects coordinator.
- `semantics/{footnotes,references}.rs`: focused footnote reset policy and reference invalidation
  integration over the internal reference index.
- `extensions/*`: internal registries for boundary plugins and pending transformers.
- `mdstream-tokio/src/{sender,receiver,actor,options}.rs`: Tokio feeding strategy around the runtime-agnostic core.

### Stable boundary detection

The stable boundary detector scans only new lines and advances a “stable boundary” when the previous block can no longer change.

Key contexts (inspired by Incremark):

- fenced code blocks: keep pending until closing fence arrives
- containers (optional): keep pending until container end marker arrives
- footnote definitions: handle continuation indentation
- block quotes & lists: conservative boundary rules to avoid splitting nested structures
- HTML blocks: tag-stack based closure (best-effort) to avoid merging following paragraphs
- table/thematic/setext candidates: wait for newline when an incomplete line could still be
  invalidated by later chunk bytes

### Streaming transforms (pending pipeline)

The pending pipeline runs only on the pending block and produces `display`.

Default design is inspired by Streamdown `remend` but implemented in Rust:

- only scans a tail window (eg 16KiB) to keep per-tick cost bounded
- never modifies committed text
- owns the display cache so `append()` and `append_ref()` share the same pending-display behavior

Note: `mdstream` does not include domain-specific transforms (eg tool-call JSON repair).
Consumers can implement them via `PendingTransformer` when needed.

## Cross-block Semantics Strategy

Some Markdown constructs are inherently document-scoped:

- footnote references/definitions
- reference-style link definitions (`[id]: url`)

`mdstream` supports two strategies:

1. **SingleBlock**: if footnotes are detected, treat the whole document as a single block (Streamdown-like).
2. **Invalidate**: keep blocks, but when a new definition arrives, emit `invalidated` IDs so adapters can selectively re-parse (Incremark-like).

The default can prioritize streaming stability (SingleBlock for footnotes) while still allowing advanced consumers to opt into invalidation.

Today, invalidation events are implemented for reference-style link definitions. `FootnotesMode::Invalidate`
keeps footnote definitions as regular streaming blocks instead of forcing a whole-document reset.

Internally, document-scoped behavior is coordinated by `DocumentSemantics`, with focused footnote
and reference effect modules behind it. Reference handling is intentionally deeper than the semantics
wrapper: `reference.rs` owns the shared label normalization, definition extraction, committed-usage
index, and pulldown definition prelude state. This keeps block boundaries separate from effects that
may invalidate earlier committed blocks or require a full reset, while preventing the core and adapter
from drifting on reference-definition rules.

### Footnote definition boundary rules

When not in SingleBlock mode, `mdstream` tracks footnote definitions as their own block kind (`FootnoteDefinition`).
For streaming stability (and to match Incremark-style incremental boundaries), the block ends when:

- a blank line is followed by a non-indented line
- a non-indented, non-empty line arrives (no blank line required)
- a new footnote definition starts (`[^id]:`)

## Reset semantics

Some transitions cannot be expressed as "append-only committed blocks" without breaking Streamdown parity.
In such cases, `mdstream` emits `Update { reset: true, .. }` so consumers can drop cached blocks and
rebuild from the current state. The primary example is switching into SingleBlock footnote mode when
`[^id]` / `[^id]:` is detected mid-stream.

## Invariants

- `committed` blocks are append-only, stable, and never re-emitted with changed text.
- At most one `pending` block exists at a time.
- `append()` does not allocate proportional to total history (no full re-parse).
- `append()` and `append_ref().to_owned()` must produce equivalent public updates.
- Production code should avoid panic-based invariants; tests may intentionally panic only to
  exercise recovery paths such as poisoned mutex handling.

## Engineering guardrails

The repo has three complementary hardening layers:

- deterministic tests and property tests under `mdstream/tests/`, including generated
  Markdown-ish chunking invariance cases;
- Criterion benchmarks in `mdstream/benches/streaming.rs`, focused on public hot paths rather than
  private modules;
- standalone fuzz targets under `fuzz/`, kept outside the default workspace so normal `cargo test
  --workspace` remains deterministic.

CI compiles benchmarks and runs deterministic gates. Full Criterion measurements and long
`cargo-fuzz` sessions are local or scheduled hardening workflows, not per-PR timing requirements.
