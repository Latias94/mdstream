---
title: Agent TUI Presentation State Machine - Plan
type: refactor
date: 2026-07-25
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# Agent TUI Presentation State Machine

## Goal Capsule

- **Objective:** Replace the rich TUI example's source-cursor reveal with a
  canonical, identity-aware presentation state machine that demonstrates how an
  AI terminal host can combine a settled transcript, a mutable streaming tail,
  adaptive pacing, semantic correction, Tree-sitter highlighting, and stable
  scrolling without reparsing Markdown or moving presentation policy into
  mdstream.
- **Authority:** The Product Contract below, ADR 0005, and
  `docs/ARCHITECTURE.md` govern this work. Where the current example disagrees,
  the example may be deleted or rewritten without compatibility shims.
- **Execution profile:** Breaking refactor of the feature-gated rich TUI
  example, its focused tests, smoke contract, and example documentation. This
  plan does not change public Rust or binding interfaces. If implementation
  uncovers a shared fact that existing hosts cannot derive efficiently, record
  the evidence for a separate protocol plan.
- **Stop condition:** The TUI never advances through invisible pending source,
  never displays queued content twice, reconciles corrections and full
  replacement by qualified identity, preserves canonical final output across
  paced and reduced-motion modes, highlights only semantically safe code, and
  becomes idle after the stream settles. Focused tests, smoke verification,
  formatting, linting, simplification, and code review all pass.
- **Tail ownership:** Implementation includes proof-first tests, a real
  terminal experiment, exact documentation updates, precise conventional
  commits, and review follow-through. It does not include a first-party
  renderer, React package, animation framework, or public terminal API.

---

## Product Contract

### Summary

mdstream already emits the canonical facts an AI UI needs: typed Content IR,
stable node identity, independent node stability and document lifecycle,
projection and pending-source frontiers, atomic transition facts, and
correction-aware invalidation. The current rich TUI bypasses those semantics by
animating one global byte cursor over the full canonical source while rendering
only projected nodes. That cursor can move through bytes the UI cannot paint,
and the next parser projection then appears as a sudden block-sized jump.

This refactor makes the example teach the intended integration boundary. The
host consumes each complete actor batch into one coherent reducer state. It
derives a recursively stable root prefix and renders three non-overlapping,
ordered regions: committed stable lines, visible stable lines waiting in a FIFO,
and the latest canonical mutable tail. A pacing tick commits visible queued
lines in FIFO order and changes their presentation state without removing text
or changing geometry, with adaptive catch-up when the producer outruns the
terminal. Corrections refresh content in place under the same qualified
identity; `Stable` means structurally settled at that moment, not permanently
immutable.

The example owns terminal-specific policy. mdstream continues to own content
truth and renderer-neutral changes. This is the same boundary used by the Web
host policy and Flutter adapter even though each host chooses a different
presentation mechanism.

### Problem Frame

The existing example has five coupled defects:

- `visible_source_end` advances through `Document::source()` even when the
  corresponding bytes remain outside the typed projection, so visible progress
  and renderable progress diverge.
- The renderer walks the newest whole Content IR while text is clipped by an
  older byte cursor. Structural chrome can therefore precede its content, and
  normalized semantic text appears all at once.
- Each change inside one `ActorBatch` is presented immediately, allowing a
  transient intermediate state to enter the animation queue before a later
  change in the same batch supersedes it.
- Fixed-rate grapheme reveal ignores backlog age and actor coalescing, producing
  alternating stalls and bursts. The loop redraws at 60 FPS after all state is
  settled.
- Absolute line-number scrolling and all-at-once syntax highlighting create
  jumps when earlier content changes.

Codex's TUI provides a useful scheduling shape: a committed region, a mutable
tail, a FIFO of presentation work, and adaptive catch-up. Its source-level
newline collector, Markdown reparse, table detector, and terminal line types do
not belong in mdstream because Content IR already represents those facts and
other hosts do not share Codex's terminal constraints.

### Actors

- A1. An AI stream producer whose token chunks and actor coalescing schedule are
  arbitrary.
- A2. A terminal host integrating `mdstream-tokio`, Ratatui, Tree-sitter, and
  application-owned activity state.
- A3. An application user reading while content streams, pausing reveal,
  enabling reduced motion, resizing, or scrolling away from the tail.
- A4. An integrator using the example to learn which facts mdstream owns and
  which presentation decisions their own UI must implement.
- A5. A maintainer changing parser, protocol, actor, or example behavior while
  relying on deterministic conformance and smoke evidence.

### Requirements

#### Canonical presentation reconciliation

- R1. Apply changes in an `ActorBatch` to the reducer in order, aggregate every
  successful outcome, and reconcile presentation exactly once from the last
  successful batch-tail `Document`. An intermediate state from the same batch
  must never enter the presentation queue. If a later reducer apply fails, the
  successful prefix still reconciles once before the host records the error.
