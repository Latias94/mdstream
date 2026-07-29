import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/batch_candidate.dart';
import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'constituent-first wins every KTD3 semantic-join workload',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final workloads = <String, List<String>>{
        'one-byte': '# Linear input\n\nOne byte at a time.'.split(''),
        'bursty': <String>[
          '# Bursty\n\n',
          'A ',
          'short burst',
          ' followed by ',
          'another.\n',
        ],
        'unicode': <String>['多', '语言', ' 🙂', ' cafe\u0301', '\n'],
        'crlf': <String>['alpha\r', '\nbe', 'ta\r', 'gamma\r', '\n'],
        'golden-ai': _goldenChunks(),
      };

      for (final MapEntry(key: name, value: chunks) in workloads.entries) {
        final joined = runBatchCandidate(
          runtime,
          BatchCandidatePolicy.joinedFirst,
          chunks,
        );
        final constituent = runBatchCandidate(
          runtime,
          BatchCandidatePolicy.constituentFirst,
          chunks,
        );

        expect(joined.snapshot, constituent.snapshot, reason: '$name final IR');
        expect(joined.scanBytes, constituent.scanBytes, reason: '$name scan');
        expect(joined.replayCount, BigInt.zero, reason: '$name joined replay');
        expect(
          constituent.replayCount,
          BigInt.zero,
          reason: '$name constituent replay',
        );
        expect(
          _improvesByQuarter(
                joined.appendAttempts,
                constituent.appendAttempts,
              ) ||
              _improvesByQuarter(
                joined.encodedResultBytes,
                constituent.encodedResultBytes,
              ),
          isTrue,
          reason: '$name must demonstrate the intended batching benefit',
        );
        expect(
          _withinTwentyPercent(
                joined.appendAttempts,
                constituent.appendAttempts,
              ) &&
              _withinTwentyPercent(
                joined.encodedResultBytes,
                constituent.encodedResultBytes,
              ) &&
              _withinTwentyPercent(joined.scanBytes, constituent.scanBytes),
          isTrue,
          reason: '$name non-copy work must stay within the regression budget',
        );
        expect(
          _withinTwentyPercent(joined.joinCopyBytes, constituent.joinCopyBytes),
          isFalse,
          reason: '$name joined copy work must fail the no-regression gate',
        );

        // Printed output is copied into the pre-release migration decision table.
        // ignore: avoid_print
        print(
          'KTD3 $name: joined attempts=${joined.appendAttempts} '
          'encoded=${joined.encodedResultBytes} scan=${joined.scanBytes} '
          'copy=${joined.joinCopyBytes}; constituent '
          'attempts=${constituent.appendAttempts} '
          'encoded=${constituent.encodedResultBytes} '
          'scan=${constituent.scanBytes} copy=${constituent.joinCopyBytes}; '
          'decision=constituent-first',
        );
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    '10k nodes and 100k reads materialize only 16 explicit views',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine(
        options: MdstreamSessionOptions(
          protocol: MdstreamProtocolLimits(maxOperations: '40000'),
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

List<String> _goldenChunks() {
  final scenario = loadFixture('examples/fixtures/golden-ai-stream.json');
  final episodes = record(scenario['episodes'], 'episodes');
  final mainline = record(episodes['mainline'], 'mainline');
  return list(mainline['actions'], 'actions')
      .map((action) => record(action, 'action'))
      .where((action) => action['kind'] == 'append')
      .map((action) => action['chunk']! as String)
      .toList(growable: false);
}

bool _improvesByQuarter(BigInt candidate, BigInt baseline) =>
    candidate * BigInt.from(4) <= baseline * BigInt.from(3);

bool _withinTwentyPercent(BigInt candidate, BigInt baseline) =>
    candidate * BigInt.from(5) <= baseline * BigInt.from(6);
