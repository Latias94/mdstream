import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    '10k nodes and 100k reads materialize only 16 explicit views',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine(
        options: MdstreamSessionOptions(
          protocol: const {'max_operations': '40000'},
        ),
      );
      try {
        final source = StringBuffer();
        for (var index = 0; index < 10000; index += 1) {
          source.write('paragraph $index\n\n');
        }
        final appended = engine.append(source.toString());
        final finished = engine.finish();

        final roots = engine.state.currentState.document?.roots?.children;
        expect(roots, hasLength(10000));
        expect(engine.reducerMetrics.nodeViewPayloads, '0');
        expect(engine.metrics.snapshotPayloads, '0');
        expect(engine.reducerMetrics.snapshotPayloads, '0');
        expect(
          appended.reducerResults.expand((result) => result.transitionFacts),
          isEmpty,
        );
        expect(
          finished.reducerResults.expand((result) => result.transitionFacts),
          isEmpty,
        );

        final accessed = roots!.take(16).toList(growable: false);
        final first = accessed
            .map(engine.state.nodeView)
            .toList(growable: false);
        var referencesStable = true;
        for (var index = 0; index < 100000; index += 1) {
          final slot = index % accessed.length;
          referencesStable =
              referencesStable &&
              identical(engine.state.nodeView(accessed[slot]), first[slot]);
        }

        expect(referencesStable, isTrue);
        expect(engine.reducerMetrics.nodeViewPayloads, '16');
        expect(engine.metrics.snapshotPayloads, '0');
        expect(engine.reducerMetrics.snapshotPayloads, '0');
      } finally {
        engine.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    timeout: const Timeout(Duration(minutes: 2)),
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}