- R2. Derive the queueable region as the maximum leading root prefix for which
  every node in every root subtree is recursively `Stable`. A stable parent
  containing a provisional descendant is part of the mutable tail.
- R3. The mutable tail begins at the enqueue boundary, not the committed
  boundary. A newly stable root becomes one latest `PresentedRoot` projection
  with a monotonic committed-line frontier; FIFO entries reference the
  projection's remaining root-local line ordinals. A queued logical line is
  excluded from the tail but remains visible exactly once at the same canonical
  position. Moving a root from mutable to presented, or advancing its line
  frontier, changes state and styling without making text disappear, reappear,
  or change geometry.
- R4. Presentation identity is qualified by continuity generation, epoch, and
  canonical node identity. Full replacement clears committed presentation,
  queued work, tail state, syntax cache, activity state, and scroll anchors
  before rebuilding from the replacement document.
- R5. Stable nodes remain correction-capable. Node, structure, resource, or
  removal impact affecting already queued or displayed content invalidates the
  relevant presentation projection and refreshes it from the latest canonical
  document without appending a duplicate. A correction to an already presented
  root replaces its projection immediately, drops stale FIFO entries for that
  owner, and marks the corrected projection committed instead of replaying it as
  fresh text.
- R6. Pending source is never counted as displayed canonical content. The
  example may show a factual pending indicator, but raw pending bytes do not
  enter the typed transcript or get reparsed by the host.

#### Scheduling and interaction

- R7. One FIFO item is one rendered logical line carrying a qualified root
  owner, root-local line ordinal, enqueue ordinal, and enqueue time. Normal mode
  commits one queued line per tick. Backlog depth and oldest-item age select an
  adaptive catch-up quantum with explicit hysteresis so the UI neither falls
  indefinitely behind nor oscillates between modes. Queue order follows the
  batch-tail root projection, never fact order or numeric `NodeId`.
- R8. Pause stops only fresh queued-line commits. Actor input, canonical
  reduction, correction/removal/full-replace handling, queued and mutable
  rendering, status updates, and finalization continue. Follow-tail remains an
  independent user setting, so pausing does not silently move or freeze the
  viewport; the status text names this precisely as paused commits.
- R9. Enabling reduced motion drains all currently queued work atomically unless
  commits are paused. While paused it changes the motion preference without
  overriding the pause; resuming then drains atomically. Subsequent eligible
  content commits immediately while reduced motion remains enabled. Disabling
  it affects only future pacing and never requeues committed content. A
  `--reduced-motion` startup option applies before the first document update;
  the runtime key toggles that same host preference.
- R10. A persistent interval drives commit animation only while work is queued,
  unpaused, and paced. Redraws are dirty-state driven; a settled, inactive TUI
  blocks on external events instead of continuously sleeping and drawing.
- R11. Follow-tail tracks the latest content. Manual scrolling uses a
  continuity-qualified content owner plus a block-local visual row where
  possible. Correction keeps the nearest surviving row of the same owner;
  removal falls back to the previous surviving owner, then the next owner, then
  the top. Fallback never silently re-enables follow-tail. Resize and wrap
  changes may recompute geometry but cannot duplicate or lose content.

#### Semantic rendering and extension boundary

- R12. Render only canonical Content IR and semantic resources. The host does
  not parse Markdown, infer tables from source lines, or create a second content
  model.
- R13. Tree-sitter analyzes only a complete, recursively stable code block body
  within the configured byte budget. Provisional code stays plainly styled.
  Stabilization moves the same text into the visible queued-stable region and
  may replace styling, but preserves line topology and position. Cache keys
  include qualified identity and processor input version or equivalent complete
  invalidation facts, and eviction is bounded and deterministic.
- R14. Mermaid stays typed canonical code. Merman remains an optional,
  standalone processor example path; the TUI must not make Merman, Tree-sitter,
  Ratatui, or animation dependencies part of mdstream's default graph.
- R15. Activity rows, colors, pacing, reduced motion, layout, terminal widgets,
  and scroll behavior remain private host policy. Transition facts expose why
  canonical state changed but never prescribe an animation.

#### Core ownership gate

- R16. Do not add or change any public mdstream or binding API in this plan. A
  helper that merely hides a short traversal, or a terminal line queue used by
  one example, has no public value. If two existing hosts are later shown to
  require the same non-derivable fact, preserve that evidence for a separate
  protocol plan with cross-binding conformance and migration scope.
- R17. Documentation and tests must state the reusable mdstream contract:
  projection and pending source are distinct; root and descendant stability
  must be considered together for presentation holdback; stable identity permits
  later semantic correction; and hosts reconcile once per coherent operation
  batch.
- R18. The example remains feature-gated and runnable from a published
  `mdstream-tokio` crate archive. Default library consumers incur no terminal,
  syntax grammar, Unicode animation, or example-state-machine dependency.
- R19. Actor output closure is not equivalent to successful finalization. The
  host joins exactly once, applies unread batches and any
  `ActorFailure.completed` outputs as coherent ordered groups, and distinguishes
  `Completed`, `Failed`, and intentional `Cancelled` exits. Only a finalized
  `Completed` actor can enter the successful settled state.
