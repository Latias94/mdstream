# mdstream for Dart

`mdstream` is the framework-neutral Dart state binding for the mdstream
streaming content engine. It exposes canonical deltas, stable node identities,
readonly Content IR views, explicit recovery, and derived processor artifacts.
It does not ship widgets, themes, renderers, Markdown parsing in Dart, or a
Flutter dependency.

## Native library

The standalone package expects the host to supply a compatible `mdstream-ffi`
dynamic library. Use an explicit path on Dart VM hosts:

```dart
final runtime = MdstreamRuntime.openPath(
  '/absolute/path/to/libmdstream_ffi.dylib',
);
```

Platform plugins that already link the native library can use
`MdstreamRuntime.fromDynamicLibrary`, including `DynamicLibrary.process()` on
supported Apple builds. Runtime initialization checks ABI version 1, both 0.4
binding schemas, and every result-structure layout before creating a session.

From an mdstream source checkout, build and record the current host library
with:

```sh
dart run tool/build_native.dart
```

Repository verification uses `dart run tool/test_native.dart`; it builds (or
validates `MDSTREAM_NATIVE_LIBRARY`) and makes a missing native library a hard
test failure instead of silently skipping transport coverage.

Turnkey library discovery and multi-platform native packaging belong to the
separate Flutter plugin. No native binary is included in this package.

## Streaming state

An engine owns a private canonical Rust reducer. Normal append and finish calls
return deltas and typed reducer updates; they never serialize an implicit full
snapshot.

```dart
final engine = runtime.createEngine();

try {
  final result = engine.append('# Hello\n');
  for (final update in result.updates) {
    for (final nodeId in update.impact.changedNodeIds) {
      final node = engine.state.nodeView(nodeId);
      if (node != null) {
        renderWithYourFramework(node.node.id, node);
      }
    }
  }

  engine.finish();
  final recovery = engine.createRecoverySnapshot();
} finally {
  engine.close();
}
```

`MdstreamStateView` deliberately exposes no apply, recover, or close method.
Node, resource, and artifact views are materialized lazily and retain object
identity until their exact changed ID is invalidated. Epochs, sequences,
cursors, request generations, and other Rust-domain counters remain canonical
decimal strings.

`engine.state.pendingSourceView()` lazily returns the exact source suffix not
yet covered by Content IR, including its UTF-8 byte range, or `null` when the
projection is current. The value is cached until source, projection, or epoch
replacement invalidates it; it is not embedded in every reducer update.

Use `runtime.createReducer()` for replication. A gap or conflicting current
sequence moves the reducer to `needs_snapshot`; only
`recoverSnapshot(CanonicalSnapshotBytes)` resumes it.

## Transition facts

Transition capture is optional and disabled by default. Enable it with a finite
protocol profile whose worst legal reducer update fits the configured wire
budget:

```dart
final options = MdstreamSessionOptions(
  captureTransitions: true,
  protocol: const {
    'max_source_bytes': '1048576',
    'max_nodes': '4096',
    'max_resources': '256',
    'max_operations': '4096',
    'max_change_structural_items': '4096',
    'max_children_per_list': '4096',
  },
);
final engine = runtime.createEngine(options: options);

final result = engine.append('streamed text');
for (final facts in result.transitionFacts) {
  scheduleHostPresentation(facts, engine.state);
}
```

`EngineResult.transitionFacts` and `ReducerResult.transitionFacts` preserve
wire order and are immutable. Facts distinguish projected text append,
replacement, insertion/removal, stability, child-list changes, resource
correction, lifecycle, and full replacement. Current node/resource values stay
lazy on `engine.state`; facts do not duplicate complete Content IR views.

The host owns grapheme pacing, color, opacity, easing, layout measurement,
scrolling, and reduced-motion behavior. A full replacement clears host
presentation continuity. An immediate mode must preserve the same content and
state meaning as an animated mode.

## Batching and processors

`engine.createBatcher(maxBatchBytes)` coalesces small token chunks without
dropping bytes. `push`, `finish`, and `reset` return every produced result in
wire order, including the two-result case where pending input is flushed before
an oversized chunk. Metrics report input, forwarded, pending, copy, payload,
and native append counts as decimal strings.

Processor methods expose native begin, complete, fail, and cancel leases. Their
text, binary, or citation artifacts are keyed by epoch, node, processor, node
version, configuration, and request generation. Artifacts remain outside the
canonical snapshot.

All engine and reducer handles support idempotent `close()`. Explicit close is
the lifecycle contract; short-lived native outputs and buffers are copied and
released synchronously on both success and error paths.
