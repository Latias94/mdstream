# Changelog

This project follows a pragmatic changelog format during early development.
Version numbers follow SemVer, but the public API is expected to change rapidly until `1.0`.

## Unreleased

Version 0.4 turns mdstream into a headless, cross-framework streaming rich-content state engine with one replayable Rust state contract.

### Added

- Added `mdstream-protocol` and the lifecycle-aware `StreamEngine`, which produce ordered, replayable `ChangeSet` batches for one canonical `Reducer` with typed Content IR, deterministic `NodeId`/`NodeVersion` identity, explicit snapshot recovery, semantic correction, and chunk-schedule conformance.
- Added `mdstream-processors` with version-checked requests, cancellation, stale-result rejection, hard artifact limits, `mdstream.citation/1`, and the optional standalone `mdstream-merman` adapter; processor artifacts remain outside canonical document state, and the Merman adapter requires Rust 1.95.
- Added Rust-backed WASM and framework-neutral `@mdstream/core` stores for Node 24 tooling with lossless batching, focused node/resource/artifact views, bounded on-demand pending source, explicit recovery, and processor scheduling; mdstream ships no first-party React package, hooks, renderer, or theme.
- Added opt-in `mdstream.transitions/1` facts across Rust, WASM/TypeScript, C FFI, Dart, and Flutter. Hosts can distinguish projected text append, correction, stability, structure/resource changes, lifecycle, and full replacement while keeping pacing, animation, layout, scrolling, and accessibility policy outside mdstream.
- Added a stable C ABI, a Flutter-independent Dart package using a host-supplied native library, and the turnkey `mdstream_flutter` plugin with bundled Android, iOS, macOS, Linux, and Windows libraries plus focused state notifications without widgets or rendering policy.
- Added compile-tested headless, egui, GPUI, and Tokio integration examples that demonstrate stable-key invalidation without adding renderer or UI-framework dependencies to the core.
- Added shared replay fixtures, deterministic resource/work limits, cross-runtime conformance, exact release-archive verification, and absolute WASM/npm/Dart/Flutter artifact ceilings.

### Breaking Changes

- Removed the complete 0.3 block/update/analyzer surface, including `MdStream`, `MdStreamBuilder`, `Options`, `Block`, `BlockStatus`, `Update`, `UpdateRef`, `PendingBlockRef`, `DocumentState`, `AnalyzedStream`, and `BlockAnalyzer`, without deprecated aliases.
- Removed runtime boundary plugins, pending transformers, mutable committed/cache access, pending-repair and public syntax helpers, the Pulldown adapter, and the `pulldown`/`sync` Cargo features; `pulldown-cmark` is now an internal, non-optional compiler dependency.
- Replaced `mdstream-tokio::spawn_mdstream_actor` and owned `Update` output with `spawn_stream_engine_actor`, `ActorCommand`, `StreamEngineActor`, and fallible `ActorResult` change-set batches.
- Removed lossy canonical-input behavior from `mdstream-tokio`: `BackpressurePolicy::DropNew` and `SendOutcome::Dropped` no longer exist, `DeltaSender::set_policy` is now async and fallible, and buffered senders must be flushed before they are dropped.
- Renamed the unreleased 0.4 binding wire limit `max_impact_bytes` to `max_reducer_update_bytes`; the old spelling is rejected rather than aliased.

### Migration

Add a direct `mdstream-protocol = "0.4"` dependency wherever the application owns canonical state, and remove `features = ["pulldown", "sync"]` from the `mdstream` dependency declaration.

