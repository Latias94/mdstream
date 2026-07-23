import 'dart:convert';
import 'dart:typed_data';

import 'package:mdstream/src/errors.dart';
import 'package:test/test.dart';

void main() {
  group('MdstreamException', () {
    test('preserves the native structured error envelope', () {
      final error = MdstreamException.fromJsonBytes(
        Uint8List.fromList(
          utf8.encode(
            jsonEncode(<String, Object?>{
              'schema': 'mdstream.bindings/0.4',
              'ok': false,
              'status': 9,
              'status_name': 'MDSTREAM_NEEDS_SNAPSHOT',
              'detail_code': 'protocol.sequence_gap',
              'message': 'a snapshot is required',
              'split_safety': 'retry_at_original_boundaries',
            }),
          ),
        ),
      );

      expect(error.schema, 'mdstream.bindings/0.4');
      expect(error.status, 9);
      expect(error.statusName, 'MDSTREAM_NEEDS_SNAPSHOT');
      expect(error.detailCode, 'protocol.sequence_gap');
      expect(error.splitSafety, SplitSafety.retryAtOriginalBoundaries);
      expect(error.message, 'a snapshot is required');
      expect(error.toString(), contains('a snapshot is required'));
    });

    test('supports direct construction by the FFI layer', () {
      final cause = StateError('native call failed');
      final error = MdstreamException(
        'operation failed',
        status: 6,
        statusName: 'MDSTREAM_TERMINAL',
        detailCode: 'engine.finished',
        schema: 'mdstream.bindings/0.4',
        cause: cause,
      );

      expect(error.status, 6);
      expect(error.statusName, 'MDSTREAM_TERMINAL');
      expect(error.detailCode, 'engine.finished');
      expect(error.schema, 'mdstream.bindings/0.4');
      expect(error.cause, same(cause));
      expect(error.splitSafety, SplitSafety.notSafe);
    });

    test('normalizes malformed host errors without hiding the cause', () {
      final malformed = Uint8List.fromList(<int>[0xff]);
      final error = MdstreamException.fromJsonBytes(
        malformed,
        fallbackStatus: 7,
      );

      expect(error.status, 7);
      expect(error.statusName, 'MDSTREAM_INTERNAL_ERROR');
      expect(error.detailCode, 'bindings.invalid_error_payload');
      expect(error.cause, isNotNull);
    });

    test('uses typed defaults for non-envelope Dart failures', () {
      final cause = ArgumentError('bad input');
      final error = MdstreamException.fromObject(cause);

      expect(error.status, 12);
      expect(error.statusName, 'MDSTREAM_INTERNAL_ERROR');
      expect(error.detailCode, 'bindings.dart_error');
      expect(error.message, contains('bad input'));
      expect(error.cause, same(cause));
    });
  });
}