- R20. User-visible state is unambiguous across initial/no-document,
  pending-only, active partial content, paused commit backlog, terminal failure,
  empty successful completion, and non-empty successful completion. Answer,
  activity, inspector, and status surfaces agree on the state. Existing
  keyboard controls remain discoverable; `q`, Escape, and Ctrl-C cancel active
  work and restore the terminal. A too-small terminal retains an exit/status
  surface instead of rendering overlapping panels.

### Key Flows

- F1. Arbitrary token chunks enter the bounded Tokio actor. The actor coalesces
  them and emits an `ActorBatch`. The host reduces all successful changes,
  records aggregate invalidation, then reconciles once against the batch-tail
  canonical document.
- F2. Reconciliation finds a recursively stable root prefix. Newly eligible
  logical lines enter FIFO order and immediately leave the mutable tail, but
  remain visible exactly once in the queued-stable region. The remaining
  provisional roots render from current Content IR, followed by a factual
  pending indicator when the projection frontier trails source.
- F3. A normal tick commits one visible queued line. If the FIFO exceeds the
  depth or age threshold, catch-up commits a larger bounded quantum until
  backlog drops below the exit threshold. Each unit has exactly one owner and
  one location: committed, queued-stable, or mutable.
- F4. A late link or citation definition corrects a node already displayed.
  The host applies the canonical correction to the existing qualified owner,
  invalidates stale rendered and syntax state, and redraws in place. It does not
  replay the node as newly streamed content.
- F5. The user pauses commits while input continues. Visible stable work
  accumulates in the FIFO, while the current mutable tail, inspector, and
  independently configured viewport remain current. Enabling reduced motion
  records the preference but respects the pause. Resuming enters catch-up when
  paced or drains atomically when reduced.
- F6. Advanced recovery replaces the document. The full-replace fact advances
  continuity, all previous presentation identity and caches are discarded, and
  the replacement begins from a clean presentation state.
- F7. The actor output closes and the host joins it once. A successful exit
  applies unread terminal batches, verifies a finalized document, makes all
  roots eligible, and drains according to motion policy. A failed exit first
  applies its already committed outputs as one coherent terminal group, then
  enters a visible terminal error state without claiming finalization. An
  intentional quit cancels the actor and restores the terminal. A successful
  final reconciliation yields exactly the same semantic lines as a direct
  render of the finalized canonical `Document`; with no queued work or events,
  the loop becomes idle.

### Acceptance Examples

- AE1. Given pending source beyond the projection cursor, when animation ticks,
  then no presentation boundary advances through those bytes and the transcript
  contains only typed Content IR.
- AE2. Given multiple changes in one actor batch that first create and then
  restructure a setext heading, table, list, or emphasis run, when the batch is
  applied, then only the batch-tail form can enter the queue. If a later change
  is invalid, the successful prefix reconciles once before the error is shown.
- AE3. Given one stable root followed by a stable parent with a provisional
  child, when presentation reconciles, then only the first root is queueable and
  the second remains in the mutable tail.
- AE4. Given three stable lines where two are still queued, when rendering all
  regions, then neither queued line appears in the mutable tail, both remain
  visible in canonical order, and each moves to committed state exactly once
  without a text or geometry discontinuity.
- AE5. Given a displayed reference whose definition arrives later, when
  transition facts report a correction, then its existing displayed owner
  updates to the resolved semantic value without a second copy or reveal
  animation.
- AE6. Given a paused host receiving additional batches, when inspected before
  resume, then canonical source, mutable tail, pending indicator, and activity
  state are current while the displayed stable count is unchanged.
- AE7. Given a deep or old backlog, when normal motion remains enabled, then the
  host enters catch-up, drains a larger bounded quantum, exits below the
  hysteresis threshold, and preserves FIFO order.
- AE8. Given queued work, when reduced motion is enabled and later disabled,
  then the current queue commits once, committed content is never requeued, and
  only later arrivals use paced mode. If commits are paused, enabling reduced
  motion changes preference without draining; resume performs the one atomic
  drain.
- AE9. Given a full replacement after highlighted code and manual scrolling,
  when recovery applies, then no old qualified key, queue item, syntax entry,
  activity state, or scroll anchor survives.
- AE10. Given provisional code that later stabilizes, when it is rendered, then
  its text and line geometry remain stable while Tree-sitter styling changes
  from plain to highlighted exactly once.
- AE11. Given paced and reduced-motion runs over adversarial UTF-8 chunk
  schedules, when both settle, then their final canonical source, lifecycle,
  semantic lines, and normalized presentation digest equal a direct finalized
  render.
- AE12. Given a finalized and fully drained TUI with no user event, when the
  event loop waits, then neither animation ticks nor redraw counters continue
  increasing.
- AE13. Given an actor whose coalesced flush commits one constituent and fails
  on the next, when output closes, then the host applies the completed prefix
  once, reports the terminal error, preserves an open lifecycle when
  appropriate, and never labels the presentation successfully settled.
