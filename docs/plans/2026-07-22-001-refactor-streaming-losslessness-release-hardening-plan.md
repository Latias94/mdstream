---
title: Streaming Losslessness and Release Hardening - Plan
type: refactor
date: 2026-07-22
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
deepened: 2026-07-22
---

# Streaming Losslessness and Release Hardening

## Goal Capsule

- **Objective:** Close the remaining PR audit gaps so every optimized streaming path preserves canonical input order and recoverability, every binding exposes the same bounded failure semantics, and release evidence executes the exact Flutter archive that will be published.
- **Authority:** This plan refines the completed 0.4 architecture without reopening its headless product boundary. The Product Contract below and ADRs 0002, 0004, and 0005 are authoritative where older examples or adapter behavior disagree.
- **Execution profile:** Breaking refactor across Rust core, Tokio, bindings-core, WASM/FFI, TypeScript, Dart/Flutter host integration, Web host policy, Flutter package tooling, CI, documentation, and changelogs. Compatibility shims and superseded append/coalescing helpers may be deleted.
- **Stop condition:** Joined-input rejection cannot lose, reorder, or silently discard original chunks; Web catch-up cannot overtake paced text; native and Web inputs are rejected at their earliest enforceable bounded admission point; Android failures retain the primary cause and terminate; Android, Windows, and macOS CocoaPods consume the producer's exact Flutter archive; every locally supported gate and all required CI checks pass. Only the documented local Apple framework mismatch may remain as an environment exception, with exact command and environment evidence; it never waives the exact macOS CI consumer.
- **Tail ownership:** Implementation includes focused red/green tests, deterministic performance evidence, migration notes, package checks, simplification, full code review, precise conventional commits, push to the existing PR branch, and CI follow-through.

---

## Product Contract

### Summary

mdstream 0.4 already has the intended product shape: a headless Rust streaming-content engine with canonical Content IR, stable identity, lifecycle and recovery, framework-neutral bindings, optional processors, and host-owned rendering and animation policy. The remaining audit findings are not reasons to add another renderer or parser. They expose one cross-cutting invariant that must be made explicit: batching and coalescing may change execution granularity, but they must never erase the original acceptance boundaries required to preserve input, failure, and retry semantics.

This refactor makes that invariant uniform. Rust classifies whether an atomic joined transition may be retried over its original chunks. Tokio, TypeScript, and Dart retain those chunks until acceptance is known. Web pacing preserves authoritative transition order when pending text becomes canonical. Native binding admission applies source-aware bounds before UTF-8 or JSON work. Release tests consume the same Flutter archive that the producer job uploads.

### Problem Frame

The PR audit found several manifestations of the same missing contract:

- Tokio flattens coalesced actor input before knowing whether the joined transition is admissible, so a resource rejection can discard accepted sender input.
- TypeScript and Dart lossless batchers retain chunks but expose only one result and become permanently blocked after the same joined-transition rejection.
- Repeated single-byte Tokio input rescans the growing buffer, turning coalescing into quadratic work and copying owned input before thresholds are known.
- The Web presentation policy can append already-painted pending text ahead of older paced graphemes, producing a correct final byte set in the wrong visible order.
- Native append checks a broad wire budget before decoding, but applies the smaller canonical source budget only after scanning or allocating a much larger input.
- Android smoke cleanup can replace the real failure, subprocesses can hang indefinitely, and release workflows do not execute the exact archive on every supported package-consumer lane.

These are consistency and evidence failures at transport and host boundaries. They do not justify a first-party React renderer, LALRPOP, a second Markdown IR, or moving Merman into the core dependency graph.

### Actors

- A1. A model-stream producer sending arbitrary UTF-8-safe token chunks whose partitioning is not controlled by mdstream.
- A2. A Rust/Tokio service using coalescing to reduce transition overhead while requiring byte-lossless ordered input.
- A3. A TypeScript or Dart host batching model chunks around a canonical native engine.
- A4. A Web host applying its own grapheme pacing, color, opacity, layout, or reduced-motion policy from transition facts.
- A5. A Flutter package consumer installing the exact archive on Android, Windows, iOS, or macOS.
- A6. A maintainer diagnosing resource failures, packaging failures, timeouts, and cleanup behavior from deterministic evidence.

### Requirements

#### Lossless transition semantics

- R1. Any optimization that joins multiple accepted input chunks must retain their original ordered boundaries until the joined transition either commits or is conclusively rejected.
- R2. Rust must expose one closed, typed split-safety classification for transition failures, independent of how many caller chunks produced the attempted text. Only a pre-commit, atomic, transaction-local resource failure for which retrying at existing caller boundaries can change admissibility is split-safe.
- R3. Cumulative document limits, including source and node ceilings, lifecycle errors, malformed input, schema failures, internal invariants, and unclassified future errors are never split-safe. Adapters must also require more than one original constituent before replay, so a single chunk is never recursively subdivided.
- R4. A split-safe joined failure causes at most one ordered pass over the original chunks. Replay never recursively subdivides a constituent. The joined rejection is counted as an internal attempt, not published as a committed result.
- R5. If constituent replay fails after a successful prefix, the successful prefix remains committed and every completed result remains ordered. TypeScript and Dart retain the failing constituent and untouched suffix until explicit retry, take, or discard. The Tokio actor instead terminates execution and returns its engine, original error, completed results, unresolved coalescer state, already queued commands, and ownership of the closed input receiver to the caller. Commands sent through permits reserved before receiver closure remain accepted into that returned receiver and are never misreported as already drained. No finish, reset, recovery, barrier, close, or input-close path may silently cross unresolved input.
- R6. Barrier operations execute only after pending input is empty; otherwise the owning batcher refuses them or the actor returns them unexecuted in its terminal outcome. Sender acceptance, actor receipt, output publication, cancellation, retry, receiver closure, and post-terminal drain behavior must have explicit ownership points so cancellation or an outstanding permit cannot duplicate or erase input or metadata.

#### Performance and admission

- R7. Tokio actor, sender, and receiver coalescing must scan each incoming chunk for newline at most once, move the first owned chunk without copying, and decide byte or constituent-budget flushes before copying the next chunk into a growing buffer.
- R8. Tokio, TypeScript, and Dart must enforce a hard pending-constituent or boundary-metadata budget in addition to pending bytes. Empty chunks affect input-attempt metrics but create no retained boundary. Before accepting a chunk that would exceed the budget, the adapter losslessly flushes or applies backpressure; if that pre-flush fails, the new chunk was not accepted. Coalescing also retains message count, byte count, newline state, deadline, constituent boundaries where semantic replay is supported, and receiver metadata across cancellation. Runtime option changes reevaluate cached state without rescanning the full buffer or losing the original deadline.
- R9. Deterministic work counters must distinguish input attempts, successful appends, committed bytes, pending bytes, pending constituents, boundary-metadata bytes, join-copy bytes, replay count, and scan bytes. Rejected joined work advances attempt/scan/copy counters but not successful-append, committed-byte, or published-result counters. A one-byte-chunk workload must remain near-linear under the established growth-ratio budget.
- R10. Native append admission must use an engine-owned conservative raw-byte ceiling derived from current canonical source capacity and newline-normalizer state. Oversized native bytes are rejected before full UTF-8 decode or generic JSON allocation. TypeScript queries the same ceiling before crossing wasm-bindgen, first rejects when JavaScript UTF-16 length alone proves overflow, then performs a bounded, allocation-free UTF-8 count with early exit. Exact-limit, Unicode, CRLF, and trailing-CR inputs remain admissible whenever precise normalization can accept them.
- R11. There must be one native streaming-content append path. Generic lifecycle/control command transport must not offer a second JSON string append path with different allocation, limit, or error behavior. WASM and FFI remain thin transports over the same engine contract.

#### Binding and host behavior

