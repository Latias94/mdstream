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
        expect(artifact.value?.state, ArtifactState.ready);
        expect(_artifactText(artifact.value), 'derived output');
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
        expect(controller.artifacts.view(slot)?.state, ArtifactState.failed);
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
        expect(controller.artifacts.view(slot)?.state, ArtifactState.ready);
        expect(
          _artifactText(controller.artifacts.view(slot)),
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
        expect(replica.artifacts.view(slot)?.state, ArtifactState.ready);

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
        expect(replica.artifacts.view(slot)?.state, ArtifactState.ready);
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
    'processor registration rejects invalid identities before scanning',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      const invalidValues = <String>['', 'bad/id', 'non-ascii-é'];
      final values = <String>[
        ...invalidValues,
        List<String>.filled(129, 'x').join(),
      ];
      try {
        controller.append('identity validation input');
        controller.finish();
        for (final value in values) {
          expect(
            () => controller.registerProcessor(
              _IdentityProcessor(
                id: value,
                version: 'v1',
                configuration: 'default',
              ),
            ),
            throwsArgumentError,
          );
          expect(
            () => controller.registerProcessor(
              _IdentityProcessor(
                id: 'test.flutter.valid-id',
                version: value,
                configuration: 'default',
              ),
            ),
            throwsArgumentError,
          );
          expect(
            () => controller.registerProcessor(
              _IdentityProcessor(
                id: 'test.flutter.valid-id',
                version: 'v1',
                configuration: value,
              ),
            ),
            throwsArgumentError,
          );
        }
        await controller.whenProcessorsIdle();
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
          if (artifact.value?.state == ArtifactState.pending) {
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
          if (!reset && artifact.value?.state == ArtifactState.pending) {
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
        expect(_artifactText(artifact.value), 'current');
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
        expect(artifact.value?.state, ArtifactState.pending);

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
    'rematches a topology-only input change with a stable node version',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamReplicaController.fromRuntime(runtime);
      final fixture = loadFixture(
        'bindings/flutter/test/fixtures/processor-topology.json',
      );
      final changes = list(
        fixture['changes'],
        'changes',
      ).map(encodeChange).toList(growable: false);
      final matchedVersions = <NodeVersion>[];
      final matchedChildrenVersions = <StructureVersion>[];
      final matchedChildCounts = <int>[];
      var reentered = false;
      final processor = _CallbackProcessor(
        id: 'test.flutter.topology-freshness',
        acceptsProvisional: true,
        allowProvisional: true,
        matchesCallback: (node) {
          if (node.content is! BlockQuoteContentView) {
            return false;
          }
          matchedVersions.add(node.version);
          matchedChildrenVersions.add(node.children.version);
          matchedChildCounts.add(node.children.children.length);
          if (!reentered) {
            reentered = true;
            controller.applyChange(changes[1]);
          }
          return true;
        },
      );
      controller.registerProcessor(processor);

      try {
        controller.applyChange(changes[0]);
        await controller.whenProcessorsIdle();

        expect(reentered, isTrue);
        expect(matchedChildCounts, <int>[1, 2]);
        expect(matchedVersions, hasLength(2));
        expect(matchedVersions[1], matchedVersions[0]);
        expect(matchedChildrenVersions, hasLength(2));
        expect(matchedChildrenVersions[1], isNot(matchedChildrenVersions[0]));
        expect(processor.requests, hasLength(1));
        final finalView = controller.nodeView(NodeId.parse('1'))!;
        expect(
          processor.requests.single.key.inputVersion,
          finalView.processorInputVersion,
        );
        expect(
          processor.requests.single.input.node.children.children,
          hasLength(2),
        );
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
          processor: MdstreamProcessorLimits(maxInFlightJobs: '2'),
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
            ArtifactState.ready,
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
    'continues past a permanent resource limit while another job is active',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInputBytes: '1024',
            maxInFlightJobs: '2',
          ),
        ),
      );
      final processor = _CapacityProcessor();

      try {
        final oversized = List<String>.filled(4000, 'x').join();
        controller.append('first\n\n$oversized\n\nthird');
        controller.finish();
        controller.registerProcessor(processor);

        await processor.waitForRequests(2).timeout(const Duration(seconds: 2));
        expect(
          processor.requests.map((request) => request.input.body),
          <String>['first', 'third'],
        );
        expect(
          controller.processorErrors.value?.phase,
          ProcessorErrorPhase.begin,
        );
        expect(
          (controller.processorErrors.value?.error as MdstreamException)
              .detailCode,
          'processor.resource_limit.input_bytes',
        );

        processor.releaseAll();
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
    'derives candidate capacity from the native processor slot limit',
    () async {
      const candidateCount = 4097;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          protocol: MdstreamProtocolLimits(maxOperations: '20000'),
          processor: MdstreamProcessorLimits(
            maxSlots: '4097',
            maxRetainedArtifacts: '4097',
            maxRetainedArtifactBytes: '10000000',
            maxPendingChanges: '4097',
            maxPendingChangeBytes: '4000000',
          ),
          wire: MdstreamWireLimits(maxArtifactEventBytes: '25000000'),
        ),
      );
      final processor = _TextProcessor();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(candidateCount));
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
    'yields after one dispatch quantum before starting remaining jobs',
    () async {
      const candidateCount = 64;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '$candidateCount',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      final processor = _CapacityProcessor();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);

        await Future<void>.value();
        expect(processor.requests, hasLength(32));

        processor.releaseAll();
        await controller.whenProcessorsIdle();
        expect(processor.requests, hasLength(candidateCount));
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
    'lets event-queue work run between synchronous dispatch quanta',
    () async {
      const candidateCount = 96;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '$candidateCount',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      final processor = _TextProcessor();
      final eventMarker = Completer<int>();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);
        Timer.run(() {
          if (!eventMarker.isCompleted) {
            eventMarker.complete(processor.requests.length);
          }
        });

        expect(await eventMarker.future, 32);
        expect(processor.requests, hasLength(32));

        await controller.whenProcessorsIdle();
        expect(processor.requests, hasLength(candidateCount));
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
    'lets timer work run before a large tree scan completes',
    () async {
      const candidateCount = 256;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '$candidateCount',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      var matchCalls = 0;
      final processor = _CallbackProcessor(
        id: 'test.flutter.scan-quantum',
        matchesCallback: (node) {
          matchCalls += 1;
          return node.content is ParagraphContentView;
        },
      );
      final matchesAtTimer = Completer<int>();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);
        Timer.run(() => matchesAtTimer.complete(matchCalls));

        final observedMatchCalls = await matchesAtTimer.future;
        await controller.whenProcessorsIdle();

        expect(matchCalls, greaterThan(observedMatchCalls));
        expect(processor.requests, hasLength(candidateCount));
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
    'counts each processor match against the scan quantum',
    () async {
      const processorCount = 256;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      var matchCalls = 0;
      final matchesAtTimer = Completer<int>();

      try {
        controller.append('one node');
        controller.finish();
        for (var index = 0; index < processorCount; index += 1) {
          controller.registerProcessor(
            _CallbackProcessor(
              id: 'test.flutter.match-quantum.$index',
              matchesCallback: (node) {
                matchCalls += 1;
                return false;
              },
            ),
          );
        }
        Timer.run(() => matchesAtTimer.complete(matchCalls));

        final observedMatchCalls = await matchesAtTimer.future;
        await controller.whenProcessorsIdle();

        expect(observedMatchCalls, greaterThan(0));
        expect(observedMatchCalls, lessThan(processorCount));
        expect(matchCalls, greaterThanOrEqualTo(processorCount));
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
    'stops a large tree scan after its last processor is unregistered',
    () async {
      const candidateCount = 256;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(maxSlots: '$candidateCount'),
        ),
      );
      var matchCalls = 0;
      final processor = _CallbackProcessor(
        id: 'test.flutter.scan-dispose',
        matchesCallback: (node) {
          matchCalls += 1;
          return false;
        },
      );
      final disposed = Completer<int>();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        final registration = controller.registerProcessor(processor);
        Timer.run(() {
          final calls = matchCalls;
          registration.dispose();
          disposed.complete(calls);
        });

        final callsAtDispose = await disposed.future;
        await controller.whenProcessorsIdle();
        await Future<void>.delayed(Duration.zero);

        expect(callsAtDispose, greaterThan(0));
        expect(callsAtDispose, lessThan(candidateCount));
        expect(matchCalls, callsAtDispose);
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
    'removes one unregistered processor from an active shared scan',
    () async {
      const candidateCount = 128;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(maxSlots: '$candidateCount'),
        ),
      );
      var firstMatchCalls = 0;
      var secondMatchCalls = 0;
      final first = _CallbackProcessor(
        id: 'test.flutter.shared-dispose.first',
        matchesCallback: (node) {
          firstMatchCalls += 1;
          return false;
        },
      );
      final second = _CallbackProcessor(
        id: 'test.flutter.shared-dispose.second',
        matchesCallback: (node) {
          secondMatchCalls += 1;
          return false;
        },
      );
      final disposed = Completer<int>();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        final firstRegistration = controller.registerProcessor(first);
        controller.registerProcessor(second);
        Timer.run(() {
          final calls = firstMatchCalls;
          firstRegistration.dispose();
          disposed.complete(calls);
        });

        final callsAtDispose = await disposed.future;
        await controller.whenProcessorsIdle();

        expect(callsAtDispose, greaterThan(0));
        expect(firstMatchCalls, callsAtDispose);
        expect(secondMatchCalls, greaterThan(callsAtDispose));
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
    'resumes an input-budget block only after capacity changes',
    () async {
      const candidateCount = 130;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInputBytes: '2048',
            maxInFlightJobs: '2',
            maxInFlightInputBytes: '1536',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      final processor = _CapacityProcessor();

      try {
        final body = List<String>.filled(512, 'x').join();
        controller.append(
          List<String>.filled(candidateCount, body).join('\n\n'),
        );
        controller.finish();
        controller.registerProcessor(processor);

        await processor.waitForRequests(1);
        await Future<void>.delayed(const Duration(milliseconds: 10));
        expect(processor.requests, hasLength(1));

        processor.releaseAll();
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(candidateCount));
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
    'lets event-queue work run between synchronous completion refills',
    () async {
      const candidateCount = 3;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '1',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      final processor = _TextProcessor();
      final eventMarker = Completer<void>();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);
        Timer.run(() {
          if (!eventMarker.isCompleted) {
            eventMarker.complete();
          }
        });

        await eventMarker.future;
        expect(processor.requests, hasLength(1));
        final firstRequest = processor.requests.single;
        expect(
          controller.artifacts
              .view(
                ArtifactSlot(
                  epoch: firstRequest.key.epoch,
                  nodeId: firstRequest.key.nodeId,
                  processorId: firstRequest.key.processorId,
                ),
              )
              ?.state,
          ArtifactState.ready,
        );

        await controller.whenProcessorsIdle();
        expect(processor.requests, hasLength(candidateCount));
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
    'reset invalidates a dispatch continuation from the previous tree',
    () async {
      const candidateCount = 64;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '$candidateCount',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      final processor = _CapacityProcessor();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'paragraph $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);

        await Future<void>.value();
        expect(processor.requests, hasLength(32));

        controller.reset();
        await Future<void>.value();
        await Future<void>.value();
        expect(processor.requests, hasLength(32));

        processor.releaseAll();
        await controller.whenProcessorsIdle();
        expect(processor.requests, hasLength(32));
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
    'reset invalidates a tree scan continuation from the previous tree',
    () async {
      const candidateCount = 256;
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '$candidateCount',
            maxSlots: '$candidateCount',
          ),
        ),
      );
      final replacement = 'old ${candidateCount - 1}';
      NodeId? replacementNodeId;
      var replacementMatchCalls = 0;
      final processor = _CallbackProcessor(
        id: 'test.flutter.reset-scan',
        matchesCallback: (node) {
          if (node.id == replacementNodeId) {
            replacementMatchCalls += 1;
          }
          return node.content is ParagraphContentView;
        },
      );
      final resetMarker = Completer<int>();

      try {
        final source = List<String>.generate(
          candidateCount,
          (index) => 'old $index',
        ).join('\n\n');
        controller.append(source);
        controller.finish();
        controller.registerProcessor(processor);
        Timer.run(() {
          final requestCount = processor.requests.length;
          controller.reset();
          controller.append(replacement);
          controller.finish();
          replacementNodeId = controller.value.document!.roots!.children.single;
          resetMarker.complete(requestCount);
        });

        final oldRequestCount = await resetMarker.future;
        await controller.whenProcessorsIdle();

        expect(
          processor.requests
              .skip(oldRequestCount)
              .map((request) => request.input.body),
          <String>[replacement],
        );
        expect(replacementMatchCalls, 1);
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
    'reports candidate queue saturation once until capacity recovers',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '1',
            maxSlots: '2',
          ),
        ),
      );
      final processor = _CapacityProcessor();
      var queueLimitErrors = 0;
      var slotLimitErrors = 0;
      controller.processorErrors.addListener(() {
        final error = controller.processorErrors.value?.error;
        if (error is MdstreamException) {
          if (error.detailCode == 'processor.candidate_queue_limit') {
            queueLimitErrors += 1;
          } else if (error.detailCode == 'processor.resource_limit.slots') {
            slotLimitErrors += 1;
          }
        }
      });

      try {
        controller.append('one\n\ntwo\n\nthree\n\nfour\n\nfive');
        controller.finish();
        controller.registerProcessor(processor);

        await processor.waitForRequests(1);
        expect(queueLimitErrors, 1);
        processor.releaseAll();
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(2));
        expect(slotLimitErrors, 3);
        expect(queueLimitErrors, 1);

        controller.reset();
        controller.append('six\n\nseven\n\neight\n\nnine\n\nten');
        controller.finish();
        await controller.whenProcessorsIdle();

        expect(processor.requests, hasLength(4));
        expect(slotLimitErrors, 6);
        expect(queueLimitErrors, 2);
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
    'reset from a saturation notification cancels the paused scan',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(
        runtime,
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '1',
            maxSlots: '2',
          ),
        ),
      );
      final processor = _CallbackProcessor(
        id: 'test.flutter.saturation-reset',
        matchesCallback: (node) => node.content is ParagraphContentView,
      );
      var reset = false;
      controller.processorErrors.addListener(() {
        final error = controller.processorErrors.value?.error;
        if (!reset &&
            error is MdstreamException &&
            error.detailCode == 'processor.candidate_queue_limit') {
          reset = true;
          controller.reset();
          controller.finish();
        }
      });

      try {
        controller.append('one\n\ntwo\n\nthree\n\nfour\n\nfive');
        controller.finish();
        controller.registerProcessor(processor);

        await controller.whenProcessorsIdle().timeout(
          const Duration(seconds: 2),
        );

        expect(reset, isTrue);
        expect(processor.requests, isEmpty);
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
          processor: MdstreamProcessorLimits(maxInFlightJobs: '1'),
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
          processor: MdstreamProcessorLimits(maxInFlightJobs: '1'),
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
  if (_artifactText(artifact.value) == expected) {
    return Future<void>.value();
  }
  final completer = Completer<void>();
  late final VoidCallback listener;
  listener = () {
    if (_artifactText(artifact.value) == expected) {
      artifact.removeListener(listener);
      completer.complete();
    }
  };
  artifact.addListener(listener);
  return completer.future;
}

String? _artifactText(ArtifactView? view) => switch (view?.artifact?.payload) {
  TextArtifactPayloadView(:final text) => text,
  _ => null,
};

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

final class _IdentityProcessor implements ContentProcessor {
  const _IdentityProcessor({
    required this.id,
    required this.version,
    required this.configuration,
  });

  final String id;
  final String version;
  final String configuration;

  @override
  ContentProcessorDescriptor get descriptor =>
      ContentProcessorDescriptor(id: id, version: version);

  @override
  String get configurationVersion => configuration;

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => true;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) => throw StateError('invalid identity processor must not run');
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
