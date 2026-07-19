import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'support/native_library.dart';

final _capturedOptions = MdstreamSessionOptions(
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

void main() {
  final libraryPath = nativeLibraryPath();
  final skip = libraryPath == null
      ? 'run dart run ../dart/tool/build_native.dart first'
      : false;

  test('node listener can dispose during a committed transition', () async {
    final runtime = MdstreamRuntime.openPath(libraryPath!);
    final controller = MdstreamController.fromRuntime(runtime);
    final processor = _CountingProcessor('test.flutter.dispose.node');
    var disposedFromListener = false;
    var laterNodeNotifications = 0;
    try {
      controller.append('first\n\nsecond');
      final nodeIds = controller.value.document!.roots!.children;
      controller.registerProcessor(processor);
      controller.node(nodeIds.first).addListener(() {
        disposedFromListener = true;
        controller.dispose();
      });
      controller.node(nodeIds.last).addListener(() {
        laterNodeNotifications += 1;
      });

      final reportedErrors = await _captureFlutterErrors(() async {
        expect(controller.reset, returnsNormally);
        await _flushMicrotasks();
      });

      expect(reportedErrors, isEmpty);
      expect(disposedFromListener, isTrue);
      expect(laterNodeNotifications, 0);
      expect(processor.matchesCalls, 0);
      expect(processor.processCalls, 0);
    } finally {
      controller.dispose();
    }
    expect(runtime.nativeAllocations.isZero, isTrue);
  }, skip: skip);

  test('resource listener can dispose during a committed transition', () async {
    const resourceId = '154582791709149689190109869243805354114';
    final runtime = MdstreamRuntime.openPath(libraryPath!);
    final controller = MdstreamController.fromRuntime(runtime);
    final processor = _CountingProcessor('test.flutter.dispose.resource');
    var disposedFromListener = false;
    try {
      controller.append('# Adoption\n\n');
      controller.registerProcessor(processor);
      controller.resource(resourceId).addListener(() {
        disposedFromListener = true;
        controller.dispose();
      });

      final reportedErrors = await _captureFlutterErrors(() async {
        expect(
          () => controller.append(
            'See [@Engine] while this diagram streams.\n\n'
            '```mermaid\nflowchart LR\n  Token --> IR\n```\n\n'
            '[@engine]: https://mdstream.dev/engine "mdstream"\n',
          ),
          returnsNormally,
        );
        await _flushMicrotasks();
      });

      expect(reportedErrors, isEmpty);
      expect(disposedFromListener, isTrue);
      expect(processor.matchesCalls, 0);
      expect(processor.processCalls, 0);
    } finally {
      controller.dispose();
    }
    expect(runtime.nativeAllocations.isZero, isTrue);
  }, skip: skip);

  test(
    'pending source listener can dispose during a committed transition',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _CountingProcessor('test.flutter.dispose.pending');
      var disposedFromListener = false;
      try {
        controller.append('a *b');
        controller.registerProcessor(processor);
        controller.pendingSource.addListener(() {
          disposedFromListener = true;
          controller.dispose();
        });

        final reportedErrors = await _captureFlutterErrors(() async {
          expect(() => controller.append('*'), returnsNormally);
          await _flushMicrotasks();
        });

        expect(reportedErrors, isEmpty);
        expect(disposedFromListener, isTrue);
        expect(processor.matchesCalls, 0);
        expect(processor.processCalls, 0);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );

  test(
    'controller listener can dispose during a committed transition',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _CountingProcessor('test.flutter.dispose.root');
      var disposedFromListener = false;
      try {
        controller.registerProcessor(processor);
        controller.addListener(() {
          disposedFromListener = true;
          controller.dispose();
        });

        final reportedErrors = await _captureFlutterErrors(() async {
          expect(() => controller.append('paragraph'), returnsNormally);
          await _flushMicrotasks();
        });

        expect(reportedErrors, isEmpty);
        expect(disposedFromListener, isTrue);
        expect(processor.matchesCalls, 0);
        expect(processor.processCalls, 0);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );

  test(
    'processor error listener can dispose without further processor work',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _CountingProcessor('test.flutter.dispose.after-error');
      var disposedFromListener = false;
      try {
        controller.registerProcessor(_ThrowingMatchesProcessor());
        controller.registerProcessor(processor);
        controller.processorErrors.addListener(() {
          disposedFromListener = true;
          controller.dispose();
        });

        final reportedErrors = await _captureFlutterErrors(() async {
          controller.append('paragraph');
          await controller.whenProcessorsIdle();
        });

        expect(reportedErrors, isEmpty);
        expect(disposedFromListener, isTrue);
        expect(processor.matchesCalls, 0);
        expect(processor.processCalls, 0);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );

  test(
    'process error listener can dispose its in-flight registration once',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: _capturedOptions,
      );
      final phases = <ProcessorErrorPhase>[];
      int? revisionAfterDisposal;
      late final ProcessorRegistration registration;
      registration = controller.registerProcessor(_ThrowingProcessProcessor());
      controller.processorErrors.addListener(() {
        phases.add(controller.processorErrors.value!.phase);
        registration.dispose();
        revisionAfterDisposal = controller.transitions.value.revision;
      });

      final reportedErrors = await _captureFlutterErrors(() async {
        controller.append('paragraph');
        await controller.whenProcessorsIdle();
      });

      expect(reportedErrors, isEmpty);
      expect(phases, <ProcessorErrorPhase>[ProcessorErrorPhase.process]);
      expect(revisionAfterDisposal, isNotNull);
      expect(controller.transitions.value.revision, revisionAfterDisposal);
      controller.dispose();
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );

  test(
    'transition listeners unsubscribe and isolate failures without reentry',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: _capturedOptions,
      );
      var selfNotifications = 0;
      final laterRevisions = <int>[];
      late VoidCallback selfRemoving;
      selfRemoving = () {
        selfNotifications += 1;
        controller.transitions.removeListener(selfRemoving);
        expect(controller.reset, throwsStateError);
      };
      controller.transitions.addListener(selfRemoving);
      controller.transitions.addListener(
        () => throw StateError('isolated transition listener failure'),
      );
      controller.transitions.addListener(
        () => laterRevisions.add(controller.transitions.value.revision),
      );

      final reportedErrors = await _captureFlutterErrors(() async {
        controller.append('first');
        controller.append(' second');
        await _flushMicrotasks();
      });

      expect(selfNotifications, 1);
      expect(laterRevisions, <int>[1, 2]);
      expect(reportedErrors, hasLength(2));
      expect(controller.value.document?.coordinate.sequence, '1');
      controller.dispose();
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );

  test(
    'transition listener can dispose before invalidations and processor scans',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: _capturedOptions,
      );
      final processor = _CountingProcessor('test.flutter.dispose.transition');
      var transitionNotifications = 0;
      var rootNotifications = 0;
      controller.registerProcessor(processor);
      controller.addListener(() => rootNotifications += 1);
      controller.transitions.addListener(() {
        transitionNotifications += 1;
        controller.dispose();
      });

      final reportedErrors = await _captureFlutterErrors(() async {
        expect(() => controller.append('paragraph'), returnsNormally);
        await _flushMicrotasks();
      });

      expect(reportedErrors, isEmpty);
      expect(transitionNotifications, 1);
      expect(rootNotifications, 0);
      expect(processor.matchesCalls, 0);
      expect(processor.processCalls, 0);
      controller.dispose();
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );

  test(
    'in-flight registration disposal is guarded before transition side effects',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: _capturedOptions,
      );
      final processor = _PendingTransitionProcessor();
      final registration = controller.registerProcessor(processor);
      controller.append('first');
      await _flushMicrotasks();
      expect(processor.processCalls, 1);

      var guardedCalls = 0;
      void guardRegistration() {
        guardedCalls += 1;
        expect(registration.dispose, throwsStateError);
      }

      controller.transitions.addListener(guardRegistration);
      controller.append(' second');
      await _flushMicrotasks();

      expect(guardedCalls, greaterThan(0));
      expect(processor.processCalls, 2);
      controller.transitions.removeListener(guardRegistration);
      registration.dispose();
      controller.dispose();
      expect(registration.dispose, returnsNormally);
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: skip,
  );
}