- R12. TypeScript and Dart batcher operations must return ordered result collections because one logical operation can commit multiple constituent transitions. Each engine grants at most one active batching lease; direct append, finish, reset, recovery, close, and second-batcher creation are rejected for the entire lifetime of that lease. The lease may be released only after pending input commits, is transferred, or is explicitly discarded. Public operation publication still produces one coherent batch-tail host notification.
- R13. TypeScript and Dart must consume Rust's stable replay classification rather than infer replay safety from a generic resource status or language-specific error strings. Metrics and partial-failure behavior must be equivalent across both bindings.
- R14. Explicit pending-input inspection and discard/take recovery operations must replace any implicit reset/finish behavior that could abandon a failed batch. The migration must make data loss an explicit caller decision.
- R15. Every Web paced entry must retain its exact UTF-8 source interval and stable enqueue ordinal. When a canonical projection append represents text already displayed from pending source, the host synchronously commits exactly the causally earlier queue prefix, then commits catch-up text once and leaves later entries paced. Example-local ordered delivery records distinguish fresh projection, causally forced paced prefix, and pending catch-up without exporting animation policy from `@mdstream/core`.
- R16. Correction, removal, full replacement, continuity change, immediate mode, and reduced-motion mode must preserve canonical order while retaining their existing queue-discard or immediate-delivery semantics. Enabling reduced motion during a partially drained queue synchronously commits the remainder once; disabling it never requeues displayed text and affects only future fresh delivery. Hosts remain free to implement color and layout animation from delivery facts; mdstream ships no animation policy.

#### Release evidence and diagnostics

- R17. Every Android smoke subprocess has a phase-appropriate timeout, and the workflow has an outer job timeout. Timeout errors preserve the command phase and bounded diagnostics.
- R18. Cleanup is secondary to the primary smoke result. With an active primary failure, uninstall/cleanup failure is attached without replacing it; cleanup failure alone still fails the smoke test.
- R19. Android, Windows x64, and macOS CocoaPods package-consumer jobs must download and execute the exact Flutter archive uploaded by the producer job. Consumers must not rebuild or silently substitute repository-local native libraries.
- R20. Exact-archive consumers must retain safe extraction, native magic/architecture, ABI, package schema, and binding schema checks. Static workflow tests must prove the producer-consumer dependency and prohibit source-tree fallback.

#### Product boundaries

- R21. The TypeScript package remains framework-neutral and contains no React package, hook, component, CSS, renderer, theme, or animation dependency.
- R22. Markdown continues to use the existing incremental framing and `pulldown-cmark` compiler path. No LALRPOP or second grammar implementation is introduced.
- R23. Merman remains a standalone optional processor on its own Rust toolchain. Mermaid source stays canonical content; generated SVG stays a derived, version-checked, host-sanitized or isolated artifact.

### Key Flows

- F1. A Tokio actor receives chunks that are individually valid but exceed a per-transition operation limit when joined. If its value gate retained semantic joining, it attempts the join atomically, receives the typed split-safe classification, and replays original chunks once. Full replay publishes ordered results before the following finish barrier. A constituent failure terminates the actor and returns engine/coalescer/closed-receiver ownership without executing that barrier; commands admitted by pre-existing permits remain drainable from the returned receiver.
- F2. A TypeScript or Dart batcher holds the engine's only batching lease and replays a joined failure only when joining passed its value gate. If a middle constituent still fails, completed prefix results are returned on the composite error, the failing constituent and suffix remain inspectable, and a later explicit caller action retries, takes, or discards them. Direct engine mutation remains blocked until the lease is released.
- F3. A cumulative source limit rejects joined input. Because Rust marks it non-split-safe, no prefix commits and all pending chunks remain intact.
- F4. Web pacing has older queued graphemes with source intervals and enqueue ordinals when a transition moves already-displayed pending bytes into canonical projection. The policy emits fresh/prefix/catch-up delivery records, delivers the causally earlier queue prefix, installs catch-up bytes exactly once without a fresh-animation marker, and continues pacing only later entries.
- F5. Native FFI receives an input much larger than the remaining canonical source capacity. The source-aware raw ceiling rejects it before UTF-8 scanning or JSON construction and leaves canonical state unchanged for a smaller retry. A Web caller rejects obvious overflow by UTF-16 length and bounded UTF-8 counting before invoking WASM.
- F6. A Flutter archive is produced once. Android, Windows, and macOS CocoaPods jobs download that artifact, safely extract it, validate its native contract, build a minimal consumer, and load or execute the packaged native library without rebuilding it.
- F7. Android build, install, launch, log polling, or uninstall hangs or fails. Inner command timeout and outer job timeout bound the run, and diagnostics report the original failure plus any cleanup note.

### Acceptance Examples

- AE1. Given chunks `a`, `b`, and `c` that individually fit but whose joined transition exceeds a transaction-local operation limit, when Tokio, TypeScript, and Dart flush them, then final canonical source is `abc`, ordered constituent results are observable, pending input is empty, and normalized final IR equals the direct per-chunk baseline.
- AE2. Given the same batch under a cumulative source or node limit, when the joined transition fails, then no constituent is replayed, canonical coordinates are unchanged, and every original chunk remains pending.
- AE3. Given split-safe chunks where the second constituent fails, when fallback runs, then only the first result commits and the composite failure contains it. A TypeScript/Dart batcher retains the second constituent plus suffix for retry/take/discard; a Tokio actor terminates and returns its engine, unresolved input, queued barriers/commands, and closed receiver without duplication or execution. A reserved permit crossing termination can still enqueue exactly once into that returned receiver and is visible to its explicit drain API.
- AE4. Given one constituent whose error is split-safe in isolation, when an adapter flushes it, then cardinality prevents replay, no recursive split or busy loop occurs, and ownership remains stable.
- AE5. Given a Tokio one-byte stream of increasing size, when coalesced without newline, then deterministic scan work grows linearly, pending constituent metadata stays under its hard budget, empty chunks retain no boundary, the first owned allocation is moved, and a threshold-crossing chunk is not copied into the previous buffer before flush.
- AE6. Given receiver coalescing with accumulated message metadata, when a receive future is cancelled and retried, then bytes and metadata are returned exactly once. Given sender-local coalescing cancellation, accepted bytes are neither duplicated nor silently lost.
- AE7. Given paced `abc` followed by already-painted catch-up `def`, when the host policy applies the catch-up transition, then source intervals and enqueue ordinals yield visible `abcdef`, never `defabc`; delivery records mark only eligible fresh/prefix text for host animation and never mark catch-up. Emoji and combining graphemes remain intact after partial drain and across multiple node keys.
- AE8. Given correction, removal, full replacement, immediate mode, or reduced motion, when the same canonical trace is applied, then final rendered content and identity are equal and no already-painted catch-up bytes are reanimated. Enabling reduced motion mid-drain commits the queue once; disabling it paces only future fresh text.
- AE9. Given remaining source capacity at exact, exact-plus-one, Unicode, CRLF, and cross-chunk trailing-CR boundaries, when raw append enters Rust, then conservative preflight never rejects an input precise normalization would accept; true overflow and oversized invalid UTF-8 fail before disproportionate scanning and leave canonical state unchanged. Given an obviously oversized JavaScript string, TypeScript rejects before invoking WASM or encoding the full string.
- AE10. Given Android smoke command timeout, logcat timeout, install failure, cleanup-only failure, or simultaneous primary and cleanup failure, when the harness exits, then runtime is bounded and the primary diagnostic precedence is deterministic.
- AE11. Given the producer's Flutter archive, when Android 16 KiB-page, Windows x64, and macOS CocoaPods consumers run, then each uses the downloaded archive's plugin and native library, validates contract versions and architecture, and performs no native rebuild or source fallback.
- AE12. Given the Golden AI Stream partitioned into adversarial token schedules, when it passes through Tokio, TypeScript, and Dart batchers and the Web host policy, then final canonical snapshots and host-visible text match the unbatched baseline while transition counts may remain schedule-local.
- AE13. Given a 0.4 adopter following the runnable TypeScript or Dart batching example, when it handles `push`, `flush`, and `finish`, then the normal path consumes one ordered collection shape throughout; only a partial failure reads completed results and chooses retry, take, or discard. Both migration guides name every changed method and removed implicit behavior.

