# ADR 0004: Keep Web Bindings Framework-Neutral

- Status: Accepted
- Date: 2026-07-17
- Scope: `mdstream` 0.4 web bindings and adoption freeze

## Context

The implementation plan originally included a first-party React state package
after the WASM and TypeScript bindings. That package would only wrap an external
store already exposed by `@mdstream/core`; it would not own canonical reduction,
Markdown parsing, rendering, processor execution, or recovery semantics.

React already has mature Markdown renderers and streaming UI libraries such as
Streamdown and Incremark. Competing at that layer would tie mdstream's release
and test surface to one UI framework without strengthening the protocol. The
cross-framework value is instead the Rust reducer, stable Content IR identities,
delta semantics, recovery laws, processor artifacts, and conformance evidence.

## Decision

`@mdstream/core` is the complete first-party web state surface.

1. It exposes the Rust/WASM engine and reducer through typed, framework-neutral
   engine, store, view, batching, recovery, and processor APIs.
2. Its stores provide `subscribe` and `getSnapshot` contracts plus focused
   node, resource, and artifact views. UI integrations may adapt those contracts
   to their framework's native external-store primitive.
3. mdstream will not create or publish a first-party React package, hooks,
   components, renderer, theme, or React-specific state implementation.
4. React, Streamdown, and Incremark remain consumer choices and compatibility
   references. They are not production dependencies of the WASM or TypeScript
   packages.
5. Protocol 0.4 adoption is validated through production-shaped native Rust,
   TypeScript/WASM, and standalone Merman paths before freeze. The TypeScript
   path proves the same stable identity, changed-view, recovery, and artifact
   contracts that any UI-framework adapter would consume.
6. Flutter remains a first-party integration because it must also deliver and
   locate native libraries across supported platforms. Its state controller is
   part of that native plugin contract, not a renderer or a precedent for
   framework-specific web packages.

This decision supersedes the React-specific parts of U15, U18, R19, R34, F7,
AE8, AE9, the verification matrix, and the Definition of Done in the original
0.4 implementation plan. The plan remains unchanged as a historical decision
artifact.

## Required Boundaries

1. The pnpm workspace contains no first-party React package.
2. `@mdstream/core` has no React, renderer, Markdown parser, Streamdown,
   Incremark, or Merman production dependency.
3. No framework adapter decodes canonical wire data or reimplements reducer
   transitions.
4. The TypeScript/WASM adoption fixture uses only high-level engine, store,
   view, recovery, and processor APIs.
5. Protocol changes discovered by any adoption path return to their owning
   layer and rerun candidate conformance before the final 0.4 freeze.

## Rejected Alternatives

- A first-party `@mdstream/react` hook package: rejected because it adds a
  shallow wrapper, framework lifecycle obligations, and release coupling while
  duplicating no difficult mdstream-owned behavior.
- A first-party React renderer: rejected because rendering policy, themes,
  widgets, layout, and syntax presentation are outside mdstream's product
  identity and already have strong ecosystem implementations.
- No TypeScript store contract: rejected because every web framework would then
  need to handle raw wire values, recovery, subscriptions, and processor
  freshness independently.

## Consequences

React applications can consume `@mdstream/core` with `useSyncExternalStore` or
their own state layer, and third parties may publish richer adapters without
becoming protocol authorities. mdstream's web compatibility surface stays small
and durable across React, Vue, Svelte, Solid, and future frameworks. Adoption
work shifts from React component behavior to proving the reusable TypeScript
state contract under production-shaped workloads.
