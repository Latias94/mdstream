# Example Learning Path

The examples use one deterministic, provider-free Golden AI Stream where that shared context helps adoption. They demonstrate mdstream's state contract without turning example presentation code into a package API.

mdstream owns canonical Content IR, stable identity, lifecycle, recovery, and factual transition batches. The host owns rendering, timing, animation, layout, scrolling, accessibility, URL policy, and artifact trust. Immediate and paced presentation may look different, but they must settle to the same content and state meaning.

Example roles are deliberate:

- **Tutorial:** the shortest supported path to a first result.
- **Interactive showcase:** a real host that makes replaceable presentation policy visible.
- **Focused recipe:** one integration concern with minimal surrounding application code.
- **Machine contract probe:** deterministic evidence intended for verification and diagnostics, not starter UI code.

## Recommended order

1. Run the [Rust minimal tutorial](#rust-minimal) for the canonical engine/reducer model.
2. Run the [framework-neutral Web flagship](#web-flagship) to see host-owned presentation policy.
3. Choose the [Dart](#dart-headless) or [Flutter](#flutter-host) path if that is your target runtime.
4. Add asynchronous transport with the [Tokio actor](#tokio-actor), then inspect a complete host composition in the [Tokio rich workbench](#tokio-rich-workbench) when building an agent TUI.
5. Use the focused recipes and contract probes for recovery, processors, custom syntax, and release diagnostics.

<!-- example:rust-minimal -->
## Rust minimal

- Role: First-success tutorial
- Source: [`mdstream/examples/minimal.rs`](../mdstream/examples/minimal.rs)
- Prerequisites: A source checkout and Rust 1.85 or newer; no credentials, provider, network service, or graphical environment.
- Run: `cargo run -p mdstream --example minimal -- --assert`
- Expect: Named Golden AI Stream checkpoints, explicit mdstream/host ownership lines, and `ASSERTIONS_OK scenario=golden-ai-stream`.
- Next: [Framework-neutral Web flagship](../examples/web/README.md)
- Availability: Source checkout and the published `mdstream` crate archive.

The tutorial streams prose, Rust code, Mermaid source, and a late citation definition through `StreamEngine` and `Reducer`. It exposes pending source, stabilization, semantic correction, stable node identity, and finalized canonical state without rendering Markdown.
<!-- /example -->

<!-- example:web-flagship -->
## Web flagship

- Role: Interactive visual showcase
- Source: [`examples/web/src/main.ts`](../examples/web/src/main.ts)
- Prerequisites: A source checkout, Node 24, pnpm 11.9.0, Rust 1.85, the `wasm32-unknown-unknown` target, `wasm-pack`, and the workspace's pinned `wasm-opt`; no credentials or external runtime service.
- Run: `pnpm web:prepare && pnpm --filter @mdstream/example-web dev` after installing the workspace, then open the printed local URL.
- Expect: Stream settled with finalized canonical content. The host also exposes stable keys, a canonical digest, pending-source catch-up, semantic-correction status, and equal settled meaning in Immediate and Paced modes.
- Next: [Flutter host](#flutter-host)
- Availability: Repository-only private workspace; it is not included in `@mdstream/core`.

The showcase imports only the published `@mdstream/core` surface. DOM composition, citation URL allowlisting, grapheme pacing, motion, layout, scrolling, focus, reduced motion, and announcements live in example-local host code. It uses typed focused views and never reparses Markdown or treats motion or color as the only correction signal.
<!-- /example -->

<!-- example:dart-headless -->
## Dart headless

- Role: Headless binding tutorial
- Source: [`bindings/dart/example/golden_stream.dart`](../bindings/dart/example/golden_stream.dart)
- Prerequisites: Dart 3.8 or newer and an absolute path to a trusted, compatible `mdstream-ffi` dynamic library; loading the library executes native code in the current process.
- Run: `cd bindings/dart && LIBRARY=$(dart run tool/build_native.dart) && dart run example/golden_stream.dart --library "$LIBRARY" --assert`
- Expect: Named checkpoints, ordered transition categories, stable final node IDs, `final_lifecycle=finalized`, `assertions=passed`, and `native_allocations=zero`.
- Next: [Flutter host](#flutter-host)
- Availability: Source checkout and the published Dart package archive; the package does not include a native binary.

The example reads pending source and focused state only when requested and closes every native handle. `--library` and the documented environment variables are compatibility inputs, not authenticity checks.
<!-- /example -->

<!-- example:flutter-host -->
## Flutter host

- Role: Interactive native host
- Source: [`bindings/flutter/example/lib/main.dart`](../bindings/flutter/example/lib/main.dart)
- Prerequisites: Flutter 3.32.1 or newer, a supported Android, iOS, macOS, Linux, or Windows toolchain, and a runnable device such as `macos`.
- Run: `python3 bindings/flutter/tool/build_native.py macos && cd bindings/flutter/example && flutter create --empty --platforms macos --project-name mdstream_flutter_example --org io.mdstream.example --no-pub . && dart run configure_host.dart macos && flutter run -d macos`; replace the platform and device values when appropriate. The helper raises generated Apple deployment targets to the native package minimum.
- Expect: Settled canonical content in an answer-first Golden stream with replay and presentation controls, stable `MdstreamNodeKey` widgets, focused pending/transition state, and semantic status announcements.
- Next: [Merman artifact](#merman-artifact)
- Availability: Source checkout and the published `mdstream_flutter` package example; widget composition remains example-owned.

The primary command above is for a source checkout, where `build_native.py` stages the selected platform library. From an extracted published package, start at `cd example && flutter create --empty --platforms macos --project-name mdstream_flutter_example --org io.mdstream.example --no-pub . && dart run configure_host.dart macos && flutter run -d macos`; its native artifacts are already staged. Platform runners are generated on demand and stay outside the package. The repository-only supported-platform probe uses the same generation command, then `dart run configure_host.dart macos`, before `flutter test integration_test/golden_stream_smoke_test.dart -d macos`; test sources are excluded from the package archive. `mdstream_flutter` supplies native loading and headless controllers, not a Markdown widget, renderer, animation system, theme, or bundled Merman binary.
<!-- /example -->

<!-- example:tokio-actor -->
## Tokio actor

- Role: Machine smoke probe
- Source: [`mdstream-tokio/examples/agent_tui.rs`](../mdstream-tokio/examples/agent_tui.rs)
- Prerequisites: Rust 1.88 or newer; interactive mode also needs a terminal.
- Run: `cargo +1.88.0 run -p mdstream-tokio --example agent_tui -- --smoke`
- Expect: `SMOKE_OK` with a finalized lifecycle, zero errors, and bounded-channel command, batch, and change counters from the real actor path.
- Next: [Web flagship](#web-flagship)
- Availability: Source checkout and the published `mdstream-tokio` crate archive.

Omit `--smoke` for the scrollable Ratatui host. This example adds lossless coalescing, bounded backpressure, actor shutdown, follow-tail, and host scrolling policy; the TUI is not a first-party renderer contract.
<!-- /example -->

<!-- example:tokio-rich-workbench -->
## Tokio rich workbench

- Role: Interactive agent-TUI host composition.
- Source: [`mdstream-tokio/examples/agent_tui_rich.rs`](../mdstream-tokio/examples/agent_tui_rich.rs)
- Prerequisites: Rust 1.88 or newer, a C compiler for Tree-sitter grammar build steps, and a terminal for interactive mode.
- Run (smoke): `cargo +1.88.0 run -p mdstream-tokio --features rich-tui --example agent_tui_rich -- --smoke`
- Run (interactive): `cargo +1.88.0 run -p mdstream-tokio --features rich-tui --example agent_tui_rich`
- Run (reduced motion): `cargo +1.88.0 run -p mdstream-tokio --features rich-tui --example agent_tui_rich -- --reduced-motion`
- Expect: `RICH_SMOKE_OK` with finalized canonical content, nonzero semantic lines and Tree-sitter captures, a drained stable-line queue, direct-render-equivalent final lines, and completed host activity events.
- Next: [Merman artifact](#merman-artifact)
- Availability: Source checkout and the published `mdstream-tokio` crate archive; the optional `rich-tui` feature is not part of the default library build.

The three-pane Ratatui workbench reduces every ordered result in an actor batch, then reconciles once from the coherent batch-tail `TransitionReducer` document. It derives the recursively stable leading root prefix and keeps three non-overlapping visible regions: committed lines, visible queued stable lines waiting in a host-owned FIFO, and the latest mutable Content IR tail. Qualified transition facts invalidate a private stable-root projection cache, so unchanged history is reused while corrections are rerendered. Pending raw source appears only as a factual status; it is neither transcript content nor input to a host Markdown parser. Stable identity remains correction-capable, so late semantic changes refresh the same qualified owner without replaying it as fresh content.

Line pacing, animation, reduced motion, Tree-sitter analysis, layout, scrolling, highlighting, and tool activity remain host-local policy. Tree-sitter receives only complete, recursively stable Rust and JSON `CodeBlock` bodies, while provisional code stays plain. The terminal-specific line queue is not a shared presentation API or renderer; the Web and Flutter examples deliberately use different host state and presentation mechanisms. Mermaid remains a typed code node until a host explicitly hands it to an artifact processor such as Merman.

A host that wants per-text motion can aggregate qualified `NodeTransition` facts while reducing the batch: `TextTransition::ProjectionAppend` identifies newly projected text, while `Replacement`, removal, and full replacement are correction barriers that should update immediately instead of replaying old content. This example demonstrates the companion pattern at stable-root line granularity with owner-qualified stages and a dim-to-committed color change. Other hosts can map the text facts to fades, highlights, or layout transitions without putting animation state into mdstream or reparsing Markdown.
<!-- /example -->

<!-- example:merman-artifact -->
## Merman artifact

- Role: Processor recipe
- Source: [`mdstream-merman/examples/render_golden.rs`](../mdstream-merman/examples/render_golden.rs)
- Prerequisites: Rust 1.95 or newer; `mdstream-merman` is a standalone package outside the core workspace.
- Run: `cargo +1.95.0 run --manifest-path mdstream-merman/Cargo.toml --example render_golden -- --assert`
- Expect: A generation-qualified artifact request key, `mdstream.mermaid.svg/1`, `image/svg+xml`, `host_handoff=sanitizeSvgArtifact`, and `mdstream-merman golden stream: ok`.
- Next: [Extensions and processors](EXTENSIONS.md)
- Availability: Source checkout and the published `mdstream-merman` crate archive; Merman is absent from default Rust, WASM, TypeScript, Dart, and Flutter dependency graphs.

The recipe starts with streamed Markdown, selects the typed stable Mermaid node, and produces an opaque derived artifact without mutating Content IR. It never prints or mounts SVG. Sanitization or process-isolated display, timeouts, and resource-loading policy belong to the host; cooperative cancellation and byte limits are not a sandbox.
<!-- /example -->

## Focused Rust recipes

### Stable keyed state

- Role: Focused identity and invalidation recipe.
- Source: [`mdstream/examples/headless_state.rs`](../mdstream/examples/headless_state.rs)
- Prerequisites: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example headless_state`
- Expect: One paragraph identity survives append, stabilization, and finish; reset reports removals for every prior-epoch host key.
- Next: [Processor lifecycle recipe](#processor-lifecycle)

### Processor lifecycle

- Role: Focused generic derived-artifact recipe.
- Source: [`mdstream/examples/processor_lifecycle.rs`](../mdstream/examples/processor_lifecycle.rs)
- Prerequisites: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example processor_lifecycle`
- Expect: An applied artifact leaves canonical state unchanged, then a completion from before reset is rejected as stale.
- Next: [Custom blocks recipe](#custom-blocks)

### Custom blocks

- Role: Focused typed-syntax extension recipe.
- Source: [`mdstream/examples/custom_blocks.rs`](../mdstream/examples/custom_blocks.rs)
- Prerequisites: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example custom_blocks`
- Expect: A stable `app.thinking/1` node and a versioned derived text artifact with unchanged canonical state.
- Next: [Replica recovery recipe](#replica-recovery)

### Replica recovery

- Role: Focused continuity and snapshot-recovery recipe.
- Source: [`mdstream/examples/replica_recovery.rs`](../mdstream/examples/replica_recovery.rs)
- Prerequisites: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example replica_recovery`
- Expect: `retained_same_floor`, `replaced_advanced`, and `new_epoch` decisions with the corresponding host-key action.
- Next: [Transition trace](#transition-trace)

### Transition trace

- Role: Machine contract probe for fixed-schedule host work, not starter application code.
- Source: [`mdstream/examples/transition_trace.rs`](../mdstream/examples/transition_trace.rs)
- Prerequisites: Rust 1.85 or newer.
- Run: `cargo run -p mdstream --example transition_trace`
- Expect: Deterministic JSON with schedule-local reconstruction traces and an equal shared final snapshot; raw intermediate facts are not claimed to match across schedules.
- Next: [Web flagship](#web-flagship)

## Binding and processor contract probes

### TypeScript transition probe

- Role: Machine contract probe for lazy focused views, pending intervals, transition ordering, and host retention.
- Source: [`bindings/typescript/examples/transition-host.mjs`](../bindings/typescript/examples/transition-host.mjs)
- Prerequisites: The Web flagship toolchain prerequisites and an installed pnpm workspace.
- Run: `pnpm --filter @mdstream/core build && node bindings/typescript/examples/transition-host.mjs --assert`
- Expect: A JSON report ending with `"assertions": "passed"` and equal immediate/paced semantic results while the fact-driven host retains less state than the old-view baseline.
- Next: [Dart headless](#dart-headless)

### Citation processor contract

- Role: Machine contract probe for resolved citation artifacts and stale-result safety.
- Source: [`mdstream-processors/tests/citation_processor.rs`](../mdstream-processors/tests/citation_processor.rs)
- Prerequisites: Rust 1.85 or newer and `cargo-nextest`.
- Run: `cargo +1.85.0 nextest run -p mdstream-processors --test citation_processor`
- Expect: Typed citation resources produce versioned artifacts while unsupported, unresolved, changed, or stale input is rejected deterministically.
- Next: [Merman artifact](#merman-artifact)

### C ABI consumer

- Role: Machine contract probe for C ownership and static/dynamic linking.
- Source: [`mdstream-ffi/tests/c_consumer_smoke.c`](../mdstream-ffi/tests/c_consumer_smoke.c)
- Prerequisites: Rust 1.85 or newer, `cargo-nextest`, and a supported C compiler toolchain.
- Run: `cargo +1.85.0 nextest run -p mdstream-ffi --test c_consumer_smoke`
- Expect: The external C consumer compiles, links, runs against both library forms, and releases every owned handle and buffer through the C ABI.
- Next: [Dart headless](#dart-headless)

## Golden scenario authority

[`examples/fixtures/golden-ai-stream.json`](../examples/fixtures/golden-ai-stream.json) is the only hand-edited shared example timeline. Package-local copies are generated and checked byte-for-byte. It is a repository-only adoption schema, not the runtime wire protocol. The exact runtime oracle remains `mdstream.conformance/0.4`, and legal token chunking may change intermediate transition batches while preserving named schedule-independent observations and final canonical state.