- AE14. Given each user-visible lifecycle state and a terminal below the normal
  panel threshold, when the TUI renders, then the answer and footer communicate
  one consistent state, exit remains discoverable, and no panel overlaps.

### Success Metrics

- Zero invisible-byte progress, duplicate presentation units, stale
  post-recovery identities, or intermediate batch states in deterministic
  tests.
- Paced, catch-up, paused/resumed, and reduced-motion paths converge to one
  canonical final presentation.
- Backlog is bounded by producer completion and demonstrably catches up under
  the demo workload.
- Syntax analysis occurs only for stable code, stays within byte/cache limits,
  and leaves default builds untouched.
- The settled event loop performs no periodic redraw work.
- No public API or default dependency is introduced. Any newly discovered
  cross-host need is captured as evidence outside this implementation.

### Scope

#### In scope

- Rewrite the rich TUI example's presentation state and scheduler.
- Refactor renderer inputs around displayed stable content and a canonical
  mutable tail.
- Strengthen identity, correction, full-replace, pause, reduced-motion,
  Tree-sitter, scrolling, idle-loop, smoke, and documentation evidence.
- Add or clarify conformance documentation for the existing mdstream/host
  responsibility boundary.
- Delete superseded source-cursor reveal code and misleading smoke counters.
- Add compiler conformance for the stable-root-prefix shape and clarify the
  existing `ActorBatch` batch-tail host contract without changing signatures.

#### Out of scope

- Public renderer, terminal, animation, layout, or scroll APIs.
- First-party React, Vue, Svelte, Solid, GPUI, egui, or Flutter rendering
  components.
- A second Markdown parser, LALRPOP grammar, Codex's newline collector, or
  source-level table holdback.
- Moving Ratatui, Tree-sitter, Unicode segmentation, or Merman into a default
  library dependency graph.
- Reworking the canonical parser, Content IR wire schema, bindings, or processor
  artifact protocol without new failing evidence.
- Any public Rust or binding interface change; new evidence is routed to a
  separate plan.

### Product Contract Key Decisions

- KTD1. `user-approved:` Fearlessly replace the current `visible_source_end`
  design and delete compatibility paths; preserving a flawed animation model
  has no migration value because the example exposes no supported library API.
- KTD2. `session-settled:` Keep mdstream headless and framework-neutral.
  Rejected alternative: a public presentation IR, renderer, or React layer that
  would duplicate Content IR and couple canonical state to one UI lifecycle.
- KTD3. `session-settled:` Keep animation and layout host-owned. Rejected
  alternative: protocol fields for timing, color, easing, geometry, reduced
  motion, or scroll.
- KTD4. Borrow Codex's state-machine shape, not its parser implementation.
  mdstream's typed roots and recursive stability replace newline collection,
  Markdown reparse, and source-level table detection.
- KTD5. Define committed presentation as “paced once,” not “immutable
  transcript.” Canonical correction can update already displayed content in
  place under the same qualified identity.
- KTD6. Reconcile only after the complete `ActorBatch`; actor coalescing is an
  operation boundary and intermediate reducer states are not host publication
  points.
- KTD7. Use recursive subtree stability for queue eligibility. Root-only
  stability is insufficient because the protocol permits a stable parent with
  provisional descendants.
- KTD8. Keep the presentation state machine private to the example. Rejected
  alternative: a shallow public `stable_root_prefix_len` or generic line queue
  used by only one host.
- KTD9. Use deterministic state and work counters for correctness. Timing
  measurements support the terminal experiment but do not substitute for FIFO,
  convergence, or idle-state assertions.
- KTD10. Adapt Codex's hidden commit FIFO to a full-screen TUI as a visible
  queued-stable region. Rejected alternative: removing an already visible
  mutable block until its queued lines are committed, which creates the same
  layout jump this refactor is meant to remove.
- KTD11. Pause has precedence over reduced motion for fresh commits. Canonical
  corrections and destructive changes still apply immediately because pause is
  a presentation preference, not stale-state permission.
- KTD12. Store one current projection per stable root with a committed-line
  frontier. Rejected alternative: assigning the whole root to either committed
  or queued state, which cannot pace a long root without duplicating ownership.

---

## Planning Contract

### Current-State Evidence

- `mdstream-tokio/examples/agent_tui_rich.rs` owns a global
  `visible_source_end`, advances it against all source bytes, and clips typed
  semantic text by that historical cursor.
- The same file calls `observe_transition` after every reducer apply inside one
  actor batch, handles only full replacement and removed syntax entries, creates
  a fresh 16 ms sleep every loop, and redraws before every wait.
- `mdstream-protocol/src/document.rs` already exposes canonical roots, nodes,
  parents, projection cursor, pending source, lifecycle, and continuity
  generation.
- `mdstream-protocol/src/transition.rs` already exposes qualified node keys,
  before/after state stamps, text append versus replacement, structural
  splices, resource impact, and full replacement.
