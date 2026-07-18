import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'processor artifacts notify by slot and stay outside canonical snapshots',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _TextProcessor();
      controller.registerProcessor(processor);

      try {
        controller.append('processor body');
        final document = controller.value.document!;
        final nodeId = document.roots!.children.single;
        final slot = ArtifactSlot(
          epoch: document.coordinate.epoch,
          nodeId: nodeId,
          processorId: processor.descriptor.id,
        );
        final artifact = controller.artifacts.artifact(slot);
        final unrelated = controller.artifacts.artifact(
          ArtifactSlot(
            epoch: document.coordinate.epoch,
            nodeId: nodeId,
            processorId: 'test.flutter.unrelated',
          ),
        );
        var notifications = 0;
        var unrelatedNotifications = 0;
        artifact.addListener(() => notifications += 1);
        unrelated.addListener(() => unrelatedNotifications += 1);

        controller.finish();

        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(1));
        expect(artifact.value?.state, 'ready');
        expect(artifact.value?.artifact?.payload.text, 'derived output');
        expect(notifications, 2);
        expect(unrelatedNotifications, 0);
        final snapshot = utf8.decode(
          controller.createRecoverySnapshot()!.bytes,
        );
        expect(snapshot, isNot(contains(processor.descriptor.id)));
        expect(snapshot, isNot(contains('derived output')));

        controller.reset();
        expect(artifact.value, isNull);
        expect(notifications, 3);
        expect(unrelatedNotifications, 1);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'processor exceptions become structured failed artifacts',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _ThrowingProcessor();
      controller.registerProcessor(processor);
      try {
        controller.append('processor failure');
        controller.finish();
        await controller.whenProcessorsIdle();

        final document = controller.value.document!;
        final slot = ArtifactSlot(
          epoch: document.coordinate.epoch,
          nodeId: document.roots!.children.single,
          processorId: processor.descriptor.id,
        );
        expect(
          controller.processorErrors.value?.phase,
          ProcessorErrorPhase.process,
        );
        expect(controller.artifacts.view(slot)?.state, 'failed');
        expect(controller.artifacts.view(slot)?.failure?.code, 'panic');
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'processor registration scans nodes that already exist',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _TextProcessor();
      try {
        controller.append('- existing processor input');
        controller.finish();
        final document = controller.value.document!;
        final root = controller.nodeView(document.roots!.children.single)!;
        final item = controller.nodeView(root.node.children.children.single)!;
        final nodeId = item.node.children.children.single;
        final slot = ArtifactSlot(
          epoch: document.coordinate.epoch,
          nodeId: nodeId,
          processorId: processor.descriptor.id,
        );

        controller.registerProcessor(processor);
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(1));
        expect(controller.artifacts.view(slot)?.state, 'ready');
        expect(
          controller.artifacts.view(slot)?.artifact?.payload.text,
          'derived output',
        );
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'same-epoch source-only recovery rebuilds registered artifacts',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final fixture = loadFixture(
        'conformance/fixtures/adoption/headless-rich-content.json',
      );
      final trace = list(fixture['traces'], 'traces')
          .map((value) => record(value, 'trace'))
          .singleWhere((value) => value['id'] == 'adversarial');
      final changes = list(
        trace['changes'],
        'changes',
      ).take(3).map(encodeChange).toList(growable: false);
      final source = runtime.createReducer();
      final replica = MdstreamReplicaController.fromRuntime(runtime);
      final processor = _RecoveryProcessor();

      try {
        source.applyChange(changes[0]);
        replica.applyChange(changes[0]);
        replica.registerProcessor(processor);
        await replica.whenProcessorsIdle();

        final firstRequest = processor.requests.single;
        final slot = ArtifactSlot(
          epoch: firstRequest.key.epoch,
          nodeId: firstRequest.key.nodeId,
          processorId: firstRequest.key.processorId,
        );
        expect(replica.artifacts.view(slot)?.state, 'ready');

        source.applyChange(changes[1]);
        final recovery = source.createRecoverySnapshot()!;
        replica.applyChange(changes[2]);
        expect(replica.value.needsSnapshot, isTrue);
        final recovered = replica.recoverSnapshot(recovery);

        expect(recovered.updates.single.impact.fullReplace, isTrue);
        expect(recovered.updates.single.impact.changedNodeIds, isEmpty);
        expect(replica.artifacts.view(slot), isNull);
        await replica.whenProcessorsIdle();

        expect(processor.requests, hasLength(2));
        expect(
          processor.requests.last.key.generation,
          isNot(firstRequest.key.generation),
        );
        expect(replica.artifacts.view(slot)?.state, 'ready');
      } finally {
        replica.dispose();
        source.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'processor registration snapshots its stable identity',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _MutableIdentityProcessor();
      try {
        final registration = controller.registerProcessor(processor);
        processor.id = 'test.flutter.mutated';
        registration.dispose();

        final replacement = _MutableIdentityProcessor();
        final replacementRegistration = controller.registerProcessor(
          replacement,
        );
        replacementRegistration.dispose();
        await controller.whenProcessorsIdle();
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'unregistering from a pending notification prevents processor execution',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _TextProcessor();
      try {
        controller.append('existing processor input');
        controller.finish();
        final document = controller.value.document!;
        final slot = ArtifactSlot(
          epoch: document.coordinate.epoch,
          nodeId: document.roots!.children.single,
          processorId: processor.descriptor.id,
        );
        final artifact = controller.artifacts.artifact(slot);
        late final ProcessorRegistration registration;
        artifact.addListener(() {
          if (artifact.value?.state == 'pending') {
            registration.dispose();
          }
        });

        registration = controller.registerProcessor(processor);
        await controller.whenProcessorsIdle();

        expect(processor.requests, isEmpty);
        expect(artifact.value, isNull);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'reset from a pending notification prevents processor execution',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _TextProcessor();
      try {
        controller.append('existing processor input');
        controller.finish();
        final document = controller.value.document!;
        final slot = ArtifactSlot(
          epoch: document.coordinate.epoch,
          nodeId: document.roots!.children.single,
          processorId: processor.descriptor.id,
        );
        final artifact = controller.artifacts.artifact(slot);
        var reset = false;
        artifact.addListener(() {
          if (!reset && artifact.value?.state == 'pending') {
            reset = true;
            controller.reset();
          }
        });

        controller.registerProcessor(processor);
        await controller.whenProcessorsIdle();

        expect(reset, isTrue);
        expect(processor.requests, isEmpty);
        expect(artifact.value, isNull);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'late node-version result cannot replace the current generation artifact',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _DeferredProcessor();
      controller.registerProcessor(processor);
      try {
        final firstStarted = processor.nextInvocation;
        controller.append('A');
        final first = await firstStarted;

        final secondStarted = processor.nextInvocation;
        controller.append('B');
        final second = await secondStarted;
        expect(first.context.isCancelled, isTrue);
        expect(
          second.request.key.generation,
          isNot(first.request.key.generation),
        );

        final slot = ArtifactSlot(
          epoch: second.request.key.epoch,
          nodeId: second.request.key.nodeId,
          processorId: processor.descriptor.id,
        );
        final artifact = controller.artifacts.artifact(slot);
        final currentReady = _nextArtifactText(artifact, 'current');
        second.output.complete(
          const ProcessorTextOutput(
            protocol: 'test.flutter.deferred/1',
            mediaType: 'text/plain',
            text: 'current',
          ),
        );
        await currentReady;

        first.output.complete(
          const ProcessorTextOutput(
            protocol: 'test.flutter.deferred/1',
            mediaType: 'text/plain',
            text: 'stale',
          ),
        );
        await controller.whenProcessorsIdle();
        expect(artifact.value?.artifact?.payload.text, 'current');
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'reset cancels an old-epoch lease and rejects its late result',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _DeferredProcessor();
      controller.registerProcessor(processor);
      try {
        final started = processor.nextInvocation;
        controller.append('old epoch');
        final invocation = await started;
        final slot = ArtifactSlot(
          epoch: invocation.request.key.epoch,
          nodeId: invocation.request.key.nodeId,
          processorId: processor.descriptor.id,
        );
        final artifact = controller.artifacts.artifact(slot);
        expect(artifact.value?.state, 'pending');

        controller.reset();
        expect(invocation.context.isCancelled, isTrue);
        expect(artifact.value, isNull);

        invocation.output.complete(
          const ProcessorTextOutput(
            protocol: 'test.flutter.deferred/1',
            mediaType: 'text/plain',
            text: 'too late',
          ),
        );
        await controller.whenProcessorsIdle();
        expect(artifact.value, isNull);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'rechecks a same-epoch candidate changed synchronously by matches',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      var reentered = false;
      late final _CallbackProcessor processor;
      processor = _CallbackProcessor(
        id: 'test.flutter.reentrant-version',
        acceptsProvisional: true,
        allowProvisional: true,
        matchesCallback: (node) {
          if (node.content is! ParagraphContentView) {
            return false;
          }
          final matchedEnd = node.source.end;
          if (!reentered) {
            reentered = true;
            controller.append('b');
          }
          return matchedEnd == '1';
        },
      );
      controller.registerProcessor(processor);

      try {
        controller.append('a');
        await controller.whenProcessorsIdle();

        expect(reentered, isTrue);
        expect(controller.value.document?.coordinate.sourceCursor, '2');
        expect(processor.requests, isEmpty);
        expect(controller.processorErrors.value, isNull);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'rechecks an old-epoch candidate after matches resets the engine',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      var reentered = false;
      late final _CallbackProcessor processor;
      processor = _CallbackProcessor(
        id: 'test.flutter.reentrant-reset',
        acceptsProvisional: true,
        allowProvisional: true,
        matchesCallback: (node) {
          if (node.content is! ParagraphContentView) {
            return false;
          }
          if (!reentered) {
            reentered = true;
            controller.reset();
            controller.append('# replacement\n');
          }
          return true;
        },
      );
      controller.registerProcessor(processor);

      try {
        controller.append('old paragraph');
        await controller.whenProcessorsIdle();

        expect(reentered, isTrue);
        expect(controller.value.document?.coordinate.epoch, '2');
        expect(processor.requests, isEmpty);
        expect(controller.processorErrors.value, isNull);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'queues matching candidates until native dispatch credit is available',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: const {'max_in_flight_jobs': '2'},
        ),
      );
      final processor = _CapacityProcessor();

      try {
        controller.append('one\n\ntwo\n\nthree\n\nfour\n\nfive\n');
        controller.finish();
        controller.registerProcessor(processor);

        await processor.waitForRequests(2);
        expect(processor.requests, hasLength(2));
        processor.releaseAll();
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(5));
        expect(processor.peakActive, lessThanOrEqualTo(2));
        expect(controller.processorErrors.value, isNull);
        for (final request in processor.requests) {
          expect(
            controller.artifacts
                .view(
                  ArtifactSlot(
                    epoch: request.key.epoch,
                    nodeId: request.key.nodeId,
                    processorId: request.key.processorId,
                  ),
                )
                ?.state,
            'ready',
          );
        }
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'lets timer-backed jobs make progress while dispatch is saturated',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: const {'max_in_flight_jobs': '1'},
        ),
      );
      final processor = _TimerProcessor();

      try {
        controller.append('one\n\ntwo\n');
        controller.finish();
        controller.registerProcessor(processor);
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(2));
        expect(processor.completedTimers, 2);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'compacts invalidated candidates without changing survivor order',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: const {'max_in_flight_jobs': '1'},
        ),
      );
      final processor = _ChurnProcessor();

      try {
        controller.append('one\n\ntwo\n\nthree');
        controller.registerProcessor(processor);
        await processor.firstStarted.future;

        // Each pass invalidates and requeues the same tail node while dispatch is full.
        for (var index = 0; index < 256; index += 1) {
          controller.append('x');
          await Future<void>.value();
        }
        processor.releaseFirst();
        await controller.whenProcessorsIdle();

        expect(
          processor.requests.map((request) => request.input.body),
          <String>['one', 'two', 'three${List.filled(256, 'x').join()}'],
        );
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );
}

Future<void> _nextArtifactText(
  ValueListenable<ArtifactView?> artifact,
  String expected,
) {
  if (artifact.value?.artifact?.payload.text == expected) {
    return Future<void>.value();
  }
  final completer = Completer<void>();
  late final VoidCallback listener;
  listener = () {
    if (artifact.value?.artifact?.payload.text == expected) {
      artifact.removeListener(listener);
      completer.complete();
    }
  };
  artifact.addListener(listener);
  return completer.future;
}

final class _TextProcessor implements ContentProcessor {
  final List<ProcessorRequestView> requests = <ProcessorRequestView>[];

  @override
  ContentProcessorDescriptor get descriptor =>
      const ContentProcessorDescriptor(id: 'test.flutter.text', version: 'v1');

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    requests.add(request);
    return const ProcessorTextOutput(
      protocol: 'test.flutter.text/1',
      mediaType: 'text/plain',
      text: 'derived output',
    );
  }
}

final class _ThrowingProcessor implements ContentProcessor {
  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.throwing',
    version: 'v1',
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) => throw StateError('processor exploded');
}

