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

## Completed in current 0.2 development

- `snapshot_blocks()` convenience API
- Improved HTML block handling and table/list heuristics
- Expanded remend parity tests and Streamdown/Incremark-inspired regression suites
- Tokio glue crate for coalescing deltas, sender backpressure policies, and actor helpers
- Optional `sync` feature for `Send + Sync` extension points
- Criterion benchmarks, performance guide, fuzz target compilation checks, and stronger CI/release gates

## Next: Cross-block semantics and 1.0 API shaping

- Broader public 1.0 API review across exported symbols
- More adapter-facing semantics around document-scoped constructs
- Scheduled fuzz campaigns and quantitative performance thresholds

## Later

- Additional renderer-specific adapters beyond the existing optional pulldown adapter
