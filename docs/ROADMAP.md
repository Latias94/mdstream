# Roadmap

This roadmap is intentionally practical: it prioritizes streaming stability and compatibility with Streamdown + Incremark behaviors.

## v0.1 (MVP)

- Block stream model: `committed + pending`
- Stable boundary detection (core block-level constructs)
- Pending termination (remend-like)
- Minimal configuration options
- Unit tests covering streaming edge cases

## v0.2 (Adapters + Ergonomics)

- Optional `pulldown-cmark` adapter (feature-gated)
  - parse committed blocks once and cache events
  - parse pending block on each tick
- Add `snapshot_blocks()` and `snapshot_text()` convenience APIs
- Improve HTML block handling and table/list heuristics

## v0.3 (Cross-block semantics)

- Reference-style link definition tracking:
  - emit `invalidated` block IDs for adapters (opt-in mode)
- Footnote mode improvements:
  - default remains stability-first
  - optional invalidation-based strategy for advanced consumers

## v0.4+ (Extensions)

- Extension points for custom containers / directives
- More built-in analyzers for code fence info strings (mermaid, json, etc.)
- Performance benchmarks and regression suite