- `docs/ARCHITECTURE.md` explicitly states that stable nodes may be corrected
  under the same identity and that lifecycle, stability, and correction are
  independent axes.
- `examples/web/src/host-policy.ts` is a second real host policy, but it uses
  source-interval grapheme queues rather than terminal lines. Flutter renders
  keyed Content IR directly. Their different state shapes fail the evidence
  gate for a shared presentation API.
- `repo-ref/codex/codex-rs/tui/src/streaming/` demonstrates a committed region,
  mutable tail, FIFO, adaptive catch-up, and final canonical consolidation.
  Its source collector and Markdown/table parsing solve Codex-specific input
  constraints that mdstream does not share.

### High-Level Technical Design

```text
token chunks
    |
    v
StreamEngineActor -- ordered ActorBatch -----------------------------+
                                                                     |
                           apply every successful ChangeSet           |
                                                                     v
                                              TransitionReducer + Document
                                                                     |
                                  reconcile once at batch tail        |
                                                                     v
             +---------------- RichPresentation ---------------------+
             |                                                       |
             | recursively stable prefix                             | remaining
             v                                                       v
   visible identity FIFO ---> paced/catch-up commit ---> committed projection
                      \__________________________________/
                                      +
                              canonical mutable tail
                                      +
                              pending indicator
                                                                     |
                                                                     v
                                       Ratatui layout / Tree-sitter / scroll
```

Presentation location invariant:

```text
canonical root/subtree
        |
        +-- recursively stable and newly eligible --> PresentedRoot
        |                                               |
        |                                  [committed prefix | queued suffix]
        |                                               |
        |                                  tick advances line frontier
        |
        +-- otherwise ----------------------------> mutable tail

Each logical line is visible exactly once. Each stable owner has one current
projection, even while its line frontier splits committed and queued lines.
Correction replaces that projection, drops stale queued references, and commits
the corrected owner without replaying it.
Full replacement deletes all three locations before rebuilding.
```

Scheduler state:

```text
Settled --new work--> Smooth --depth/age high--> CatchUp
   ^                    |                            |
   | queue empty        +------ pause --------------+  (drain stops only)
   +--------------------+                            |
                         <-- below exit threshold ---+

Reduced motion drains Smooth or CatchUp atomically and affects future work.
```

### Implementation Strategy

Use a private, pure presentation module inside the example target. Its inputs
are a batch-tail canonical document, aggregate impact/facts, motion mode, and
explicit ticks. Its outputs are identity-owned rendered block projections plus
factual counters. Keep terminal I/O, event polling, Ratatui layout, and
Tree-sitter mechanics in an outer adapter. This seam permits deterministic unit
tests without pretending to be a reusable mdstream runtime API.

Represent committed and queued content as qualified `PresentedRoot` projections
containing logical lines, not immutable naked terminal lines. Each projection
stores a committed-line frontier; a queue entry references its owner and a line
ordinal at or beyond that frontier. On any correction affecting an owner,
regenerate its projection from the canonical document; an already committed or
partly committed corrected owner becomes immediately current rather than
replaying as fresh text. The enqueue boundary is the count of recursively stable
roots already owned by presented state; the mutable tail starts after that
boundary. If structure or fact detail cannot identify a narrower valid suffix,
fall back to a private full presentation rebuild while preserving compatible
qualified owners. A full replacement clears everything.

Keep wrapping late in Ratatui so resize does not require rebuilding semantic
queues. Store scroll intent as follow-tail or a qualified owner plus local
visual offset, resolving it against current geometry each draw. Highlight
stable code only, and use a bounded access-aware cache rather than evicting the
smallest numeric node ID.

### Impact Map

- **Data flow:** `ActorBatch` reduction changes from per-change presentation
  updates to one batch-tail reconciliation.
- **State:** Global source reveal becomes qualified stable-root projections with
  committed-line frontiers, a line-reference FIFO, a mutable tail, scheduler
  state, and scroll anchors.
- **Rendering:** The renderer consumes complete owner projections and a mutable
  canonical suffix instead of clipping every semantic field by one byte cursor.
- **Invalidation:** Transition facts and `ChangeImpact` refresh corrected or
  removed owners and syntax entries.
- **Runtime:** One persistent interval is enabled only with queued work;
  terminal redraw is dirty-driven.
- **Dependencies:** No runtime or default dependency changes. Existing
  feature-gated dev dependencies remain.
- **Documentation:** Example docs explain the stable-transcript/mutable-tail
  recipe and the distinction between stable identity and immutability.
- **Removal:** Delete `visible_source_end`, `presentation_limit`,
  source-clipping helpers, misleading animation-tick assertions, and any cache
  behavior superseded by qualified ownership.

### Risks And Controls

- **RISK1: Correction of already displayed stable content.** Control: store
  owner identity, rebuild from canonical state on impact, and test late
  reference resolution.
- **RISK2: Stable parent with provisional descendants enters the queue.**
  Control: recursive stability traversal with an explicit protocol fixture.
