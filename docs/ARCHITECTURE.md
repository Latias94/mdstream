# Architecture

mdstream is a headless streaming content state engine. It owns canonical
Markdown-derived state, not presentation. The public architecture separates
source ingestion, deterministic reduction, host state projection, and derived
content processing.

```text
token chunks
    -> mdstream::StreamEngine
    -> mdstream_protocol::ChangeSet
    -> mdstream_protocol::Reducer
    -> Content IR views and ChangeImpact
    -> optional atomic TransitionFacts
    -> native or foreign-language state adapter

ContentNode + semantic resource
    -> mdstream_processors::ArtifactHost
    -> version-checked derived artifact
```

## Ownership

| Module | Owns | Does not own |
| --- | --- | --- |
| `mdstream-protocol` | Content IR, IDs, versions, changes, snapshots, reducer, lifecycle, wire schema, canonical-state limits | Markdown parsing, parser work budgets, UI state, processor execution |
| `mdstream` | Streaming input, framing, Markdown compilation and its work budgets, reconciliation, semantic correction | Async runtime, renderer, artifact storage |
| `mdstream-processors` | Processor requests, freshness keys, cancellation, artifact state and limits | Scheduling threads, sandboxing, canonical state |
| `mdstream-tokio` | Lossless async input transport and actor lifecycle | Alternative document semantics |
| `mdstream-bindings-core` | Stateful foreign-language sessions, command envelopes, typed transport errors | A second reducer or parser |
| `mdstream-wasm` / `mdstream-ffi` | Thin ABI transports | Host framework state or rendering |
| `@mdstream/core` / Dart / Flutter | Typed host views, subscriptions, batching, recovery ergonomics | Canonical reduction or Markdown rendering |
| `mdstream-merman` | Optional Mermaid-to-SVG processor adapter | Default dependency or canonical Mermaid state |

`mdstream-conformance` is private test infrastructure. It owns fixtures,
compatibility characterizations, schedule generation, replay laws, and frozen
resource budgets.

## Canonical State

One `Document` owns source text. Content nodes store source/body ranges rather
than copied body strings. A reducer accepts atomic changes only when epoch,
sequence, predecessor, expected versions, and limits are valid. Invalid changes
cannot partially mutate the document.

Source progress and typed projection coverage use separate cursors. This keeps
uncovered streaming bytes observable without requiring a second incremental
CommonMark parser. See [ADR 0002](ADR_0002_PROJECTION_FRONTIER.md).

Resource limits follow the same ownership boundary. `ProtocolLimits` bounds
legal Content IR and reducer state without naming a parser. `CompilerLimits`
bounds Markdown-specific event/classification work plus the compiler's retained
definition registry, reverse dependency edges, and definition metadata. Engine,
processor-host, and binding-wire limits remain separate because they constrain
different failure domains.

## Identity

`NodeId` is stable across chunk schedules and semantic correction inside one
continuity generation. `NodeVersion` is a deterministic opaque compare-and-set
value and changes when the node projection changes. Source offsets may guide
reconciliation but are not identity.

`ChangeImpact.changed_nodes` is the authoritative invalidation set for complete
materialized node views. An equal `NodeVersion` does not prove that the full view
is unchanged: `ContentNode.children.version` independently covers direct child
identity and order, while resource changes may invalidate nodes that reference
the resource.

`ProcessorInputVersion` is a separate deterministic compare token for the
complete node-local processor input: the node projection, body text, referenced
resource, and direct child-list topology. Adapters use it for matching caches
and conditional processor admission. They must not substitute `NodeVersion`,
because processor-visible context can change while the node projection version
remains stable.

Across advanced recovery or another full replacement, hosts qualify UI identity
with `(continuity generation, epoch, NodeId)`. A capture-disabled host advances
its own generation whenever `ChangeImpact.full_replace` is true. A
capture-enabled host receives the authoritative generation in transition facts.

Document lifecycle, node stability, and correction are independent axes:

- an open document may contain provisional and stable nodes;
- a stable node may receive a corrected projection under the same ID;
- finalization is one terminal document transition, not a node status.

## Runtime Boundaries

The engine and reducer are synchronous and runtime-independent. Tokio, browser
workers, Dart isolates, and application task schedulers live above that
boundary. Processor code runs outside reducer and FFI critical sections. An
in-process processor is trusted cooperative code; untrusted processors require
caller-provided process or worker isolation.

## Framework Boundary

Rust UIs consume the reducer directly. `@mdstream/core` is the complete
framework-neutral web surface. React, Vue, Svelte, Solid, and other frameworks
bind its external stores to their native state primitive. mdstream does not
ship a React package or renderer. Flutter is first-party because the package
also owns native binary delivery and platform loading; it still ships no
widgets or rendering policy. See [ADR 0004](ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md).
Hosts that need change classification opt into the same transition-facts
contract without adopting an mdstream renderer. See
[ADR 0005](ADR_0005_HOST_TRANSITION_FACTS.md).
