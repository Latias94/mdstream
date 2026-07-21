# mdstream for Dart

`mdstream` is the framework-neutral Dart state binding for the mdstream
streaming content engine. It exposes canonical deltas, stable node identities,
readonly Content IR views, explicit recovery, and derived processor artifacts.
It does not ship widgets, themes, renderers, Markdown parsing in Dart, or a
Flutter dependency.

Use the [Dart headless entry in the example learning path](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#dart-headless) for the supported command, expected checkpoints, prerequisites, and next Flutter step.

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
Reducer creation also reads the native session's effective processor scheduler
limits, so adapters do not duplicate Rust defaults or option normalization.
Loading a dynamic library executes native code in the current process. Accept a
path only from a source you trust: ABI, schema, and layout checks establish
compatibility, not authenticity, integrity, or sandboxing.

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

## Golden AI Stream example

The standalone example is the shortest end-to-end Dart host. It requires Dart
3.8 or newer and an absolute path to a trusted, compatible `mdstream-ffi`
library. From a source checkout, the build helper prints that path, so the
complete assertion-mode run is:

```sh
LIBRARY=$(dart run tool/build_native.dart)
dart run example/golden_stream.dart --library "$LIBRARY" --assert
```

The example deliberately does not read repository build metadata. A host can
provide the same explicit path through an existing environment variable:

```sh
MDSTREAM_NATIVE_LIBRARY=/absolute/path/to/libmdstream_ffi.dylib \
  dart run example/golden_stream.dart --assert
```

The replay prints named checkpoints, pending source only at checkpoints that
request it, transition categories in wire order, stable final node IDs, the
canonical source, and `final_lifecycle=finalized`. A successful assertion run
ends with `assertions=passed` and `native_allocations=zero`; drift exits
nonzero with a concise diagnostic. Use `--scenario PATH` to diagnose a modified
repository scenario without changing the bundled authority.

This example teaches explicit runtime setup, focused canonical state reads,
optional transition capture, stable identity, and deterministic cleanup. The
recommended next step is the [Flutter host example](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#flutter-host), which binds the same headless state to a widget lifecycle without moving widget or animation policy into `mdstream_flutter`.

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
  protocol: MdstreamProtocolLimits(
    maxSourceBytes: '1048576',
    maxNodes: '4096',
    maxResources: '256',
    maxOperations: '4096',
    maxChangeStructuralItems: '4096',
    maxChildrenPerList: '4096',
  ),
  compiler: MdstreamCompilerLimits(
    maxMarkdownEvents: '300000',
    maxMarkdownOverlapWork: '1000000',
    maxDefinitions: '100000',
    maxDefinitionEdges: '100000',
    maxDefinitionMetadataBytes: '16777216',
  ),
);
final engine = runtime.createEngine(options: options);

final result = engine.append('streamed text');
for (final facts in result.transitionFacts) {
  scheduleHostPresentation(facts, engine.state);
}
```

`MdstreamProtocolLimits` contains only parser-neutral Content IR and reducer
limits. Parser work and retained definition-registry budgets belong to
`MdstreamCompilerLimits`. All five limit groups expose only supported
camel-case fields and encode the native snake-case schema internally. Arbitrary
native-schema maps are not part of the public Dart API, and compiler fields are
available only on `MdstreamCompilerLimits`.

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
text, binary, or citation artifacts are keyed by epoch, node identity, processor
identity, node projection version, complete input version, processor
implementation version, configuration version, and request generation.
Conditional admission reads the complete input version from the materialized
node view:

```dart
void beginCurrentProcessor(MdstreamEngine engine, NodeId nodeId) {
  final document = engine.state.currentState.document;
  final nodeView = engine.state.nodeView(nodeId);
  if (document != null && nodeView != null) {
    engine.beginProcessorIfCurrent(
      expectedEpoch: document.coordinate.epoch,
      nodeId: nodeView.node.id,
      expectedInputVersion: nodeView.processorInputVersion,
      processorId: 'app.example',
      processorVersion: '1',
      configurationVersion: 'default',
      acceptsProvisional: false,
      allowProvisional: false,
    );
  }
}
```

Artifacts remain outside the canonical snapshot.
`engine.processorSchedulerLimits` and the equivalent reducer property expose
the native effective concurrency and candidate-queue capacities for framework
adapters.

Processor completion outcomes and artifact slot states are exposed as
`ProcessorCompletionOutcome` and `ArtifactState`. Artifact changes and payloads
are sealed variants, so consumers switch on the concrete variant instead of
checking strings and nullable fields. Variant-specific fields are non-nullable,
and decoding rejects inconsistent native state/payload combinations.

All engine and reducer handles support idempotent `close()`. Explicit close is
the lifecycle contract; short-lived native outputs and buffers are copied and
released synchronously on both success and error paths.