final class _RecoveryProcessor implements ContentProcessor {
  final List<ProcessorRequestView> requests = <ProcessorRequestView>[];

  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.recovery',
    version: 'v1',
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => node.content is HeadingContentView;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    requests.add(request);
    return const ProcessorTextOutput(
      protocol: 'test.flutter.recovery/1',
      mediaType: 'text/plain',
      text: 'recovered output',
    );
  }
}

final class _MutableIdentityProcessor implements ContentProcessor {
  String id = 'test.flutter.mutable';

  @override
  ContentProcessorDescriptor get descriptor =>
      ContentProcessorDescriptor(id: id, version: 'v1');

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => false;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) => throw StateError('mutable identity processor must not run');
}

final class _DeferredInvocation {
  const _DeferredInvocation({
    required this.request,
    required this.context,
    required this.output,
  });

  final ProcessorRequestView request;
  final ProcessorContext context;
  final Completer<ProcessorOutput> output;
}

final class _DeferredProcessor implements ContentProcessor {
  Completer<_DeferredInvocation> _next = Completer<_DeferredInvocation>();

  Future<_DeferredInvocation> get nextInvocation {
    final future = _next.future;
    return future;
  }

  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.deferred',
    version: 'v1',
    acceptsProvisional: true,
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => true;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    final output = Completer<ProcessorOutput>();
    final started = _next;
    _next = Completer<_DeferredInvocation>();
    started.complete(
      _DeferredInvocation(request: request, context: context, output: output),
    );
    return output.future;
  }
}

