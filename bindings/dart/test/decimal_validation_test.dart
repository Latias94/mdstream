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
}
