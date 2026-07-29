# @mdstream/core

Framework-neutral TypeScript bindings for mdstream's Rust/WASM streaming
content engine. The package exposes external stores, changed-node views,
explicit snapshot recovery, lossless input batching, and host-side processor
scheduling without a renderer or UI-framework dependency.

This package is the complete first-party web state surface. Frameworks consume
its `subscribe`/`getSnapshot` stores and focused node, resource, and artifact
views through their native state primitives. mdstream intentionally does not
publish a React package or renderer; see
[`ADR 0004`](https://github.com/Latias94/mdstream/blob/main/docs/ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md).

## Visual adoption example

The repository-only [framework-neutral Web flagship](https://github.com/Latias94/mdstream/tree/main/examples/web) is the primary visual consumer of this package. From a source checkout with Node 24, pnpm 11.9.0, Rust 1.85, the `wasm32-unknown-unknown` target, `wasm-pack`, and the pinned `wasm-opt`, run:

```sh
pnpm install
pnpm web:prepare
pnpm --filter @mdstream/example-web dev
```

The Golden AI Stream settles with equal visible content, digest, lifecycle, stable keys, and accessible status in Immediate and Paced modes. `@mdstream/core` supplies canonical state, focused views, and transition facts. The private example owns DOM composition, citation URL policy, pacing, animation, layout, scrolling, focus, reduced motion, and announcements; none of that host code ships in this npm package.

Continue with the [complete example learning path](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#web-flagship) or the machine-readable transition probe below.

An engine owns its synchronized reducer and exposes a read-only `engine.store`
facade. Use `runtime.createStore()` only when applying a replicated change
stream and recovering it from an explicit snapshot. Both surfaces use the final
`mdstream.content/0.4` protocol implemented by Rust.

When accepted source temporarily runs ahead of typed Content IR,
`engine.store.pendingSource()` exposes a focused external store for the exact
uncovered UTF-8 byte range and text. The view is materialized only when read,
retains object identity until source or projection coverage changes, and is
`undefined` when the projection is current. Consumers may render that text as
pending content, but must not parse it into competing Markdown semantics.

## Lossless input batching

An engine grants at most one live batching lease. The lease retains original
non-empty chunks behind independent byte and constituent limits, appends them
in order inside one coherent host operation, and returns every committed result
as an ordered collection. Direct engine mutation and a second batcher are
rejected until the current batcher is explicitly released.

```ts
const batcher = engine.createBatcher({
  maxBatchBytes: 64 * 1024,
  maxPendingChunks: 2048,
});

for (const chunk of modelChunks) {
  for (const result of batcher.push(chunk)) {
    replicate(result.changes);
  }
}
for (const result of batcher.finish()) {
  replicate(result.changes);
}
batcher.release();
```

If a constituent fails after a committed prefix, `BatchOperationError` exposes
`completedResults`, the typed `cause`, the failed `operation`, an immutable
`pending` snapshot, and whether a triggering `push` input was accepted. The
batcher then rejects ordinary input and lifecycle operations. Call
`retryPending()`, `takePending()`, or `discardPending()` before releasing the
lease; only discard makes data loss an explicit caller decision. Boundary
metadata metrics use a deterministic logical cost of eight bytes per retained
constituent and exclude JavaScript allocator spare capacity.

The repository's runnable
[`lossless-batching.mjs`](https://github.com/Latias94/mdstream/blob/main/bindings/typescript/examples/lossless-batching.mjs)
shows both the normal ordered-collection path and partial-failure transfer:

```sh
pnpm --filter @mdstream/core build
node bindings/typescript/examples/lossless-batching.mjs --assert
```

## Transition facts

Hosts that need to distinguish fresh text, semantic corrections, structural
movement, and continuity resets can opt into transition capture. Capture is
disabled by default. The enabled configuration must use protocol limits whose
worst legal reducer update fits `maxReducerUpdateBytes`; construction fails
before any state exists when that proof cannot be made.

```ts
const engine = runtime.createEngine({
  captureTransitions: true,
  protocol: {
    maxSourceBytes: "1048576",
    maxNodes: "4096",
    maxResources: "256",
    maxOperations: "4096",
    maxChangeStructuralItems: "4096",
    maxChildrenPerList: "4096",
  },
  compiler: {
    maxMarkdownEvents: "300000",
    maxMarkdownOverlapWork: "1000000",
    maxDefinitions: "100000",
    maxDefinitionEdges: "100000",
    maxDefinitionMetadataBytes: "16777216",
  },
  wire: { maxReducerUpdateBytes: "67108864" },
});

const unsubscribe = engine.store.subscribeTransitions((batch) => {
  for (const facts of batch.facts) {
    if (facts.scope === "full_replace") {
      hostPresentation.clearContinuity(facts.after.continuityGeneration);
      continue;
    }
    hostPresentation.observe(facts);
  }
});
```

`protocol` contains only parser-neutral Content IR and reducer limits. Parser
work and retained definition-registry budgets belong to the independent
`compiler` group. Compiler fields are available only in that group, and the
native binding schema rejects unknown or misplaced option fields.

Processor scheduling uses the effective limits reported by the native reducer
session. Host adapters do not duplicate Rust defaults, so omitted and custom
processor budgets stay consistent across WASM versions.

Binary artifact snapshots expose `ImmutableBytesView` instead of a mutable
`Uint8Array`. Read their size without copying, and request an owned mutable copy
only when a consumer needs bytes:

```ts
const artifact = engine.store.getArtifactSnapshot(slot);
if (
  artifact?.state === "ready" &&
  artifact.artifact?.payload.kind === "binary"
) {
  const retainedBytes = artifact.artifact.payload.bytes.byteLength;
  const ownedBytes = artifact.artifact.payload.bytes.copyBytes();
  consumeBinaryArtifact(ownedBytes, retainedBytes);
}
```

Each `copyBytes()` call returns an independent `Uint8Array`; mutating that copy
does not change the retained store snapshot.

The callback is an ordered event feed, not a latest-value external store. One
callback represents one public operation and may contain multiple reducer
commits; equal and empty batches remain observable. All batch-tail state and
cache invalidations are coherent before the callback runs, while ordinary
store subscribers run afterward. A callback may read node, resource, document,
pending-source, or artifact views and may unsubscribe. It must not append,
finish, reset, recover, register or dispose processors, or close the session
until the callback returns. Listener failures are isolated from later listeners.

Transition facts are schedule-local observations rather than a replay stream.
The current store exposes only the operation's tail state. In particular, an
ordered `A -> B -> A` batch preserves both facts, but an intermediate `B` view
is not queryable after the operation commits.

A renderer can map the facts to its own policy:

| Fact | Host decision |
| --- | --- |
| `projection_append` | Reveal immediately, queue graphemes, or animate fresh text after subtracting any pending range already painted. |
| `replacement` or resource correction | Read the tail view and replace, announce, or cross-fade existing output. |
| node insertion/removal and structure splice | Mount/unmount stable keys and optionally measure host geometry for a layout transition. |
| parent or order change | Preserve the continuity-qualified key and let the UI framework choose its movement policy. |
| stability or lifecycle change | Settle provisional presentation or finalize host pacing. |
| `full_replace` | Clear presentation continuity, pending effects, and stale geometry. |

mdstream does not provide timing, easing, colors, opacity, geometry, scrolling,
components, or animation dependencies. Token pacing, grapheme grouping, reduced
motion, viewport ownership, and scroll anchoring remain host state. An immediate
mode must preserve the same content and state meaning; motion or color must not
be the only signal for a correction, removal, or replacement.

Use `useSyncExternalStore` or an equivalent framework primitive for the normal
document and focused stores. Process `subscribeTransitions` callbacks into the
host's own ordered queue; adapting this event feed as a single latest snapshot
can let framework batching collapse distinct operations.

The repository's [`transition-host.mjs`](https://github.com/Latias94/mdstream/blob/main/bindings/typescript/examples/transition-host.mjs) is a machine contract probe rather than starter UI code. From an installed source workspace, run:

```sh
pnpm --filter @mdstream/core build
node bindings/typescript/examples/transition-host.mjs --assert
```

It emits JSON with `"assertions": "passed"`, demonstrates host-owned reveal and layout decisions, and compares transition facts with an old-view/parent-index reconstruction baseline without shipping that policy in `@mdstream/core`.
