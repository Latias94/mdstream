# @mdstream/core

Framework-neutral TypeScript bindings for mdstream's Rust/WASM streaming
content engine. The package exposes external stores, changed-node views,
explicit snapshot recovery, lossless input batching, and host-side processor
scheduling without a renderer or UI-framework dependency.

This package is the complete first-party web state surface. Frameworks consume
its `subscribe`/`getSnapshot` stores and focused node, resource, and artifact
views through their native state primitives. mdstream intentionally does not
publish a React package or renderer; see
[`ADR 0004`](https://github.com/Latias94/mdstream/blob/main/docs/ADR_0004_FRAMEWORK_NEUTRAL_WEB_BINDINGS.md).
