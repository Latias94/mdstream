import 'package:mdstream/src/options.dart';
import 'package:test/test.dart';

void main() {
  test(
    'omits defaulted custom-block booleans and preserves explicit false',
    () {
      const defaulted = MdstreamCustomBlock(
        namespace: 'app',
        name: 'defaulted',
      );
      const explicit = MdstreamCustomBlock(
        namespace: 'app',
        name: 'explicit',
        opaque: false,
        caseInsensitive: false,
      );
      final options = MdstreamSessionOptions(
        customBlocks: const [defaulted, explicit],
      );

      expect(defaulted.opaque, isNull);
      expect(defaulted.caseInsensitive, isNull);
      expect(options.toJson('mdstream.bindings-options/0.4'), {
        'schema': 'mdstream.bindings-options/0.4',
        'custom_blocks': [
          {'namespace': 'app', 'name': 'defaulted'},
          {
            'namespace': 'app',
            'name': 'explicit',
            'opaque': false,
            'case_insensitive': false,
          },
        ],
      });
    },
  );

  test('maps transition capture and reducer-update budget options', () {
    final options = MdstreamSessionOptions(
      captureTransitions: true,
      wire: const {'max_reducer_update_bytes': '32768'},
    );

    expect(options.captureTransitions, isTrue);
    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
      'capture_transitions': true,
      'wire': {'max_reducer_update_bytes': '32768'},
    });
  });

  test('omits disabled transition capture and accepts the native wire key', () {
    final options = MdstreamSessionOptions(
      wire: const {'max_reducer_update_bytes': '65536'},
    );

    expect(options.captureTransitions, isFalse);
    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
      'wire': {'max_reducer_update_bytes': '65536'},
    });
  });

  test('strictly rejects removed impact-budget spellings', () {
    expect(
      () => MdstreamSessionOptions(wire: const {'max_impact_bytes': '32768'}),
      throwsArgumentError,
    );
    expect(
      () => MdstreamSessionOptions(wire: const {'maxImpactBytes': '32768'}),
      throwsArgumentError,
    );
  });
}