### Success Metrics

- No accepted canonical input is lost, reordered, duplicated, or silently discarded in actor, batcher, cancellation, barrier, finish, reset, or recovery paths.
- Rust is the single authority for replay safety, and cumulative resource failures remain atomic in every language.
- One-byte coalescing satisfies the existing deterministic near-linear work ratio without raising budgets; zero-copy ownership assertions cover the first and threshold-crossing owned chunks.
- TypeScript and Dart expose equivalent ordered multi-result, single-lease, bounded-pending, and partial-failure behavior with no hidden pending-data abandonment; runnable examples keep the common migration path compact.
- Web host-policy tests prove paced/catch-up order and factual animation eligibility for ASCII, Unicode, multiple keys, partial drains, mid-stream reduced-motion changes, corrections, and full replacement while architecture tests continue to forbid React.
- Raw native overflow is rejected before full decode/allocation; Web overflow is rejected at the earliest zero-allocation lower bound and then by bounded UTF-8 counting before WASM. All paths produce equivalent typed errors and unchanged canonical state.
- Exact Android, Windows, and macOS CocoaPods archive consumers pass from the producer artifact, and Android hangs cannot exceed configured command/job bounds.
- No verification budget, byte limit, timeout, or artifact size is weakened merely to make the new tests pass.

### Scope Boundaries

#### In scope

- Typed replay classification, original-chunk retention, ordered constituent replay, multi-result batcher APIs, pending-input recovery controls, Tokio coalescing ownership/performance, cancellation metadata, Web paced/catch-up ordering, raw append admission, Android timeout/error precedence, exact archive runtime consumers, migration, changelog, and deletion of superseded paths.

#### Outside this product's identity

- React integration, first-party animation components, CSS, easing, timing, color, transforms, FLIP/layout calculations, scroll anchoring, or grapheme pacing policy beyond the private Web adoption example.
- A new Markdown grammar, LALRPOP, a second parser, reparsing in adapters, or a presentation IR.
- Moving Merman into core/default bindings, treating generated SVG as canonical state, or claiming in-process processor cancellation is a security sandbox.
- Provider message schemas, tool-call/reasoning orchestration, networking, persistence, or a global multi-session token scheduler.
- Physical-device release qualification, package-registry OIDC policy, or host-owned sanitizer/process-isolation implementations beyond documenting their trust boundary.

### Assumptions

- The user authorizes breaking APIs, schema refreeze before the first 0.4 publication, deletion of obsolete code, and intermediate conventional commits.
- The active branch and PR are the implementation target; unrelated concurrent edits remain untouched and are never restored or reset.
- The joined engine attempt is transactional. Focused tests must prove canonical state remains unchanged before constituent replay is allowed; attempt/scan/copy observability counters may advance.
- Version 0.4 has not been published to downstream registries. If registry evidence contradicts this assumption, binding/options schemas advance instead of being silently refrozen.
- Exact-archive helpers can be extracted into a small shared tooling module without weakening path, link, duplicate-entry, native-magic, or dependency checks.
- Local Apple framework incompatibility may leave one Apple smoke observation to macOS CI, but the exact-archive CocoaPods consumer must exist and run in CI.
- No launch-blocking product question remains; exact public symbol names may follow repository conventions while preserving this plan's semantics.

---

## Planning Contract

### Key Technical Decisions

- KTD1. Preserve original chunk boundaries through every join optimization. Flattening is an execution detail, not an ownership transfer, until the joined transition commits.
- KTD2. Define a Rust-owned split-safety classification that describes the error, not caller batch cardinality. It is narrower than a generic resource error and excludes cumulative document limits, lifecycle errors, malformed data, and internal failures. An exhaustive matrix covers every engine, compiler, and protocol resource constructor; new categories must be explicitly classified. Foreign bindings transport this stable meaning instead of maintaining error-string allowlists, then require more than one original chunk before replay.
- KTD3. Gate semantic joining per adapter only after that adapter can run both private joined-first and constituent-first candidates with final deterministic counters. On each named one-byte, bursty, Unicode, CRLF, and Golden AI workload independently, the retained candidate must improve canonical append attempts or encoded result bytes by at least 25%; the other metric and deterministic scan/copy work must each worsen by no more than 20%. Aggregate averages cannot hide a workload regression. An adapter that misses the gate deletes joined-first behavior from production and uses constituent-first canonical appends inside one coherent host operation. An adapter that passes deletes constituent-first behavior from production, attempts one join, and performs at most one original-constituent replay pass under KTD2. A non-published test evaluator remains for both outcomes: it composes the two policies over fresh engines using the same production pending/counter primitives, so CI recomputes the decision without retaining two shipping implementations. Each U2/U3 decision is recorded immediately in the named `CHANGELOG.md` pre-release API migration table before its public recovery surface is finalized.
- KTD4. Make partial fallback explicit. TypeScript and Dart retain failed input behind an active batching lease and expose retry/take/discard. A Tokio actor cannot safely accept an out-of-order resolution command behind an already queued barrier, and closing a Tokio receiver does not invalidate permits reserved before closure. Any accepted append failure therefore terminates execution and returns the engine, error, completed results, unresolved coalescer state, queued commands already available, and the closed receiver through its join outcome. The caller owns a bounded explicit drain API for commands that arrive through outstanding permits; no instantaneous fully drained cutoff is claimed. The breaking change is preferable to an actor that silently crosses or deadlocks a barrier.
- KTD5. Change TypeScript and Dart batcher result surfaces from an optional single result to ordered result collections, and give each engine at most one live batching lease. One public batcher operation may cause multiple engine commits, while one host operation notification still observes the coherent batch tail; direct mutation is rejected until pending input is empty or explicitly transferred/discarded and the lease is released.
- KTD6. Use a private owned Tokio coalescing module for byte count, newline state, message count, deadline, and optional constituent boundaries. Every coalescer/batcher has a hard `max_pending_chunks`-equivalent budget; empty chunks do not occupy it, and exceeding it triggers lossless pre-flush/backpressure before acceptance. Actor, sender, and receiver share scanning/ownership primitives but retain distinct backpressure, replay, and metadata semantics.
- KTD7. Treat cancellation and actor termination as ownership contracts. State moves only at a documented acceptance point; receiver metadata survives cancelled waits, sender retry cannot duplicate a buffer modified before an await, and terminal actor failure returns ownership of every accepted but unprocessed command or of the closed receiver that can still receive from pre-existing permits. Tests cover borrowed and owned permits, cancelled sends, and multiple sender clones across termination.
- KTD8. Use deterministic counters and allocation identity tests rather than timing-only benchmarks for linearity and zero-copy claims. Timing remains supporting evidence because scheduler noise cannot prove algorithmic work.
- KTD9. Give the engine authority over conservative raw admission, including CRLF normalization and pending trailing-CR state. Remove append from the generic JSON command surface so native content has one pre-decode limit and error path. WASM additionally exposes the ceiling without receiving the content first; TypeScript performs a UTF-16 lower-bound check and bounded allocation-free UTF-8 count before passing the string to wasm-bindgen.
- KTD10. Preserve the Web queue's global causal order with exact source intervals plus stable enqueue ordinals. Before synchronous catch-up, commit exactly the queued prefix whose source order precedes it; equal intervals retain enqueue order. Corrections/removals still discard invalidated entries and later entries remain paced.
- KTD11. Keep transition facts factual and host-owned. The private Web example maps them to ordered delivery records with factual origin and animation eligibility, enabling Cherry Studio-style text color, opacity, and layout animation without naming animations, exporting the records from `@mdstream/core`, shipping presentation components, or interpreting reduced-motion preferences in the library.
- KTD12. Make exact-archive execution the release evidence boundary. Static inspection and source-tree smoke tests remain useful, but cannot substitute for loading the producer artifact in Android, Windows, and CocoaPods consumers.
- KTD13. session-settled: continue shipping only framework-neutral `@mdstream/core`; reject a first-party React package or renderer because mature React renderers already exist and the project differentiates through portable state semantics.
- KTD14. session-settled: do not add LALRPOP or a second Markdown parser. The audited defects are downstream transport, host-ordering, and release-evidence problems; the current incremental framing plus `pulldown-cmark` remains the grammar boundary.
- KTD15. session-settled: keep Merman standalone and optional. Mermaid source is canonical typed content, while Merman's SVG is derived, freshness-checked, untrusted output consumed through a host sanitizer or isolation boundary.
- KTD16. user-directed: accept a breaking, deletion-friendly refactor instead of compatibility adapters. Version control and migration documentation preserve history; runtime shims would keep ambiguous loss and result semantics alive.

