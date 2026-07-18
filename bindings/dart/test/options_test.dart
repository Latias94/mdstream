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
}
