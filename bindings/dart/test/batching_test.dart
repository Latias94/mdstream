import 'package:mdstream/mdstream.dart';
import 'package:mdstream/src/batching.dart' as internal;
import 'package:test/test.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  group('UTF-8 admission', () {
    test('counts without allocation and rejects malformed UTF-16', () {
      expect(utf8ByteLength('aé👩‍💻'), 14);
      expect(() => utf8ByteLength('\ud800'), throwsFormatException);
      expect(() => utf8ByteLength('\udc00'), throwsFormatException);
    });

    test('stops once the configured byte ceiling is exceeded', () {
      expect(utf8ByteLengthAtMost('abcd', 3), isFalse);
      expect(utf8ByteLengthAtMost('é', 2), isTrue);
      expect(utf8ByteLengthAtMost('éé', 3), isFalse);
      expect(() => utf8ByteLengthAtMost('\ud800', 4), throwsFormatException);
    });
  });

  group('constituent queue contract', () {
    test('retries only blocked ownership and commits a fail-once suffix', () {
      var rejectedOnce = false;
      final queue = internal.BatchInputQueue<String>(
        maxBatchBytes: 16,
        maxPendingChunks: 8,
        append: (chunk, _) {
          if (chunk == 'b' && !rejectedOnce) {
            rejectedOnce = true;
            throw StateError('reject b once');
          }
          return chunk;
        },
        outputPayloadBytes: utf8ByteLength,
      );

      expect(
        queue.retryPending,
        throwsA(
          isA<MdstreamException>().having(
            (error) => error.detailCode,
            'detailCode',
            'bindings.batch_pending',
          ),
        ),
      );
      queue.push('a');
      queue.push('b');
      queue.push('c');
      expect(
        queue.retryPending,
        throwsA(
          isA<MdstreamException>().having(
            (error) => error.detailCode,
            'detailCode',
            'bindings.batch_pending',
          ),
        ),
      );

      expect(
        queue.flush,
        throwsA(
          isA<BatchOperationException<String>>()
              .having(
                (error) => error.completedResults,
                'completedResults',
                <String>['a'],
              )
              .having(
                (error) => error.pending?.chunks,
                'pending.chunks',
                <String>['b', 'c'],
              ),
        ),
      );
      expect(queue.retryPending(), <String>['b', 'c']);
      expect(queue.inspectPending(), isNull);
      expect(queue.metrics.appendAttempts, '4');
      expect(queue.metrics.successfulAppends, '3');
      expect(queue.metrics.publishedResults, '3');
      expect(queue.metrics.inputAttempts, '3');
      expect(queue.metrics.scanBytes, '3');
      expect(
        queue.retryPending,
        throwsA(
          isA<MdstreamException>().having(
            (error) => error.detailCode,
            'detailCode',
            'bindings.batch_pending',
          ),
        ),
      );
    });

    test('wraps append and lifecycle callback failures uniformly', () {
      final appendCause = StateError('append failed');
      final standalone = internal.BatchInputQueue<String>(
        maxBatchBytes: 1,
        maxPendingChunks: 4,
        append: (_, _) => throw appendCause,
        outputPayloadBytes: utf8ByteLength,
      );
      expect(
        () => standalone.push('xx'),
        throwsA(
          isA<BatchOperationException<String>>()
              .having((error) => error.cause, 'cause', same(appendCause))
              .having(
                (error) => error.completedResults,
                'completedResults',
                isEmpty,
              )
              .having(
                (error) => error.operation,
                'operation',
                BatchOperation.push,
              )
              .having(
                (error) => error.newInputAccepted,
                'newInputAccepted',
                isFalse,
              )
              .having((error) => error.pending, 'pending', isNull),
        ),
      );

      for (final operation in <BatchOperation>[
        BatchOperation.finish,
        BatchOperation.reset,
      ]) {
        final queue = _stringQueue();
        queue.push('a');
        queue.push('b');
        final cause = StateError('${operation.wireValue} failed');
        expect(
          () => queue.runResultOperation(operation, () => throw cause),
          throwsA(
            isA<BatchOperationException<String>>()
                .having((error) => error.cause, 'cause', same(cause))
                .having(
                  (error) => error.completedResults,
                  'completedResults',
                  <String>['a', 'b'],
                )
                .having((error) => error.operation, 'operation', operation)
                .having((error) => error.pending, 'pending', isNull),
          ),
        );
      }

      final recovery = _stringQueue();
      final recoveryCause = StateError('snapshot failed');
      expect(
        () => recovery.runValueOperation<String>(
          BatchOperation.recoverySnapshot,
          () => throw recoveryCause,
        ),
        throwsA(
          isA<BatchOperationException<String>>()
              .having((error) => error.cause, 'cause', same(recoveryCause))
              .having(
                (error) => error.completedResults,
                'completedResults',
                isEmpty,
              )
              .having(
                (error) => error.operation,
                'operation',
                BatchOperation.recoverySnapshot,
              ),
        ),
      );
    });

    test('keeps pre-admission failures raw while recording scan work', () {
      final cause = MdstreamException(
        'raw admission failed',
        status: BindingStatus.resourceLimitExceeded.value,
        statusName: BindingStatus.resourceLimitExceeded.statusName,
        detailCode: 'bindings.resource_limit',
      );
      var appendCalled = false;
      final queue = internal.BatchInputQueue<String>(
        maxBatchBytes: 16,
        maxPendingChunks: 4,
        append: (chunk, bytes) {
          appendCalled = true;
          return chunk;
        },
        outputPayloadBytes: utf8ByteLength,
        inputByteLength: (_, observeScan) {
          observeScan(2);
          throw cause;
        },
      );

      expect(() => queue.push('é'), throwsA(same(cause)));
      expect(appendCalled, isFalse);
      expect(queue.inspectPending(), isNull);
      expect(queue.metrics.inputAttempts, '1');
      expect(queue.metrics.inputBytes, '0');
      expect(queue.metrics.scanBytes, '2');
      expect(queue.metrics.appendAttempts, '0');
    });

    test(
      'reports linear work while committing ten thousand constituents',
      () {
        const count = 10000;
        final queue = internal.BatchInputQueue<String>(
          maxBatchBytes: count + 1,
          maxPendingChunks: count + 1,
          append: (chunk, _) => chunk,
          outputPayloadBytes: (_) => 0,
        );
        for (var index = 0; index < count; index += 1) {
          queue.push('x');
        }

        expect(queue.flush(), hasLength(count));
        expect(queue.metrics.appendAttempts, '$count');
        expect(queue.metrics.successfulAppends, '$count');
        expect(queue.metrics.committedBytes, '$count');
        expect(queue.metrics.scanBytes, '$count');
        expect(queue.metrics.pendingConstituents, '0');
      },
      timeout: const Timeout(Duration(seconds: 5)),
    );
  });

  final libraryPath = nativeLibraryPath();
  group(
    'MdstreamInputBatcher',
    () {
      late MdstreamRuntime runtime;

      setUp(() {
        runtime = MdstreamRuntime.openPath(libraryPath!);
      });

      tearDown(() {
        expect(runtime.nativeAllocations.isZero, isTrue);
      });

      test('returns zero, one, and many append results in wire order', () {
        final engine = runtime.createEngine();
        final batcher = engine.createBatcher(
          maxBatchBytes: 4,
          maxPendingChunks: 2,
        );
        try {
          expect(batcher.flush(), isEmpty);
          expect(batcher.push('a'), isEmpty);
          expect(batcher.push('b'), isEmpty);

          final crossed = batcher.push('12345');
          expect(crossed, hasLength(3));
          expect(
            crossed.expand((result) => result.changes),
            hasLength(greaterThanOrEqualTo(3)),
          );
          expect(batcher.inspectPending(), isNull);

          expect(batcher.push('z'), isEmpty);
          expect(batcher.flush(), hasLength(1));
          expect(batcher.flush(), isEmpty);

          final snapshot = batcher.createRecoverySnapshot();
          expect(snapshot.flushed, isEmpty);
          expect(decodeSnapshot(snapshot.snapshot!)['source'], 'ab12345z');
        } finally {
          _release(batcher);
          engine.close();
        }
      });

      test('bounds retained constituents and ignores empty chunks', () {
        final engine = runtime.createEngine();
        final batcher = engine.createBatcher(
          maxBatchBytes: 128,
          maxPendingChunks: 2,
        );
        try {
          expect(batcher.push('a'), isEmpty);
          for (var index = 0; index < 100; index += 1) {
            expect(batcher.push(''), isEmpty);
          }
          expect(batcher.inspectPending()?.chunks, <String>['a']);
          expect(batcher.push('b'), isEmpty);
          expect(batcher.inspectPending()?.chunks, <String>['a', 'b']);

          final preflushed = batcher.push('c');
          expect(preflushed, hasLength(2));
          final pending = batcher.inspectPending();
          expect(pending?.chunks, <String>['c']);
          expect(() => pending!.chunks.add('mutate'), throwsUnsupportedError);
          expect(batcher.inspectPending()?.chunks, <String>['c']);

          final metrics = batcher.metrics;
          expect(metrics.maxPendingChunks, '2');
          expect(metrics.inputAttempts, '103');
          expect(metrics.pendingConstituents, '1');
          expect(metrics.boundaryMetadataBytes, '8');
          expect(metrics.scanBytes, '3');
        } finally {
          batcher.discardPending();
          batcher.release();
          engine.close();
        }
      });

      test(
        'retains a failed constituent and suffix after a committed prefix',
        () {
          final rejected = 'x' * 65;
          final engine = runtime.createEngine(
            options: MdstreamSessionOptions(
              protocol: MdstreamProtocolLimits(maxSourceBytes: '256'),
              wire: MdstreamWireLimits(maxCommandBytes: '64'),
            ),
          );
          final batcher = engine.createBatcher(
            maxBatchBytes: 256,
            maxPendingChunks: 8,
          );
          try {
            batcher.push('a');
            batcher.push(rejected);
            batcher.push('suffix');

            expect(
              batcher.flush,
              throwsA(
                isA<BatchOperationException<EngineResult>>()
                    .having(
                      (error) => error.completedResults,
                      'completedResults',
                      hasLength(1),
                    )
                    .having(
                      (error) => error.cause,
                      'cause',
                      isA<MdstreamException>().having(
                        (error) => error.splitSafety,
                        'splitSafety',
                        SplitSafety.retryAtOriginalBoundaries,
                      ),
                    )
                    .having(
                      (error) => error.operation,
                      'operation',
                      BatchOperation.flush,
                    )
                    .having(
                      (error) => error.newInputAccepted,
                      'newInputAccepted',
                      isNull,
                    )
                    .having(
                      (error) => error.pending?.chunks,
                      'pending.chunks',
                      <String>[rejected, 'suffix'],
                    ),
              ),
            );
            expect(batcher.inspectPending()?.chunks, <String>[
              rejected,
              'suffix',
            ]);

            for (final operation in <void Function()>[
              batcher.flush,
              batcher.finish,
              batcher.reset,
              batcher.createRecoverySnapshot,
              () => batcher.push('late'),
            ]) {
              expect(
                operation,
                throwsA(
                  isA<MdstreamException>().having(
                    (error) => error.detailCode,
                    'detailCode',
                    'bindings.batch_unresolved',
                  ),
                ),
              );
            }
            expect(batcher.metrics.inputAttempts, '4');
            expect(batcher.metrics.inputBytes, '72');
            expect(batcher.metrics.scanBytes, '72');

            final beforeRetry = batcher.metrics.appendAttempts;
            expect(
              batcher.retryPending,
              throwsA(
                isA<BatchOperationException<EngineResult>>()
                    .having(
                      (error) => error.operation,
                      'operation',
                      BatchOperation.retryPending,
                    )
                    .having(
                      (error) => error.newInputAccepted,
                      'newInputAccepted',
                      isNull,
                    )
                    .having(
                      (error) => error.cause,
                      'cause',
                      isA<MdstreamException>().having(
                        (error) => error.splitSafety,
                        'splitSafety',
                        SplitSafety.retryAtOriginalBoundaries,
                      ),
                    ),
              ),
            );
            expect(
              BigInt.parse(batcher.metrics.appendAttempts),
              BigInt.parse(beforeRetry) + BigInt.one,
            );
            expect(batcher.inspectPending()?.chunks, <String>[
              rejected,
              'suffix',
            ]);

            final transferred = batcher.takePending();
            expect(transferred?.chunks, <String>[rejected, 'suffix']);
            expect(transferred?.bytes, '71');
            expect(transferred?.constituents, '2');
            expect(batcher.inspectPending(), isNull);
            batcher.release();

            final snapshot = engine.createRecoverySnapshot();
            expect(decodeSnapshot(snapshot!)['source'], 'a');
          } finally {
            if (!batcher.isReleased) {
              batcher.discardPending();
              batcher.release();
            }
            engine.close();
          }
        },
      );

      test('does not accept a new chunk when boundary pre-flush fails', () {
        final rejected = 'x' * 65;
        final engine = runtime.createEngine(
          options: MdstreamSessionOptions(
            protocol: MdstreamProtocolLimits(maxSourceBytes: '256'),
            wire: MdstreamWireLimits(maxCommandBytes: '64'),
          ),
        );
        final batcher = engine.createBatcher(
          maxBatchBytes: 256,
          maxPendingChunks: 1,
        );
        try {
          expect(batcher.push(rejected), isEmpty);
          expect(
            () => batcher.push('new'),
            throwsA(
              isA<BatchOperationException<EngineResult>>()
                  .having(
                    (error) => error.operation,
                    'operation',
                    BatchOperation.push,
                  )
                  .having(
                    (error) => error.newInputAccepted,
                    'newInputAccepted',
                    isFalse,
                  )
                  .having(
                    (error) => error.pending?.chunks,
                    'pending.chunks',
                    <String>[rejected],
                  ),
            ),
          );
          expect(batcher.inspectPending()?.chunks, <String>[rejected]);
          expect(batcher.metrics.inputAttempts, '2');
          expect(batcher.metrics.inputBytes, '68');
        } finally {
          batcher.discardPending();
          batcher.release();
          engine.close();
        }
      });

      test(
        'marks input accepted when its exact byte limit auto-flush fails',
        () {
          final rejected = 'x' * 65;
          final engine = runtime.createEngine(
            options: MdstreamSessionOptions(
              protocol: MdstreamProtocolLimits(maxSourceBytes: '256'),
              wire: MdstreamWireLimits(maxCommandBytes: '64'),
            ),
          );
          final batcher = engine.createBatcher(
            maxBatchBytes: 65,
            maxPendingChunks: 8,
          );
          try {
            expect(
              () => batcher.push(rejected),
              throwsA(
                isA<BatchOperationException<EngineResult>>()
                    .having(
                      (error) => error.operation,
                      'operation',
                      BatchOperation.push,
                    )
                    .having(
                      (error) => error.newInputAccepted,
                      'newInputAccepted',
                      isTrue,
                    )
                    .having(
                      (error) => error.pending?.chunks,
                      'pending.chunks',
                      <String>[rejected],
                    ),
              ),
            );
          } finally {
            batcher.discardPending();
            batcher.release();
            engine.close();
          }
        },
      );

      test(
        'rejects obvious native overflow before accepting pending input',
        () {
          final engine = runtime.createEngine(
            options: MdstreamSessionOptions(
              protocol: MdstreamProtocolLimits(maxSourceBytes: '1'),
            ),
          );
          final batcher = engine.createBatcher(
            maxBatchBytes: 64,
            maxPendingChunks: 8,
          );
          try {
            expect(
              () => batcher.push('xxx'),
              throwsA(
                isA<MdstreamException>().having(
                  (error) => error.detailCode,
                  'detailCode',
                  'bindings.resource_limit',
                ),
              ),
            );
            expect(batcher.inspectPending(), isNull);
            expect(batcher.metrics.inputAttempts, '1');
            expect(batcher.metrics.inputBytes, '0');
            expect(batcher.metrics.scanBytes, '0');
            expect(batcher.metrics.appendAttempts, '0');
          } finally {
            batcher.release();
            engine.close();
          }
        },
      );

      test('records bounded Unicode scan work before raw rejection', () {
        final engine = runtime.createEngine(
          options: MdstreamSessionOptions(
            protocol: MdstreamProtocolLimits(maxSourceBytes: '1'),
          ),
        );
        final batcher = engine.createBatcher(
          maxBatchBytes: 64,
          maxPendingChunks: 8,
        );
        try {
          expect(
            () => batcher.push('éé'),
            throwsA(
              isA<MdstreamException>()
                  .having(
                    (error) => error.detailCode,
                    'detailCode',
                    'bindings.resource_limit',
                  )
                  .having(
                    (error) => error,
                    'not composite',
                    isNot(isA<BatchOperationException<EngineResult>>()),
                  ),
            ),
          );
          expect(batcher.inspectPending(), isNull);
          expect(batcher.metrics.inputAttempts, '1');
          expect(batcher.metrics.inputBytes, '0');
          expect(batcher.metrics.scanBytes, '4');
          expect(batcher.metrics.appendAttempts, '0');
        } finally {
          _release(batcher);
          engine.close();
        }
      });

      test('keeps an oversized failed push outside pending ownership', () {
        final engine = runtime.createEngine(
          options: MdstreamSessionOptions(
            protocol: MdstreamProtocolLimits(maxSourceBytes: '3'),
          ),
        );
        final batcher = engine.createBatcher(
          maxBatchBytes: 2,
          maxPendingChunks: 8,
        );
        try {
          batcher.push('a');
          expect(
            () => batcher.push('bbbb'),
            throwsA(
              isA<BatchOperationException<EngineResult>>()
                  .having(
                    (error) => error.completedResults,
                    'completedResults',
                    hasLength(1),
                  )
                  .having(
                    (error) => error.operation,
                    'operation',
                    BatchOperation.push,
                  )
                  .having(
                    (error) => error.newInputAccepted,
                    'newInputAccepted',
                    isFalse,
                  )
                  .having((error) => error.pending, 'pending', isNull),
            ),
          );
          expect(batcher.inspectPending(), isNull);
          expect(batcher.metrics.committedBytes, '1');
          batcher.release();
          expect(
            decodeSnapshot(engine.createRecoverySnapshot()!)['source'],
            'a',
          );
        } finally {
          if (!batcher.isReleased) {
            batcher.discardPending();
            batcher.release();
          }
          engine.close();
        }
      });

      test('never retries a single split-safe constituent recursively', () {
        final rejected = 'x' * 65;
        final engine = runtime.createEngine(
          options: MdstreamSessionOptions(
            protocol: MdstreamProtocolLimits(maxSourceBytes: '256'),
            wire: MdstreamWireLimits(maxCommandBytes: '64'),
          ),
        );
        final batcher = engine.createBatcher(
          maxBatchBytes: 256,
          maxPendingChunks: 8,
        );
        try {
          batcher.push(rejected);
          expect(
            batcher.flush,
            throwsA(isA<BatchOperationException<EngineResult>>()),
          );
          expect(batcher.metrics.appendAttempts, '1');
          expect(batcher.metrics.replayCount, '0');
          expect(
            batcher.retryPending,
            throwsA(
              isA<BatchOperationException<EngineResult>>().having(
                (error) => error.operation,
                'operation',
                BatchOperation.retryPending,
              ),
            ),
          );
          expect(batcher.metrics.appendAttempts, '2');
          expect(batcher.metrics.replayCount, '0');
          expect(batcher.inspectPending()?.chunks, <String>[rejected]);
        } finally {
          batcher.discardPending();
          batcher.release();
          engine.close();
        }
      });

      test('preserves typed cumulative failures without hidden replay', () {
        final engine = runtime.createEngine(
          options: MdstreamSessionOptions(
            protocol: MdstreamProtocolLimits(maxSourceBytes: '1'),
          ),
        );
        final batcher = engine.createBatcher(
          maxBatchBytes: 64,
          maxPendingChunks: 8,
        );
        try {
          batcher.push('a');
          batcher.push('b');
          expect(
            batcher.flush,
            throwsA(
              isA<BatchOperationException<EngineResult>>().having(
                (error) => error.cause,
                'cause',
                isA<MdstreamException>().having(
                  (error) => error.splitSafety,
                  'splitSafety',
                  SplitSafety.notSafe,
                ),
              ),
            ),
          );
          expect(batcher.inspectPending()?.chunks, <String>['b']);
          expect(batcher.metrics.replayCount, '0');
          expect(batcher.discardPending()?.chunks, <String>['b']);
          batcher.release();
        } finally {
          if (!batcher.isReleased) {
            batcher.discardPending();
            batcher.release();
          }
          engine.close();
        }
      });

      test('requires one explicit engine lease release', () {
        final engine = runtime.createEngine();
        final batcher = engine.createBatcher(
          maxBatchBytes: 16,
          maxPendingChunks: 4,
        );
        try {
          for (final operation in <void Function()>[
            () => engine.append('direct'),
            engine.finish,
            engine.reset,
            engine.createRecoverySnapshot,
            engine.close,
            () => engine.createBatcher(maxBatchBytes: 16, maxPendingChunks: 4),
          ]) {
            expect(
              operation,
              throwsA(
                isA<MdstreamException>().having(
                  (error) => error.detailCode,
                  'detailCode',
                  'bindings.batch_lease_active',
                ),
              ),
            );
          }

          expect(batcher.flush(), isEmpty);
          expect(
            () => engine.append('still blocked'),
            throwsA(isA<MdstreamException>()),
          );
          batcher.push('owned');
          expect(
            batcher.release,
            throwsA(
              isA<MdstreamException>().having(
                (error) => error.detailCode,
                'detailCode',
                'bindings.batch_pending',
              ),
            ),
          );
          expect(batcher.flush(), hasLength(1));
          batcher.release();
          expect(batcher.isReleased, isTrue);
          expect(
            batcher.flush,
            throwsA(
              isA<MdstreamException>().having(
                (error) => error.detailCode,
                'detailCode',
                'bindings.batch_released',
              ),
            ),
          );

          engine.append(' direct');
          final next = engine.createBatcher(
            maxBatchBytes: 16,
            maxPendingChunks: 4,
          );
          next.release();
        } finally {
          if (!batcher.isReleased) {
            batcher.discardPending();
            batcher.release();
          }
          engine.close();
        }
      });

      test('keeps lifecycle after pending and metrics fully explicit', () {
        final engine = runtime.createEngine();
        final batcher = engine.createBatcher(
          maxBatchBytes: 128,
          maxPendingChunks: 16,
        );
        try {
          batcher.push('line\r');
          batcher.push('');
          batcher.push('\nnext');
          final finished = batcher.finish();
          expect(finished, hasLength(3));

          final snapshot = batcher.createRecoverySnapshot();
          expect(snapshot.flushed, isEmpty);
          expect(decodeSnapshot(snapshot.snapshot!)['source'], 'line\nnext');

          final metrics = batcher.metrics;
          expect(metrics.maxBatchBytes, '128');
          expect(metrics.maxPendingChunks, '16');
          expect(metrics.inputAttempts, '3');
          expect(metrics.inputBytes, '10');
          expect(metrics.appendAttempts, '2');
          expect(metrics.successfulAppends, '2');
          expect(metrics.committedBytes, '10');
          expect(metrics.pendingBytes, '0');
          expect(metrics.pendingConstituents, '0');
          expect(metrics.boundaryMetadataBytes, '0');
          expect(metrics.scanBytes, '10');
          expect(metrics.joinCopyBytes, '0');
          expect(metrics.replayCount, '0');
          expect(
            BigInt.parse(metrics.outputPayloadBytes),
            greaterThan(BigInt.zero),
          );
          expect(metrics.publishedResults, '3');
        } finally {
          _release(batcher);
          engine.close();
        }
      });

      test('validates both limits before acquiring a lease', () {
        final engine = runtime.createEngine();
        try {
          for (final limits in <(int, int)>[(0, 1), (1, 0), (-1, 1)]) {
            expect(
              () => engine.createBatcher(
                maxBatchBytes: limits.$1,
                maxPendingChunks: limits.$2,
              ),
              throwsRangeError,
            );
          }
          engine.append('lease was not retained');
        } finally {
          engine.close();
        }
      });
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}

void _release(MdstreamInputBatcher batcher) {
  if (batcher.isReleased) {
    return;
  }
  if (batcher.inspectPending() != null) {
    batcher.discardPending();
  }
  batcher.release();
}

internal.BatchInputQueue<String> _stringQueue() =>
    internal.BatchInputQueue<String>(
      maxBatchBytes: 16,
      maxPendingChunks: 8,
      append: (chunk, _) => chunk,
      outputPayloadBytes: utf8ByteLength,
    );
