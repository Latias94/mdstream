/// Metrics for lossless input batching.
///
/// Counters use decimal strings so their representation stays exact on every
/// Dart target and matches the rest of the mdstream binding surface.
final class BatchMetrics {
  /// Creates an immutable snapshot of batching counters.
  const BatchMetrics({
    required this.maxBatchBytes,
    required this.inputChunks,
    required this.inputBytes,
    required this.forwardedBytes,
    required this.pendingBytes,
    required this.joinCopyBytes,
    required this.outputPayloadBytes,
    required this.batchCount,
    required this.appendCalls,
  });

  /// Configured upper bound for accumulated input, in UTF-8 bytes.
  final String maxBatchBytes;

  /// Number of chunks supplied by callers.
  final String inputChunks;

  /// Total UTF-8 bytes supplied by callers.
  final String inputBytes;

  /// Total UTF-8 bytes forwarded to the append callback.
  final String forwardedBytes;

  /// UTF-8 bytes currently retained in the pending batch.
  final String pendingBytes;

  /// Bytes copied while joining batches that contained multiple chunks.
  final String joinCopyBytes;

  /// Total output bytes reported by the optional result measurer.
  final String outputPayloadBytes;

  /// Number of batches forwarded to the append callback.
  final String batchCount;

  /// Number of append callback invocations.
  final String appendCalls;

  /// Returns a read-only JSON-compatible metrics object.
  Map<String, String> toJson() =>
      Map<String, String>.unmodifiable(<String, String>{
        'max_batch_bytes': maxBatchBytes,
        'input_chunks': inputChunks,
        'input_bytes': inputBytes,
        'forwarded_bytes': forwardedBytes,
        'pending_bytes': pendingBytes,
        'join_copy_bytes': joinCopyBytes,
        'output_payload_bytes': outputPayloadBytes,
        'batch_count': batchCount,
        'append_calls': appendCalls,
      });
}

/// Reports a failed compound batch operation without hiding committed results.
final class BatchOperationException<Result> implements Exception {
  /// Creates an error that preserves results committed before [cause].
  const BatchOperationException({
    required this.completedResults,
    required this.cause,
    required this.stackTrace,
  });

  /// Results committed before the later operation failed.
  final List<Result> completedResults;

  /// Error thrown by the failed append or lifecycle callback.
  final Object cause;

  /// Original callback stack trace.
  final StackTrace stackTrace;

  @override
  String toString() =>
      'BatchOperationException after ${completedResults.length} result(s): '
      '$cause';
}

/// Coalesces input without changing its text or UTF-8 byte sequence.
///
/// A chunk larger than [maxBatchBytes] is forwarded as one append operation;
/// the limit bounds accumulated batching memory and never splits a caller's
/// string at an unsafe UTF-8 boundary.
final class LosslessInputBatcher<Result> {
  /// Creates a lossless batcher backed by the supplied lifecycle callbacks.
  ///
  /// [outputPayloadBytes] may report the encoded size of each result for
  /// metrics. Negative measurements are rejected.
  LosslessInputBatcher({
    required int maxBatchBytes,
    required Result Function(String chunk) append,
    Result Function()? finish,
    Result Function()? reset,
    Result Function()? createRecoverySnapshot,
    int Function(Result result)? outputPayloadBytes,
  }) : _maxBatchBytes = maxBatchBytes,
       _append = append,
       _finish = finish,
       _reset = reset,
       _createRecoverySnapshot = createRecoverySnapshot,
       _outputPayloadBytes = outputPayloadBytes {
    if (maxBatchBytes <= 0) {
      throw RangeError.range(maxBatchBytes, 1, null, 'maxBatchBytes');
    }
  }

  final int _maxBatchBytes;
  final Result Function(String chunk) _append;
  final Result Function()? _finish;
  final Result Function()? _reset;
  final Result Function()? _createRecoverySnapshot;
  final int Function(Result result)? _outputPayloadBytes;

  final List<String> _chunks = <String>[];
  int _pendingBytes = 0;
  BigInt _inputChunks = BigInt.zero;
  BigInt _inputBytes = BigInt.zero;
  BigInt _forwardedBytes = BigInt.zero;
  BigInt _joinCopyBytes = BigInt.zero;
  BigInt _outputBytes = BigInt.zero;
  BigInt _batchCount = BigInt.zero;

  /// Maximum bytes retained before the current batch is flushed.
  int get maxBatchBytes => _maxBatchBytes;

