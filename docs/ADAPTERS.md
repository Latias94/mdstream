# Adapters

`mdstream` is renderer-agnostic. Adapters allow downstream consumers to reuse existing ecosystems (eg pulldown-cmark) without sacrificing streaming performance.

## pulldown-cmark (optional)

### Why adapter instead of core dependency

- `mdstream` aims to support multiple render targets and parsers.
- pulldown-cmark is a great baseline for Rust, but binding core to it reduces flexibility.

### Recommended adapter design

- Parse **committed blocks** once, cache results by `BlockId`.
- Re-parse **pending block** on each update.
- Expose events per block instead of a single monolithic iterator:
  - `events_for_block(id) -> &[Event]`
  - `iter_committed_events() -> impl Iterator<Item = (BlockId, Event)>`

### Cross-block semantics

Reference-style links and footnotes can require earlier content to be interpreted differently after a new definition arrives.

Two possible behaviors:

1. **Stability-first (default)**: do not re-parse earlier blocks; interpretation may be delayed.
2. **Invalidate mode (opt-in)**: when a definition arrives, `mdstream` emits `invalidated` IDs; adapter re-parses those blocks and updates caches.

## Other adapters (future)

- `markdown-it` style token streams are out-of-scope for Rust, but a similar strategy applies.
- A TUI adapter can render directly from blocks without a full Markdown AST by using a lightweight inline parser (future exploration).

