import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

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
      final replica = MdstreamReplicaController.fromRuntime(runtime);
      var rootNotifications = 0;
      replica.addListener(() => rootNotifications += 1);

      try {
        for (final change in changes.take(3)) {
          source.applyChange(change);
        }
        final recovery = source.createRecoverySnapshot()!;

        replica.applyChange(changes[0]);
        final lastGood = replica.value.document;
        expect(rootNotifications, 1);

        replica.applyChange(changes[0]);
        expect(rootNotifications, 1);

        replica.applyChange(changes[2]);
        expect(replica.value.needsSnapshot, isTrue);
        expect(replica.value.document, same(lastGood));
        expect(rootNotifications, 2);

        final missingResource = replica.resource('999');
        var resourceNotifications = 0;
        missingResource.addListener(() => resourceNotifications += 1);
        replica.recoverSnapshot(recovery);

        expect(replica.value.needsSnapshot, isFalse);
        expect(replica.value.impact.fullReplace, isTrue);
        expect(resourceNotifications, 1);
        expect(rootNotifications, 3);
        replica.applyChange(changes[3]);
        expect(replica.value.document?.lifecycle, 'finalized');
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
}