- **RISK3: A structural rewrite moves the enqueue boundary backward.** Control:
  invalidate and rebuild from the earliest affected root instead of treating
  committed lines as append-only history.
- **RISK4: Scheduler tests become wall-clock flaky.** Control: pass explicit
  logical `Instant` values into pure tick decisions. The PTY experiment is a
  required experience gate, but its machine-specific wall-clock measurements
  remain non-gating observations.
- **RISK5: Scroll anchor complexity obscures the example.** Control: keep one
  small `FollowTail | Anchored` state and deterministic owner-removal fallback;
  do not implement a general viewport framework.
- **RISK6: The refactor leaks Ratatui concepts into core.** Control: prohibit
  public API changes in this plan, keep all presentation types private, and run
  a final dependency/public-diff audit. Any shared-fact evidence becomes a
  separate plan.
- **RISK7: A large example becomes hard to review.** Control: split only at the
  pure presentation-state/terminal-adapter boundary, with private modules and
  focused tests; do not split by line count alone.

### Execution Order

`U1 -> U2 -> U3 -> U4`

- U1 establishes failing invariants, actor-terminal ownership, and the pure
  state model.
- U2 makes rendering, highlighting, scrolling, and the runtime loop consume that
  model.
- U3 locks the existing compiler and actor contracts with conformance and
  durable adoption guidance, without expanding the public surface.
- U4 runs convergence, terminal, quality, and review gates and removes
  superseded code.

---

## Implementation Units

### U1. Build The Canonical Presentation State Machine

- **Goal:** Replace source-cursor reveal with deterministic, correction-aware
  stable-root projections, line FIFO, and mutable tail reconciled once per actor
  batch.
- **Requirements:** R1-R9, R16, R19
- **Acceptance examples:** AE1-AE9, AE13
- **Files:**
  - Modify: `mdstream-tokio/examples/agent_tui_rich.rs`
  - Create if the private seam remains deep:
    `mdstream-tokio/examples/agent_tui_rich/presentation.rs`
  - Modify: `mdstream-tokio/tests/actor.rs`
- **Approach:**
  1. Add proof-first tests for pending-source invisibility, batch-tail-only
     reconciliation, recursive stability, queue/tail exclusivity, correction,
     full replacement, pause, catch-up, and reduced motion.
  2. Before choosing incremental invalidation, verify that existing
     `Document`, `ChangeImpact`, and `TransitionFacts` cover removal, root move,
     resource correction, and full replacement. Use a private wider suffix or
     full rebuild whenever the facts cannot safely prove a narrower update.
  3. Introduce qualified stable-root projections with committed-line frontiers,
     a FIFO of owner/line references, and a separate mutable suffix.
  4. Reduce a whole `ActorBatch`, aggregate successful outcomes, then reconcile
     once from its tail document.
  5. If a later reducer apply fails, reconcile the successful prefix once before
     recording the error.
  6. Handle actor join exactly once, applying unread and failure-completed
     outputs before classifying the terminal state.
  7. Rebuild the smallest provably safe owner suffix on
     node/structure/resource impact; clear all presentation state on full
     replacement.
  8. Delete the global byte cursor and all source-clipping compatibility code.
- **Invariants:**
  - One stable owner has one latest projection and one monotonic committed-line
    frontier; every logical line stays continuously visible exactly once.
  - Queue order follows canonical root order.
  - No pending byte is reported as displayed.
  - Corrections update identity; they do not manufacture new identity.
- **Test scenarios:**
  - A stable root queues once, leaves the mutable tail immediately, and remains
    visible through its single presented projection.
  - A stable root with a provisional descendant remains mutable.
  - Two changes in one batch expose only their tail state.
  - Late reference correction refreshes a displayed owner.
  - Paused and reduced-motion paths preserve canonical order.
  - Full replacement leaves no old owner or queued projection.
  - Actor partial failure applies its completed prefix and never claims success.
- **Verification:**
  - `cargo +1.88.0 nextest run -p mdstream-tokio --features rich-tui presentation`
  - Focused unit tests imported through `mdstream-tokio/tests/actor.rs`

### U2. Rewire Semantic Rendering And Terminal Scheduling

- **Goal:** Render the new state model with stable geometry, bounded syntax
  analysis, anchored scrolling, adaptive ticks, and no settled redraw loop.
- **Requirements:** R7-R15, R18, R20
- **Acceptance examples:** AE7-AE12, AE14
- **Files:**
  - Modify: `mdstream-tokio/examples/agent_tui_rich.rs`
  - Create if justified by U1:
    `mdstream-tokio/examples/agent_tui_rich/render.rs`
  - Modify: `mdstream-tokio/tests/actor.rs`
