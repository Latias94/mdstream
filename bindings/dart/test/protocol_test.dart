import 'dart:typed_data';

import 'package:mdstream/src/errors.dart';
import 'package:mdstream/src/protocol.dart';
import 'package:test/test.dart';

void main() {
  group('binding protocol', () {
    test('publishes the frozen transition schema', () {
      expect(transitionSchema, 'mdstream.transitions/1');
    });

    test('keeps C payload discriminants and view kinds aligned', () {
      expect(BindingPayloadKind.change.value, 1);
      expect(BindingPayloadKind.snapshot.value, 2);
      expect(BindingPayloadKind.reducerUpdate.value, 3);
      expect(BindingPayloadKind.artifactView.value, 9);
      expect(BindingPayloadKind.pendingSourceView.value, 10);
      expect(
        BindingPayloadKind.pendingSourceView.viewKind,
        'pending_source_view',
      );
      expect(
        BindingPayloadKind.fromValue(10),
        BindingPayloadKind.pendingSourceView,
      );
      expect(
        BindingPayloadKind.fromValue(6),
        BindingPayloadKind.processorRequest,
      );
      expect(
        BindingPayloadKind.processorCompletion.viewKind,
        'processor_completion',
      );
    });

    test('keeps C status discriminants and names aligned', () {
      expect(BindingStatus.ok.value, 0);
      expect(BindingStatus.needsSnapshot.value, 9);
      expect(BindingStatus.panic.statusName, 'MDSTREAM_PANIC');
      expect(BindingStatus.fromValue(11), BindingStatus.resourceLimitExceeded);
    });

    test('accepts only canonical unsigned decimal strings', () {
      expect(requireDecimalString('0', 'counter'), '0');
      expect(
        requireDecimalString('18446744073709551615', 'counter'),
        '18446744073709551615',
      );

      for (final value in <Object?>[1, -1, '', '-1', '+1', '01', ' 1']) {
        expect(
          () => requireDecimalString(value, 'counter'),
          throwsA(isA<MdstreamException>()),
          reason: '$value must not cross the binding as a decimal string',
        );
      }
    });

    test('rejects unknown wire discriminants', () {
      expect(
        () => BindingPayloadKind.fromValue(99),
        throwsA(isA<MdstreamException>()),
      );
      expect(
        () => BindingStatus.fromValue(99),
        throwsA(isA<MdstreamException>()),
      );
    });

    test('keeps canonical change and snapshot bytes opaque and immutable', () {
      final source = Uint8List.fromList(<int>[1, 2, 3]);
      final change = CanonicalChangeBytes(source);
      final snapshot = CanonicalSnapshotBytes(source);

      source[0] = 9;
      expect(change.bytes, <int>[1, 2, 3]);
      expect(snapshot.bytes, <int>[1, 2, 3]);
      expect(change.byteLength, 3);
      expect(snapshot.byteLength, 3);

      final exposed = change.bytes..[1] = 9;
      expect(exposed, <int>[1, 9, 3]);
      expect(change.bytes, <int>[1, 2, 3]);
    });

    test(
      'internal canonical bytes adopt owned native buffers without copying',
      () {
        final changeSource = Uint8List.fromList(<int>[1, 2, 3]);
        final snapshotSource = Uint8List.fromList(<int>[4, 5, 6]);
        final change = canonicalChangeBytesFromOwned(changeSource);
        final snapshot = canonicalSnapshotBytesFromOwned(snapshotSource);
        final changeView = canonicalChangeBytesView(change);
        final snapshotView = canonicalSnapshotBytesView(snapshot);

        changeSource[0] = 7;
        snapshotSource[0] = 8;
        expect(changeView, <int>[7, 2, 3]);
        expect(snapshotView, <int>[8, 5, 6]);
        expect(() => changeView[0] = 9, throwsUnsupportedError);
        expect(() => snapshotView[0] = 9, throwsUnsupportedError);
      },
    );

    test('rejects non-octets in canonical byte wrappers', () {
      expect(() => CanonicalChangeBytes(<int>[-1]), throwsRangeError);
      expect(() => CanonicalSnapshotBytes(<int>[256]), throwsRangeError);
    });
  });
}