### High-Level Technical Design

The diagrams define responsibility and ordering, not exact public type or method names.

#### Streaming ownership and replay

```text
model token chunks
        |
        v
owned pending chunks + boundaries + cached scan facts
        |
        +---- value gate failed ----> constituent appends, one host operation
        |
        +---- value gate passed ----> joined StreamEngine transaction
        |                              |
        |                    +---------+----------+
        |                    |                    |
        |                 committed       rejected, typed class
        |                    |                    |
        |                    v          +---------+----------+
        |              ordered result   |                    |
        |                         split-safe +         non-split-safe
        |                         chunks > 1
        |                               |                    |
        +-------------------------------+                    v
                 one ordered constituent pass          retain all pending
                         |
                 +-------+--------+
                 |                |
              all commit     prefix commits, then failure
                 |                |
                 v                v
           clear pending    TS/Dart: retain failure + suffix
                            Tokio: terminate and return ownership
```

The engine classifies split safety independently of batch cardinality. Each adapter combines that class with its retained boundary count, while the value gate decides whether that adapter semantically joins at all.

#### Native admission and host publication

```text
engine-derived conservative raw ceiling
        |
        +----------------------+----------------------+
        v                                             v
native raw bytes                              JavaScript string
length reject before decode             UTF-16 lower-bound reject
        |                                bounded UTF-8 count
        |                                             |
        +----------------------+----------------------+
                               v
                  reject before avoidable allocation
        |
        v
UTF-8 + exact newline normalization preflight
        |
        v
canonical StreamEngine append
        |
        v
Reducer updates -> coherent batch-tail state -> transition callback -> invalidation callback
                                                 |
                                                 v
                                      host pacing / animation / layout
```

Lifecycle/control commands may remain on structured command transport. Streaming content does not pass through a JSON string command variant.

#### Web catch-up ordering

```text
global paced queue: [A1(range, ordinal), B1(range, ordinal), A2(range, ordinal)]
                                  ^
canonical catch-up arrives after older B1 but before later A2

1. commit queue prefix [older A1, older B1]
2. emit prefix delivery records in source/ordinal order
3. commit already-painted catch-up exactly once with catch-up origin
4. never attach a fresh-animation marker to catch-up
5. leave [later A2] paced
6. correction/removal/full-replace may discard invalidated queued entries
```

The host can color or animate newly delivered entries. Catch-up is an identity/order event, not a request to reveal the same bytes again.

#### Exact archive evidence

```text
Flutter package job
        |
        v
one verified archive artifact
        |
        +----------------+------------------+
        v                v                  v
Android 16 KiB      Windows x64       macOS CocoaPods
safe extract        safe extract      safe extract
ABI/schema check    ABI/schema check  ABI/schema check
build + launch      build + load       pod install + build/load
        \                |                  /
         +---------------+-----------------+
                         v
             release evidence for exact bytes
```

### System-Wide Impact

- **Core engine:** Gains a stable distinction between split-safe transition-local rejection and non-split-safe failure, plus a conservative raw admission query/path. Exact append remains transactional and canonical limits retain authority; the engine never guesses caller chunk cardinality.
- **Bindings-core, FFI, and WASM:** Carry the replay classification and route streaming content through one append admission seam. Removing JSON append is a deliberate schema-breaking cleanup; lifecycle command behavior remains structured and thin.
- **Tokio:** Actor output may contain multiple ordered results for one coalesced input. Any accepted append failure closes intake and the join outcome returns engine/input/command ownership rather than waiting forever behind a barrier. Sender/receiver cancellation and metadata behavior become tested contracts instead of incidental implementation.
- **TypeScript and Dart:** Batcher `flush`, pre-flush `push`, `finish`, and `reset` paths can return multiple results or a composite partial failure. A single active lease blocks direct engine mutation for its entire lifetime and may be released only after pending input commits, transfers, or is explicitly discarded. Callers migrate from nullable single-result handling to ordered collection handling and explicit pending recovery.
- **Flutter:** Native Dart semantics change underneath the controller, but controller publication continues to expose one coherent operation batch. Package tooling and CI consume exact archives on more platforms.
- **Web example:** Presentation policy adds source-ordered, example-local delivery records and mid-stream reduced-motion behavior; state authority, transition facts, and freedom to implement motion/color/layout remain unchanged.
- **Errors and observability:** A joined attempt that fails before successful fallback is counted as work but not published as a committed transition. Error precedence preserves the first semantic failure while attaching cleanup diagnostics.
- **Data integrity:** Successful constituent prefixes are real canonical commits and therefore cannot be rolled back by a later constituent failure. Pending ownership makes this explicit and recoverable rather than pretending batch atomicity across multiple engine transactions.
- **Compatibility:** Public batching and command schemas break intentionally. The 0.3-to-0.4 migration remains the major migration; this refactor updates the unreleased 0.4 contract before publication.
- **Security and trust:** Source-aware admission reduces allocation/CPU amplification. Safe archive extraction and native checks remain mandatory. No change claims that Merman or in-process processors are isolated from hostile compute.

### Risks & Dependencies

- **Risk: an overly broad split-safety classification turns an atomic cumulative-limit failure into a partially committed prefix.** Mitigation: define classification in Rust, default every error to non-split-safe, test source/node limits explicitly, and make foreign adapters consume only the typed marker.
- **Risk: retaining every model-controlled boundary amplifies memory before byte thresholds fire.** Mitigation: enforce a hard pending-constituent/metadata budget, ignore empty boundaries, pre-flush before acceptance, and cover one-byte/empty floods with counters and resource tests.
- **Risk: a conservative raw ceiling rejects valid CRLF-heavy input.** Mitigation: derive it from normalizer state, property-test exact/Unicode/CRLF/trailing-CR boundaries against precise normalization, and preserve exact engine preflight as the final authority.
- **Risk: Rust-side admission cannot prevent wasm-bindgen from first encoding a JavaScript string.** Mitigation: export the current ceiling without receiving content, use JavaScript UTF-16 length as a zero-allocation lower bound, then count UTF-8 with bounded early exit before invoking WASM.
- **Risk: partial replay surprises callers that assumed batch atomicity.** Mitigation: ordered result collections, composite errors, explicit pending APIs, migration examples, and no claim of multi-transition rollback.
- **Risk: direct engine calls bypass batcher ownership.** Mitigation: one active engine batching lease, internal mutation capability for its owner, rejection of public mutation/second-batcher creation during the lease, and mirrored release/bypass tests.
- **Risk: sharing Tokio coalescing code erases sender/receiver-specific cancellation semantics.** Mitigation: share only owned buffer/scanning mechanics; retain separate state machines and dedicated cancellation tests.
- **Risk: Web synchronous prefix delivery creates a large frame.** Mitigation: deliver only the causally required prefix, keep later entries paced, expose immediate/reduced modes, and measure deterministic queue work. Correct order takes precedence over a transient frame budget.
- **Risk: inner subprocess timeouts conflict with slow CI machines.** Mitigation: configure phase-specific bounds, include bounded diagnostics, retain a larger outer job deadline, and test timeout classification without relying on wall-clock sleeps.
- **Risk: an archive consumer accidentally imports repository-local plugin code or rebuilds native artifacts.** Mitigation: construct consumers from the extracted archive, sanitize environment/path dependencies, assert producer-job dependency statically, and inspect build logs/package contents for forbidden fallback.
- **Dependency: binding schema can be refrozen only while 0.4 remains unpublished.** Mitigation: repeat registry/tag evidence before final freeze and bump consistently if publication is discovered.
- **Dependency: Apple runtime validation requires compatible macOS/Xcode infrastructure.** Mitigation: keep deterministic static checks local and make the exact CocoaPods consumer a required macOS CI lane with its limitation documented when local execution is unavailable.