| 0.3 surface | 0.4 action |
| --- | --- |
| `MdStream` / `MdStreamBuilder` | `StreamEngine` / `StreamEngineBuilder` |
| `append` / `finalize` | Call `StreamEngine::append` / `finish` and handle `Result<EngineOutput, EngineError>`; `finish` is terminal and `reset` starts a new epoch. |
| `Update` / `UpdateRef` / `DocumentState` | Apply every ordered `EngineOutput::into_changes()` item through `mdstream_protocol::Reducer` and invalidate only identities reported by `ChangeImpact`. |
| `Block` / `BlockStatus` / collection position keys | Use typed `ContentNode`, `NodeStability`, stable `NodeId`, and cache-validating `NodeVersion`. |
| `AnalyzedStream` / `BlockAnalyzer` | Consume typed Content IR directly or run a versioned `mdstream-processors` processor and keep its artifact as derived state. |
| `BoundaryPlugin` / runtime grammar mutation | Register setup-only `CustomBlockSpec` values through `StreamEngine::builder()` before accepting input. |
| `TerminatorOptions` / `terminate_markdown` / pending transformers | Read bounded pending source on demand and keep incomplete-Markdown display repair in host rendering policy. |
| `spawn_mdstream_actor` | Send `ActorCommand` values to `spawn_stream_engine_actor`, receive `ActorResult` batches through `StreamEngineActor::recv`, and use `join` to drain unread output. |
| `BackpressurePolicy::DropNew` / `SendOutcome::Dropped` | Use `BackpressurePolicy::Block` or `BackpressurePolicy::CoalesceLocal` for canonical input and place replaceable status signals on a separate lossy channel. |

Await `DeltaSender::set_policy(...)`, handle its `SendError`, and call `flush().await` before dropping a sender after any `SendOutcome::Buffered` result.

Consumers that tested an unreleased 0.4 binding checkout must rename `wire.max_impact_bytes` to `wire.max_reducer_update_bytes`. Transition capture is optional and requires a finite protocol profile whose worst legal update fits that bound.

## 0.3.0 - 2026-07-07

This release focuses on a cleaner public API, safer streaming edge cases, and
stronger release verification.

### Breaking Changes

- Low-level `mdstream::pending` and `mdstream::syntax` module paths are now
  internal. Import `TerminatorOptions`, `terminate_markdown`, and syntax helpers
  from the crate root instead.

### Added

- Added `MdStreamBuilder` for setup-heavy streams that register boundary plugins
  or pending transformers before runtime.
- Added benchmark, property-test, and fuzzing coverage for streaming hot paths
  and chunk-boundary robustness.

### Fixed

- Fixed custom tag analysis for non-standalone closing tags such as
  `</tag> trailing`.
- Fixed custom tag analysis so code-indented tag-like lines are not treated as
  application tags.
- Fixed incomplete table delimiter handling across streaming chunk boundaries.
- Removed avoidable production panic paths, including sync pulldown
  scratch-buffer recovery.

### Changed

- Centralized Streamdown-compatible defaults plus internal container/reference
  handling so plugins, analyzers, core invalidation, and the pulldown adapter
  share one interpretation.
- Expanded CI and release checks for nextest, doc tests, examples,
  benchmark/fuzz compilation, packaging, and split MSRV validation.
- Upgraded direct dependency requirements: `pulldown-cmark` 0.13.4,
  `tokio` 1.52.3, `ratatui` 0.30.2, `crossterm` 0.29.0, and
  `unicode-width` 0.2.2.
- Raised `mdstream-tokio` MSRV to Rust 1.88.0. `mdstream` remains Rust 1.85+.

## 0.2.0

Highlights:
- Bugfix: code fence opening line no longer closes the fence immediately (thanks @omgpointless, #1).
- New: opt-in `sync` feature to require `Send + Sync` for `PendingTransformer` and `BoundaryPlugin`.
- New: `mdstream-tokio` crate (newline/time-window delta coalescing + an actor helper for owned `Update`s).
- New: `agent_tui` example (`cargo run -p mdstream-tokio --example agent_tui`) showing a Codex/Gemini-CLI style streaming UI:
  channel-fed updates, follow-tail, and pending code fence truncation to reduce flicker.
- Performance: borrowed update API (`append_ref` / `finalize_ref`) and faster pending code fence display updates for large blocks.
- Highlight: improved streaming smoothness for large code fences (fewer allocations + safer pending display).

## 0.1.0

Initial experimental release.

Highlights:
- Streaming-first block splitter: stable committed blocks + a single pending block.
- Pending terminator (Streamdown/remend-inspired) to reduce flicker from incomplete Markdown.
- Render-agnostic core designed for UI integrations (egui, gpui/Zed, TUI, etc.).
- Optional `pulldown-cmark` adapter (`pulldown` feature) with best-effort invalidation support.
- Custom boundary plugins (tag containers, `:::` containers) inspired by Streamdown + Incremark.
- Memory guardrail: optional buffer compaction via `Options.max_buffer_bytes`.
