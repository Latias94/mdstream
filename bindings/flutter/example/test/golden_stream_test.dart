import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' show SemanticsFlag;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';
import 'package:mdstream_flutter_example/bootstrap.dart';
import 'package:mdstream_flutter_example/configure_host.dart';
import 'package:mdstream_flutter_example/golden_stream_host.dart';

void main() {
  final nativeLibrary = _nativeLibraryPath();

  test('generated Apple hosts use the native package deployment targets', () {
    final root = Directory.systemTemp.createTempSync('mdstream-flutter-host-');
    try {
      for (final target
          in <
                String,
                ({
                  String setting,
                  String before,
                  String after,
                  String podPlatform,
                })
              >{
                'ios': (
                  setting: 'IPHONEOS_DEPLOYMENT_TARGET',
                  before: '13.0',
                  after: '14.0',
                  podPlatform: 'ios',
                ),
                'macos': (
                  setting: 'MACOSX_DEPLOYMENT_TARGET',
                  before: '10.14',
                  after: '11.0',
                  podPlatform: 'osx',
                ),
              }
              .entries) {
        final platformRoot = Directory(
          '${root.path}${Platform.pathSeparator}${target.key}',
        );
        final project = File(
          '${platformRoot.path}${Platform.pathSeparator}Runner.xcodeproj'
          '${Platform.pathSeparator}project.pbxproj',
        );
        final podfile = File(
          '${platformRoot.path}${Platform.pathSeparator}Podfile',
        );
        project.parent.createSync(recursive: true);
        project.writeAsStringSync(
          '${target.value.setting} = ${target.value.before};\n',
        );
        podfile.writeAsStringSync(
          "# platform :${target.value.podPlatform}, '${target.value.before}'\n",
        );

        configureHost(projectRoot: root, platform: target.key);
        configureHost(projectRoot: root, platform: target.key);
        expect(
          project.readAsStringSync(),
          contains('${target.value.setting} = ${target.value.after};'),
        );
        expect(
          podfile.readAsStringSync(),
          contains(
            "platform :${target.value.podPlatform}, '${target.value.after}'",
          ),
        );
      }

      expect(
        () => configureHost(projectRoot: root, platform: 'windows'),
        throwsA(
          isA<ConfigureHostException>().having(
            (error) => error.message,
            'message',
            contains('unsupported Apple platform'),
          ),
        ),
      );
    } finally {
      root.deleteSync(recursive: true);
    }
  });

  testWidgets(
    'focused nodes retain their key and unrelated roots do not rebuild',
    (tester) async {
      final clock = _GateClock();
      final builds = <NodeId, int>{};
      final bootstrap = _nativeBootstrap(nativeLibrary!, clock: clock);
      await tester.runAsync(() => bootstrap.initialize(autoPlay: false));

      await tester.pumpWidget(
        GoldenStreamExample(
          bootstrap: bootstrap,
          autoPlay: false,
          onNodeBuild: (id) =>
              builds.update(id, (value) => value + 1, ifAbsent: () => 1),
        ),
      );
      unawaited(bootstrap.replay(mode: GoldenPlaybackMode.paced));
      await tester.pump();

      expect(bootstrap.currentActionId, 'open-inline-reference');
      final roots = bootstrap.controller!.value.document!.roots!.children;
      expect(roots, hasLength(2));
      final headingId = roots.first;
      final headingKey = bootstrap.controller!.nodeKey(headingId);
      final headingBuilds = builds[headingId];

      clock.release();
      await tester.pump();

      expect(bootstrap.currentActionId, 'open-rust-fence');
      expect(builds[headingId], headingBuilds);
      expect(bootstrap.controller!.nodeKey(headingId), same(headingKey));

      await _settlePlayback(tester, bootstrap, clock);
      expect(bootstrap.phase, GoldenHostPhase.settled);
      expect(bootstrap.finalCheckpoint?.id, 'finalized');
      expect(bootstrap.controller!.value.isFinalized, isTrue);

      await tester.pumpWidget(const SizedBox.shrink());
      expect(bootstrap.runtime!.nativeAllocations.isZero, isTrue);
    },
    skip: nativeLibrary == null,
  );

  testWidgets(
    'phone and wide layouts keep controls and answer before diagnostics',
    (tester) async {
      final bootstrap = _nativeBootstrap(nativeLibrary!);
      await tester.runAsync(() async {
        await bootstrap.initialize(autoPlay: false);
        await bootstrap.replay(mode: GoldenPlaybackMode.immediate);
      });

      for (final size in <Size>[const Size(390, 844), const Size(1280, 900)]) {
        tester.view.physicalSize = size;
        tester.view.devicePixelRatio = 1;
        await tester.pumpWidget(
          GoldenStreamExample(bootstrap: bootstrap, autoPlay: false),
        );
        await tester.pumpAndSettle();

        final controls = tester.getTopLeft(find.byKey(goldenControlsKey));
        final answer = tester.getTopLeft(find.byKey(goldenAnswerKey));
        final inspector = tester.getTopLeft(find.byKey(goldenInspectorKey));
        expect(controls.dy, lessThan(answer.dy));
        if (size.width < 900) {
          expect(answer.dy, lessThan(inspector.dy));
        } else {
          expect(answer.dx, lessThan(inspector.dx));
        }
        expect(tester.takeException(), isNull);
      }

      await tester.pumpWidget(const SizedBox.shrink());
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      expect(bootstrap.runtime!.nativeAllocations.isZero, isTrue);
    },
    skip: nativeLibrary == null,
  );

  testWidgets(
    'pending source renders once and correction status is not live',
    (tester) async {
      final clock = _GateClock();
      final bootstrap = _nativeBootstrap(nativeLibrary!, clock: clock);
      await tester.runAsync(() => bootstrap.initialize(autoPlay: false));
      await tester.pumpWidget(
        GoldenStreamExample(bootstrap: bootstrap, autoPlay: false),
      );
      unawaited(bootstrap.replay(mode: GoldenPlaybackMode.paced));
      await tester.pump();

      await _advancePlaybackTo(tester, bootstrap, clock, 'stream-rust-info');
      final pending = bootstrap.controller!.pendingSource.value;
      expect(pending, isNotNull);
      expect(find.text(pending!.text), findsOneWidget);

      clock.release();
      await tester.pump();
      expect(bootstrap.currentActionId, 'stream-rust-body');
      await _advanceUntilPendingClears(tester, bootstrap, clock);
      expect(find.text(pending.text), findsNothing);

      await _settlePlayback(tester, bootstrap, clock);
      final transition = tester.getSemantics(
        find.byKey(goldenTransitionStatusKey),
      );
      expect(transition.label, contains('Correction / replacement'));
      expect(transition.hasFlag(SemanticsFlag.isLiveRegion), isFalse);
      expect(
        tester.getSemantics(find.byKey(goldenReplayKey)).label,
        'Replay stream',
      );
      expect(
        tester
            .getSemantics(find.text('[engine]'))
            .hasFlag(SemanticsFlag.isLink),
        isFalse,
      );

      await tester.pumpWidget(const SizedBox.shrink());
      clock.release();
      await tester.pump();
      expect(tester.takeException(), isNull);
      expect(bootstrap.runtime!.nativeAllocations.isZero, isTrue);
    },
    skip: nativeLibrary == null,
  );

  testWidgets(
    'active replay can be interrupted and disposed without leaks',
    (tester) async {
      final clock = _GateClock();
      final bootstrap = _nativeBootstrap(nativeLibrary!, clock: clock);
      await tester.runAsync(() => bootstrap.initialize(autoPlay: false));
      await tester.pumpWidget(
        GoldenStreamExample(bootstrap: bootstrap, autoPlay: false),
      );
      unawaited(bootstrap.replay(mode: GoldenPlaybackMode.paced));
      await tester.pump();

      expect(
        tester.getSemantics(find.byKey(goldenInterruptKey)).label,
        'Interrupt stream',
      );
      await tester.tap(find.byKey(goldenInterruptKey));
      await tester.pump();
      expect(bootstrap.phase, GoldenHostPhase.interrupted);
      expect(bootstrap.controller!.value.isFinalized, isFalse);
      final status = tester.getSemantics(find.byKey(goldenStatusKey));
      expect(status.label, 'Stream status: Interrupted');

      await tester.pumpWidget(const SizedBox.shrink());
      clock.release();
      await tester.pump();
      expect(tester.takeException(), isNull);
      expect(bootstrap.runtime!.nativeAllocations.isZero, isTrue);
    },
    skip: nativeLibrary == null,
  );

  testWidgets(
    'bootstrap ownership follows widget replacements during paced playback',
    (tester) async {
      final scenario = File('assets/golden_ai_stream.json').readAsStringSync();
      final firstClock = _ObservedGateClock();
      final secondClock = _ObservedGateClock();
      final secondScenario = Completer<String>();
      var firstLoads = 0;
      var secondLoads = 0;
      final first = GoldenStreamBootstrap(
        loadScenario: () async {
          firstLoads += 1;
          return scenario;
        },
        openRuntime: () => MdstreamRuntime.openPath(nativeLibrary!),
        clock: firstClock,
      );
      final second = GoldenStreamBootstrap(
        loadScenario: () {
          secondLoads += 1;
          return secondScenario.future;
        },
        openRuntime: () => MdstreamRuntime.openPath(nativeLibrary!),
        clock: secondClock,
      );

      await tester.runAsync(() => first.initialize(autoPlay: false));
      final firstRuntime = first.runtime!;
      await tester.pumpWidget(
        GoldenStreamExample(bootstrap: first, autoPlay: false),
      );
      unawaited(first.replay(mode: GoldenPlaybackMode.paced));
      await tester.pump();

      expect(first.phase, GoldenHostPhase.streaming);
      expect(firstClock.isWaiting, isTrue);
      expect(firstLoads, 1);

      await tester.pumpWidget(
        GoldenStreamExample(bootstrap: second, autoPlay: false),
      );
      await tester.pump();

      expect(firstClock.cancelledWaits, 1);
      expect(firstRuntime.nativeAllocations.isZero, isTrue);
      expect(secondLoads, 1);
      expect(second.phase, GoldenHostPhase.booting);
      expect(second.runtime, isNull);

      secondScenario.complete(scenario);
      await tester.pumpAndSettle();

      expect(second.phase, GoldenHostPhase.readyEmpty);
      expect(second.runtime, isNotNull);
      expect(tester.takeException(), isNull);

      final secondRuntime = second.runtime!;
      final secondCancelsBeforeUnmount = secondClock.cancels;
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();

      expect(secondClock.cancels, secondCancelsBeforeUnmount + 1);
      expect(secondRuntime.nativeAllocations.isZero, isTrue);
      expect(secondLoads, 1);
      expect(tester.takeException(), isNull);
    },
    skip: nativeLibrary == null,
  );

  testWidgets(
    'immediate, paced, and reduced-motion policies settle identically',
    (tester) async {
      final clock = _CountingClock();
      final bootstrap = _nativeBootstrap(nativeLibrary!, clock: clock);
      await tester.runAsync(() => bootstrap.initialize(autoPlay: false));
      await tester.pumpWidget(
        GoldenStreamExample(bootstrap: bootstrap, autoPlay: false),
      );

      await bootstrap.replay(mode: GoldenPlaybackMode.immediate);
      await tester.pump();
      final immediateCheckpoint = bootstrap.finalCheckpoint!;
      final immediateSemantics = _settledSemantics(tester);
      expect(clock.waits, 0);

      await bootstrap.replay(mode: GoldenPlaybackMode.paced);
      await tester.pump();
      final pacedCheckpoint = bootstrap.finalCheckpoint!;
      final pacedSemantics = _settledSemantics(tester);
      expect(clock.waits, greaterThan(0));
      expect(pacedCheckpoint.rootIds, immediateCheckpoint.rootIds);
      expect(pacedCheckpoint.lifecycle, immediateCheckpoint.lifecycle);
      expect(pacedSemantics, immediateSemantics);

      final waitsBeforeReducedMotion = clock.waits;
      await bootstrap.replay(
        mode: GoldenPlaybackMode.paced,
        reducedMotion: true,
      );
      await tester.pump();
      final reducedCheckpoint = bootstrap.finalCheckpoint!;
      expect(clock.waits, waitsBeforeReducedMotion);
      expect(reducedCheckpoint.rootIds, immediateCheckpoint.rootIds);
      expect(_settledSemantics(tester), immediateSemantics);

      final status = tester.getSemantics(find.byKey(goldenStatusKey));
      expect(status.hasFlag(SemanticsFlag.isLiveRegion), isTrue);
      final answer = tester.getSemantics(
        find.text('Why stable streaming matters'),
      );
      expect(answer.hasFlag(SemanticsFlag.isLiveRegion), isFalse);

      await tester.pumpWidget(const SizedBox.shrink());
      expect(bootstrap.runtime!.nativeAllocations.isZero, isTrue);
    },
    skip: nativeLibrary == null,
  );

  testWidgets('scenario failures are recoverable without loading native code', (
    tester,
  ) async {
    var attempts = 0;
    final bootstrap = GoldenStreamBootstrap(
      loadScenario: () async {
        attempts += 1;
        return '{"schema":"wrong"}';
      },
      openRuntime: () => throw StateError('runtime must not open'),
    );

    await tester.pumpWidget(GoldenStreamExample(bootstrap: bootstrap));
    await tester.pumpAndSettle();

    expect(bootstrap.phase, GoldenHostPhase.error);
    expect(find.text('Unable to start the stream'), findsOneWidget);
    expect(find.byKey(goldenRetryKey), findsOneWidget);
    expect(attempts, 1);

    await tester.tap(find.byKey(goldenRetryKey));
    await tester.pumpAndSettle();

    expect(attempts, 2);
    expect(bootstrap.phase, GoldenHostPhase.error);
    final semantics = tester.getSemantics(find.byKey(goldenRetryKey));
    expect(semantics.hasFlag(SemanticsFlag.isButton), isTrue);
    expect(
      tester.binding.focusManager.primaryFocus?.debugLabel,
      'Retry stream',
    );

    await tester.pumpWidget(const SizedBox.shrink());
  });
}

