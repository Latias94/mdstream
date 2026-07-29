# Framework-Neutral Web Flagship

This private Vite workspace is the visual step after the Rust `minimal --assert`
quickstart. It replays the repository's Golden AI Stream through the published
`@mdstream/core` API. It needs Node 24, pnpm 11.9.0, Rust 1.85, `wasm-pack`, and
the pinned `wasm-opt` used by the workspace build. It needs no credentials,
provider connection, or external runtime service.

From the repository root:

```sh
pnpm install
pnpm web:prepare
pnpm --filter @mdstream/example-web dev
```

Open the printed local URL. The answer streams automatically and settles with a
finalized lifecycle, a canonical digest, continuity-qualified host keys, a
derived Mermaid summary, and a semantic correction in the status log. Switch
between Immediate and Paced to replay through fresh sessions; both settle to the
same visible content, digest, lifecycle, keys, and accessible status.

mdstream owns canonical Content IR, stable identity, lifecycle, focused views,
and factual transitions. This example owns DOM composition, citation URL policy,
grapheme pacing, color, motion, layout, scrolling, and announcements. The local
policy in `src/host-policy.ts` is intentionally example code, not a proposed
package API. The DOM host never reparses Markdown or decodes canonical wire data.

Run the automated checks with:

```sh
pnpm --filter @mdstream/example-web typecheck
pnpm --filter @mdstream/example-web test
pnpm --filter @mdstream/example-web exec playwright install chromium
pnpm --filter @mdstream/example-web test:e2e
```

Continue with `bindings/typescript/examples/transition-host.mjs` for the
machine-readable retention comparison, then the Rust processor and recovery
recipes for artifact freshness and continuity recovery.
