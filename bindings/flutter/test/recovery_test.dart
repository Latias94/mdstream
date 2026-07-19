import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'support/fixtures.dart';
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

  test(
    'replica controller exposes needs-snapshot and full-replace recovery',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final fixture = loadFixture(
        'conformance/fixtures/protocol-linear-source.json',
      );
      final trace = list(fixture['traces'], 'traces')
          .map((value) => record(value, 'trace'))
          .singleWhere((value) => value['id'] == 'characters');
      final changes = list(
        trace['changes'],
        'changes',
      ).map(encodeChange).toList(growable: false);
      final source = runtime.createReducer();
      final replica = MdstreamReplicaController.fromRuntime(
        runtime,
        options: _capturedOptions,
      );
      final transitionBatches = <MdstreamTransitionBatch>[];
      replica.transitions.addListener(() {
        transitionBatches.add(replica.transitions.value);
      });
      var rootNotifications = 0;
      replica.addListener(() => rootNotifications += 1);

      try {
        for (final change in changes.take(3)) {
          source.applyChange(change);
        }
        final recovery = source.createRecoverySnapshot()!;

        replica.applyChange(changes[0]);
        final lastGood = replica.value.document;
        final pending = replica.pendingSource;
        final lastGoodPending = pending.value;
        var pendingNotifications = 0;
        pending.addListener(() => pendingNotifications += 1);
        expect(lastGoodPending?.text, 'a');
        expect(rootNotifications, 1);
        expect(transitionBatches, hasLength(1));
        expect(transitionBatches.single.revision, 1);
        expect(transitionBatches.single.facts, isNotEmpty);

        final sameFloor = replica.createRecoverySnapshot()!;
        expect(transitionBatches.last.revision, 2);
        expect(transitionBatches.last.facts, isEmpty);

        replica.applyChange(changes[0]);
        expect(transitionBatches.last.revision, 3);
        expect(transitionBatches.last.facts, isEmpty);
        expect(rootNotifications, 1);
        expect(pendingNotifications, 0);
        expect(pending.value, same(lastGoodPending));

        replica.applyChange(changes[2]);
        expect(transitionBatches.last.revision, 4);
        expect(transitionBatches.last.facts, isEmpty);
        expect(replica.value.needsSnapshot, isTrue);
        expect(replica.value.document, same(lastGood));
        expect(rootNotifications, 2);
        expect(pendingNotifications, 0);
        expect(pending.value, same(lastGoodPending));

        replica.recoverSnapshot(sameFloor);
        expect(transitionBatches.last.revision, 5);
        expect(transitionBatches.last.facts, isEmpty);
        expect(replica.value.needsSnapshot, isFalse);
        expect(rootNotifications, 3);
        expect(pendingNotifications, 0);
        expect(pending.value, same(lastGoodPending));

        replica.applyChange(changes[2]);
        expect(transitionBatches.last.revision, 6);
        expect(transitionBatches.last.facts, isEmpty);
        expect(replica.value.needsSnapshot, isTrue);
        expect(rootNotifications, 4);
        expect(pendingNotifications, 0);
        expect(pending.value, same(lastGoodPending));

        final missingResource = replica.resource('999');
        var resourceNotifications = 0;
        missingResource.addListener(() => resourceNotifications += 1);
        replica.recoverSnapshot(recovery);

        expect(transitionBatches.last.revision, 7);
        expect(transitionBatches.last.facts, hasLength(1));
        expect(transitionBatches.last.facts.single.scope, 'full_replace');
        expect(replica.value.needsSnapshot, isFalse);
        expect(replica.value.impact.fullReplace, isTrue);
        expect(resourceNotifications, 1);
        expect(rootNotifications, 5);
        expect(pendingNotifications, 1);
        expect(pending.value, isNot(same(lastGoodPending)));
        expect(pending.value?.text, 'abc');
        replica.applyChange(changes[3]);
        expect(transitionBatches.last.revision, 8);
        expect(transitionBatches.last.facts, isNotEmpty);
        expect(replica.value.document?.lifecycle, 'finalized');
        expect(pendingNotifications, 2);
        expect(pending.value, isNull);
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
    'node keys retain same-floor identity and cross same-epoch barriers',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final source = MdstreamController.fromRuntime(runtime);
      final replica = MdstreamReplicaController.fromRuntime(runtime);
      final capturedReplica = MdstreamReplicaController.fromRuntime(
        runtime,
        options: _capturedOptions,
      );
      try {
        final initial = source.append('paragraph');
        final initialChange = initial.changes.single;
        replica.applyChange(initialChange);
        final nodeId = replica.value.document!.roots!.children.single;
        final before = replica.nodeKey(nodeId);
        final sameFloor = replica.createRecoverySnapshot()!;

        source.append(' one');
        final skipped = source.append(' two').changes.single;
        replica.applyChange(skipped);
        expect(replica.value.needsSnapshot, isTrue);
        replica.recoverSnapshot(sameFloor);

        final afterSameFloor = replica.nodeKey(nodeId);
        expect(afterSameFloor, same(before));
        expect(afterSameFloor.continuityGeneration, 0);

        replica.applyChange(skipped);
        expect(replica.value.needsSnapshot, isTrue);
        replica.recoverSnapshot(source.createRecoverySnapshot()!);

        final recoveredNodeId = replica.value.document!.roots!.children.single;
        expect(recoveredNodeId, nodeId);
        final afterAdvanced = replica.nodeKey(recoveredNodeId);
        expect(afterAdvanced.epoch, before.epoch);
        expect(afterAdvanced.nodeId, before.nodeId);
        expect(
          afterAdvanced.continuityGeneration,
          before.continuityGeneration + 1,
        );
        expect(afterAdvanced, isNot(before));
        expect(afterAdvanced, isNot(equals(before)));
        expect(replica.transitions.value.revision, 0);

        capturedReplica.applyChange(initialChange);
        capturedReplica.applyChange(skipped);
        capturedReplica.recoverSnapshot(source.createRecoverySnapshot()!);
        final capturedFacts = capturedReplica.transitions.value.facts.single;
        final capturedNodeId =
            capturedReplica.value.document!.roots!.children.single;
        expect(capturedFacts.scope, 'full_replace');
        expect(
          capturedReplica.nodeKey(capturedNodeId).continuityGeneration,
          int.parse(capturedFacts.after.continuityGeneration),
        );
      } finally {
        capturedReplica.dispose();
        replica.dispose();
        source.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );
}
