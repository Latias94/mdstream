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
      protocol: MdstreamProtocolLimits(maxNodes: '1024'),
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
      () => MdstreamSessionOptions(
        protocol: MdstreamProtocolLimits(maxNodes: '01'),
      ),
      throwsArgumentError,
    );
  });

  test(
    'runtime exposes the validated transition schema before sessions',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      expect(runtime.transitionSchema, transitionSchema);

      final reducer = runtime.createReducer();
      expect(reducer.processorSchedulerLimits.maxInFlightJobs, 32);
      expect(reducer.processorSchedulerLimits.maxQueuedCandidates, 256);
      reducer.close();

      final engine = runtime.createEngine(
        options: MdstreamSessionOptions(
          processor: MdstreamProcessorLimits(
            maxInFlightJobs: '3',
            maxSlots: '7',
          ),
        ),
      );
      expect(engine.processorSchedulerLimits.maxInFlightJobs, 3);
      expect(engine.processorSchedulerLimits.maxQueuedCandidates, 7);
      engine.close();
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'engine rejects raw source overflow before encoding and keeps state usable',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine(
        options: MdstreamSessionOptions(
          protocol: MdstreamProtocolLimits(maxSourceBytes: '1'),
        ),
      );
      try {
        expect(
          () => engine.append('éé'),
          throwsA(
            isA<MdstreamException>()
                .having((error) => error.status, 'status', 11)
                .having(
                  (error) => error.detailCode,
                  'detailCode',
                  'bindings.resource_limit',
                )
                .having(
                  (error) => error.splitSafety,
                  'splitSafety',
                  SplitSafety.notSafe,
                ),
          ),
        );
        expect(engine.state.currentState.document, isNull);

        expect(
          () => engine.append('é'),
          throwsA(
            isA<MdstreamException>()
                .having((error) => error.status, 'status', 11)
                .having(
                  (error) => error.detailCode,
                  'detailCode',
                  'protocol.source_too_large',
                )
                .having(
                  (error) => error.splitSafety,
                  'splitSafety',
                  SplitSafety.notSafe,
                ),
          ),
        );
        expect(engine.state.currentState.document, isNull);

        engine.append('a');
        final acceptedDocument = engine.state.currentState.document;
        expect(acceptedDocument, isNotNull);
        expect(
          () => engine.append('b'),
          throwsA(
            isA<MdstreamException>().having(
              (error) => error.status,
              'status',
              11,
            ),
          ),
        );
        expect(engine.state.currentState.document, same(acceptedDocument));
      } finally {
        engine.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'finalized append preserves the native terminal error',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine();
      try {
        engine.finish();
        expect(
          () => engine.append('late'),
          throwsA(
            isA<MdstreamException>()
                .having((error) => error.status, 'status', 6)
                .having(
                  (error) => error.detailCode,
                  'detailCode',
                  'engine.finished',
                )
                .having(
                  (error) => error.splitSafety,
                  'splitSafety',
                  SplitSafety.notSafe,
                ),
          ),
        );
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
