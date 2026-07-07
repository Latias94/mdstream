# Plans

Implementation plans are historical decision artifacts. They explain why a refactor happened, but
they are not the current architecture reference after execution.

Current canonical docs:

- `README.md` for public API usage and release gates.
- `docs/ARCHITECTURE.md` for the current internal module map and invariants.
- `docs/EXTENSIONS.md` for boundary plugin, pending transformer, and analyzer extension points.

Plan history:

| Plan | Status |
|---|---|
| `2026-07-06-001-refactor-deepen-streaming-architecture-plan.md` | Completed by the first architecture split. See `docs/ARCHITECTURE.md` for the current structure. |
| `2026-07-06-002-refactor-engineering-hardening-plan.md` | Completed by the testing, benchmark, fuzz, CI, release, dependency, and MSRV hardening work. |
| `2026-07-06-003-refactor-remaining-architecture-deepening-plan.md` | Partially completed, then superseded by `2026-07-07-001-refactor-architecture-deepening-plan.md` for the next refactor wave. |
| `2026-07-07-001-refactor-architecture-deepening-plan.md` | Current wave source plan for markup container syntax, reference handling, stream engine setup, and docs cleanup. |