final class _CallbackProcessor implements ContentProcessor {
  _CallbackProcessor({
    required this.id,
    required this.matchesCallback,
    this.acceptsProvisional = false,
    this.allowProvisional = false,
  });

  final String id;
  final bool Function(ContentNodeView node) matchesCallback;
  final bool acceptsProvisional;
  final List<ProcessorRequestView> requests = <ProcessorRequestView>[];

  @override
  ContentProcessorDescriptor get descriptor => ContentProcessorDescriptor(
    id: id,
    version: 'v1',
    acceptsProvisional: acceptsProvisional,
  );

  @override
  String get configurationVersion => '$id.default';

  @override
  final bool allowProvisional;

  @override
  bool matches(ContentNodeView node) => matchesCallback(node);

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    requests.add(request);
    return ProcessorTextOutput(
      protocol: '$id/1',
      mediaType: 'text/plain',
      text: 'must not be installed',
    );
  }
}

final class _CapacityProcessor implements ContentProcessor {
  final List<ProcessorRequestView> requests = <ProcessorRequestView>[];
  final List<Completer<ProcessorOutput>> _blocked =
      <Completer<ProcessorOutput>>[];
  Completer<void> _changed = Completer<void>();
  bool _releaseImmediately = false;
  int active = 0;
  int peakActive = 0;

  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.dispatch-credit',
    version: 'v1',
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  Future<void> waitForRequests(int count) async {
    while (requests.length < count) {
      final changed = _changed.future;
      await changed;
    }
  }

