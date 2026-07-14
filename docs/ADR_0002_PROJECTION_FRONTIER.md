# ADR 0002: Separate Source Progress from Projection Coverage

- Status: Accepted
- Date: 2026-07-14
- Scope: Streaming Content Engine 0.4

## Context

The original U4 design required every append to keep a fully typed provisional Markdown tree at the canonical source cursor. That requirement forces either a second incremental CommonMark/GFM parser or a full parse whenever an append cannot extend the current rightmost semantic leaf.

Deterministic U4 evidence falsified the full-parse fallback. A 16 KiB table streamed one byte at a time caused 329 full frontier parses and more than 61 units of compiler work per source byte, exceeding the planned 32x container/table ceiling. Incremark also reparses its complete pending tail on each append, while Streamdown lexes the complete input; neither supplies an incremental AST algorithm that can satisfy mdstream's bound.

## Decision

Canonical source progress and typed projection coverage are separate protocol coordinates.

- Every accepted append advances the source cursor losslessly.
- The projection cursor identifies the source prefix represented by canonical Content IR.
- A cheap append may advance both cursors when the compiler can update the current projection exactly.
- Otherwise, the source advances while the projection cursor remains unchanged. The uncovered source range is explicit pending source, not silently discarded semantic state.
- Structural boundaries, geometric checkpoints, and finish compile the frontier and atomically advance projection coverage.
- Finish is legal only when projection coverage reaches the final source cursor.
- Adapters may display the uncovered source slice as raw pending text. They must not reparse it or treat it as stable Content IR.

The compiler remains the only Markdown semantic compiler. Incremental framing supplies stability and cheap exact updates where available, without growing into a second Markdown implementation.

## Required Laws

1. `projection_cursor <= source_cursor` after every accepted change.
2. Projection coverage never regresses within an epoch.
3. New or replaced node ranges are contained by projection coverage.
4. A projection advance uses compare-and-set semantics against the retained cursor.
5. Source-only appends replay identically and expose the exact uncovered source range.
6. Finalized snapshots have equal source and projection cursors.
7. Final typed snapshots and identities remain invariant across chunk schedules, even when intermediate coverage differs.

## Consequences

This boundary keeps append work bounded and makes temporary semantic lag observable. A UI can always show new bytes, but rich formatting for a difficult frontier may catch up at the next checkpoint. Compatibility profiles and processors consume only typed covered nodes. Future specialized incremental compilers may reduce pending coverage without changing the protocol.

The rejected alternatives were silent stale projection, which loses observable pending content, and a second incremental Markdown grammar, which duplicates parser semantics and makes conformance unmaintainable.
