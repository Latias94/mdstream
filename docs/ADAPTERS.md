# Adapter Contracts

mdstream adapters project canonical state into a host UI or language runtime.
They do not parse Markdown, reduce protocol operations independently, or store
processor artifacts inside Content IR.

```text
token chunks
    -> StreamEngine
    -> ChangeSet
    -> canonical Reducer
    -> changed keys and typed views
    -> host UI state

typed ContentNode
    -> optional processor
    -> ArtifactHost
    -> artifact view keyed by node and processor
```

## Stable State

- Within one continuity generation, `NodeId` is the stable identity. Across a
  full replacement, use `(continuity generation, epoch, NodeId)` as the UI key.
  Source offsets and collection positions are never keys.
- `NodeVersion` invalidates a cached node view and processor input.
- `changed_nodes` is the set of invalidated node keys. It includes removed
  nodes; `removed_nodes` is the subset that no longer has a view. Resource
  impacts follow the same rule.
- Consumers materialize only invalidated views. A missing view removes the
  corresponding host object.
- Pending source is a separate, on-demand view of
  `projection_cursor..source_cursor`. It is invalidated by source or projection
  changes, carries exact UTF-8 byte cursors, and is never embedded in every
  reducer update or parsed by an adapter.
- `full_replace` invalidates all retained canonical and derived host views,
  including state from a prior epoch.

## Transition Observations

`ChangeImpact` remains the cache-invalidation surface. Hosts that need to
classify visible change can opt into a separate atomic transition feed. Facts
carry continuity-qualified keys, before/after stamps, exact projection-append
text, normalized child-list splices, resource corrections, lifecycle changes,
and coarse full-replace barriers. They do not carry timing, easing, geometry,
colors, scrolling policy, or framework component metadata.

Language adapters publish an operation batch only after the batch-tail state
and cache invalidations are coherent, and before ordinary invalidation
listeners. Facts inside a batch stay ordered and are not deduplicated. The
current view is only the tail view; an intermediate `A -> B -> A` fact sequence
does not make an intermediate B view queryable. Different legal chunk schedules
may produce different intermediate batches while converging to the same final
canonical state.

Capture is disabled by default. With capture enabled, TypeScript exposes
`store.subscribeTransitions` and Flutter exposes a revisioned `transitions`
listenable. A no-op, failed, artifact-only, or same-floor operation publishes a
new empty enabled-capture batch; capture-disabled sessions publish nothing.
Mutation during a transition callback is rejected, while reads and synchronous
listener disposal remain safe. See
[ADR 0005](ADR_0005_HOST_TRANSITION_FACTS.md).

## Recovery

Ordered changes are the normal transport. If a consumer receives a gap, fork,
or unannounced epoch, the Rust reducer enters `NeedsSnapshot` and rejects later
ordinary changes. The host obtains one explicit snapshot, applies it atomically,
then resumes with the next continuous change. Snapshots are not emitted during
normal append or finish operations.

## Rust

Native consumers use `StreamEngine` and `mdstream_protocol::Reducer` directly.
The `headless_state` example shows changed-key projection and a separate
`ArtifactHost` without a UI-framework dependency:

```sh
cargo run -p mdstream --example headless_state
```

GPUI, egui, TUI, and other native frameworks should keep their view cache above
this reducer boundary. They do not need an mdstream-specific renderer.

## TypeScript and WASM

`@mdstream/core` is the complete first-party web state surface. Its engine owns
the producer and its synchronized Rust reducer. `engine.store` is a read-only
external-store facade with root, pending-source, node, resource, and artifact
subscriptions.
`runtime.createStore()` is the separate mutable reducer surface for replicated
change streams and explicit recovery.

Framework integrations bind `subscribe` and `getSnapshot` to their native state
primitive. React can use `useSyncExternalStore`; mdstream intentionally ships no
React hooks, components, renderer, or theme. See
[ADR 0004](ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md).

## Dart and Flutter

The Dart package wraps the C ABI and requires a host-supplied native-library
path. It exposes the same Rust engine/reducer handles, typed views, lossless
batching, and explicit recovery without depending on Flutter.

`mdstream_flutter` adds native library delivery and no-path loading for Android,
iOS, macOS, Linux, and Windows. Its producer and replica controllers implement
`ValueListenable`, publish continuity-qualified node keys, and expose focused
pending-source/node/resource/artifact listenables. Processor scheduling
snapshots registration identity and rejects late generations. The package
contains no widgets, renderer, theme, or default Merman binary.

When transition capture is enabled, Dart keeps ordered facts on reducer results
and Flutter publishes a `MdstreamTransitionBatch` before ordinary invalidation
listeners. Flutter node keys include a controller-local continuity generation,
so same-epoch advanced recovery cannot retain stale widget identity even when
capture is disabled.

## Processors and Merman

Processors consume typed node and resource context after reduction. Results are
accepted only while epoch, node ID, input version, processor/configuration
versions, and request generation remain current. Artifacts are derived state
and never appear in canonical snapshots.

`mdstream-merman` is an optional standalone adapter on its own Rust toolchain
lane. The default Rust, WASM, TypeScript, Dart, and Flutter dependency graphs do
not contain Merman. Applications may process the typed Mermaid code-block node
with Merman and render its SVG artifact without changing Content IR.

Generated SVG remains opaque and untrusted until a host-owned sanitizer or
isolated renderer accepts it. Active content, external references, URL loading,
and embedding policy are outside the processor protocol. Merman cancellation is
cooperative and cannot interrupt synchronous parse/layout/render work, so
untrusted input requires host-owned timeout and worker/process isolation.

## AI Message Parts

mdstream owns a Markdown content session, not an AI message envelope. The host
keeps message IDs, text/reasoning/tool/attachment part types, global ordering,
token pacing, layout, and scrolling. Each Markdown-capable part owns one
independent session identified by a stable host `partKey` plus a monotonically
new part generation:

- create on the part's first content and append only to that session;
- finish each part independently;
- preserve the session when stable part keys reorder around tool or reasoning
  parts;
- for historical replacement, reset/replay only the affected part;
- on removal, close that part and cancel its processor work;
- if a removed key is reused, allocate a new generation and reject callbacks
  from the retired generation.

There is intentionally no cross-session transition order in mdstream. The
message host composes per-part batches with tool state and its own presentation
clock.

## Adoption Evidence

Protocol 0.4 was promoted to final only after the shared
`adoption/headless-rich-content` fixture passed production-shaped native Rust,
TypeScript/WASM, and standalone Merman flows. Those flows cover adversarial
chunking, stable keys, targeted citation correction, gap recovery, processor
artifacts, reset, and stale-result rejection without Markdown reparsing or an
adapter-local reducer.