List<String> _settledSemantics(WidgetTester tester) => <String>[
  tester.getSemantics(find.byKey(goldenStatusKey)).label,
  tester.getSemantics(find.text('Why stable streaming matters')).label,
  tester.getSemantics(find.text('Mermaid source')).label,
];

GoldenStreamBootstrap _nativeBootstrap(
  String nativeLibrary, {
  GoldenPlaybackClock? clock,
}) => GoldenStreamBootstrap(
  loadScenario: () => File('assets/golden_ai_stream.json').readAsString(),
  openRuntime: () => MdstreamRuntime.openPath(nativeLibrary),
  clock: clock,
);

Future<void> _settlePlayback(
  WidgetTester tester,
  GoldenStreamBootstrap bootstrap,
  _GateClock clock,
) async {
  for (var step = 0; step < 32; step += 1) {
    if (bootstrap.phase == GoldenHostPhase.settled) {
      return;
    }
    if (bootstrap.phase == GoldenHostPhase.error) {
      fail('playback failed: ${bootstrap.errorLabel}');
    }
    clock.release();
    await tester.pump();
  }
  fail(
    'playback did not settle; phase=${bootstrap.phase} '
    'action=${bootstrap.currentActionId}',
  );
}

Future<void> _advancePlaybackTo(
  WidgetTester tester,
  GoldenStreamBootstrap bootstrap,
  _GateClock clock,
  String actionId,
) async {
  for (var step = 0; step < 32; step += 1) {
    if (bootstrap.currentActionId == actionId) {
      return;
    }
    if (bootstrap.phase == GoldenHostPhase.error) {
      fail('playback failed: ${bootstrap.errorLabel}');
    }
    clock.release();
    await tester.pump();
  }
  fail('playback did not reach $actionId');
}