- **Approach:**
  1. Render complete identity-owned projections and the canonical mutable suffix;
     remove semantic-text byte clipping and premature structural chrome.
  2. Run Tree-sitter only for recursively stable code, qualify cache ownership,
     and replace smallest-ID eviction with bounded recent-use eviction.
  3. Replace absolute manual scroll state with follow-tail or a qualified owner
     and local row, preserving intent through earlier-content correction.
  4. Define the visible state matrix for no document, pending-only, partial,
     paused backlog, terminal error, empty completion, and successful completion.
     Preserve the documented keyboard controls, make active-stream quit cancel
     and restore the terminal, keep narrow layouts operable, and accept
     `--reduced-motion` before the first update.
  5. Use one persistent missed-tick-skipping interval. Gate its select branch on
     queued, unpaused paced work and draw only when canonical, presentation,
     input, or geometry state is dirty.
  6. Replace raw animation-tick smoke metrics with queue, catch-up,
     reconciliation, final-equivalence, syntax, and idle counters.
- **Invariants:**
  - Plain and highlighted code have identical text and line topology.
  - Wrapping and resize change geometry only.
  - No animation timer is active without queued work.
  - Final rendered lines equal a direct finalized-document render.
- **Test scenarios:**
  - Provisional code remains plain; stabilization enables syntax styling.
  - Corrected or removed code invalidates the right cache entry.
  - Manual anchor survives earlier insert/correction and falls back on removal.
  - Backlog enters and exits catch-up without reordering.
  - A settled loop reports no additional timer-driven redraw.
  - Pausing leaves follow-tail behavior independent and accurately labeled.
  - A too-small terminal retains exit/status affordances and does not panic.
- **Verification:**
  - `cargo +1.88.0 nextest run -p mdstream-tokio --features rich-tui`
  - `cargo +1.88.0 clippy -p mdstream-tokio --all-targets --features rich-tui -- -D warnings`

### U3. Document And Verify The mdstream/Host Boundary

- **Goal:** Make the example reusable as architectural guidance while proving
  that no new public presentation API is warranted.
- **Requirements:** R12-R18
- **Acceptance examples:** AE1, AE3, AE5, AE10-AE12
- **Files:**
  - Modify: `mdstream-tokio/README.md`
  - Modify: `mdstream-tokio/src/actor.rs`
  - Modify: `docs/EXAMPLES.md`
  - Modify if clarification is needed: `docs/ARCHITECTURE.md`
  - Modify: `mdstream/tests/content_frontier.rs`
- **Approach:**
  1. Explain the coherent-batch, recursively stable prefix, mutable tail,
     correction, pending-source, and host-policy recipe next to the runnable
     command.
  2. Clarify on `ActorBatch` that hosts reduce the complete ordered batch before
     publishing one coherent batch-tail presentation update.
  3. Add compiler conformance that engine-produced stable roots form a leading
     prefix and every emitted root subtree shares its root stability across
     ambiguous syntax and chunk schedules. Keep this a compiler guarantee, not
     a general protocol-producer law.
  4. Compare the TUI, Web, and Flutter host shapes against R16. Record why
     pacing queues, terminal lines, animation, and scroll remain host-local.
  5. Audit manifests and public exports to prove no renderer or optional example
     dependency entered the normal graph.
- **Invariants:**
  - Documentation never describes `Stable` as permanently immutable.
  - Example guidance distinguishes pending source from projected Content IR.
  - No first-party framework renderer or animation policy is implied.
- **Test scenarios:**
  - Published crate metadata still includes the runnable feature-gated example.
  - Default feature graph excludes rich TUI dependencies.
  - Compiler stability conformance passes adversarial chunk schedules.
- **Verification:**
  - `cargo +1.88.0 metadata --no-deps --format-version 1`
  - `cargo +1.88.0 package -p mdstream-tokio --allow-dirty --no-verify --list`
  - Repository architecture tests covering framework-neutral boundaries

### U4. Convergence, Experiment, Simplification, And Review

- **Goal:** Prove the new example behaves naturally in a real terminal, converges
  under supported modes, and leaves no obsolete architecture behind.
- **Requirements:** R1-R20
- **Acceptance examples:** AE1-AE14
- **Files:**
  - Modify as findings require: files owned by U1-U3
  - Do not edit generated lockfiles or unrelated user changes
- **Approach:**
  1. Run the deterministic rich smoke in paced and reduced-motion test paths and
     compare both with a direct canonical render.
  2. Run the interactive example in a PTY, inspect burst/stall behavior,
     structural jumps, highlighting transitions, pause/resume, scrolling, and
     settled CPU behavior.
  3. Run formatter, focused tests, full package tests, Clippy, package checks,
     and diff hygiene serially to avoid competing Cargo builds.
  4. Apply the simplify-code pass, then a comprehensive diff-scoped code review;
     fix every validated correctness, maintainability, testing, or standards
     finding.
  5. Delete dead reveal helpers, stale counters, redundant abstractions, and
     misleading documentation. Stage only owned files and create logical
     Conventional Commits.
- **Invariants:**
  - Automated correctness does not depend on terminal timing.
  - No public API or dependency expansion is permitted by this plan.
  - The worktree preserves unrelated user changes.
