# Changelog

## 0.4.0

- Added turnkey no-path loading for bundled mdstream native libraries on Android, iOS, macOS, Linux, and Windows with Flutter 3.32+ and Dart 3.8+, while keeping widgets, renderers, themes, and Merman out of the package.
- Added local producer and replica `ValueListenable` controllers with immutable state, continuity-qualified stable node keys, focused node/resource/artifact/pending-source notifications, and explicit snapshot recovery.
- Added opt-in revisioned `MdstreamTransitionBatch` notifications for host-defined reveal, correction, and layout effects without adding widgets, animation dependencies, or presentation policy.
- Added bounded asynchronous processor scheduling with cancellation, stale-result safety, structured failures, full-replace artifact rebuilding, and notification-safe deterministic disposal.
- Added exact multi-platform package assembly and native-library verification so the archive uploaded to pub.dev is the same validated artifact exercised by CI.