### Alternatives Considered

- **Treat every resource error as split-safe:** Simple in adapters, but corrupts cumulative-limit atomicity and lets each language accidentally broaden semantics.
- **Never retry joined transitions:** Preserves atomic rejection but defeats lossless coalescing whenever batching alone creates the resource failure; accepted producer input can remain permanently blocked.
- **Always append constituents and coalesce only host publication:** Removes split recovery and is the fallback selected per adapter when its U2/U3 value gate does not demonstrate enough reduction in engine attempts or encoded result work.
- **Disable joining near all limits:** Avoids one failure mode but requires duplicating engine limit logic and still loses correctness under option changes or other transition-local budgets.
- **Keep a single-result batcher API and expose only the last result:** Minimal migration, but discards ordered transition facts, completed-prefix evidence, and processor/invalidation work from earlier commits.
- **Make batcher fallback fully transactional by snapshot rollback:** Adds expensive state cloning and identity/recovery complexity for a contract that naturally consists of ordered engine transactions.
- **Delay catch-up until the paced queue drains:** Preserves order but makes canonical projection lag behind already-visible pending text and can reintroduce flicker. Committing only the required prefix is both truthful and bounded by causal order.
- **Retain JSON append beside raw append:** Avoids a schema break but leaves two allocation and error-ordering paths. One content path is easier to verify across FFI and WASM.
- **Validate only package file contents:** Fast and deterministic, but cannot prove platform build systems load the uploaded native library. Exact runtime consumers are the required release evidence.
- **Add React helpers, LALRPOP, or core Merman integration while touching the architecture:** None addresses the audited failures; each weakens the already-settled framework-neutral, single-parser, optional-processor boundaries.

### Resolved During Planning

- Replay fallback is permitted only by a Rust-owned typed split-safety classification plus an adapter-owned `constituent_count > 1` check, never by generic status code alone.
- Cumulative source/node limits are non-split-safe even if constituent chunks might allow a prefix to commit.
- Constituent fallback is one pass over original boundaries and never recursively splits.
- TypeScript and Dart use ordered result collections, one active engine batching lease, and explicit pending recovery rather than compatibility wrappers.
- Tokio returns terminal ownership after an accepted append failure; it does not add resolution commands that could sit behind an already queued barrier.
- Web catch-up uses source intervals plus enqueue ordinals to commit the causally prior global queue prefix; limiting this to the same node key or raw array order would still permit cross-node transition reordering.
- Streaming append leaves the generic JSON command surface; raw admission is engine-owned and binding transports stay thin.
- Exact Flutter archive consumption includes Android, Windows, and macOS CocoaPods rather than treating static archive inspection as equivalent evidence.

### Deferred to Implementation

- Exact public type and method names may be adjusted to existing naming conventions, provided replay classification and pending ownership remain stable and identical across languages.
- The conservative raw ceiling formula may be represented as remaining bytes, an admission object, or a preflight method; property equivalence with precise normalization determines the simplest safe API.
- Tokio's private coalescing module may store constituent `String` values or compact boundary offsets around a joined allocation. The chosen representation must satisfy move/scan counters and replay ownership tests.
- Existing archive extraction helpers may move into a shared tool module if that eliminates duplication without creating private-import cycles or weakening policy checks.

---

## Implementation Units

### U1. Define split-safe failure and raw admission in Rust

- **Goal:** Make Rust the single authority for split safety and raw admission, and remove the duplicate JSON append route.
- **Requirements:** R2-R4, R9-R11, R13, R21-R23.
- **Dependencies:** None.
- **Files:** `mdstream/src/engine/input.rs`, `mdstream/src/engine/lifecycle.rs`, `mdstream/src/engine/mod.rs`, `mdstream/src/lib.rs`, `mdstream/tests/resource_limits.rs`, `mdstream/tests/engine_lifecycle.rs`, `mdstream/benches/`, `mdstream-bindings-core/src/commands.rs`, `mdstream-bindings-core/src/engine.rs`, `mdstream-bindings-core/src/errors.rs`, `mdstream-bindings-core/tests/session.rs`, `mdstream-ffi/src/handles.rs`, `mdstream-ffi/src/lib.rs`, `mdstream-ffi/tests/abi.rs`, `mdstream-wasm/src/lib.rs`, `mdstream-wasm/tests/wasm.rs`, `bindings/typescript/src/wasm.ts`.
- **Approach:** Characterize transactional rejection, then build an authoritative exhaustive matrix for every current engine, compiler, and protocol resource error: scope, pre-commit atomicity, whether existing caller boundaries can change admissibility, and split class. Encode the class at typed error construction/mapping sites rather than field strings; exhaustive matching forces future categories to choose. Add an engine-owned conservative raw admission path that checks native length before UTF-8 and delegates to exact newline/source preflight. Export its current ceiling without content so TypeScript can reject by UTF-16 lower bound and bounded UTF-8 count before wasm-bindgen. Remove content append from the generic JSON command enum and its dead decoding/tests; keep lifecycle commands structured and FFI/WASM thin. U1 defines the counters and comparison contract required by KTD3 but makes no adapter keep/delete decision from pre-refactor behavior.
- **Patterns to follow:** Typed `EngineError` and `BindingError` mapping, transactional preflight in the engine input path, strict options/command schema tests, and thin FFI/WASM transport tests.
- **Test scenarios:** Joined-versus-constituent witnesses for every split-safe matrix row; non-split tests for every cumulative/compiler/protocol/lifecycle/internal row; adapter cardinality is not part of the Rust class; raw exact limit and limit-plus-one; ASCII, multibyte Unicode, CRLF compression, cross-chunk trailing CR, empty chunk, terminal state, overflow then smaller retry; obviously oversized JavaScript rejected without WASM call/full encoding; bounded ambiguous JavaScript counted then rejected; bindings-core/FFI/WASM error and canonical-state parity; removed JSON append rejected as unknown command.
- **Verification:** Focused suites prove classification is exhaustive and typed, raw preflight never rejects a precisely admissible input, and true native overflow is rejected before full decode. On rejection, canonical engine/reducer state and pending ownership remain unchanged; attempt/scan/failed-join counters advance deterministically while successful-append, committed-byte, and published-result counters do not. The KTD3 counter contract is runnable by U2/U3, and no UI/parser/Merman dependency enters the affected crates.

### U2. Rebuild Tokio coalescing around owned pending chunks

