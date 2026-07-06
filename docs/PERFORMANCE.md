# Performance

`mdstream` is designed for streaming Markdown UIs where each incoming chunk should update only the new text and the current pending block.

The benchmark suite exists to catch accidental regressions in the hot paths that matter for LLM-style streams:

- many small blocks
- large tables
- large code fences
- whole-buffer, line, character, and pseudo-random chunking
- owned updates through `append`
- borrowed updates through `append_ref`

## Running Benchmarks

Run the core benchmark target with:

```bash
cargo bench -p mdstream --bench streaming
```

For a quick compile-only check, use:

```bash
cargo check -p mdstream --benches
```

Criterion reports throughput and statistical summaries under `target/criterion/`.
Treat those numbers as machine-local baselines unless a future CI job records stable historical data on dedicated hardware.

## Interpreting Results

The benchmarks compare public API paths rather than private modules.
That keeps the results aligned with user-visible behavior and lets internal modules keep changing.

Use the scenarios this way:

- `append_owned` shows the cost for users who need owned `Update` values.
- `append_borrowed` shows the intended hot path for UI threads that own `MdStream`.
- `large_code_fence_*` catches pending-display cache regressions.
- `large_table_*` catches expensive table and line-scanning changes.
- `*_chars` and `*_random_chunks` catch chunk-boundary overhead.

The normal CI gate should compile the benchmark target, but it should not fail pull requests on Criterion timing variance from shared GitHub runners.
Use full benchmark runs locally before and after performance-sensitive refactors.