Future<void> _flushMicrotasks() => Future<void>.delayed(Duration.zero);

Future<List<FlutterErrorDetails>> _captureFlutterErrors(
  FutureOr<void> Function() callback,
) async {
  final previousHandler = FlutterError.onError;
  final reportedErrors = <FlutterErrorDetails>[];
  FlutterError.onError = reportedErrors.add;
  try {
    await callback();
  } finally {
    FlutterError.onError = previousHandler;
  }
  return reportedErrors;
}

final class _CountingProcessor implements ContentProcessor {
  _CountingProcessor(this.id);

  final String id;
  int matchesCalls = 0;
  int processCalls = 0;

  @override
  ContentProcessorDescriptor get descriptor => ContentProcessorDescriptor(
    id: id,
    version: 'v1',
    acceptsProvisional: true,
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => true;

  @override
  bool matches(ContentNodeView node) {
    matchesCalls += 1;
    return true;
  }

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    processCalls += 1;
    return const ProcessorTextOutput(
      protocol: 'test.flutter.dispose/1',
      mediaType: 'text/plain',
      text: 'unexpected',
    );
  }
}

final class _ThrowingMatchesProcessor implements ContentProcessor {
  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.dispose.throwing',
    version: 'v1',
    acceptsProvisional: true,
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => true;

  @override
  bool matches(ContentNodeView node) =>
      throw StateError('processor matcher failed');

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) => throw StateError('throwing matcher must not process');
}

final class _ThrowingProcessProcessor implements ContentProcessor {
  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.dispose.throwing-process',
    version: 'v1',
    acceptsProvisional: true,
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => true;

  @override
  bool matches(ContentNodeView node) => true;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) => throw StateError('processor failed');
}

final class _PendingTransitionProcessor implements ContentProcessor {
  int processCalls = 0;

  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.transition-registration',
    version: 'v1',
    acceptsProvisional: true,
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => true;

  @override
  bool matches(ContentNodeView node) => node.content.kind == 'paragraph';

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    processCalls += 1;
    return Completer<ProcessorOutput>().future;
  }
}