- **Goal:** Eliminate actor data loss and quadratic scanning while making actor, sender, and receiver ownership/cancellation behavior explicit.
- **Requirements:** R1-R9.
- **Dependencies:** U1.
- **Files:** `mdstream-tokio/src/coalesce.rs`, `mdstream-tokio/src/options.rs`, `mdstream-tokio/src/actor.rs`, `mdstream-tokio/src/receiver.rs`, `mdstream-tokio/src/sender.rs`, `mdstream-tokio/src/stats.rs`, `mdstream-tokio/src/lib.rs`, `mdstream-tokio/tests/glue.rs`, `mdstream-tokio/tests/backpressure.rs`, `mdstream-tokio/tests/actor.rs`, `mdstream-tokio/examples/agent_tui.rs`, `mdstream-tokio/README.md`, `CHANGELOG.md`.
- **Approach:** Introduce one private owned pending-chunk module that caches byte/message/newline/deadline facts, enforces byte and constituent-metadata budgets, ignores empty retained boundaries, and can preserve original boundaries. Move the first owned chunk, preflight thresholds before acceptance/join, and scan only new input. Implement private joined-first and constituent-first candidates over the final counters, run KTD3 per named workload on fresh engines, record the decision in `CHANGELOG.md`, and delete the losing production path before finalizing recovery behavior. Keep a non-published evaluator in the workload tests that recomposes both policies from production pending/counter primitives so the gate remains reproducible. If joining survives, a split-safe join replays only when boundary count exceeds one. Any accepted append failure closes intake and returns an actor exit value owning the engine, original error, committed prefix results, unresolved coalescer state, commands already queued, and the closed receiver. Its explicit drain API exposes commands that can still arrive through permits reserved before closure without executing them. Receiver keeps accumulated metadata in cancellation-safe state; sender preserves its accepted-input boundary across awaits. Runtime option updates reevaluate cached facts without resetting deadlines or rescanning content.
- **Patterns to follow:** Existing Tokio channel/backpressure state machines, actor ordered output, sender cancellation regression tests, and deterministic work counters used elsewhere in mdstream.
- **Test scenarios:** AE1-AE6; newline, byte, constituent-budget, deadline, and channel-close flush; one-byte and empty floods; oversized first/later chunks; lowered byte/constituent thresholds and newline policy while buffered; successful barrier/finish ordering; failure before any commit; failure after committed prefix; accepted appends and barriers queued behind failure returned unexecuted; borrowed permit and `OwnedPermit` reserved before failure; cancelled send and multiple sender clones across failure; post-terminal receiver drain; input close; output cancellation; receiver cancelled wait retains bytes/message count; sender cancelled flush/send does not duplicate; first/standalone owned allocation retained; one-byte scan work linear.
- **Verification:** Before public recovery behavior is finalized, the Tokio workload harness records per-workload append attempts, encoded result bytes, scan work, copy work, and the KTD3 keep/delete decision in `CHANGELOG.md`. Tokio nextest on Rust 1.88 passes, successful final source/IR matches direct per-chunk baselines, terminal failures return complete ordered ownership including the closed receiver, deterministic counters meet existing budgets, and actor/receiver/sender contain no duplicate growing-buffer newline scans or superseded join helpers.

### U3. Redesign TypeScript and Dart lossless batchers

- **Goal:** Give foreign hosts ordered multi-result fallback, an enforceable single-owner batching lease, bounded pending metadata, and identical replay/error semantics.
- **Requirements:** R1-R5, R8-R14.
- **Dependencies:** U1.
- **Files:** `bindings/typescript/src/engine.ts`, `bindings/typescript/src/store.ts`, `bindings/typescript/src/wasm.ts`, `bindings/typescript/src/index.ts`, `bindings/typescript/examples/lossless-batching.mjs`, `bindings/typescript/tests/batching.test.ts`, `bindings/typescript/tests/host_transitions.test.ts`, `bindings/typescript/tests/workload.test.ts`, `bindings/dart/lib/src/batching.dart`, `bindings/dart/lib/src/engine.dart`, `bindings/dart/lib/src/errors.dart`, `bindings/dart/lib/src/options.dart`, `bindings/dart/lib/mdstream.dart`, `bindings/dart/example/lossless_batching.dart`, `bindings/dart/test/batching_test.dart`, `bindings/dart/test/runtime_test.dart`, `bindings/dart/test/workload_test.dart`, `CHANGELOG.md`.
- **Approach:** Replace nullable single-result flush surfaces with ordered collections and add a configurable hard pending-constituent budget beside bytes. Empty chunks are measured but not retained. One live batcher acquires an engine lease and an internal mutation capability; direct mutation and second-batcher creation fail for the lease's entire lifetime, and release is allowed only after pending input commits, transfers, or is explicitly discarded. Implement private joined-first and constituent-first candidates over aligned final counters, run KTD3 independently for TypeScript and Dart on fresh engines, record both decisions in `CHANGELOG.md`, and delete each losing production path before finalizing public recovery behavior. Keep non-published evaluators in the workload tests that recompose both policies from production pending/counter primitives so the gates remain reproducible. Where joining survives, keep chunks through the joined attempt, combine Rust split safety with `count > 1`, replay once, remove each prefix only after commit, and attach completed results to a composite failure. Otherwise append constituents directly inside the same operation runner. Add pending inspect/retry/take/discard and refuse lifecycle operations while unresolved. Apply the Web ceiling check before WASM. Include rejected join work in counters and add structurally parallel runnable migration examples.
- **Patterns to follow:** Existing `BatchOperationError`/`BatchOperationException.completedResults`, TypeScript `DocumentOperationRunner`, Dart ordered native result draining, and binding workload fixtures.
- **Test scenarios:** AE1-AE4 and AE9, AE12-AE13; zero/one/many results; byte/constituent pre-flush during push; failed pre-flush does not accept the new chunk; one-byte/empty flood; flush/finish/reset/recovery after full, partial, and failed work; cumulative limit never replays; single chunk does not loop; prefix removed once; failure plus suffix retained; inspect/retry/take/discard; direct append/finish/reset/recovery/close and second batcher rejected during lease; lease release and reacquire; obvious Web overflow never invokes WASM; one transition callback sees all committed results and coherent tail; processor requests/invalidation preserved; runnable examples compile/run; TS/Dart metrics and errors match.
- **Verification:** Before public recovery behavior is finalized, the TypeScript and Dart workload harnesses record per-workload append attempts, encoded result bytes, scan work, copy work, and their KTD3 keep/delete decisions in `CHANGELOG.md`. TypeScript typecheck/test/build and Dart native/analyze suites pass. Golden fixtures and runnable examples prove ordered results, lease/pending state, errors, counters, migration ergonomics, and final snapshots agree across both languages and direct Rust execution.

### U4. Repair Web paced catch-up ordering

- **Goal:** Preserve causal text order while keeping animation, pacing, layout, and reduced-motion policy fully host-owned.
- **Requirements:** R15-R16, R21.
- **Dependencies:** None.
- **Files:** `examples/web/src/host-policy.ts`, `examples/web/src/host-state.ts`, `examples/web/src/content-ir-view.ts`, `examples/web/src/styles.css`, `examples/web/tests/host-policy.test.ts`, `examples/web/tests/golden-stream.spec.ts`, `bindings/typescript/tests/architecture.test.ts`.
- **Approach:** Give every segmented queue entry its exact UTF-8 source interval and stable enqueue ordinal. Refactor queue mutation into source-ordered prefix-commit and invalidation-drop operations. Emit example-local ordered delivery records carrying continuity-qualified key, exact range/text, sequence, and factual origin for fresh projection, forced paced prefix, and pending catch-up. Only animation-eligible fresh/prefix records drive the example's transient text-color marker; catch-up never does. Preserve correction/removal/full-replace invalidation. Enabling reduced motion mid-drain commits the remainder once and keeps future delivery immediate; disabling it re-enables pacing only for future fresh text. Keep all delivery/presentation policy private to the example.
- **Patterns to follow:** Current transition-fact host policy, `Intl.Segmenter` grapheme grouping, batch-tail publication order, and architecture dependency prohibitions.
- **Test scenarios:** AE7-AE8 and AE12; queued `abc` plus catch-up `def`; partial `a` drain; multiple node keys whose node-ID order differs from source order; equal interval ordinal tie; emoji, combining marks, split graphemes; delivery records and fresh-color eligibility; correction/removal/continuity/full replacement; reentrant drain; enable reduced motion mid-queue, then disable without replay; settled Golden screenshot/text assertions.
- **Verification:** Focused policy/browser tests show no `defabc`, cross-node reorder, duplicate reveal/color marker, or identity/content divergence. Architecture tests prove delivery records remain example-local and `@mdstream/core` exports no React, CSS/motion package, animation, or renderer surface.

### U5. Harden Android smoke execution and diagnostics