  void releaseAll() {
    _releaseImmediately = true;
    for (final output in _blocked.toList(growable: false)) {
      output.complete(_output('released'));
    }
    _blocked.clear();
  }

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    requests.add(request);
    active += 1;
    if (active > peakActive) {
      peakActive = active;
    }
    if (!_changed.isCompleted) {
      _changed.complete();
    }
    _changed = Completer<void>();
    if (_releaseImmediately) {
      active -= 1;
      return _output(request.input.body);
    }
    final output = Completer<ProcessorOutput>();
    _blocked.add(output);
    return output.future.whenComplete(() => active -= 1);
  }

  ProcessorTextOutput _output(String text) => ProcessorTextOutput(
    protocol: 'test.flutter.dispatch-credit/1',
    mediaType: 'text/plain',
    text: text,
  );
}

final class _TimerProcessor implements ContentProcessor {
  final List<ProcessorRequestView> requests = <ProcessorRequestView>[];
  int completedTimers = 0;

  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.timer-progress',
    version: 'v1',
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  @override
  Future<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) async {
    requests.add(request);
    await Future<void>.delayed(const Duration(milliseconds: 1));
    completedTimers += 1;
    return ProcessorTextOutput(
      protocol: 'test.flutter.timer-progress/1',
      mediaType: 'text/plain',
      text: request.input.body,
    );
  }
}

final class _ChurnProcessor implements ContentProcessor {
  final List<ProcessorRequestView> requests = <ProcessorRequestView>[];
  final Completer<void> firstStarted = Completer<void>();
  final Completer<ProcessorOutput> _firstOutput = Completer<ProcessorOutput>();

  @override
  ContentProcessorDescriptor get descriptor => const ContentProcessorDescriptor(
    id: 'test.flutter.candidate-churn',
    version: 'v1',
    acceptsProvisional: true,
  );

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => true;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  void releaseFirst() => _firstOutput.complete(_output(requests.first));

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    requests.add(request);
    if (requests.length == 1) {
      firstStarted.complete();
      return _firstOutput.future;
    }
    return _output(request);
  }

  ProcessorTextOutput _output(ProcessorRequestView request) =>
      ProcessorTextOutput(
        protocol: 'test.flutter.candidate-churn/1',
        mediaType: 'text/plain',
        text: request.input.body,
      );
}
