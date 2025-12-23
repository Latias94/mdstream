# MVP

This document defines the MVP scope and acceptance criteria for `mdstream`.

## MVP Goals

1. Provide a stable block stream for streaming Markdown.
2. Ensure per-chunk processing is bounded and avoids O(n²).
3. Match practical streaming behavior of Streamdown + Incremark in edge cases.

## MVP Feature Set

### Block boundary detection (MUST)

- Headings (`#` ATX)
- Paragraph boundaries via blank lines
- Thematic breaks (`---`, `***`, `___`)
- Fenced code blocks (``` / ~~~)
- Blockquotes (`>`)
- Lists (ordered/unordered) with conservative nesting behavior
- Tables (GFM-style delimiter row)
- HTML blocks (conservative merge for multi-line blocks)
- `$$ ... $$` math blocks (treat unmatched `$$` as pending)
- Footnote definitions (`[^id]:`) with continuation indentation

### Pending transformation (MUST)

Remend-like termination for incomplete syntax near the tail (pending only):

- emphasis markers (`*`, `**`, `***`, `_`, `__`)
- inline code backticks
- strikethrough (`~~`)
- links/images:
  - incomplete link URL becomes `](streamdown:incomplete-link)` (configurable)
  - incomplete images are removed (configurable)
- setext heading protection (avoid misclassifying partial list markers as heading underlines)
- `$$` balancing for pending math blocks

### Optional (SHOULD)

- Fenced JSON repair inside ```json / ```jsonc / ```json5 (opt-in, using `jsonrepair`)
- `pulldown-cmark` adapter (feature-gated) for Rust UI/TUI ecosystems

## Behavioral Contracts (Acceptance)

### Incrementality

- When appending `k` chunks with total length `n`, total processing time should scale ~O(n), not O(n²).
- `append(chunk)` must not rescan or split the entire buffer when chunk does not contain newlines.

### Stability

- Once a block is emitted in `committed`, its `raw` text never changes.
- Only the `pending` block may change between ticks.

### Edge-case tests to include (MVP test list)

The following scenarios must be covered by unit tests (ported conceptually from Streamdown/Incremark):

- Incomplete emphasis at chunk boundary: `**bold` then ` text**`
- Incomplete inline code: `` `code`` then `` ` ``
- Incomplete link: `[text](` then `url)`; and partial `[text` then `](`
- Nested brackets in links: `[a [b] c](` split across chunks
- Images: `![alt](` split across chunks should not produce partial artifacts
- Fenced code blocks spanning multiple chunks; ensure no early commit inside fence
- `$$` math blocks across chunks; ensure pending until balanced
- Footnote definition continuation:
  - `[^1]: line1\n    line2\n` split across chunks
- HTML block spanning tokens: `<div>\n...` split; keep as one block until closed when possible
- HTML block closure without blank line: `<div>...\n</div>\nAfter` should not merge `After` into the HTML block
- Streamdown benchmark parity for HTML blocks:
  - simple multi-line HTML
  - nested HTML
  - multiple HTML blocks with markdown between
- Streamdown benchmark parity for tables:
  - simple table
  - large table (100 rows)
- Streamdown benchmark parity for footnotes:
  - document with footnotes
  - document with many footnotes
- Streamdown benchmark parity for streaming simulation:
  - streaming text (50 steps)
  - streaming code fence (9 steps)
- List + emphasis interaction edge cases (avoid mis-termination)
- Newline normalization:
  - CRLF split across chunks (`"\r"` then `"\n"`) must become a single `\n`
- Setext heading protection with indentation (`"  -"` / `"  ="`) should be protected consistently
- Lists with multiline content starting at `- **...` should not be auto-closed across newlines (remend parity)

## Out of Scope for MVP

- Full CommonMark conformance test suite
- Arbitrary plugin ecosystems beyond the initial extension points (see `docs/EXTENSIONS.md`)
- Advanced cross-block semantics invalidation beyond reference-style link definitions (post-MVP)
