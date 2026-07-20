# mdstream for Flutter

`mdstream_flutter` packages the native mdstream content engine and exposes its
canonical state through Flutter `ValueListenable` controllers. It provides
stable node keys, focused node/resource/artifact notifications, snapshot
recovery, and host-side processor leases. Rendering, widgets, themes, and rich
content presentation remain application concerns.

## Golden stream host

The package archive includes a runnable host under `example/`. It replays the
repository's deterministic Golden AI Stream through a real
`MdstreamController` and renders typed prose, Rust code, citation state, and
Mermaid source. The example deliberately does not parse Markdown, bundle
Merman, or export a reusable content widget.

From the repository root, stage the native plugin artifact for the target and
run the example. For macOS:

```console
python3 bindings/flutter/tool/build_native.py macos
cd bindings/flutter/example
flutter pub get
flutter run -d macos
```

The replay mode, text color transition, responsive layout, focus behavior, and
reduced-motion policy live in the example. Canonical root order comes from the
controller, each node listens through `controller.node(id)`, widget identity
comes from `controller.nodeKey(id)`, and the inspector reads only focused
pending-source and transition surfaces. This is the intended seam for a host
to substitute its own widgets and animation policy.

The repository widget suite uses an explicit host library and does not depend
on a bundled desktop artifact:

```console
cd bindings/dart
dart run tool/build_native.dart
cd ../flutter/example
flutter pub get
flutter test test/golden_stream_test.dart
```

In a source checkout, the repository-only supported-platform integration
target uses the same bundled bootstrap as `main.dart` and verifies the final
named checkpoint. Test sources are intentionally excluded from the publish
archive:

```console
flutter create --empty --platforms macos --project-name mdstream_flutter_example --org io.mdstream.example --no-pub .
flutter test integration_test/golden_stream_smoke_test.dart -d macos
```

Replace `macos` in both positions for another supported platform. The example
keeps generated platform runners out of the package so hosts are created only
for the platform being exercised.

## Local streams

The default constructor locates the library bundled for the current platform.
No native path is required.

```dart
final controller = MdstreamController.open();

try {
  controller.append('# Hello\n\nStreaming content');
  controller.finish();

  final roots = controller.value.document?.roots?.children ?? const [];
  for (final nodeId in roots) {
    final key = controller.nodeKey(nodeId);
    final node = controller.node(nodeId);
    attachToYourView(key, node);
  }
} finally {
  controller.dispose();
}
```

`MdstreamController` owns a local producer and does not accept external
changes. `MdstreamReplicaController` is the separate consumer surface for
ordered canonical changes, gap/fork detection, and explicit snapshot
recovery. Keeping those roles separate prevents producer and replica state
from diverging.

## Transition facts

Applications that need host-defined reveal, correction, or layout effects can
enable transition capture with a finite protocol profile:

```dart
final controller = MdstreamController.open(
  options: MdstreamSessionOptions(
    captureTransitions: true,
    protocol: const {
      'max_source_bytes': '1048576',
      'max_nodes': '4096',
      'max_resources': '256',
      'max_operations': '4096',
      'max_change_structural_items': '4096',
      'max_children_per_list': '4096',
    },
  ),
);

controller.transitions.addListener(() {
  final batch = controller.transitions.value;
  scheduleHostPresentation(batch, controller);
});
```

`transitions` is a revisioned `ValueListenable<MdstreamTransitionBatch>`.
Capture-enabled public operations publish one batch even when its facts are
empty; capture-disabled controllers remain at revision zero. Tail state and
focused values are coherent before transition listeners run, and ordinary
focused/root listeners run afterward. Transition callbacks may read state or
unsubscribe, and may dispose the controller synchronously. Document mutation,
processor registration, and processor-registration disposal are rejected until
the callback returns.

Use `MdstreamNodeKey`, which combines continuity generation, epoch, and node ID,
for keyed widget state. Advanced recovery crosses a continuity barrier even
inside the same epoch; same-floor recovery preserves the key. Flutter retains
ownership of animation controllers, timing, colors, geometry, scrolling, and
accessibility policy. mdstream supplies facts, not widgets or motion behavior.

## Focused state

The controller itself is a `ValueListenable<MdstreamControllerState>`. Use
`node`, `resource`, and `artifacts.artifact` when a view should update only for
one stable identity. A full snapshot replacement notifies every materialized
focused listenable; ordinary deltas notify only their changed IDs.

`pendingSource` is a lazy focused `ValueListenable<PendingSourceView?>` for the
source suffix not yet covered by typed Content IR. It is absent from the root
controller state, remains last-good through a recovery-required gap, and
refreshes when source/projection coverage or a full replacement changes.
Applications may display its text as pending content but must not reparse it.

Processor artifacts remain outside canonical snapshots. Registered processors
run after native transitions, receive cancellation when their input becomes
stale, and settle through the Rust request-generation checks. Await
`whenProcessorsIdle()` when a workflow needs all scheduled processor work to
finish.

## Supported platforms

Version 0.4 packages native libraries for Android arm64/armv7/x86_64, iOS
device and simulator, universal macOS, Linux x86_64, and Windows x64. The
standalone `mdstream` Dart package remains available for hosts that supply an
explicit native-library path.
