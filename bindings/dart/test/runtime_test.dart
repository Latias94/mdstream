import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test('runtime maps a missing host-supplied library to a typed error', () {
    expect(
      () => MdstreamRuntime.openPath(
        '/path-that-cannot-exist/mdstream-ffi-for-runtime-test',
      ),
      throwsA(
        isA<MdstreamException>().having(
          (error) => error.detailCode,
          'detailCode',
          'ffi.library_open',
        ),
      ),
    );
  });

  test('session options encode decimal strings and reject JSON numbers', () {
    final options = MdstreamSessionOptions(
      protocol: const {'max_nodes': '1024'},
      customBlocks: const [
        MdstreamCustomBlock(namespace: 'app', name: 'panel'),
      ],
    );

    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
      'protocol': {'max_nodes': '1024'},
      'custom_blocks': [
        {'namespace': 'app', 'name': 'panel'},
      ],
    });
    expect(
      () => MdstreamSessionOptions(protocol: const {'max_nodes': '01'}),
      throwsArgumentError,
    );
  });

  test(
    'runtime exposes the validated transition schema before sessions',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      expect(runtime.transitionSchema, transitionSchema);

      final reducer = runtime.createReducer();
      reducer.close();
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}