- **Test scenarios:**
  - Rich smoke reports finalized source, empty queue/tail, canonical render
    equality, syntax captures, catch-up evidence, and zero errors.
  - Default and rich feature test suites both pass.
  - Diff review finds no remaining `visible_source_end` compatibility path.
- **Verification:**
  - `cargo fmt --all -- --check`
  - `cargo +1.88.0 nextest run -p mdstream-tokio`
  - `cargo +1.88.0 nextest run -p mdstream-tokio --features rich-tui`
  - `cargo +1.88.0 clippy -p mdstream-tokio --all-targets --features rich-tui -- -D warnings`
  - `cargo +1.88.0 run -p mdstream-tokio --features rich-tui --example agent_tui_rich -- --smoke`
  - `git diff --check`

---

## Verification Contract

### Proof Strategy

1. **Characterize the defect:** tests demonstrate that pending source cannot be
   treated as visible and that intermediate states inside one actor batch are
   not presentation boundaries.
2. **Prove state laws:** pure tests cover recursive stability, FIFO ownership,
   correction, full replacement, pause, reduced motion, and catch-up using
   explicit logical time.
3. **Prove semantic equivalence:** paced and immediate presentation settle to a
   direct render of the final `Document`.
4. **Prove adapter behavior:** rich-feature actor tests and the smoke command
   exercise the real bounded actor, coalescing, reducer, transition facts,
   renderer, and Tree-sitter integration.
5. **Prove terminal ownership:** success, partial actor failure, and intentional
   cancellation apply every committed output once and restore the terminal.
6. **Inspect the experience:** a PTY run verifies that pacing is legible and
   stable, not just mathematically equivalent.
7. **Audit the boundary:** metadata, package list, public diff, simplification,
   and code review verify that host policy did not leak into mdstream core.

### Required Gates

| Gate | Evidence | Blocking |
| --- | --- | --- |
| Presentation laws | Focused pure tests for R1-R11 | Yes |
| Canonical convergence | Paced/reduced-motion/direct-render equality | Yes |
| Actor integration | `mdstream-tokio` rich-feature nextest suite | Yes |
| Syntax safety | Stable-only highlight and bounded-cache tests | Yes |
| Runtime idleness | Deterministic dirty/timer state test | Yes |
| Actor terminal ownership | Success, partial failure, and cancellation tests | Yes |
| Interactive quality | PTY observation recorded in implementation summary | Yes |
| Default isolation | Default nextest and metadata/package audit | Yes |
| Code quality | fmt, Clippy `-D warnings`, diff check | Yes |
| Architecture | Simplification and comprehensive code review | Yes |

### Non-Gating Observations

- Wall-clock frame timing and terminal scheduling vary by machine. Use them to
  diagnose feel, never as the sole correctness oracle.
- Terminal color support differs. Semantic content and identity, not exact RGB
  output, are the acceptance boundary.
- PTY capture may omit interactive resize behavior; deterministic geometry and
  anchor tests remain authoritative for that flow.

---

## Definition of Done

- [ ] `visible_source_end`, source-wide presentation clipping, and their
      compatibility helpers are deleted.
- [ ] A private presentation state reconciles once per actor batch, stores one
      projection per stable qualified owner, and assigns every logical line to
      committed, visible FIFO, or mutable state exactly once.
- [ ] Queue eligibility recursively checks every descendant's stability.
- [ ] Pending source is represented factually and never counted or rendered as
      typed transcript content.
- [ ] Correction, removal, structure/resource invalidation, and full replacement
      update or clear the correct presentation owners and syntax state.
- [ ] Smooth pacing, adaptive catch-up, pause, and reduced motion preserve FIFO
      order and converge to the same final output.
- [ ] One logical rendered line is the pacing quantum; multi-line roots never
      enter as one block-sized animation step.
- [ ] Tree-sitter runs only on stable complete code, with qualified bounded
      caching and unchanged text geometry.
- [ ] Follow-tail and anchored manual scrolling behave deterministically through
      correction, removal, wrapping, and resize.
- [ ] Actor completion, partial failure, cancellation, and terminal restoration
      each preserve all already committed output exactly once.
- [ ] The event loop uses a persistent conditional timer and performs no
      periodic redraw once settled.
- [ ] Smoke evidence compares the final presentation with a direct canonical
      render and reports meaningful queue/catch-up/idle facts.
- [ ] Example documentation teaches the coherent-batch, stable-prefix,
      mutable-tail, pending-source, and correction model without implying a
      first-party renderer.
- [ ] No public presentation API or default dependency is added. Any cross-host
      API evidence is recorded for a separate plan rather than implemented here.
- [ ] All Verification Contract gates pass serially; local environment
      exceptions, if any, are named with the exact failed command.
- [ ] Simplification and comprehensive code review produce no unresolved
      correctness, maintainability, testing, or project-standards findings.
- [ ] Owned changes are committed precisely with Conventional Commit messages;
      unrelated user changes remain untouched.
