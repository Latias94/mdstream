# Roadmap

This roadmap is intentionally practical: it prioritizes streaming stability and compatibility with Streamdown + Incremark behaviors.

## v0.1 (MVP)

- Block stream model: `committed + pending`
- Stable boundary detection (core block-level constructs)
- Pending termination (remend-like)
- Minimal configuration options
- MVP extension points:
  - `BoundaryPlugin` (custom containers/directives)
  - `PendingTransformer`
  - `BlockAnalyzer`
- Unit tests covering streaming edge cases
- Regression tests ported from Streamdown benchmarks (incrementally)
- Reference-style link definitions invalidation (opt-in mode)
- Optional `pulldown-cmark` adapter (feature-gated)

## v0.2 (Ergonomics + Robustness)

- Add `snapshot_blocks()` and `snapshot_text()` convenience APIs
- Improve HTML block handling and table/list heuristics
- More remend parity tests (regression suite)
- Streaming UX helpers (coalescing deltas; newline/time-window flush) (optional feature)
- Concurrency compatibility:
  - `sync` feature proposal for `Send + Sync` extension points
  - optional “actor” wrapper for cross-thread consumers

## v0.3 (Cross-block semantics)

- Footnote mode improvements:
  - default remains stability-first
  - optional invalidation-based strategy for advanced consumers

## v0.4+ (Performance)

- Performance benchmarks and regression suite
