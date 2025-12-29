# ADR 0001: Streaming Ownership, Concurrency, and Optional `sync` / async Glue

## Status

Proposed

## Context

`mdstream` is designed for token-by-token / chunk-by-chunk Markdown streams (LLM output).
In real UIs, the hot path is not parsing once, but updating *very frequently*:

- If consumers re-parse and re-render the whole document on each tick, they hit the classic **O(n²)**
  latency and visible flicker.
- UI frameworks often have a single “UI thread” or render loop. Background tasks (network/model)
  should not mutate UI state directly.
- Some ecosystems require “state objects” to be `Send + Sync`, which can tempt library authors to
  force `Send + Sync` bounds on everything.

However, the core `mdstream` type (`MdStream`) is an in-memory state machine that is mutated through
`&mut self`. Making extension traits `Send + Sync` does not automatically make streaming safe or
useful to call from multiple threads concurrently.

We want an API that is:

- **Ergonomic by default** for UI use (especially TUI / agent CLIs).
- **Fast** (avoid per-tick allocations for the pending tail).
- **Composable** with async runtimes without forcing an async dependency.
- **Optionally compatible** with `Send + Sync` constraints via feature flags.

## Decision

### 1) Single-owner model (default)

The recommended architecture is:

- The **UI thread owns `MdStream`** (and typically a `DocumentState`).
- Background tasks send deltas to the UI over a **channel**.
- The UI applies deltas with `append_ref()` (borrowed update) or `append()` (owned update).

This keeps the API simple and matches common “agent CLI” streaming practices (coalescing + stable
UI updates).

### 2) Borrowed vs owned updates

- `append_ref()` / `finalize_ref()` return `UpdateRef`, which borrows from the stream and avoids
  cloning large pending buffers. This is the preferred UI hot path.
- `append()` / `finalize()` return `Update` (owned). Use this when updates must cross thread/task
  boundaries.
- `UpdateRef::to_owned()` is an explicit escape hatch (allocates) when a borrowed update needs to be
  queued or sent elsewhere.

### 3) Async is a “feeding strategy”, not a core requirement

`mdstream` stays runtime-agnostic. Instead of making the core async, we provide a recommended
pattern (and optional glue) for coalescing and sending deltas into the UI thread.

### 4) Optional `sync` feature (proposal)

We propose a `sync` Cargo feature that makes extension points compatible with `Send + Sync` state
containers:

- Under `sync`, traits like `PendingTransformer` / `BoundaryPlugin` require `Send + Sync`.
- Default build keeps the current ergonomic, single-thread model without forcing `Send + Sync`.

This balances ergonomics (default) and compatibility (opt-in).

## Consequences

- Pros:
  - Default usage is simple and matches real UI architectures.
  - High-frequency updates avoid unnecessary allocations (`append_ref`).
  - Async users can integrate cleanly via channels.
  - `Send + Sync` requirements do not break existing transformer/plugin implementations unless the
    user opts in.
- Cons:
  - Some frameworks may prefer “everything `Send + Sync` by default”. They will need to enable the
    `sync` feature (once implemented) or use an actor wrapper (see future work).

## Recommended UI Pattern (TUI / agent CLI)

In practice, updating on every token is still too frequent. Coalesce deltas before applying them:

- Flush on newline (`\n`) when possible (newline-gated).
- Add a small time-based fallback (e.g. 30–100ms) to avoid “no progress” when the model streams long
  lines without newlines.

This reduces flicker and stabilizes scrolling.

## Future Work

### A) `sync` feature (compatibility)

- Add `sync` feature with `Send + Sync` bounds on extension traits.
- Provide a migration guide for common patterns (interior mutability, shared caches).

### B) Optional async glue (ergonomics)

Provide optional helpers behind `tokio` (or `async`) features:

- `Coalescer` utilities (newline-gated + time-window flush).
- An **actor** wrapper that owns `MdStream` on a dedicated task/thread and accepts deltas over a
  channel, emitting owned `Update` for consumers that cannot keep the stream on the UI thread.

### C) Pending rendering policies

Expose enough pending metadata to let UIs choose better pending rendering, e.g.:

- For large pending code fences: render only the last N lines + a “generating…” indicator.
- For tables/lists in-progress: avoid premature reflow that causes jumping.

### D) Performance and regression testing

- Add benchmarks for long pending blocks (especially code fences).
- Keep a regression suite of streaming edge cases (chunk boundaries, fence markers, nested lists).