- **Goal:** Bound every Android smoke phase and preserve the primary failure when cleanup also fails.
- **Requirements:** R17-R18.
- **Dependencies:** None.
- **Files:** `bindings/flutter/tool/android_smoke.py`, `bindings/flutter/tool/package_smoke.py`, `bindings/flutter/tool/test_packaging.py`, `.github/workflows/flutter-platforms.yml`.
- **Approach:** Extract or reuse a small subprocess runner with phase-specific timeouts and bounded command diagnostics. Convert timeout expiry into the established package-smoke error hierarchy. Track whether install succeeded before uninstall. During cleanup, propagate a cleanup-only failure but attach cleanup diagnostics to an existing primary exception without replacing its type, message, or traceback. Add a larger workflow job timeout as a second boundary.
- **Patterns to follow:** Timeout and diagnostic handling already used by `package_smoke.py`, Python exception notes/chaining supported by the workflow runtime, and mocked packaging tool tests.
- **Test scenarios:** Build timeout; adb install/launch timeout; hung logcat/poll timeout; install failure performs no uninstall; successful primary plus cleanup failure; primary failure plus cleanup failure; command output truncation; configured phase timeout reaches subprocess; workflow outer timeout exceeds every legitimate inner phase sequence.
- **Verification:** Python packaging tests deterministically inject timeout/failure without sleeping, Android smoke diagnostics retain primary precedence, and workflow static tests confirm the outer bound.

### U6. Execute the producer's exact Flutter archive on every package lane

- **Goal:** Turn Android, Windows, and macOS CocoaPods from source/static checks into consumers of the exact release candidate archive.
- **Requirements:** R19-R20.
- **Dependencies:** U5.
- **Files:** `scripts/archive_policy.py`, `bindings/flutter/tool/package_smoke.py`, `bindings/flutter/tool/android_smoke.py`, `bindings/flutter/tool/test_packaging.py`, `scripts/verify-packages.py`, `scripts/test_verify_packages.py`, `.github/workflows/flutter-platforms.yml`, `RELEASE_CHECKLIST.md`.
- **Approach:** Reuse one safe archive-source context across package consumers. Add archive input to Android smoke and construct all consumers from extracted package paths. Wire Android 16 KiB, Windows x64, and macOS CocoaPods jobs to depend on the producer, download the same artifact, validate native/schema contracts, and prohibit native rebuild or repository-local Flutter plugin/native fallback. A local override for the unpublished Dart dependency remains allowed during PR CI. Extend the static verifier to inspect workflow dependency and exact-archive arguments without duplicating extraction policy.
- **Patterns to follow:** Existing Linux and SwiftPM exact-archive consumers, archive traversal/link/duplicate-entry protections, native magic/dependency verification, and package workflow contract tests.
- **Test scenarios:** AE11; valid archive on all three lanes; absolute/parent path, symlink, hardlink, duplicate, wrong architecture/magic, missing library, ABI mismatch, package/binding schema mismatch; repository-local fallback rejected; consumer job missing producer dependency rejected; logs prove no native build step; archive cleanup on success and failure.
- **Verification:** Tooling unit tests and static verifier pass locally; CI consumers build/load the exact producer bytes on Android, Windows, and CocoaPods; no archive budget or security check is relaxed.

### U7. Complete conformance, migration, simplification, and release review

- **Goal:** Prove the refactor as one cross-stack contract, delete superseded behavior, document migration, and leave the PR fully reviewed and release-ready.
- **Requirements:** R1-R23.
- **Dependencies:** U2-U6.
- **Files:** `mdstream-conformance/src/lib.rs`, `mdstream-conformance/tests/transition_contract.rs`, `mdstream-conformance/tests/protocol_fixtures.rs`, `conformance/fixtures/adoption/headless-rich-content.json`, `docs/ARCHITECTURE.md`, `docs/PERFORMANCE.md`, `docs/ADAPTERS.md`, `docs/ADR_0002_PROJECTION_FRONTIER.md`, `docs/ADR_0005_HOST_TRANSITION_FACTS.md`, `README.md`, `CHANGELOG.md`, `mdstream-ffi/README.md`, `bindings/typescript/README.md`, `bindings/dart/README.md`, `bindings/dart/CHANGELOG.md`, `bindings/flutter/README.md`, `bindings/flutter/CHANGELOG.md`, `scripts/check-registry-version.py`, `scripts/test_verify_packages.py`, `RELEASE_CHECKLIST.md`.
- **Approach:** Extend the existing provider-free Golden AI Stream fixture rather than creating a second scenario; add Tokio, TypeScript, Dart, and Web batching/error/counter expectations while normalizing only schedule-independent final state. Consolidate replay/error/counter fixtures, run exhaustive bounded UTF-8 partitions plus seeded adversarial schedules, and retain schedule-local transition observations. Update public docs for multi-result batching, single-lease ownership, runnable migration paths, animation delivery records, source admission, and archive evidence. Add a backward-compatible `audit-workspace VERSION --root PATH --remote NAME` mode to `scripts/check-registry-version.py`: it inventories every publishable Rust package through the repository package contract, includes `@mdstream/core`, `mdstream`, and `mdstream_flutter`, checks local exact refs plus authoritative remote `VERSION` and `vVERSION` tags with `git ls-remote`, emits one complete deterministic evidence report, continues all probes after an individual error, returns an error if any registry or remote probe is indeterminate, and reports present/missing without treating either definitive state as a transport failure. Run `python3 scripts/check-registry-version.py audit-workspace 0.4.0 --root . --remote origin` before refreezing; if any local/remote tag or 0.4 package exists, advance binding/options versions consistently and refresh fixtures/migration. Preserve the legacy registry CLI's 0/1/2 exit contract and test shallow-clone missing-local-tag, remote-present-tag, remote failure, and complete-report behavior. Verify the U2/U3 `CHANGELOG.md` semantic-join decisions and complete the same named pre-release API migration table with every removed symbol/path. Delete only superseded implementations and compatibility shims made obsolete by this plan, then run full simplification and independent code review.
- **Patterns to follow:** Existing conformance trace normalization, Golden AI Stream fixtures, version-freeze documentation, architecture dependency scans, changelog migration table, and checked-in deterministic budgets.
- **Test scenarios:** All AE1-AE13 across direct and optimized schedules; empty/one-byte/Unicode/CRLF inputs; operation/source/node/constituent limits; finish/reset/recovery/barriers; cancellation and actor terminal ownership; batcher lease bypass/release; processor/invalidation publication after fallback; Web immediate/paced/reduced and runtime-toggle parity; malformed cross-binding facts; exact archive contract; old single-result and JSON append APIs absent except the named migration table; no React/LALRPOP/default-Merman dependency.
- **Verification:** Registry/tag evidence and the resulting version decision are recorded before fixture freeze. Every gate below passes or has only the narrow reproducible local Apple exception; changelogs distinguish the unreleased 0.4 refinement from the 0.3 migration; full-depth simplification and multi-lens PR review leave no unresolved actionable finding; task-owned commits are pushed to the existing PR branch and CI is green.

---

## Verification Contract

