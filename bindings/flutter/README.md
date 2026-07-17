# mdstream for Flutter

`mdstream_flutter` packages the native mdstream content engine and exposes its
canonical state through Flutter `ValueListenable` controllers. It provides
stable node keys, focused node/resource/artifact notifications, snapshot
recovery, and host-side processor leases. Rendering, widgets, themes, and rich
content presentation remain application concerns.

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

## Focused state

The controller itself is a `ValueListenable<MdstreamControllerState>`. Use
`node`, `resource`, and `artifacts.artifact` when a view should update only for
one stable identity. A full snapshot replacement notifies every materialized
focused listenable; ordinary deltas notify only their changed IDs.

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
