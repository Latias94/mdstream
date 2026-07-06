# Compatibility & Edge Cases

This document tracks compatibility goals with:

- Streamdown (block splitting + termination behavior)
- Incremark (stable boundary detection + streaming edge cases)
- pulldown-cmark ecosystem (optional adapter)

## Scope Rule

`mdstream` follows a strict scope boundary:

- If Streamdown's `parseMarkdownIntoBlocks` supports a streaming-stability behavior at the block
  boundary level, `mdstream` must support it (with stable `committed + pending` semantics).
- If Streamdown does not support it (e.g. provider-specific tool calls, message parts, citations as
  business semantics), `mdstream` does not implement it in the core library.

Any behavior outside Streamdown parity must be explicitly agreed as project scope before adding.

## Streamdown parity checklist (block splitting)

Baseline: `repo-ref/streamdown/packages/streamdown/lib/parse-blocks.tsx`.

- Basic parsing: headings + paragraphs (covered: `tests/stream_streamdown_basic_parsing.rs`)
- Code fences: ``` / ~~~ (covered: `tests/stream_streamdown_code_blocks.rs`)
- Math blocks: `$$ ... $$` (covered: `tests/stream_block_splitting.rs`)
- HTML blocks: conservative merge with tag stack (covered: `tests/stream_streamdown_html_blocks.rs`)
- Footnotes: if any footnotes exist, return single block (covered: `tests/stream_block_splitting.rs`)
- Tables: GFM delimiter row (covered: `tests/stream_streamdown_tables.rs`)
- Streaming simulation (covered: `tests/stream_streamdown_simulation.rs`)
- Streaming parity (incremental vs full re-parse): (covered: `tests/stream_streamdown_streaming_simulation_parity.rs`)
- Mixed content scenario (covered: `tests/stream_streamdown_mixed_content.rs`)

### Streamdown benchmark coverage

The Streamdown benchmark suite `repo-ref/streamdown/packages/streamdown/__benchmarks__/parse-blocks.bench.ts`
is tracked by the following tests:

- `Basic Parsing` -> `tests/stream_streamdown_basic_parsing.rs`
- `Code Blocks` -> `tests/stream_streamdown_code_blocks.rs`
- `Math Blocks` -> `tests/stream_block_splitting.rs`
- `HTML Blocks` -> `tests/stream_streamdown_html_blocks.rs`
- `Footnotes` -> `tests/stream_block_splitting.rs`
- `Tables` -> `tests/stream_streamdown_tables.rs`
- `Streaming Simulation` -> `tests/stream_streamdown_simulation.rs` and `tests/stream_streamdown_streaming_simulation_parity.rs`
- `Mixed Content` -> `tests/stream_streamdown_mixed_content.rs`

## Known differences vs Streamdown (documented)

Streamdown's block splitting uses `marked` tokenization plus a small post-merge step
(`repo-ref/streamdown/packages/streamdown/lib/parse-blocks.tsx`). `mdstream` implements a
streaming-first line scanner. As a result, there are some intentional differences:

- HTML tag detection:
  - Streamdown uses regexes like `/<(\\w+)[\\s>]/` and `/<\\/(\\w+)>/` (tag name = `\\w+`).
  - `mdstream` recognizes ASCII tag names that start with a letter and continue with
    alphanumerics/`_`. This is closer to CommonMark-style HTML blocks and avoids many false positives
    in chat-like text, while still matching Streamdown's `\\w+` for most practical cases.
- HTML stack handling:
  - Streamdown only pops one closing tag per `html` token and does not attempt to parse multiple tags
    within one token.
  - `mdstream` scans each line and maintains a best-effort stack (multiple tags per line supported).
- HTML comments:
  - Streamdown behavior depends on `marked` tokenization (comments may or may not be merged depending
    on how tokens are produced).
  - `mdstream` treats multi-line `<!-- ... -->` as a single HTML block for stability.
- Tables/lists/blockquote nuances:
  - Streamdown relies on `marked`'s idea of tables/lists/quotes.
  - `mdstream` uses lightweight heuristics designed to be chunking-invariant; some non-benchmark edge
    cases may split differently.
  - Streaming-only stability tweaks exist to preserve chunking invariance (e.g. avoiding premature
    list commits when a list marker is split across chunks, or waiting for newline before treating an
    incomplete table delimiter candidate as stable).

These differences are acceptable as long as Streamdown benchmark parity tests and chunking invariance
tests remain green (see `tests/chunking_invariance_suite.rs`).

## Randomized and fuzz coverage

Deterministic compatibility tests are backed by randomized guards:

- `tests/proptest_chunking.rs` generates Markdown-ish documents with Unicode, CRLF, blank-line runs,
  fences, tables, HTML/comment fragments, math markers, footnotes, links, and reference definitions.
- The property checks compare final `(BlockKind, raw)` output for whole input, line chunks, character
  chunks, and pseudo-random char-boundary chunks.
- `fuzz/fuzz_targets/stream_chunking.rs` stresses whole vs chunked parsing and owned vs borrowed
  update equivalence.
- `fuzz/fuzz_targets/terminator.rs` stresses pending Markdown termination options.

The fuzz package is intentionally excluded from the default workspace; see `fuzz/README.md` for
nightly `cargo-fuzz` usage.

## Streaming edge cases (must handle)

### Incomplete inline constructs

- emphasis markers: `*`, `**`, `***`, `_`, `__`
- inline code: backticks
- strikethrough: `~~`
- links/images:
  - incomplete URL
  - incomplete link text with nested brackets

### Block constructs spanning chunks

- fenced code blocks
- blockquotes + lists (nested)
- HTML blocks
- tables
- math blocks with `$$`
- footnote definitions with continuation indentation

## Footnotes and reference definitions

These constructs are document-scoped and can force either:

- stability-first behavior (single block)
- invalidation behavior (selective re-parse in adapters)

The chosen default should prioritize streaming stability.

`mdstream` may emit `Update { reset: true, .. }` when entering SingleBlock footnote mode mid-stream so
consumers can drop cached blocks and rebuild (Streamdown parity behavior).

## Non-standard and ecosystem behaviors

### Incomplete link placeholder

Streamdown uses a special URL marker: `streamdown:incomplete-link`.

`mdstream` should:

- default to the same marker for compatibility
- allow configuring it

### Images

Streamdown removes incomplete images (because partial images cannot display meaningfully).

`mdstream` should support the same default behavior.