| Gate | Command | Applies |
|---|---|---|
| Formatting | `cargo fmt --all -- --check` | All Rust units |
| Rust lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | U1-U3, U7 |
| Rust workspace tests | `cargo nextest run --workspace --all-features` | U1-U3, U7 |
| Rust docs | `cargo test --workspace --all-features --doc` | U1-U3, U7 |
| Core MSRV | `cargo +1.85.0 nextest run -p mdstream-protocol -p mdstream-processors -p mdstream -p mdstream-bindings-core -p mdstream-ffi --all-features` | U1, U7 |
| Tokio MSRV | `cargo +1.88.0 nextest run -p mdstream-tokio --all-features` | U2, U7 |
| Conformance | `cargo nextest run -p mdstream-conformance --all-features` | U1-U4, U7 |
| WASM target | `pnpm wasm:check` | U1, U3, U7 |
| WASM runtime | `pnpm wasm:test` | U1, U3, U7 |
| TypeScript tests | `pnpm --dir bindings/typescript test` | U3-U4, U7 |
| TypeScript types | `pnpm --dir bindings/typescript typecheck` | U3-U4, U7 |
| TypeScript package | `pnpm --dir bindings/typescript build` | U3-U4, U7 |
| Web host tests | `pnpm --dir examples/web test` | U4, U7 |
| Web browser Golden | `pnpm --dir examples/web test:e2e` | U4, U7 |
| Binding artifact budgets | `pnpm artifacts:check` | U1, U3, U7 |
| Dart native suite | `(cd bindings/dart && dart run tool/test_native.dart)` | U3, U7 |
| Dart analyze | `(cd bindings/dart && dart analyze)` | U3, U7 |
| Flutter tests | `(cd bindings/flutter && flutter test)` | U5-U7 |
| Flutter analyze | `(cd bindings/flutter && flutter analyze)` | U5-U7 |
| Packaging tooling | `python3 -m unittest bindings/flutter/tool/test_packaging.py scripts/test_verify_packages.py` | U5-U7 |
| Package static validation | `python3 scripts/verify-packages.py --phase static` | U6-U7 |
| Semantic-join value gates | U2: `cargo +1.88.0 nextest run -p mdstream-tokio --test glue value_gate`; U3: `pnpm --dir bindings/typescript test -- tests/workload.test.ts` and `(cd bindings/dart && dart run tool/build_native.dart && MDSTREAM_REQUIRE_NATIVE=1 dart test test/workload_test.dart)`; each named workload satisfies KTD3 and records its keep/delete decision before public recovery behavior is finalized | U2-U3 |
| Version-freeze evidence | `python3 scripts/check-registry-version.py audit-workspace 0.4.0 --root . --remote origin` checks local and authoritative remote Git tags plus every publishable 0.4 Rust, npm, Dart, and Flutter package; its complete definitive result is recorded in the changelog migration table | U7 before schema/fixture freeze |
| Workflow lint | `actionlint` | U5-U7 |
| Merman boundary | `cargo +1.95.0 nextest run --manifest-path mdstream-merman/Cargo.toml --all-features` | U7 |
| Fuzz compile | `cargo check --manifest-path fuzz/Cargo.toml --bins` | U1-U2, U7 |
| Benchmark compile | `cargo check -p mdstream --benches --all-features` | U1-U2, U7 |
| Diff integrity | `git diff --check` | Every unit and final tail |

Focused red/green tests precede broad gates. Exact Android, Windows, and CocoaPods archive execution is CI-required. A local Apple framework mismatch may be recorded only with the exact failing command and environment evidence; it does not waive the CI consumer or any other locally supported gate.

---

## Definition of Done

- Every R-ID and AE-ID is traced to a passing focused, conformance, binding, host, packaging, or documentation test.
- Tokio, TypeScript, and Dart retain original chunk boundaries until acceptance is known, enforce byte plus constituent-metadata budgets, ignore empty retained boundaries, and never lose, duplicate, reorder, or silently discard canonical input.
- Rust exposes one exhaustive split-safety classification independent of caller cardinality; adapters also require multiple constituents before one original-boundary replay, while cumulative limits remain atomic and non-split-safe.
- U2 and U3 record the semantic-join value gate after final counters exist and before public recovery behavior is finalized. Joining survives only where every named workload clears the Pareto benefit/regression thresholds; other adapters append constituents inside one coherent host operation and delete unused recovery machinery.
- Partial constituent failure commits and reports only the successful prefix. TypeScript/Dart retain the failure and suffix behind an exclusive engine lease for explicit retry/take/discard; Tokio terminates and returns engine, input, completed-result, and queued-command ownership without executing a barrier.
- TypeScript and Dart publish ordered result collections and equivalent composite errors, single-lease/pending controls, transition batches, processor requests, invalidations, deterministic metrics, and runnable common-path migration examples.
- Tokio coalescing scans each chunk once, moves owned chunks where promised, preserves deadline/message metadata across option changes and cancellation, returns complete terminal ownership including a closed receiver for outstanding permits, and satisfies established near-linear work budgets without weakening thresholds.
- Web paced/catch-up order uses source intervals and enqueue ordinals and is correct for ASCII, Unicode, partial drains, multiple keys, correction, removal, full replacement, immediate, steady reduced-motion, and runtime preference changes. Example-local delivery records enable host color/layout animation while catch-up bytes never receive fresh-animation eligibility.
- Native append applies source-aware raw admission before disproportionate UTF-8/JSON work; TypeScript applies the earliest UTF-16 lower bound and bounded UTF-8 count before WASM. All paths preserve CRLF/trailing-CR correctness, use one content append contract, and return equivalent canonical-state-preserving errors.
- Android smoke has inner phase timeouts and an outer job bound, performs cleanup only after successful install, preserves primary error identity, and reports cleanup-only failure.
- Android 16 KiB, Windows x64, and macOS CocoaPods execute the producer's exact Flutter archive with safe extraction, native architecture/magic/dependency, ABI, package schema, and binding schema verification and no native rebuild/source fallback.
- Repository tags plus package-registry checks prove whether 0.4 schemas may be refrozen; any discovered publication advances binding/options versions consistently before fixtures or migration text freeze.
- Public docs, runnable examples, and changelogs explain the breaking batcher/command migration, exclusive lease and explicit pending ownership, animation-extension boundary, and release evidence without promising React, UI policy, a second parser, or core Merman.
- `@mdstream/core` remains framework-neutral; architecture checks reject React/renderer/CSS/motion dependencies. Markdown adds no LALRPOP. Merman remains standalone Rust 1.95 optional derived processing.
- The named `CHANGELOG.md` pre-release API migration table records the join value-gate decisions and every deleted file/symbol, limited to obsolete JSON append, single-result batching, duplicated coalescing, abandoned recovery helpers, or compatibility paths superseded by this plan. Unrelated user work is untouched.
- All Verification Contract gates pass. Only the documented local Apple framework mismatch may carry an exact environment exception; it never waives required CI. Deterministic budgets are not weakened, and full simplification plus independent PR review leave no unresolved actionable finding.
- Task-owned changes are split into precise Conventional Commits, pushed to `refactor/streaming-content-engine`, reflected in the existing PR, and required CI is green.

---

## Appendix

### Sources and Research

- PR audit of the branch at commit `369496449aa34e205d1d4c9f3b93bf66a71f7870`, including focused reproductions for Web queue overtaking, Tokio joined-limit input loss, and cumulative one-byte coalescing scans.
- `docs/ARCHITECTURE.md`, `docs/PERFORMANCE.md`, and `docs/ADAPTERS.md` for headless ownership, transactional rejection, typed limits, lossless transport, and host policy.
- `docs/ADR_0002_PROJECTION_FRONTIER.md` for source/projection/artifact ownership and rejection of a second parser.
- `docs/ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md` for the no-React package boundary.
- `docs/ADR_0005_HOST_TRANSITION_FACTS.md` for renderer-neutral transition facts and host-owned animation policy.
- `docs/plans/2026-07-14-001-refactor-streaming-content-engine-plan.md` for the completed 0.4 protocol, lifecycle, binding, processor, conformance, and release foundation.
- `docs/plans/2026-07-19-001-refactor-host-transition-extension-contract-plan.md` for transition facts, animation extension points, AI part ownership, no-LALRPOP, and optional-Merman decisions.
- Existing Tokio actor/sender/receiver tests, TypeScript and Dart lossless batcher tests, Web Golden AI Stream host policy, Flutter archive policy, and package workflow contracts.

### Traceability

| Requirement group | Primary units | Acceptance evidence |
|---|---|---|
| R1-R6 lossless replay and ownership | U1-U3 | AE1-AE4, AE6, AE12-AE13 |
| R7-R9 coalescing performance/cancellation | U2-U3 | AE5-AE6, AE12 |
| R10-R11 raw admission | U1, U3 | AE9 |
| R12-R14 foreign batchers | U3 | AE1-AE4, AE12-AE13 |
| R15-R16 Web host ordering | U4 | AE7-AE8, AE12 |
| R17-R18 Android diagnostics | U5 | AE10 |
| R19-R20 exact archive | U6 | AE11 |
| R21-R23 product boundaries | U1, U4, U7 | Architecture, dependency, migration, and Merman gates |
