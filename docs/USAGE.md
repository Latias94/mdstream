# Usage

Start with the one-command [Rust minimal tutorial](EXAMPLES.md#rust-minimal), then use the [Web flagship](EXAMPLES.md#web-flagship) to inspect a real host presentation policy. The [example catalog](EXAMPLES.md) lists exact prerequisites, commands, expected observations, and next steps for Rust, Web, Dart, Flutter, Tokio, Merman, processors, recovery, and C.

## Rust producer and reducer

`StreamEngine` owns local incremental compilation. Apply every emitted change in order to the canonical reducer and update only identities named by `ChangeImpact`.

```rust
use mdstream::StreamEngine;
use mdstream_protocol::{ApplyOutcome, DocumentLifecycle, Reducer};

let mut engine = StreamEngine::new();
let mut reducer = Reducer::new();

for chunk in ["# Title\n\nHello ", "world"] {
    for change in engine.append(chunk)?.into_changes() {
        match reducer.apply(change)? {
            ApplyOutcome::Applied { impact, .. }
            | ApplyOutcome::Recovered { impact, .. } => {
                for node_id in impact.changed_nodes {
                    let _current = reducer
                        .document()
                        .and_then(|document| document.node(node_id));
                    // Refresh the keyed host view, or remove it when absent.
                }
            }
            other => return Err(format!("non-continuous producer output: {other:?}").into()),
        }
    }
}
for change in engine.finish()?.into_changes() {
    reducer.apply(change)?;
}

assert_eq!(
    reducer.document().map(|document| document.lifecycle()),
    Some(DocumentLifecycle::Finalized),
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`finish` is terminal and idempotent. `reset` starts a predecessor-linked epoch. Configuration such as `CustomBlockSpec` is sealed through `StreamEngine::builder()` before the first input; runtime grammar mutation is unavailable.

## Stable host state

Use `NodeId` as identity within one continuity generation and `NodeVersion` as the cache version. A full host key is `(continuity generation, epoch, NodeId)`. Source offsets and collection positions are not identities.

`changed_nodes` and `changed_resources` identify invalidated keys, including removed values. Query the current view for each key: a missing view means remove the host object. `removed_nodes` and `removed_resources` are the subsets known to be absent.

Pending source is a separate, bounded, on-demand view of `projection_cursor..source_cursor`. It is exact raw UTF-8 source that canonical Content IR has not covered yet. A host may paint it once for responsiveness, then remove that interval atomically when projection catches up; do not pace it again or parse it into competing Markdown semantics.

Run the [headless state recipe](EXAMPLES.md#stable-keyed-state) for targeted invalidation and reset cleanup.

## Transition facts and presentation

`ChangeImpact` is sufficient for latest-state cache invalidation. Enable transition capture only when the host needs renderer-neutral facts for projected append, replacement, stabilization, node/structure/resource changes, lifecycle, or full replacement.

Transition facts are ordered observations, not replayable document snapshots. A callback reads coherent batch-tail state; an intermediate `A -> B -> A` sequence preserves both facts even though the B view is no longer queryable after the operation. Legal chunk schedules may produce different intermediate batches while converging to the same canonical result.

Presentation remains application state. The host may reveal a fresh projection append immediately or queue graphemes, cross-fade a correction, animate layout after measuring its own geometry, and preserve scroll anchoring. Full replacement clears queued presentation and continuity-qualified keys. Immediate and reduced-motion modes must preserve the same content and accessible meaning, and motion or color cannot be the only signal for correction or removal.

Run the [Web flagship](EXAMPLES.md#web-flagship) for Immediate/Paced policy and the [transition trace](EXAMPLES.md#transition-trace) for deterministic schedule-local diagnostics.

## Replication and recovery

A remote consumer applies ordered `ChangeSet` values. On a gap, fork, or unannounced epoch, stop ordinary delivery to that reducer, obtain one current snapshot, call `recover_snapshot`, and resume with the next continuous change. Snapshots are explicit recovery state and are not emitted on ordinary append or finish.

A same-floor snapshot can restore readiness without changing eligible host identity. An advanced snapshot is a full-replacement barrier: clear old canonical views, processor artifacts, animation state, pending presentation, and continuity keys before rescanning current typed nodes.

Run the [replica recovery recipe](EXAMPLES.md#replica-recovery) for all three decisions: same-floor retention, advanced replacement, and reset/new epoch.

## Processors

Use `ArtifactHost` or a binding's processor scheduler after canonical reduction. A request owns an immutable typed node/resource input plus epoch, node and input versions, processor and configuration versions, and generation. Submit completion with the original key; stale generations, removed nodes, changed input, and reset epochs are rejected.

Artifacts remain derived host state and never enter Content IR or recovery snapshots. Run [processor lifecycle](EXAMPLES.md#processor-lifecycle) for generic freshness, [citation processor](EXAMPLES.md#citation-processor-contract) for resource-backed artifacts, and [Merman](EXAMPLES.md#merman-artifact) for a real `image/svg+xml` artifact.

Merman output is opaque and untrusted until a host-owned sanitizer or isolated renderer accepts it. Cooperative cancellation and source/model/output/retention limits are not compute or peak-memory isolation; adversarial processors require host-owned timeouts and worker/process controls.

## TypeScript and Web frameworks

`@mdstream/core` initializes the Rust/WASM runtime and provides an engine with a synchronized read-only `engine.store`. Use `runtime.createStore()` only for a replicated change stream that needs explicit snapshot recovery.

Root, node, resource, pending-source, and artifact views use focused external stores. `engine.store.pendingSource()` returns `undefined` when projection is current and otherwise materializes the exact bounded uncovered range only when read.

Enable `captureTransitions: true` with finite protocol limits and a sufficient `maxReducerUpdateBytes`, then subscribe with `engine.store.subscribeTransitions(...)`. The callback runs after the batch-tail state and invalidations are coherent and before ordinary store subscribers. Do not mutate or close the session from inside the callback.

Frameworks bind `subscribe` and `getSnapshot` to their native state primitive. React may use `useSyncExternalStore`; mdstream intentionally ships no React package, hook, renderer, component, animation dependency, or theme.

Use the [Web flagship](EXAMPLES.md#web-flagship) as visual adoption code and the [TypeScript transition probe](EXAMPLES.md#typescript-transition-probe) as machine-readable retention evidence.

## Dart and Flutter

The Dart `mdstream` package requires a trusted, compatible `mdstream-ffi` dynamic-library path. ABI, schema, and layout checks prove compatibility, not authenticity. Engine and reducer handles require explicit idempotent `close()`.

```dart
final runtime = MdstreamRuntime.openPath(nativeLibraryPath);
final engine = runtime.createEngine();
try {
  engine.append('# Title');
  engine.finish();
  final pending = engine.state.pendingSourceView();
} finally {
  engine.close();
}
```

Use `runtime.createReducer()` for replicas. `EngineResult.transitionFacts` and `ReducerResult.transitionFacts` preserve wire order when capture is enabled; focused current state remains lazy on `engine.state`.

`mdstream_flutter` bundles native libraries for Android, iOS, macOS, Linux, and Windows. Its producer and replica controllers implement `ValueListenable`, publish continuity-qualified `MdstreamNodeKey` values, and expose focused pending/node/resource/artifact/transition listenables. It deliberately exports no widgets, Markdown renderer, theme, animation policy, or default Merman binary.

Run the shared Golden scenario through [Dart headless](EXAMPLES.md#dart-headless) or the example-owned [Flutter host](EXAMPLES.md#flutter-host).

## Tokio

Use `spawn_stream_engine_actor` when a bounded asynchronous owner should serialize `ActorCommand` values and return fallible `ActorResult` change batches. Canonical input supports blocking or local coalescing, not intentional dropping. Await fallible policy changes and flush buffered input before sender disposal.

The [Tokio actor example](EXAMPLES.md#tokio-actor) provides a non-interactive `--smoke` path and an optional Ratatui host. Scrolling, follow-tail, and terminal layout are example-owned policies.

## AI message parts

mdstream owns one Markdown content session, not a provider or chat message envelope. The application owns message IDs, reasoning/tool/attachment parts, global ordering, persistence, and cross-part presentation timing.

Give each Markdown-capable part an independent session identified by a stable host part key plus a monotonically new host generation. Reordering a retained part preserves its session; replacement resets only that part; removal closes it and cancels processor work; key reuse allocates a new generation so callbacks from the retired part cannot attach.

## Migrating from 0.3

Replace `MdStream`/`Update`/`DocumentState` flows with `StreamEngine`, ordered `ChangeSet` values, and `Reducer`. Replace block positions with `NodeId`, cached block values with typed `ContentNode` plus `NodeVersion`, analyzers with processors, runtime parser mutation with setup-only custom blocks, and terminator/pending-transformer output with the bounded pending-source view plus host presentation policy.

Tokio users replace `spawn_mdstream_actor` with `spawn_stream_engine_actor` and consume `ActorResult`. Replace `DropNew` with `Block` or `CoalesceLocal`; flush any buffered canonical input before dropping a sender. No deprecated 0.3 aliases are available.
