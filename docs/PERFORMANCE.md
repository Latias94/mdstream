# Performance and Resource Contracts

mdstream treats deterministic work and memory accounting as correctness
constraints. Criterion benchmarks are trend evidence; they do not replace
operation counters and hard limits.

## Streaming Work

Accepted source is emitted as suffix deltas. Normal append does not serialize a
full pending snapshot. Compiler metrics separate framing, Markdown projection,
reconciliation, semantic correction, and structural work. Reducer metrics
separate operation visits, source growth, staging, and replay.

Source and projection cursors may diverge when completing a typed frontier
would require unbounded reparse. Geometric checkpoints and structural
boundaries keep eventual compilation bounded. Finalization requires complete
projection coverage. See [ADR 0002](ADR_0002_PROJECTION_FRONTIER.md).

## Hard Limits

Typed errors enforce limits for document/pending bytes, nodes, operations,
definition edges, encoded commands/changes/views, processor input, in-flight
jobs, pending artifact changes, and retained artifacts. Admission is
transactional: a rejected operation preserves the last replayable coordinate
and document.

Canonical input transport is lossless. Tokio may block or coalesce continuous
changes but cannot use a drop-new content policy.

## Frozen Evidence

`conformance/budgets/streaming.json` records deterministic calibration and
minimal transport measurements. `bindings/budgets.json` records absolute WASM,
npm, Dart, Flutter native-library, and per-platform package ceilings plus
advisory regression bands. Absolute ceilings always win over a relative
baseline.

The checked package policy forbids Merman, React, Streamdown, and Incremark from
default artifact dependency graphs. Optional Merman size and render cost are
measured on its standalone Rust 1.95 lane.

## Reproducing Checks

```sh
scripts/calibrate-budgets.sh --check
python3 scripts/verify-budgets.py --contracts
python3 scripts/verify-budgets.py --negative-merman
pnpm artifacts:check
python3 bindings/flutter/tool/package_smoke.py --skip-native-build --skip-runtime
cargo check -p mdstream --benches --all-features
```

For performance investigations, compare deterministic counters first, retained
and transactional memory second, and wall-clock benchmarks last. A faster
result that violates replay, source preservation, or a hard ceiling is not an
optimization.
