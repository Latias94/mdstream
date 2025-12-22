# Compatibility & Edge Cases

This document tracks compatibility goals with:

- Streamdown (block splitting + termination behavior)
- Incremark (stable boundary detection + streaming edge cases)
- pulldown-cmark ecosystem (optional adapter)

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

## Non-standard and ecosystem behaviors

### Incomplete link placeholder

Streamdown uses a special URL marker: `streamdown:incomplete-link`.

`mdstream` should:

- default to the same marker for compatibility
- allow configuring it

### Images

Streamdown removes incomplete images (because partial images cannot display meaningfully).

`mdstream` should support the same default behavior.

