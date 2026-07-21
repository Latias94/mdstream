# Plans

Implementation plans are decision artifacts. They explain why a refactor happened, but superseded
plans are not current architecture references.

Current authority:

- `README.md` for public API usage and release gates.
- `CHANGELOG.md` for the published 0.3-to-0.4 migration contract.
- `docs/ADR_0002_PROJECTION_FRONTIER.md` for the incremental projection frontier.
- `docs/ADR_0003_STANDALONE_CUSTOM_BLOCKS.md` for custom block recognition.
- `docs/ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md` for the no-first-party-React boundary.
- `docs/ADR_0005_HOST_TRANSITION_FACTS.md` for renderer-neutral transition facts and host-owned
  presentation policy.
- `2026-07-20-001-feat-example-adoption-system-plan.md` for the active example, adoption, and
  release-verification work. ADRs and public release documentation take precedence if they differ.

Plan history:

| Plan | Role |
|---|---|
| `2026-07-06-001-refactor-deepen-streaming-architecture-plan.md` | Historical; superseded by the 0.4 plan. |
| `2026-07-06-002-refactor-engineering-hardening-plan.md` | Historical; its verification evidence informed the 0.4 plan. |
| `2026-07-06-003-refactor-remaining-architecture-deepening-plan.md` | Historical; superseded first by the July 7 plan and then by the 0.4 plan. |
| `2026-07-07-001-refactor-architecture-deepening-plan.md` | Historical; superseded by the 0.4 plan. |
| `2026-07-14-001-refactor-streaming-content-engine-plan.md` | Historical 0.4 foundation plan; its React and transition sections are superseded by ADR 0004, ADR 0005, and the implemented release surface. |
| `2026-07-19-001-refactor-host-transition-extension-contract-plan.md` | Historical implementation plan; ADR 0005 is the current transition-facts authority. |
| `2026-07-20-001-feat-example-adoption-system-plan.md` | Active adoption and release-verification plan; it does not override accepted ADRs or public release documentation. |
