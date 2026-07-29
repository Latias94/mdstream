# ADR 0005: Host Transition Facts

- Status: Accepted
- Date: 2026-07-19
- Decision owners: mdstream maintainers

## Context

Canonical Content IR, stable node identity, deterministic versions, and
`ChangeImpact` let a host render the latest document efficiently. They do not
tell a host whether one invalidated node was inserted, corrected, stabilized,
moved, or removed. Reconstructing those distinctions requires every adapter to
retain old node and resource views plus a complete parent index. That duplicates
knowledge already available at the reducer's atomic commit boundary.

Streaming presentation also contains policy that does not belong in a content
engine. Grapheme pacing, color, opacity, easing, geometry, scrolling, reduced
motion, and framework component lifecycles depend on host state. Streamdown and
Cherry Studio demonstrate useful presentation policies, but copying those
policies into mdstream would couple the core to a renderer. Incremark reinforces
the need for incremental consistency, not a shared UI implementation.

## Decision

mdstream exposes optional renderer-neutral transition facts beside
`ChangeImpact`.

1. Transition capture is disabled by default. The ordinary reducer remains the
   canonical low-cost path and performs no facts-specific visits, copies, or
   serialization.
2. A capture-enabled reducer emits at most one atomic fact set for each
   document-changing commit. Invalid, idempotent, stale, recovery-required, and
   control-only same-floor recovery outcomes emit no facts.
3. The closed wire contract is `mdstream.transitions/1`. Unknown fields and
   variants are rejected. Any additive or semantic change requires a new
   transition subprotocol version.
4. Node and resource keys include reducer-local continuity generation, epoch,
   and canonical identity. Advanced recovery and document replacement increment
   the generation and emit only a coarse `full_replace` fact. Same-floor
   recovery preserves the generation and pending processor work.
5. Continuous facts contain before/after document stamps and bounded facts for
   changed nodes, normalized child-list splices, and changed resources. A
   source-backed semantic append owns only its exact appended text. Pending raw
   source remains an on-demand view and is never copied into every fact set.
6. Facts are schedule-local observations, not a replay log. Legal chunk
   schedules must converge to the same final canonical state, but they may emit
   different intermediate fact sequences. A binding operation that contains
   multiple facts exposes only its batch-tail canonical views.
7. TypeScript and Flutter publish one ordered operation batch after tail state
   and focused values are coherent and before ordinary invalidation listeners.
   Capture-enabled no-op, error, artifact-only, and same-floor operations publish
   a new empty batch so hosts cannot mistake an old batch for a new trigger.
   Mutating callback reentry is rejected.
8. The C ABI and WASM remain thin transports. Transition facts use the existing
   reducer-update payload kind; schema probes fail incompatible native/package
   combinations before a session is used.

`ChangeImpact` remains the compact cache-invalidation contract. Content IR
remains the canonical semantic state. Processor artifacts remain derived,
version-checked state outside both Content IR and transition facts.

## Host Boundary

Hosts may map facts to immediate updates, announcements, reveal effects,
cross-fades, layout measurement, or other policies. mdstream does not expose
animation names, timing, colors, CSS, geometry, renderer registries, framework
components, or scroll behavior. Motion and color must never be the only way a
host communicates a semantic distinction; an immediate or reduced-motion path
must preserve the same content and state meaning.

AI message envelopes are also host-owned. Each Markdown part uses an independent
generation-qualified mdstream session. Part ordering, tools, attachments,
reasoning visibility, pacing, and cross-part layout remain application state.

Custom syntax follows four separate extension planes:

1. sealed parser declarations;
2. typed custom Content IR;
3. versioned processor artifacts;
4. host display dispatch.

Merman remains an optional standalone processor. Mermaid source is canonical
code content; SVG is an untrusted derived artifact and stays opaque until a
named sanitizer or isolated-renderer boundary.

## Value Gate

The Rust-to-TypeScript command-line host was required to pass before `/1` and
mobile bindings were frozen. The accepted implementation:

- classified pending catch-up, append, correction, structure changes, removal,
  finish, reset, and advanced recovery through real WASM;
- produced equal canonical results in paced and immediate host modes;
- derived transitions without retaining old canonical node views or a complete
  parent index, while the comparison baseline retained both;
- materialized zero node views before an explicit visible read;
- kept React, rendering, animation, and Merman dependencies out of the default
  TypeScript/WASM package; and
- passed the existing raw, stripped, gzip, Brotli, and npm archive ceilings
  without raising a budget.

Failure of this gate would have removed or redesigned the draft rather than
freezing a broader presentation API.

## Version Freeze Evidence

The latest repository tag at the decision date was `v0.3.0`. Direct registry
probes on 2026-07-19 returned HTTP 404 for version 0.4.0 of `mdstream`,
`mdstream-protocol`, `mdstream-processors`, `mdstream-bindings-core`,
`@mdstream/core`, Dart `mdstream`, and `mdstream_flutter`. Binding and options
schema 0.4 could therefore be refrozen before their first publication. Content
IR remains `mdstream.content/0.4`, the C ABI remains version 1, and transition
semantics evolve independently through `mdstream.transitions/*`.

## Consequences

Framework adapters can implement streaming text and layout behavior without
reparsing Markdown or retaining an old canonical document. The same contract is
usable from React, GPUI, egui, Flutter, and other hosts.

Capture-enabled sessions pay bounded work proportional to changed entities and
reported splice members, plus encoded fact bytes. Consumers that only need
correct latest state should leave capture disabled.

Continuity-qualified keys and request generations remain distinct. Returning to
the same deterministic node version after `A -> B -> A` does not revive an old
processor result or imply uninterrupted presentation continuity.

Processor input and artifact byte limits do not bound arbitrary render CPU or
peak memory. Untrusted processors require host-owned complexity limits and, when
cooperative cancellation is insufficient, worker or process isolation.

## Rejected Alternatives

- Expose raw `ProjectionOp`: rejected because it leaks compiler mechanics,
  omits authoritative old state, and exposes invalid intermediate operations.
- Add a presentation IR: rejected because it duplicates Content IR and turns
  host policy into protocol state.
- Require every adapter to diff old views: rejected because it duplicates
  reducer indexes and scales poorly across foreign bindings.
- Ship a React renderer or animation package: rejected because mature renderers
  already exist and the contract must remain useful outside React.
- Add arbitrary parser callbacks or JSON renderer metadata: rejected because
  they weaken deterministic parsing, validation, and cross-language conformance.
