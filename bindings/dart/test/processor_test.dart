import 'dart:convert';

import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'processor leases produce derived artifacts outside canonical snapshots',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine();
      try {
        engine.append('processor body');
        engine.finish();
        final nodeId =
            engine.state.currentState.document!.roots!.children.single;

        final begun = engine.beginProcessor(
          nodeId: nodeId,
          processorId: 'test.dart.processor',
          processorVersion: 'v1',
          configurationVersion: 'default-v1',
        );
        expect(begun.processorRequests, hasLength(1));
        expect(begun.artifactChanges.single.change.kind, 'pending');
        final request = begun.processorRequests.single;
        final completed = engine.completeProcessorText(
          requestId: request.requestId,
          protocol: 'test.dart.processor/1',
          mediaType: 'text/plain',
          text: 'derived output',
        );
        expect(completed.processorCompletions.single.outcome, 'applied');
        expect(completed.artifactChanges.single.change.kind, 'ready');

        final artifact = engine.state.artifactView(
          ArtifactSlot(
            epoch: request.key.epoch,
            nodeId: request.key.nodeId,
            processorId: request.key.processorId,
          ),
        );
        expect(artifact?.state, 'ready');
        expect(artifact?.artifact?.payload.kind, 'text');
        expect(artifact?.artifact?.payload.text, 'derived output');

        final snapshot = engine.createRecoverySnapshot()!;
        final canonical = utf8.decode(snapshot.bytes);
        expect(canonical, isNot(contains('test.dart.processor')));
        expect(canonical, isNot(contains('derived output')));
      } finally {
        engine.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}
