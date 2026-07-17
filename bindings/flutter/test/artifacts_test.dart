import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

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
  bool matches(ContentNodeView node) => node.content['kind'] == 'paragraph';

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
  bool matches(ContentNodeView node) => node.content['kind'] == 'paragraph';

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) => throw StateError('processor exploded');
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
  bool matches(ContentNodeView node) => node.content['kind'] == 'paragraph';

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
