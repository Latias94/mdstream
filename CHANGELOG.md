# Changelog

This project follows a pragmatic changelog format during early development.
Version numbers follow SemVer, but the public API is expected to change rapidly until `1.0`.

## Unreleased

Version 0.4 rebuilds mdstream as a headless, cross-framework streaming rich-content state engine so arbitrary AI token chunking converges to one recoverable Content IR across Rust, Web, and native UI hosts instead of making every renderer reparse or repair incomplete Markdown. It intentionally breaks the 0.3 block/update API.

### Added

- Added `mdstream-protocol` and rebuilt `mdstream` around lifecycle-aware `StreamEngine` output and one canonical `Reducer`: ordered `ChangeSet` deltas now produce typed Content IR, deterministic `NodeId` identity, projection-local `NodeVersion` tokens, bounded on-demand pending source, semantic correction, explicit snapshot recovery, finalized state that is invariant across legal UTF-8 chunk schedules, and separate protocol, compiler, and engine limit planes.
- Added opt-in `mdstream.transitions/1` facts across Rust, WASM/TypeScript, C FFI, Dart, and Flutter so hosts can distinguish fresh projection append, correction, stabilization, structure/resource changes, lifecycle, and full replacement while keeping pacing, animation, layout, scrolling, reduced motion, and accessibility in application code.
- Added `mdstream-processors` with complete node-local `ProcessorInputVersion` freshness, versioned requests, cooperative cancellation, stale-result rejection, deterministic artifact limits, and `mdstream.citation/1`; the optional standalone Rust 1.95 `mdstream-merman` adapter turns typed Mermaid nodes into opaque derived SVG artifacts without adding Merman to default dependency graphs, as shown by the [Merman artifact recipe](docs/EXAMPLES.md#merman-artifact).
- Added Rust-backed WASM and framework-neutral `@mdstream/core` for Node 24 with synchronized engine stores, replica recovery, lossless batching, lazy focused root/node/resource/pending/artifact views, immutable binary artifact snapshots through `ImmutableBytesView`, ordered transition subscriptions, and processor scheduling; no first-party React package, hook, renderer, animation policy, or theme is included.
- Added a stable C ABI, a Flutter-independent Dart package with typed session-limit groups and sealed reducer/artifact views using a trusted host-supplied native library, and the turnkey `mdstream_flutter` plugin with Android, iOS 14+, macOS, Linux, and Windows native delivery plus focused controllers and continuity-qualified keys without exported widgets or rendering policy.
- Added one provider-free Golden AI Stream and a [runnable adoption ladder](docs/EXAMPLES.md): the [Rust minimal tutorial](docs/EXAMPLES.md#rust-minimal) introduces the canonical engine/reducer loop, the [framework-neutral Web flagship](docs/EXAMPLES.md#web-flagship) is the primary visual step, and the [Tokio rich workbench](docs/EXAMPLES.md#tokio-rich-workbench) demonstrates correction-aware pacing, Tree-sitter styling, scrolling, reduced motion, and user-defined animation without promoting UI policy into package APIs; Dart, Flutter, processor, custom-block, transition, recovery, and Merman entries provide additional assertion or smoke paths.
- Added shared replay and recovery fixtures, exhaustive bounded and adversarial chunk checks, deterministic work/resource budgets, cross-runtime conformance, Cargo package-inventory checks, exact npm/Dart/Flutter archive verification, native binary and forbidden-path checks, and absolute artifact-size ceilings.
- Added native-reported effective processor scheduler limits across WASM and C FFI so Web, Dart, and Flutter adapters share the validated Rust configuration. Web and Flutter dispatch in bounded event-loop quanta and coalesce queue-saturation errors so large processor sets do not starve host rendering.

### Breaking Changes

- Removed the complete 0.3 block/update/analyzer surface, including `MdStream`, `MdStreamBuilder`, `Options`, `Block`, `BlockStatus`, `Update`, `UpdateRef`, `PendingBlockRef`, `DocumentState`, `AnalyzedStream`, and `BlockAnalyzer`, without deprecated aliases.
- Removed runtime boundary plugins, pending transformers, mutable committed/cache access, pending-repair and public syntax helpers, the Pulldown adapter, and the `pulldown`/`sync` Cargo features; `pulldown-cmark` is now an internal, non-optional compiler dependency.
- Replaced `mdstream-tokio::spawn_mdstream_actor` and owned `Update` output with `spawn_stream_engine_actor`, `ActorCommand`, `ActorBatch`, `StreamEngineActor`, and owned `ActorExit` completion, failure, or cancellation state. Engine failures terminate intake and return the engine, completed constituent results, unresolved chunks, unexecuted commands, and the closed command receiver instead of crossing a barrier. Borrowed `join` and `cancel` waits can be cancelled and retried without losing those ownership planes.
- Removed lossy canonical-input behavior from `mdstream-tokio`: `BackpressurePolicy::DropNew` and `SendOutcome::Dropped` no longer exist, `DeltaSender::set_policy` is now async and fallible, and buffered senders must be flushed or recovered with `take_pending` before they are dropped.
- Replaced public-field `CoalesceOptions` literals with bounded constructors and modifiers, removed `CoalescePreset`, added a hard `max_pending_chunks` boundary budget, and made receiver/sender statistics report deterministic input, scan, copy, pending-byte, pending-constituent, and logical boundary-record work.
- Made `DeltaSender::new` require explicit local byte and constituent limits; threshold-crossing input remains caller-owned until prior pending data flushes, and a closed-channel error never accepts the new borrowed delta.
- Replaced optional single-result TypeScript and Dart batching with engine-owned exclusive leases, ordered result collections, byte plus constituent bounds, explicit pending inspection/retry/transfer/discard, and composite partial-failure evidence. Direct engine mutation remains blocked until an empty batcher is explicitly released.

#### Tokio semantic-join value gate

The reproducible Rust 1.88 workload evaluator selected constituent-first canonical appends inside one atomic actor publication. Joined-first reduced append attempts and encoded result bytes, but copied every source byte while constituent-first copied none, so joined-first failed the per-workload 20% copy-work ceiling and was deleted from the production actor path. Both candidates produced equal final source, lifecycle, stable Content IR, and resources; deterministic scan work was equal between candidates.

| Workload | Joined attempts / encoded bytes / copy bytes | Constituent attempts / encoded bytes / copy bytes | Shared scan bytes | Decision |
| --- | ---: | ---: | ---: | --- |
| One-byte | 1 / 3,867 / 35 | 35 / 28,351 / 0 | 35 | Constituent-first |
| Bursty | 1 / 3,871 / 45 | 5 / 5,548 / 0 | 44 | Constituent-first |
| Unicode | 1 / 2,637 / 22 | 5 / 6,421 / 0 | 22 | Constituent-first |
| CRLF | 1 / 5,465 / 19 | 5 / 5,908 / 0 | 17 | Constituent-first |
| Golden AI Stream | 1 / 7,810 / 372 | 9 / 13,444 / 0 | 113 | Constituent-first |

#### TypeScript and Dart semantic-join value gates

The reproducible TypeScript and Dart evaluators independently selected constituent-first canonical appends inside one coherent host operation. Both bindings produced the same deterministic measurements and equal final source, lifecycle, roots, nodes, resources, and stable Content IR. Joined-first improved append attempts and encoded result bytes, but copied every source byte while constituent-first copied none, so it failed the per-workload 20% copy-work ceiling and was deleted from both production paths.

| Workload | Joined attempts / encoded bytes / scan bytes / copy bytes | Constituent attempts / encoded bytes / scan bytes / copy bytes | TypeScript decision | Dart decision |
| --- | ---: | ---: | --- | --- |
| One-byte | 1 / 5,693 / 35 / 35 | 35 / 55,320 / 35 / 0 | Constituent-first | Constituent-first |
| Bursty | 1 / 5,695 / 45 / 45 | 5 / 10,289 / 45 / 0 | Constituent-first | Constituent-first |
| Unicode | 1 / 4,341 / 22 / 22 | 5 / 11,136 / 22 / 0 | Constituent-first | Constituent-first |
| CRLF | 1 / 7,503 / 19 / 19 | 5 / 10,541 / 19 / 0 | Constituent-first | Constituent-first |
| Golden AI Stream | 1 / 10,016 / 372 / 372 | 9 / 22,114 / 372 / 0 | Constituent-first | Constituent-first |

#### 0.4 version-freeze evidence

On 2026-07-23, `python3 scripts/check-registry-version.py audit-workspace 0.4.0 --root . --remote origin` completed with the repository's release-network configuration. The package contract and all registry and tag probes were definitive:

| Evidence | Result |
| --- | --- |
| `mdstream-protocol`, `mdstream-processors`, `mdstream`, `mdstream-bindings-core`, `mdstream-tokio`, `mdstream-ffi`, `mdstream-wasm`, and `mdstream-merman` on crates.io | Missing |
| `@mdstream/core` on npm; `mdstream` and `mdstream_flutter` on pub.dev | Missing |
| Local and `origin` tags `0.4.0` and `v0.4.0` | Missing |

Decision: retain `0.4.0`; no published package or tag requires a further version advance before freezing the 0.4 schemas, fixtures, and migration table.

#### 0.4 pre-release batching migration

| Removed or changed 0.4 pre-release surface | Replacement |
| --- | --- |
| TypeScript `engine.createBatcher(maxBatchBytes)` | Call `engine.createBatcher({ maxBatchBytes, maxPendingChunks })`. The returned public interface cannot be constructed independently of its engine lease. |
| Dart public generic `LosslessInputBatcher<Result>` | Call `engine.createBatcher(maxBatchBytes: ..., maxPendingChunks: ...)`; the package-internal queue cannot bypass engine ownership. |
| Nullable `flush()` / lifecycle result | Consume the ordered result collection returned by `push`, `flush`, `retryPending`, `finish`, `reset`, and batched recovery. Empty work returns an empty collection. |
| Implicit batch abandonment during finish, reset, recovery, or close | Resolve retained input with `retryPending()`, `takePending()`, or explicit `discardPending()`, then call `release()`. Direct engine mutation remains rejected for the full lease lifetime. |
| `BatchOperationError(completedResults, cause)` and the equivalent Dart exception | Read `completedResults`, `cause`, `operation`, immutable `pending`, and push-only `newInputAccepted` before choosing the next ownership action. |
| `BatchMetrics.inputChunks`, `forwardedBytes`, `batchCount`, and append-call aliases | Use the aligned input/append attempt, successful append, committed/pending byte, pending constituent, boundary metadata, scan/copy/replay, output byte, and published-result counters. |

### Migration

Add a direct `mdstream-protocol = "0.4"` dependency wherever the application owns canonical state, remove `features = ["pulldown", "sync"]` from the `mdstream` dependency declaration, and plan a source migration because no 0.3 compatibility aliases are provided.

Use the [Rust minimal tutorial](docs/EXAMPLES.md#rust-minimal) for the new engine/reducer loop, the [stable keyed-state recipe](docs/EXAMPLES.md#stable-keyed-state) for identity-driven cache invalidation, the [Tokio actor example](docs/EXAMPLES.md#tokio-actor) for async ownership, and the [Tokio rich workbench](docs/EXAMPLES.md#tokio-rich-workbench) for host-owned animation and Tree-sitter integration; the [complete learning path](docs/EXAMPLES.md) covers Web, Dart, Flutter, processors, recovery, and Merman.

| 0.3 surface | 0.4 action |
| --- | --- |
| `MdStream` / `MdStreamBuilder` | `StreamEngine` / `StreamEngineBuilder` |
| `Options` (`footnotes`, `reference_definitions`, `terminator`, `terminator_window_bytes`, `max_buffer_bytes`) | Remove the old parsing modes: footnotes and reference definitions now use canonical semantic correction; pending repair and its display window belong to the host. Replace the old buffer-cap intent with independently owned `ProtocolLimits::max_source_bytes`, `CompilerLimits`, and `EngineLimits`; these limits reject atomically rather than compacting canonical source. |
| `append` / `finalize` | Call `StreamEngine::append` / `finish` and handle `Result<EngineOutput, EngineError>`; `finish` is terminal and `reset` starts a new epoch. |
| `Update` / `UpdateRef` / `DocumentState` | Apply every ordered `EngineOutput::into_changes()` item through `mdstream_protocol::Reducer` and invalidate only identities reported by `ChangeImpact`. |
| `Block` / `BlockStatus` / collection position keys | Use typed `ContentNode`, `NodeStability`, and stable `NodeId`; invalidate complete cached nodes through `ChangeImpact.changed_nodes`, use `NodeVersion` for projection compare-and-set, and compare `children.version` for direct child topology. |
| `AnalyzedStream` / `BlockAnalyzer` | Consume typed Content IR directly or run a versioned `mdstream-processors` processor and keep its artifact as derived state. |
| `BoundaryPlugin` / runtime grammar mutation | Register setup-only `CustomBlockSpec` values through `StreamEngine::builder()` before accepting input. |
| `TerminatorOptions` / `terminate_markdown` / pending transformers | Read bounded pending source on demand and keep incomplete-Markdown display repair in host rendering policy. |
| `spawn_mdstream_actor` | Send `ActorCommand` values to `spawn_stream_engine_actor`, receive committed `ActorBatch` values through `StreamEngineActor::recv`, and await borrowed `join` or `cancel` to obtain `ActorJoinOutcome { unread, exit }`. Handle `ActorExit::Failed` or `Cancelled` explicitly before retrying or discarding returned input. |
| `ActorResult` / `close_output` | Consume success-only `ActorBatch` output. Engine errors live only in terminal `ActorExit::Failed`; call `begin_cancel` for synchronous cancellation initiation or await the retryable borrowed `cancel` operation. |
| `CoalesceOptions { ... }` | Use `CoalesceOptions::new(max_delay, max_bytes, max_pending_chunks)` and `with_newline_flush`, `with_max_delay`, `with_max_bytes`, or `with_max_pending_chunks`. |
| `CoalescePreset` | Delete the preset and construct the exact `CoalesceOptions` policy the application owns. |
| `DeltaSender::new(sender, policy)` / mutable local-limit setters | Call `DeltaSender::new(sender, policy, max_bytes, max_pending_chunks)` once. On a closed-channel error, recover previously accepted constituents with `take_pending`; the borrowed delta was not accepted. |
| `BackpressurePolicy::DropNew` / `SendOutcome::Dropped` | Use `BackpressurePolicy::Block` or `BackpressurePolicy::CoalesceLocal` for canonical input and place replaceable status signals on a separate lossy channel. |

Await `DeltaSender::set_policy(...)`, handle its `SendError`, and call `flush().await` or `take_pending()` before dropping a sender after any `SendOutcome::Buffered` result.

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
