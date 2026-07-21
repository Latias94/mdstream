# Rust Example Guide

These examples are a learning ladder, not UI adapters. mdstream owns canonical content, stable identity, lifecycle, recovery, and factual transitions. Your host owns rendering, animation, timing, layout, scrolling, accessibility, and artifact trust policy.

## 1. Golden Stream Quickstart

- Role: tutorial and recommended first run.
- Prerequisite: Rust 1.85 or newer; no credentials or network service.
- Run: `cargo run -p mdstream --example minimal -- --assert`
- Expected result: named Golden AI Stream checkpoints followed by `ASSERTIONS_OK scenario=golden-ai-stream`.
- Concepts: `StreamEngine`, `Reducer`, pending source, stabilization, semantic correction, and the mdstream/host ownership boundary.
- Next: run `headless_state` to turn change impacts into keyed host updates.

## 2. Headless State

- Role: focused identity and invalidation recipe.
- Prerequisite: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example headless_state`
- Expected result: one retained paragraph identity across append and stabilization, followed by prior-epoch key removals on reset.
- Concepts: continuity-qualified host keys, `changed_nodes`, `removed_nodes`, and minimal cache invalidation.
- Next: run `processor_lifecycle` to keep derived rich content outside canonical state.

## 3. Processor Lifecycle

- Role: focused derived-artifact recipe.
- Prerequisite: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example processor_lifecycle`
- Expected result: an applied host-owned artifact with unchanged canonical state, then a stale late result after reset.
- Concepts: processor request identity, `ArtifactHost`, canonical-versus-derived ownership, and stale-result rejection.
- Next: run `custom_blocks` when application-specific syntax must become typed Content IR.

## 4. Custom Blocks

- Role: focused typed-extension recipe.
- Prerequisite: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example custom_blocks`
- Expected result: a stable `app.thinking/1` node and a dispatched text artifact while canonical state remains unchanged.
- Concepts: `CustomBlockSpec`, typed attributes, versioned artifact protocols, and host dispatch.
- Next: run `replica_recovery` before transporting change sets across a fallible boundary.

## 5. Replica Recovery

- Role: focused continuity recipe driven by the Golden AI Stream.
- Prerequisite: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example replica_recovery`
- Expected result: `retained_same_floor`, `replaced_advanced`, and `new_epoch` decisions with the corresponding host key policy.
- Concepts: sequence gaps, named snapshots, continuity generations, same-floor retention, and advanced full replacement.
- Next: run `transition_trace` to inspect deterministic schedule-local host work.

## 6. Transition Trace

- Role: machine-readable contract probe, not a first tutorial.
- Prerequisite: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example transition_trace`
- Expected result: deterministic JSON containing two schedule-local reconstruction traces and one shared final snapshot.
- Concepts: transition facts, reconstruction work, fixed-schedule determinism, and final chunking invariance. Raw fact sequences are intentionally not compared across schedules.
- Next: use the advanced Tokio host only when your producer needs bounded asynchronous transport.

## 7. Tokio Actor TUI

- Role: advanced interactive runtime-host example.
- Prerequisite: Rust 1.88 or newer and a terminal for interactive mode.
- Run interactively: `cargo +1.88.0 run -p mdstream-tokio --example agent_tui`
- Run without terminal control: `cargo +1.88.0 run -p mdstream-tokio --example agent_tui -- --smoke`
- Expected result: the interactive command opens a scrollable Ratatui host; smoke mode prints `SMOKE_OK` with finalized lifecycle and bounded-channel counters.
- Concepts: the stream-engine actor, lossless coalescing, bounded backpressure, scrolling policy, and automatic finalization when input closes.
- Next: integrate the actor with your own producer and host event loop; do not copy the TUI as a universal renderer.
