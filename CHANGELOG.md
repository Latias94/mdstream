# Changelog

This project follows a pragmatic changelog format during early development.
Version numbers follow SemVer, but the public API is expected to change rapidly until `1.0`.

## Unreleased

### Breaking Changes

- **`PendingTransformer` and `BoundaryPlugin` traits now require `Sync`** in addition to `Send`.
  This enables `MdStream` to be used with reactive frameworks like Leptos that require `Send + Sync`.

  **Migration**: Closures with mutable captures must use thread-safe primitives:
  ```rust
  // Before (no longer compiles)
  let mut seen = 0usize;
  FnPendingTransformer(move |input| { seen += 1; ... })

  // After
  let seen = Arc::new(AtomicUsize::new(0));
  let seen_clone = Arc::clone(&seen);
  FnPendingTransformer(move |input| {
      let count = seen_clone.fetch_add(1, Ordering::Relaxed) + 1;
      ...
  })
  ```

- `FnBoundaryPlugin` closure bounds now require `+ Sync` for `start`, `update`, and `reset` closures.

- `PulldownAdapter` internals simplified (removed `RefCell` scratch buffer) for `Sync` compatibility.

## 0.1.0

Initial experimental release.

Highlights:
- Streaming-first block splitter: stable committed blocks + a single pending block.
- Pending terminator (Streamdown/remend-inspired) to reduce flicker from incomplete Markdown.
- Render-agnostic core designed for UI integrations (egui, gpui/Zed, TUI, etc.).
- Optional `pulldown-cmark` adapter (`pulldown` feature) with best-effort invalidation support.
- Custom boundary plugins (tag containers, `:::` containers) inspired by Streamdown + Incremark.
- Memory guardrail: optional buffer compaction via `Options.max_buffer_bytes`.