  /// Adds one source chunk and returns every append result in wire order.
  ///
  /// Two results are possible when pending input must be flushed before an
  /// oversized current chunk is forwarded directly.
  List<Result> push(String chunk) {
    final results = <Result>[];
    final bytes = utf8ByteLength(chunk);
    _inputChunks += BigInt.one;
    _inputBytes += BigInt.from(bytes);
    if (bytes == 0) {
      return List<Result>.unmodifiable(results);
    }

    if (_pendingBytes > 0 && _pendingBytes + bytes > _maxBatchBytes) {
      final flushed = flush();
      if (flushed != null) {
        results.add(flushed);
      }
    }
    if (bytes > _maxBatchBytes) {
      try {
        results.add(_forward(chunk, bytes));
      } catch (error, stackTrace) {
        if (results.isEmpty) {
          rethrow;
        }
        throw BatchOperationException<Result>(
          completedResults: List<Result>.unmodifiable(results),
          cause: error,
          stackTrace: stackTrace,
        );
      }
      return List<Result>.unmodifiable(results);
    }

    _chunks.add(chunk);
    _pendingBytes += bytes;
    if (_pendingBytes == _maxBatchBytes) {
      final flushed = flush();
      if (flushed != null) {
        results.add(flushed);
      }
    }
    return List<Result>.unmodifiable(results);
  }

  /// Forwards the accumulated batch and returns the append result.
  ///
  /// Pending input is cleared only after append succeeds, allowing a caller to
  /// retry [flush] after a transient host failure.
  Result? flush() {
    if (_chunks.isEmpty) {
      return null;
    }
    final bytes = _pendingBytes;
    final joined = _chunks.length == 1 ? _chunks.first : _chunks.join();
    final result = _append(joined);

    if (_chunks.length > 1) {
      _joinCopyBytes += BigInt.from(bytes);
    }
    _chunks.clear();
    _pendingBytes = 0;
    _recordForward(bytes, result);
    return result;
  }

  /// Flushes pending input and finalizes the underlying stream.
  List<Result> finish() {
    final callback = _finish;
    if (callback == null) {
      throw StateError('this batcher does not provide a finish callback');
    }
    final results = _flushResults();
    final result = _afterCommitted(results, callback);
    _recordOutput(result);
    results.add(result);
    return List<Result>.unmodifiable(results);
  }

  /// Flushes pending input and resets the underlying stream.
  List<Result> reset() {
    final callback = _reset;
    if (callback == null) {
      throw StateError('this batcher does not provide a reset callback');
    }
    final results = _flushResults();
    final result = _afterCommitted(results, callback);
    _recordOutput(result);
    results.add(result);
    return List<Result>.unmodifiable(results);
  }

  /// Flushes pending input and requests an explicit recovery snapshot.
  List<Result> createRecoverySnapshot() {
    final callback = _createRecoverySnapshot;
    if (callback == null) {
      throw StateError(
        'this batcher does not provide a recovery snapshot callback',
      );
    }
    final results = _flushResults();
    final result = _afterCommitted(results, callback);
    _recordOutput(result);
    results.add(result);
    return List<Result>.unmodifiable(results);
  }

  /// Returns a point-in-time immutable metrics view.
  BatchMetrics get metrics => BatchMetrics(
    maxBatchBytes: _maxBatchBytes.toString(),
    inputChunks: _inputChunks.toString(),
    inputBytes: _inputBytes.toString(),
    forwardedBytes: _forwardedBytes.toString(),
    pendingBytes: _pendingBytes.toString(),
    joinCopyBytes: _joinCopyBytes.toString(),
    outputPayloadBytes: _outputBytes.toString(),
    batchCount: _batchCount.toString(),
    appendCalls: _batchCount.toString(),
  );

  Result _forward(String chunk, int bytes) {
    final result = _append(chunk);
    _recordForward(bytes, result);
    return result;
  }

  List<Result> _flushResults() {
    final results = <Result>[];
    final flushed = flush();
    if (flushed != null) {
      results.add(flushed);
    }
    return results;
  }

  Result _afterCommitted(List<Result> completed, Result Function() operation) {
    try {
      return operation();
    } catch (error, stackTrace) {
      if (completed.isEmpty) {
        rethrow;
      }
      throw BatchOperationException<Result>(
        completedResults: List<Result>.unmodifiable(completed),
        cause: error,
        stackTrace: stackTrace,
      );
    }
  }

  void _recordForward(int bytes, Result result) {
    _forwardedBytes += BigInt.from(bytes);
    _batchCount += BigInt.one;
    _recordOutput(result);
  }

  void _recordOutput(Result result) {
    final measure = _outputPayloadBytes;
    if (measure == null) {
      return;
    }
    final bytes = measure(result);
    if (bytes < 0) {
      throw StateError('output payload byte measurements cannot be negative');
    }
    _outputBytes += BigInt.from(bytes);
  }
}

/// Counts UTF-8 bytes without allocating and rejects unpaired UTF-16 units.
int utf8ByteLength(String value) {
  var bytes = 0;
  final units = value.codeUnits;
  for (var index = 0; index < units.length; index += 1) {
    final unit = units[index];
    if (unit <= 0x7f) {
      bytes += 1;
    } else if (unit <= 0x7ff) {
      bytes += 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff) {
      if (index + 1 >= units.length) {
        throw const FormatException(
          'input contains an unpaired high surrogate',
        );
      }
      final next = units[index + 1];
      if (next < 0xdc00 || next > 0xdfff) {
        throw const FormatException(
          'input contains an unpaired high surrogate',
        );
      }
      bytes += 4;
      index += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      throw const FormatException('input contains an unpaired low surrogate');
    } else {
      bytes += 3;
    }
  }
  return bytes;
}
