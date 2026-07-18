# Usage

## Rust Producer and Reducer

`StreamEngine` owns local streaming compilation. Apply every emitted change to
the canonical reducer and update only identities named by `ChangeImpact`.

```rust
use mdstream::StreamEngine;
use mdstream_protocol::{DocumentLifecycle, Reducer};

let mut engine = StreamEngine::new();
let mut reducer = Reducer::new();

for chunk in ["# Title\n\nHello ", "world"] {
    for change in engine.append(chunk)?.into_changes() {
        reducer.apply(change)?;
    }
}
for change in engine.finish()?.into_changes() {
    reducer.apply(change)?;
}

assert_eq!(
    reducer.document().map(|document| document.lifecycle()),
    Some(DocumentLifecycle::Finalized),
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`finish` is terminal and idempotent. Use `reset` for a new predecessor-linked
epoch. Configure custom blocks only through `StreamEngine::builder()` before
the first input.

## Replication and Recovery

A remote consumer applies ordered `ChangeSet` values. On a gap or fork, stop
sending ordinary deltas to that reducer, obtain one current snapshot from the
producer, call `recover_snapshot`, and resume with the next continuous change.
Do not stream snapshots during normal append.

## Processors

Use `ArtifactHost` or a language adapter's processor scheduler to process typed
nodes. Keep the returned artifact in a cache keyed by epoch, node, and processor
identity. Submit results with their original request generation; stale results
are rejected automatically.

## TypeScript and Web Frameworks

Install `@mdstream/core`, initialize its WASM runtime, and use its engine/store
interfaces. Root, node, resource, and artifact subscriptions expose focused
external stores. `store.pendingSource()` supplies the uncovered raw source
suffix on demand without reparsing it. React can pass `subscribe` and
`getSnapshot` to
`useSyncExternalStore`; Vue, Svelte, and Solid use their native equivalent.
There is no first-party React package or renderer.

## Dart and Flutter

The standalone Dart `mdstream` package requires a host-supplied path to a
compatible `mdstream-ffi` library:

```dart
final runtime = MdstreamRuntime.openPath(nativeLibraryPath);
final engine = runtime.createEngine();
try {
  engine.append('# Title');
  engine.finish();
} finally {
  engine.close();
}
```

`mdstream_flutter` bundles the native library for Android, iOS, macOS, Linux,
and Windows. `MdstreamController.open()` needs no path. The controller is a
`ValueListenable`; use focused `node`, `resource`, and `artifacts.artifact`
views plus the lazy `pendingSource` listenable to avoid rebuilding unchanged
host views. The package intentionally contains no widgets, themes, Markdown
renderer, or default Merman binary.

## Migrating From 0.3

Replace `MdStream`/`Update`/`DocumentState` flows with `StreamEngine`, ordered
`ChangeSet` values, and `Reducer`. Replace block positions with `NodeId`, cache
versions with `NodeVersion`, analyzers with typed processors, and runtime parser
mutation with setup-only custom blocks. No deprecated aliases are available.
