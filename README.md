# mdstream

`mdstream` is a headless streaming content state engine for AI-generated Markdown. It turns arbitrary token chunks into replayable canonical changes, typed Content IR, stable identities, bounded pending source, and optional factual transitions so any UI framework can own presentation without reparsing Markdown.

```text
token chunks
    -> StreamEngine
    -> ordered ChangeSet batches
    -> canonical Reducer / typed Content IR
    -> focused invalidation + optional transition facts
    -> application-owned UI state
    -> optional citation, Mermaid, math, or code processors
```

Version 0.4 is intentionally breaking. It replaces the 0.3 block-splitter and `committed + pending` update model instead of preserving compatibility wrappers.

## Start here

With Rust 1.85 or newer, a fresh checkout needs one command for the first deterministic AI stream:

```sh
cargo run -p mdstream --example minimal -- --assert
```

The tutorial prints named pending, stabilization, citation-correction, and finalization checkpoints, then ends with:

```text
ASSERTIONS_OK scenario=golden-ai-stream
```

The next step is the repository-only [framework-neutral Web flagship](https://github.com/Latias94/mdstream/tree/main/examples/web):

```sh
pnpm install
pnpm web:prepare
pnpm --filter @mdstream/example-web dev
```

Open the printed local URL and switch between Immediate and Paced. Both modes create fresh sessions and settle to the same visible content, canonical digest, lifecycle, stable keys, and accessible correction meaning. The example needs no API key, provider, or external runtime service.

The full [example learning path](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md) records prerequisites, commands, expected observations, availability, teaching role, and next step for every entry.

## Ownership boundary

| mdstream owns | The host owns |
| --- | --- |
| Canonical source, typed Content IR, lifecycle, stable IDs and versions | Widgets or DOM, typography, themes, and rich-content component composition |
| Ordered changes, explicit recovery, invalidated identities, optional factual transitions | Token pacing, grapheme grouping, animation, color, layout, scrolling, and reduced motion |
| Bounded on-demand pending source and versioned processor request/artifact identity | Accessibility announcements, focus, URL/resource policy, sanitizer or isolated renderer, and process timeouts |

mdstream does not publish a React package, Markdown renderer, Flutter widget, animation API, provider connector, theme, or layout engine. A React host can bind `@mdstream/core` stores with `useSyncExternalStore`; other frameworks use their equivalent state primitive.

## Capability map

| Capability | Teaching role | Primary runnable entry |
| --- | --- | --- |
| Canonical streaming, pending source, stable IDs, correction, recovery, and finalization | First-success tutorial | [Rust minimal](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#rust-minimal) |
| Rust/WASM stores, typed DOM composition, Immediate/Paced policy, and accessibility | Interactive visual showcase | [Web flagship](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#web-flagship) |
| Explicit native loading, focused state, transition order, and handle cleanup | Headless binding tutorial | [Dart headless](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#dart-headless) |
| Turnkey native delivery, focused listenables, stable widget keys, and host-owned motion | Interactive native host | [Flutter host](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#flutter-host) |
| Bounded asynchronous transport, lossless coalescing, and actor shutdown | Machine smoke probe | [Tokio actor](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#tokio-actor) |
| Typed Mermaid processing, artifact generations, stale rejection, and SVG trust boundary | Processor recipe | [Merman artifact](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#merman-artifact) |

## Core Rust model

Applications that own canonical Rust state depend on both `mdstream` and `mdstream-protocol`:

```toml
[dependencies]
mdstream = "0.4"
mdstream-protocol = "0.4"
```

Apply every emitted change in order. Rebuild only identities named by `ChangeImpact`:

```rust
use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{ApplyOutcome, Reducer};

fn apply(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        match reducer.apply(change).unwrap() {
            ApplyOutcome::Applied { impact, .. }
            | ApplyOutcome::Recovered { impact, .. } => {
                for node_id in impact.changed_nodes {
                    // Refresh or remove only the host view under this stable ID.
                    let _ = reducer.document().and_then(|document| document.node(node_id));
                }
            }
            outcome => panic!("unexpected producer outcome: {outcome:?}"),
        }
    }
}

let mut engine = StreamEngine::new();
let mut reducer = Reducer::new();
apply(&mut reducer, engine.append("# Title\n\nHello **wor").unwrap());
apply(&mut reducer, engine.append("ld**").unwrap());
apply(&mut reducer, engine.finish().unwrap());
assert_eq!(reducer.document().unwrap().source(), "# Title\n\nHello **world**");
```

Limit configuration follows module ownership. Use
`mdstream_protocol::ProtocolLimits` for legal Content IR and reducer state,
`mdstream::CompilerLimits` for parser work and retained compiler semantic state, and
`mdstream::EngineLimits` for emitted transaction and change sizes. The builder
accepts these independently through `protocol_limits`, `compiler_limits`, and
`engine_limits`; parser- and compiler-state-specific fields are intentionally
not part of the framework-neutral protocol crate.

`finish` is terminal and idempotent. `reset` starts a predecessor-linked epoch. A gap, fork, or unannounced epoch moves a replica reducer to `NeedsSnapshot`; one explicit current snapshot restores it before ordinary changes resume.

`NodeId` is stable within a continuity generation. `ChangeImpact.changed_nodes` is the authoritative invalidation set for complete materialized node views. `NodeVersion` is a compare-and-set token for projection-local stability, ranges, and content; an equal value does not prove that child topology or processor context is unchanged. `ContentNode.children.version` covers direct child identity and order, while `ProcessorInputVersion` covers processor matching and conditional admission across the node projection, body text, referenced resource, and direct children. Across a full replacement, host keys include continuity generation, epoch, and `NodeId`; collection position and source offsets are not keys.

When accepted source runs ahead of typed projection, adapters expose exactly `projection_cursor..source_cursor` as a bounded, lazy pending-source view. A host may paint those bytes once, but must not parse them into competing Markdown semantics.

## Transitions and presentation

`ChangeImpact` is the normal latest-state invalidation surface. Hosts that need to distinguish a fresh projection append from correction, stabilization, structure/resource changes, lifecycle, or full replacement can opt into atomic `mdstream.transitions/1` facts.

Facts contain state meaning, not presentation instructions. Different legal token schedules may produce different intermediate fact batches while converging to identical final canonical state. A host presentation queue must read coherent batch-tail views, avoid revealing already-painted pending bytes twice, and clear queued effects and continuity keys on full replacement.

Animation remains replaceable application code. Corrections, removals, and replacements must remain understandable in immediate or reduced-motion mode and cannot rely on motion or color alone.

## Extensions and processors

`StreamEngineBuilder::custom_block` registers versioned standalone source framing before the first input. Runtime parser mutation and pending transformers are intentionally absent.

Processors consume typed nodes and resources after canonical reduction. `ArtifactHost` keys results by epoch, node/input versions, processor/configuration versions, and request generation; it rejects late completions and keeps artifacts outside canonical snapshots.

`mdstream-merman` is an optional standalone Rust 1.95 adapter. Its SVG output remains opaque and untrusted until an application-owned `sanitizeSvgArtifact` boundary or isolated renderer accepts it. Byte/model limits and cooperative cancellation are accounting controls, not CPU, peak-memory, or process isolation.

## Bindings

`@mdstream/core` is the complete first-party Web state surface. It provides Rust/WASM-backed engines, replica stores, focused root/node/resource/pending/artifact views, lossless batching, ordered transition subscriptions, processor scheduling, and explicit recovery without a renderer or framework dependency.

The Dart `mdstream` package wraps the stable C ABI and requires a trusted host-supplied dynamic-library path. `mdstream_flutter` adds Android, iOS, macOS, Linux, and Windows native delivery plus focused controllers; widget composition remains in the example application.

The C ABI uses opaque handles and owned buffers. Foreign hosts must validate ABI/schema/layout compatibility, check every status, and release buffers and handles with the matching mdstream function. Compatibility checks do not authenticate executable native code.

mdstream owns one Markdown content session, not an AI message envelope. Chat message IDs, reasoning/tool/attachment parts, cross-part ordering, provider events, persistence, and scrolling stay in the application. Give every Markdown-capable part its own session and host generation.

## Workspace

| Package | Responsibility |
| --- | --- |
| `mdstream` | Synchronous streaming engine, Markdown compiler, stable identity, and resource metrics |
| `mdstream-protocol` | Versioned Content IR, deltas, snapshots, reducer, transition facts, wire schema, and recovery |
| `mdstream-processors` | Versioned processor requests, artifacts, cancellation, stale-result rejection, and limits |
| `mdstream-conformance` | Chunk schedules, replay laws, fixtures, workload generators, and budget contracts |
| `mdstream-tokio` | Lossless bounded channels and a `StreamEngine` actor on Rust 1.88 |
| `mdstream-merman` | Optional standalone Merman processor adapter on Rust 1.95 |
| `mdstream-bindings-core` | Stateful engine/reducer sessions and command envelopes shared by transports |
| `mdstream-wasm` | Thin WebAssembly transport over the shared bindings facade |
| `mdstream-ffi` | Stable C ABI with opaque handles, owned buffers, and panic containment |
| `@mdstream/core` | Framework-neutral TypeScript stores, focused views, recovery, batching, and processors |
| Dart `mdstream` | Flutter-independent native binding using a host-supplied library |
| `mdstream_flutter` | Turnkey native delivery and Flutter state controllers without widgets |

The default Rust, WASM, TypeScript, and Dart dependency graphs contain neither
Merman nor a UI framework. `mdstream_flutter` depends on the Flutter SDK but
exports state controllers only: it includes no widget, renderer, animation
policy, or Merman dependency.

## Migrating from 0.3

There are no deprecated aliases for the removed 0.3 surface.

| 0.3 surface | 0.4 action |
| --- | --- |
| `MdStream` / `MdStreamBuilder` | Use `StreamEngine` / `StreamEngineBuilder`. |
| `Options` (`footnotes`, `reference_definitions`, `terminator`, `terminator_window_bytes`, `max_buffer_bytes`) | Remove the old parsing modes: footnotes and reference definitions now use canonical semantic correction; pending repair and its display window belong to the host. Replace the old buffer-cap intent with independently owned `ProtocolLimits::max_source_bytes`, `CompilerLimits`, and `EngineLimits`; these limits reject atomically rather than compacting canonical source. |
| `append` / `finalize` | Call fallible `StreamEngine::append` / `finish`; use `reset` for a new epoch. |
| `Update` / `UpdateRef` / `DocumentState` | Apply every ordered `ChangeSet` through `mdstream_protocol::Reducer` and consume `ChangeImpact`. |
| `Block` / `BlockStatus` / collection positions | Use typed `ContentNode`, `NodeStability`, and stable `NodeId`; invalidate complete cached node views through `ChangeImpact.changed_nodes`, use `NodeVersion` for projection compare-and-set, and compare `children.version` for direct child topology. |
| `AnalyzedStream` / `BlockAnalyzer` | Read typed Content IR or use a versioned processor whose artifact remains derived host state. |
| `BoundaryPlugin` / runtime grammar mutation | Register setup-only `CustomBlockSpec` values before accepting input. |
| `TerminatorOptions` / `terminate_markdown` / pending transformers | Read bounded pending source on demand and keep incomplete-source presentation in host policy. |
| `spawn_mdstream_actor` | Use `spawn_stream_engine_actor`, send `ActorCommand`, receive `ActorResult`, and drain with `join`. |
| `BackpressurePolicy::DropNew` / `SendOutcome::Dropped` | Use `Block` or `CoalesceLocal`; canonical input is never intentionally dropped. |

For `CoalesceLocal`, await fallible policy changes and call `flush().await` before dropping a sender after any buffered result.

## Verification and limits

The conformance corpus replays whole-source, semantic-stage, adversarial, scalar, exhaustive bounded UTF-8, and randomized chunk schedules. Supported bindings converge on the same normalized final state while schedule-local intermediate facts remain free to differ.

Protocol, compiler, processor, transport, and artifact budgets fail deterministically and atomically. Release automation verifies Cargo package inventories plus the exact npm, Dart, and Flutter archives, dependency boundaries, native binary formats, forbidden paths, and absolute artifact ceilings.

## Documentation

- [Example learning path](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md)
- [Cross-language usage](https://github.com/Latias94/mdstream/blob/main/docs/USAGE.md)
- [Architecture and ownership](https://github.com/Latias94/mdstream/blob/main/docs/ARCHITECTURE.md)
- [State, lifecycle, and recovery](https://github.com/Latias94/mdstream/blob/main/docs/STATE.md)
- [Extensions and processors](https://github.com/Latias94/mdstream/blob/main/docs/EXTENSIONS.md)
- [Adapter contracts](https://github.com/Latias94/mdstream/blob/main/docs/ADAPTERS.md)
- [Compatibility profiles](https://github.com/Latias94/mdstream/blob/main/docs/COMPATIBILITY.md)
- [Performance and resource contracts](https://github.com/Latias94/mdstream/blob/main/docs/PERFORMANCE.md)
- [Roadmap and non-goals](https://github.com/Latias94/mdstream/blob/main/docs/ROADMAP.md)
- [Framework-neutral Web decision](https://github.com/Latias94/mdstream/blob/main/docs/ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md)
- [Host transition facts decision](https://github.com/Latias94/mdstream/blob/main/docs/ADR_0005_HOST_TRANSITION_FACTS.md)

## Rust versions and license

- Core engine, protocol, processor, binding, WASM, and FFI crates: Rust 1.85+
- `mdstream-tokio`: Rust 1.88+
- Standalone `mdstream-merman`: Rust 1.95+

Licensed under either Apache-2.0 or MIT at your option.