Future<void> _advanceUntilPendingClears(
  WidgetTester tester,
  GoldenStreamBootstrap bootstrap,
  _GateClock clock,
) async {
  for (var step = 0; step < 32; step += 1) {
    if (bootstrap.controller!.pendingSource.value == null) {
      return;
    }
    if (bootstrap.phase == GoldenHostPhase.error) {
      fail('playback failed: ${bootstrap.errorLabel}');
    }
    clock.release();
    await tester.pump();
  }
  fail('pending source did not clear before playback settled');
}

String? _nativeLibraryPath() {
  final environment =
      Platform.environment['MDSTREAM_NATIVE_LIBRARY'] ??
      Platform.environment['MDSTREAM_FFI_LIBRARY'];
  if (environment != null && environment.isNotEmpty) {
    return File(environment).absolute.path;
  }
  final metadata = File('../../dart/.dart_tool/mdstream/native-library.json');
  if (!metadata.existsSync()) {
    return null;
  }
  final decoded = jsonDecode(metadata.readAsStringSync());
  if (decoded is! Map<String, Object?> || decoded['library'] is! String) {
    throw const FormatException('invalid native library metadata');
  }
  return decoded['library']! as String;
}

final class _GateClock implements GoldenPlaybackClock {
  Completer<void>? _gate;

  @override
  Future<void> wait(Duration duration) {
    _gate = Completer<void>();
    return _gate!.future;
  }

  void release() {
    final gate = _gate;
    if (gate != null && !gate.isCompleted) {
      gate.complete();
    }
  }

  @override
  void cancel() => release();
}

final class _CountingClock implements GoldenPlaybackClock {
  int waits = 0;

  @override
  Future<void> wait(Duration duration) async {
    waits += 1;
  }

  @override
  void cancel() {}
}

final class _ObservedGateClock implements GoldenPlaybackClock {
  Completer<void>? _gate;
  int cancels = 0;
  int cancelledWaits = 0;

  bool get isWaiting {
    final gate = _gate;
    return gate != null && !gate.isCompleted;
  }

  @override
  Future<void> wait(Duration duration) {
    _gate = Completer<void>();
    return _gate!.future;
  }

  @override
  void cancel() {
    cancels += 1;
    final gate = _gate;
    if (gate != null && !gate.isCompleted) {
      cancelledWaits += 1;
      gate.complete();
    }
  }
}
