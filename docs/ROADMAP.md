# Roadmap

## 0.4 Foundation

The 0.4 line establishes the durable product boundary:

- final versioned Content IR and JSON wire protocol;
- chunk-invariant identity and deterministic node versions;
- explicit finish, reset, correction, replay, and snapshot recovery;
- bounded processor/artifact lifecycle with citation and optional Merman paths;
- Rust, Tokio, WASM/TypeScript, C FFI, Dart, and Flutter integrations;
- cross-runtime conformance, deterministic work gates, and package budgets.

## Candidate Follow-Ups

Future work should deepen the headless engine rather than add presentation
policy. Candidates include:

- more versioned semantic resources and processor protocols;
- binary transport only when JSON measurements show a concrete need;
- additional native/mobile architectures backed by build-and-load CI;
- persisted replay logs implemented above the canonical reducer;
- better bounded writers or external isolation for expensive processors;
- a public parser-engine abstraction only after a second real implementation
  proves the interface.

## Explicit Non-Goals

mdstream does not plan to own themes, widgets, syntax highlighting, math
layout, browser layout, networking, persistence, CRDT/OT editing, or arbitrary
historical source mutation.

A first-party React package or Markdown renderer is not on the roadmap.
Streamdown, Incremark, and framework-native renderers remain valid consumer
choices. Third parties may publish adapters over `@mdstream/core` without
becoming protocol authorities.

LALRPOP is not planned for the Markdown path. It may be considered for a future
independent, closed DSL where an LR grammar is the actual missing capability.
