import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  group('LosslessInputBatcher', () {
    test('preserves UTF-8 content across configurable batch sizes', () {
      const chunks = <String>[
        '# Batch\r',
        '',
        '\n\n',
        'emoji 👩‍💻 and ',
        'accent é',
        '\n\n```mermaid\nflowchart LR\nA-->B\n```',
      ];
      final expected = chunks.join();

      for (final maxBytes in <int>[1, 16, 128, 4096]) {
        final forwarded = <String>[];
        final batcher = LosslessInputBatcher<int>(
          maxBatchBytes: maxBytes,
          append: (chunk) {
            forwarded.add(chunk);
            return utf8ByteLength(chunk);
          },
          finish: () => 0,
          outputPayloadBytes: (result) => result,
        );

        for (final chunk in chunks) {
          batcher.push(chunk);
        }
        expect(batcher.finish().last, 0);

        expect(forwarded.join(), expected);
        expect(batcher.metrics.inputBytes, utf8ByteLength(expected).toString());
        expect(batcher.metrics.forwardedBytes, batcher.metrics.inputBytes);
        expect(batcher.metrics.pendingBytes, '0');
        expect(int.parse(batcher.metrics.appendCalls), greaterThan(0));
      }
    });

    test('does not flush or append an empty chunk', () {
      final forwarded = <String>[];
      final batcher = LosslessInputBatcher<void>(
        maxBatchBytes: 128,
        append: forwarded.add,
      );

      batcher.push('line\r');
      batcher.push('');
      expect(forwarded, isEmpty);
      expect(batcher.metrics.pendingBytes, '5');
      batcher.push('\nnext');
      batcher.flush();

      expect(forwarded, <String>['line\r\nnext']);
    });

    test('keeps pending input available when append throws', () {
      var attempts = 0;
      final forwarded = <String>[];
      final batcher = LosslessInputBatcher<void>(
        maxBatchBytes: 16,
        append: (chunk) {
          attempts += 1;
          if (attempts == 1) {
            throw StateError('transient');
          }
          forwarded.add(chunk);
        },
      );

      batcher.push('retry');
      expect(batcher.flush, throwsStateError);
      expect(batcher.metrics.pendingBytes, '5');
      batcher.flush();

      expect(forwarded, <String>['retry']);
      expect(batcher.metrics.forwardedBytes, '5');
      expect(batcher.metrics.appendCalls, '1');
    });

    test('flushes before lifecycle and snapshot callbacks', () {
      final events = <String>[];
      final batcher = LosslessInputBatcher<String>(
        maxBatchBytes: 16,
        append: (chunk) {
          events.add('append:$chunk');
          return 'append';
        },
        finish: () {
          events.add('finish');
          return 'finish';
        },
        reset: () {
          events.add('reset');
          return 'reset';
        },
        createRecoverySnapshot: () {
          events.add('snapshot');
          return 'snapshot';
        },
      );

      batcher.push('a');
      expect(batcher.createRecoverySnapshot(), <String>['append', 'snapshot']);
      batcher.push('b');
      expect(batcher.reset(), <String>['append', 'reset']);
      batcher.push('c');
      expect(batcher.finish(), <String>['append', 'finish']);

      expect(events, <String>[
        'append:a',
        'snapshot',
        'append:b',
        'reset',
        'append:c',
        'finish',
      ]);
    });

    test('returns both ordered results when one push crosses the limit', () {
      final batcher = LosslessInputBatcher<String>(
        maxBatchBytes: 4,
        append: (chunk) => chunk,
      );

      expect(batcher.push('ab'), isEmpty);
      expect(batcher.push('oversized'), <String>['ab', 'oversized']);
      expect(batcher.metrics.forwardedBytes, '11');
      expect(batcher.metrics.appendCalls, '2');
    });

    test('preserves committed results when the second operation fails', () {
      final batcher = LosslessInputBatcher<String>(
        maxBatchBytes: 4,
        append: (chunk) {
          if (chunk == 'oversized') {
            throw StateError('second append failed');
          }
          return chunk;
        },
        finish: () => throw StateError('finish failed'),
      );

      batcher.push('ab');
      expect(
        () => batcher.push('oversized'),
        throwsA(
          isA<BatchOperationException<String>>()
              .having(
                (error) => error.completedResults,
                'completedResults',
                <String>['ab'],
              )
              .having((error) => error.cause, 'cause', isA<StateError>()),
        ),
      );

      batcher.push('cd');
      expect(
        batcher.finish,
        throwsA(
          isA<BatchOperationException<String>>().having(
            (error) => error.completedResults,
            'completedResults',
            <String>['cd'],
          ),
        ),
      );
    });

    test('validates limits and rejects unpaired UTF-16 surrogates', () {
      for (final maxBytes in <int>[0, -1]) {
        expect(
          () => LosslessInputBatcher<void>(
            maxBatchBytes: maxBytes,
            append: (_) {},
          ),
          throwsRangeError,
        );
      }
      expect(() => utf8ByteLength('\ud800'), throwsFormatException);
      expect(() => utf8ByteLength('\udc00'), throwsFormatException);
    });
  });

  final libraryPath = nativeLibraryPath();
  test(
    'native batch sizes preserve source with bounded copy accounting',
    () {
      const chunks = <String>[
        '# Native\r',
        '',
        '\n\n',
        'emoji 👩‍💻 and CJK 流',
        '\n\n```mermaid\nflowchart LR\nA-->B\n```',
      ];
      final expected = chunks.join();
      final canonicalSource = expected.replaceAll('\r\n', '\n');
      final runtime = MdstreamRuntime.openPath(libraryPath!);

      for (final maxBytes in <int>[1, 16, 128, 4096]) {
        final engine = runtime.createEngine();
        try {
          final batcher = engine.createBatcher(maxBytes);
          final observed = <EngineResult>[];
          for (final chunk in chunks) {
            observed.addAll(batcher.push(chunk));
          }
          observed.addAll(batcher.finish());
          expect(observed, isNotEmpty);

          expect(engine.metrics.snapshotPayloads, '0');
          final outputBytesBeforeSnapshot = BigInt.parse(
            batcher.metrics.outputPayloadBytes,
          );
          final recovery = batcher.createRecoverySnapshot();
          expect(recovery.flushed, isEmpty);
          final snapshot = recovery.snapshot!;
          expect(engine.metrics.snapshotPayloads, '1');
          expect(
            BigInt.parse(batcher.metrics.outputPayloadBytes),
            outputBytesBeforeSnapshot + BigInt.from(snapshot.byteLength),
          );
          expect(decodeSnapshot(snapshot)['source'], canonicalSource);
          expect(
            batcher.metrics.inputBytes,
            utf8ByteLength(expected).toString(),
          );
          expect(batcher.metrics.forwardedBytes, batcher.metrics.inputBytes);
          expect(batcher.metrics.pendingBytes, '0');
          expect(
            BigInt.parse(batcher.metrics.joinCopyBytes),
            lessThanOrEqualTo(BigInt.parse(batcher.metrics.inputBytes)),
          );
          expect(
            BigInt.parse(batcher.metrics.outputPayloadBytes),
            greaterThan(BigInt.zero),
          );
        } finally {
          engine.close();
        }
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}
