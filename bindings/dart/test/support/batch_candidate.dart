import 'package:mdstream/mdstream.dart';

import 'fixtures.dart';

enum BatchCandidatePolicy { joinedFirst, constituentFirst }

final class BatchCandidateObservation {
  const BatchCandidateObservation({
    required this.snapshot,
    required this.appendAttempts,
    required this.encodedResultBytes,
    required this.scanBytes,
    required this.joinCopyBytes,
    required this.replayCount,
  });

  final Map<String, Object?> snapshot;
  final BigInt appendAttempts;
  final BigInt encodedResultBytes;
  final BigInt scanBytes;
  final BigInt joinCopyBytes;
  final BigInt replayCount;
}

BatchCandidateObservation runBatchCandidate(
  MdstreamRuntime runtime,
  BatchCandidatePolicy policy,
  Iterable<String> input,
) {
  final chunks = List<String>.unmodifiable(input);
  final retained = chunks
      .where((chunk) => chunk.isNotEmpty)
      .toList(growable: false);
  final inputBytes = chunks.fold<int>(
    0,
    (total, chunk) => total + utf8ByteLength(chunk),
  );
  final engine = runtime.createEngine(
    options: MdstreamSessionOptions(
      protocol: MdstreamProtocolLimits(maxOperations: '40000'),
    ),
  );
  MdstreamInputBatcher? batcher;

  try {
    late BigInt appendAttempts;
    late BigInt encodedResultBytes;
    late BigInt scanBytes;
    late BigInt joinCopyBytes;
    switch (policy) {
      case BatchCandidatePolicy.joinedFirst:
        final appendResults = <EngineResult>[];
        if (retained.isNotEmpty) {
          appendResults.add(engine.append(retained.join()));
        }
        appendAttempts = BigInt.from(appendResults.length);
        encodedResultBytes = appendResults.fold<BigInt>(
          BigInt.zero,
          (total, result) => total + BigInt.parse(result.outputPayloadBytes),
        );
        scanBytes = BigInt.from(inputBytes);
        joinCopyBytes = retained.length > 1
            ? BigInt.from(inputBytes)
            : BigInt.zero;
        break;
      case BatchCandidatePolicy.constituentFirst:
        final candidateBatcher = engine.createBatcher(
          maxBatchBytes: inputBytes == 0 ? 1 : inputBytes,
          maxPendingChunks: retained.length + 1,
        );
        batcher = candidateBatcher;
        for (final chunk in chunks) {
          candidateBatcher.push(chunk);
        }
        candidateBatcher.flush();
        final metrics = candidateBatcher.metrics;
        appendAttempts = BigInt.parse(metrics.appendAttempts);
        encodedResultBytes = BigInt.parse(metrics.outputPayloadBytes);
        scanBytes = BigInt.parse(metrics.scanBytes);
        joinCopyBytes = BigInt.parse(metrics.joinCopyBytes);
        candidateBatcher.release();
        break;
    }

    final finished = engine.finish();
    encodedResultBytes += BigInt.parse(finished.outputPayloadBytes);
    final snapshot = engine.createRecoverySnapshot()!;
    return BatchCandidateObservation(
      snapshot: normalizeSnapshot(decodeSnapshot(snapshot)),
      appendAttempts: appendAttempts,
      encodedResultBytes: encodedResultBytes,
      scanBytes: scanBytes,
      joinCopyBytes: joinCopyBytes,
      replayCount: BigInt.zero,
    );
  } finally {
    if (batcher != null && !batcher.isReleased) {
      if (batcher.inspectPending() != null) {
        batcher.discardPending();
      }
      batcher.release();
    }
    engine.close();
  }
}
