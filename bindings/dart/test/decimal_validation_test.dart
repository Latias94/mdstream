import 'package:mdstream/src/errors.dart';
import 'package:mdstream/src/protocol.dart';
import 'package:test/test.dart';

void main() {
  final veryLongDecimal = List.filled(4096, '9').join();

  test('caller u64 inputs use canonical invalid-argument errors', () {
    expect(
      validateDecimalU64Input('18446744073709551615', 'request_id'),
      '18446744073709551615',
    );
    for (final value in [
      '',
      '-1',
      '1.0',
      '01',
      '18446744073709551616',
      veryLongDecimal,
    ]) {
      expect(
        () => validateDecimalU64Input(value, 'request_id'),
        throwsA(
          isA<MdstreamException>()
              .having((error) => error.status, 'status', 1)
              .having(
                (error) => error.statusName,
                'statusName',
                'MDSTREAM_INVALID_ARGUMENT',
              )
              .having(
                (error) => error.detailCode,
                'detailCode',
                'bindings.decimal_id',
              ),
        ),
      );
    }
  });

  test('content IDs retain the complete u128 domain', () {
    expect(
      validateDecimalU128Input(
        '340282366920938463463374607431768211455',
        'node_id',
      ),
      '340282366920938463463374607431768211455',
    );
    for (final value in [
      '01',
      '340282366920938463463374607431768211456',
      veryLongDecimal,
    ]) {
      expect(
        () => validateDecimalU128Input(value, 'node_id'),
        throwsA(isA<MdstreamException>()),
      );
    }
  });

  test('public decimal domain constructors enforce their exact widths', () {
    expect(Epoch.parse('18446744073709551615'), '18446744073709551615');
    expect(
      NodeId.parse('340282366920938463463374607431768211455'),
      '340282366920938463463374607431768211455',
    );
    expect(
      () => Epoch.parse('18446744073709551616'),
      throwsA(
        isA<MdstreamException>()
            .having((error) => error.status, 'status', 1)
            .having(
              (error) => error.detailCode,
              'detailCode',
              'bindings.decimal_id',
            ),
      ),
    );
    expect(
      () => ResourceId.parse('340282366920938463463374607431768211456'),
      throwsA(isA<MdstreamException>()),
    );
  });

  test('opaque domain constructors accept only bounded ASCII identifiers', () {
    expect(ChangeId.parse('change:1_ok'), 'change:1_ok');
    expect(NodeVersion.parse(List.filled(128, 'a').join()), hasLength(128));
    for (final value in <String>[
      '',
      'contains space',
      'non-ascii-é',
      List.filled(129, 'a').join(),
    ]) {
      expect(
        () => ProcessorInputVersion.parse(value),
        throwsA(
          isA<MdstreamException>()
              .having((error) => error.status, 'status', 1)
              .having(
                (error) => error.detailCode,
                'detailCode',
                'bindings.opaque_id',
              ),
        ),
      );
    }
  });

  test('native domain decoders report malformed binding payloads', () {
    expect(
      () => decodeNodeId('01', 'node.id'),
      throwsA(
        isA<MdstreamException>()
            .having((error) => error.status, 'status', 12)
            .having(
              (error) => error.detailCode,
              'detailCode',
              'bindings.invalid_payload',
            ),
      ),
    );
    expect(
      () => decodeNodeVersion('bad value', 'node.version'),
      throwsA(
        isA<MdstreamException>().having((error) => error.status, 'status', 12),
      ),
    );
  });

  test('trusted counters accept non-negative internal values only', () {
    expect(decimalCounterFromTrustedInt(0), '0');
    expect(decimalCounterFromTrustedInt(42), '42');
    expect(() => decimalCounterFromTrustedInt(-1), throwsA(isA<RangeError>()));
  });
}
