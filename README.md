# mdstream

[![crates.io](https://img.shields.io/crates/v/mdstream.svg)](https://crates.io/crates/mdstream)
[![docs.rs](https://docs.rs/mdstream/badge.svg)](https://docs.rs/mdstream)
[![CI](https://github.com/Latias94/mdstream/actions/workflows/ci.yml/badge.svg)](https://github.com/Latias94/mdstream/actions/workflows/ci.yml)

`mdstream` is a headless streaming content engine for AI-generated Markdown.
It turns token chunks into versioned, replayable document changes with stable
content identity. UI frameworks consume the same canonical state without
reparsing Markdown or depending on a renderer.

```text
token chunks
    -> StreamEngine
    -> ChangeSet batches
    -> canonical Reducer / Content IR
    -> GPUI, egui, TUI, WASM / TypeScript, Flutter
    -> optional code, math, citation, and Mermaid processors
```

The 0.4 API is intentionally breaking. The previous block splitter and its
`committed + pending` update model are not retained as compatibility wrappers.

## Workspace

| Package | Responsibility |
| --- | --- |
| `mdstream` | Synchronous streaming engine, Markdown compiler, stable identity, and resource metrics |
| `mdstream-protocol` | Versioned Content IR, changes, snapshots, reducer, wire schema, and recovery laws |
| `mdstream-processors` | Version-checked processor requests, artifact lifecycle, cancellation, and limits |
| `mdstream-conformance` | Chunk schedules, replay laws, fixtures, workload generators, and budget contracts |
| `mdstream-tokio` | Lossless bounded channels and a `StreamEngine` actor |
| `mdstream-merman` | Optional standalone Merman processor adapter on its own Rust toolchain lane |

## Quick Start

The engine produces changes. A consumer applies every change through the
canonical reducer and renders only the node IDs reported by `ChangeImpact`.

```rust
use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{ApplyOutcome, Reducer};

fn apply(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        match reducer.apply(change).unwrap() {
            ApplyOutcome::Applied { impact, .. }
            | ApplyOutcome::Recovered { impact, .. } => {
                for node_id in impact.changed_nodes {
                    // Rebuild only the view cached under this stable NodeId.
                    let _ = node_id;
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

let document = reducer.document().unwrap();
assert_eq!(document.source(), "# Title\n\nHello **world**");
```

`finish` is terminal and idempotent. `reset` starts a predecessor-linked epoch.
Appending after finish returns a typed error without changing engine state.

## Setup-Only Extensions

Grammar configuration is sealed when the engine is built. Runtime grammar or
transformer mutation is deliberately unavailable.

```rust
use mdstream::{CustomBlockSpec, StreamEngine};

let mut engine = StreamEngine::builder()
    .custom_block(CustomBlockSpec::try_new("app.thinking/1", "thinking")?)
    .build()?;

let output = engine.append("<thinking>\nwork\n</thinking>\n")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Custom blocks use a small standalone line grammar. `pulldown-cmark 0.13.x`
remains the internal CommonMark/GFM semantic compiler for Markdown regions;
neither parser-specific types nor renderer artifacts enter the public protocol.

## UI State and Artifacts

- `NodeId` is the stable UI key.
- `NodeVersion` is the deterministic cache and compare-and-set version.
- `ChangeImpact` identifies changed and removed nodes/resources.
- `Snapshot` is explicit recovery state, not a payload emitted on every append.
- `ArtifactHost` stores processor output separately and rejects stale results
  using epoch, node/input versions, processor/configuration versions, and
  request generation.

See the compile-tested examples:

```sh
cargo run -p mdstream --example minimal
cargo run -p mdstream --example custom_blocks
cargo run -p mdstream --example egui_adapter
cargo run -p mdstream --example gpui_adapter
cargo +1.88.0 run -p mdstream-tokio --example agent_tui
```

The egui and GPUI examples are framework-neutral on purpose. They demonstrate
the ownership and invalidation contract without adding UI framework dependencies
to the core workspace.

For web applications, `@mdstream/core` is the first-party integration surface.
It exposes Rust/WASM-backed external stores, focused node/resource/artifact
views, and explicit recovery without a renderer or UI-framework dependency.
React consumers can bind these stores with `useSyncExternalStore`; mdstream does
not ship React hooks, components, themes, or a competing Markdown renderer. See
[`ADR 0004`](docs/ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md) for the boundary.

## Migration From 0.3

| 0.3 surface | 0.4 replacement |
| --- | --- |
| `MdStream`, `MdStreamBuilder` | `StreamEngine`, `StreamEngineBuilder` |
| `Update`, `UpdateRef` | ordered `mdstream_protocol::ChangeSet` values |
| `Block`, `BlockStatus` | typed `ContentNode`, `NodeStability`, and `DocumentLifecycle` |
| `DocumentState` | `mdstream_protocol::Reducer` |
| `AnalyzedStream`, `BlockAnalyzer` | typed Content IR plus external processors |
| runtime boundary/transformer mutation | setup-only `CustomBlockSpec` and processor configuration |
| pending Markdown repair helpers | `Document::pending_source()` plus host rendering policy |
| mutable committed/cache access | `ChangeImpact`, stable IDs, and immutable document views |

There are no deprecated aliases for the removed surface. Consumers must apply
the protocol through the reducer so sequence gaps, resets, recovery, and
semantic corrections remain observable.

## Conformance and Limits

Final Content IR, stable IDs, node versions, and lifecycle are invariant across
UTF-8-safe chunk schedules. The workspace checks replay laws, deterministic
compiler/reducer work, retained and transactional memory, processor budgets,
and frozen artifact ceilings. Hard resource failures are atomic and leave the
last accepted document unchanged.

The core engine is synchronous and runtime-independent. Tokio integration is
lossless; lossy policies are not available for canonical document input.

## Rust Versions and License

- Core engine, protocol, conformance, and processor crates: Rust 1.85+
- `mdstream-tokio`: Rust 1.88+
- Standalone `mdstream-merman`: Rust 1.95+

Licensed under either Apache-2.0 or MIT at your option.
