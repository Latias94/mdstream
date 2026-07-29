import 'dart:collection';

import 'errors.dart';

const int _logicalBoundaryBytes = 8;

/// A stable batch operation identifier shared with other mdstream bindings.
enum BatchOperation {
  /// Accept one caller chunk, potentially after a lossless pre-flush.
  push('push'),

  /// Commit ordinary pending input.
  flush('flush'),

  /// Explicitly retry input retained by an earlier failure.
  retryPending('retry_pending'),

  /// Commit pending input and finalize the stream.
  finish('finish'),

  /// Commit pending input and reset the stream.
  reset('reset'),

  /// Commit pending input and request a recovery snapshot.
  recoverySnapshot('recovery_snapshot');

  const BatchOperation(this.wireValue);

  /// Cross-binding identifier used in diagnostics and serialized evidence.
  final String wireValue;
}

/// An immutable ownership snapshot of input retained by a batcher.
final class BatchPendingInput {
  BatchPendingInput._(Iterable<_PendingChunk> pending, int bytes)
    : chunks = List<String>.unmodifiable(pending.map((chunk) => chunk.text)),
      bytes = bytes.toString(),
      constituents = pending.length.toString();

  /// Original non-empty caller chunks in acceptance order.
  final List<String> chunks;

  /// Total UTF-8 bytes retained by [chunks].
  final String bytes;

  /// Number of retained caller boundaries.
  final String constituents;
}

/// Metrics for lossless input batching.
///
/// Counters use decimal strings so their representation remains exact across
/// Dart targets. Boundary metadata uses a deterministic logical record of
/// eight bytes per retained constituent; it intentionally excludes VM object
/// headers, allocator slack, and string storage already counted by
/// [pendingBytes].
final class BatchMetrics {
  /// Creates an immutable snapshot of batching counters.
  const BatchMetrics({
    required this.maxBatchBytes,
    required this.maxPendingChunks,
    required this.inputAttempts,
    required this.inputBytes,
    required this.appendAttempts,
    required this.successfulAppends,
    required this.committedBytes,
    required this.pendingBytes,
    required this.pendingConstituents,
    required this.boundaryMetadataBytes,
    required this.scanBytes,
    required this.joinCopyBytes,
    required this.replayCount,
    required this.outputPayloadBytes,
    required this.publishedResults,
  });

  /// Maximum UTF-8 bytes retained before lossless pre-flush.
  final String maxBatchBytes;

  /// Maximum non-empty caller chunks retained at once.
  final String maxPendingChunks;

  /// Number of caller [BatchOperation.push] attempts, including empty input.
  final String inputAttempts;

  /// Total UTF-8 bytes admitted from caller push attempts.
  final String inputBytes;

  /// Number of canonical append calls attempted.
  final String appendAttempts;

  /// Number of canonical append calls that committed.
  final String successfulAppends;

  /// UTF-8 input bytes committed by successful appends.
  final String committedBytes;

  /// UTF-8 bytes currently retained for later append.
  final String pendingBytes;

  /// Non-empty caller chunks currently retained.
  final String pendingConstituents;

  /// Logical bytes occupied by retained boundary records.
  final String boundaryMetadataBytes;

  /// UTF-8 bytes scanned while checking admission, including bounded rejection.
  final String scanBytes;

  /// UTF-8 bytes copied by joined candidate attempts.
  final String joinCopyBytes;

  /// Number of one-pass replays after a rejected joined candidate.
  final String replayCount;

  /// Encoded result bytes measured after successful operations.
  final String outputPayloadBytes;

  /// Results made observable after successful operations.
  final String publishedResults;

  /// Returns a read-only JSON-compatible metrics object.
  Map<String, String> toJson() => Map<String, String>.unmodifiable({
    'max_batch_bytes': maxBatchBytes,
    'max_pending_chunks': maxPendingChunks,
    'input_attempts': inputAttempts,
    'input_bytes': inputBytes,
    'append_attempts': appendAttempts,
    'successful_appends': successfulAppends,
    'committed_bytes': committedBytes,
    'pending_bytes': pendingBytes,
    'pending_constituents': pendingConstituents,
    'boundary_metadata_bytes': boundaryMetadataBytes,
    'scan_bytes': scanBytes,
    'join_copy_bytes': joinCopyBytes,
    'replay_count': replayCount,
    'output_payload_bytes': outputPayloadBytes,
    'published_results': publishedResults,
  });
}

/// Reports one failed batch operation without hiding committed results or input.
final class BatchOperationException<Result> implements Exception {
  /// Creates an immutable composite failure.
  BatchOperationException({
    required Iterable<Result> completedResults,
    required this.cause,
    required this.stackTrace,
    required this.operation,
    required this.pending,
    required this.newInputAccepted,
  }) : completedResults = List<Result>.unmodifiable(completedResults);

  /// Results committed before [cause], in canonical wire order.
  final List<Result> completedResults;

  /// Original append or lifecycle failure.
  final Object cause;

  /// Original callback stack trace.
  final StackTrace stackTrace;

  /// Public batch operation that failed.
  final BatchOperation operation;

  /// Exact retained ownership after the failure, or `null` when none remains.
  final BatchPendingInput? pending;

  /// Whether a failing push accepted its new chunk; other operations use `null`.
  final bool? newInputAccepted;

  @override
  String toString() =>
      'BatchOperationException(${operation.wireValue}) after '
      '${completedResults.length} result(s), retaining '
      '${pending?.constituents ?? '0'} constituent(s): $cause';
}

/// Package-internal batching primitive used by the engine input batcher.
///
/// This class is deliberately not exported from `mdstream.dart`. The public
/// engine wrapper owns the single batching lease and supplies the only
/// production append capability.
final class BatchInputQueue<Result> {
  /// Creates the constituent-first production queue.
  BatchInputQueue({
    required int maxBatchBytes,
    required int maxPendingChunks,
    required Result Function(String chunk, int utf8Bytes) append,
    required int Function(Result result) outputPayloadBytes,
    int Function(String chunk, void Function(int bytes) observeScan)
        inputByteLength =
        _measureUtf8ByteLength,
  }) : _maxBatchBytes = maxBatchBytes,
       _maxPendingChunks = maxPendingChunks,
       _append = append,
       _measureOutput = outputPayloadBytes,
       _inputByteLength = inputByteLength {
    if (maxBatchBytes <= 0) {
      throw RangeError.range(maxBatchBytes, 1, null, 'maxBatchBytes');
    }
    if (maxPendingChunks <= 0) {
      throw RangeError.range(maxPendingChunks, 1, null, 'maxPendingChunks');
    }
  }

  final int _maxBatchBytes;
  final int _maxPendingChunks;
  final Result Function(String chunk, int utf8Bytes) _append;
  final int Function(Result result) _measureOutput;
  final int Function(String chunk, void Function(int bytes) observeScan)
  _inputByteLength;
  final ListQueue<_PendingChunk> _pending = ListQueue<_PendingChunk>();

  int _pendingBytes = 0;
  bool _unresolved = false;
  BigInt _inputAttempts = BigInt.zero;
  BigInt _inputBytes = BigInt.zero;
  BigInt _appendAttempts = BigInt.zero;
  BigInt _successfulAppends = BigInt.zero;
  BigInt _committedBytes = BigInt.zero;
  BigInt _scanBytes = BigInt.zero;
  final BigInt _joinCopyBytes = BigInt.zero;
  final BigInt _replayCount = BigInt.zero;
  BigInt _outputBytes = BigInt.zero;
  BigInt _publishedResults = BigInt.zero;

  /// Maximum retained UTF-8 bytes.
  int get maxBatchBytes => _maxBatchBytes;

  /// Maximum retained non-empty caller chunks.
  int get maxPendingChunks => _maxPendingChunks;

  /// Returns retained ownership, or `null` when no input is pending.
  BatchPendingInput? inspectPending() =>
      _pending.isEmpty ? null : BatchPendingInput._(_pending, _pendingBytes);

  /// Accepts one input attempt and returns any commits caused by admission.
  List<Result> push(String chunk) {
    _inputAttempts += BigInt.one;
    _assertResolved();
    final bytes = _inputByteLength(
      chunk,
      (scannedBytes) => _scanBytes += BigInt.from(scannedBytes),
    );
    _inputBytes += BigInt.from(bytes);
    if (bytes == 0) {
      return List<Result>.empty(growable: false);
    }

    final completed = <Result>[];
    if (_pending.isNotEmpty && _wouldExceed(bytes)) {
      completed.addAll(
        _applyPending(
          BatchOperation.push,
          newInputAccepted: false,
          earlierResults: completed,
        ),
      );
    }

    if (bytes > _maxBatchBytes) {
      completed.add(
        _appendStandalone(chunk, bytes, BatchOperation.push, completed),
      );
      return List<Result>.unmodifiable(completed);
    }

    _pending.addLast(_PendingChunk(chunk, bytes));
    _pendingBytes += bytes;
    if (_pendingBytes == _maxBatchBytes) {
      completed.addAll(
        _applyPending(
          BatchOperation.push,
          newInputAccepted: true,
          earlierResults: completed,
        ),
      );
    }
    return List<Result>.unmodifiable(completed);
  }

  /// Commits ordinary pending input.
  List<Result> flush() {
    _assertResolved();
    return _applyPending(
      BatchOperation.flush,
      newInputAccepted: null,
      earlierResults: List<Result>.empty(growable: false),
    );
  }

  /// Explicitly retries input retained after an accepted append failure.
  List<Result> retryPending() {
    if (!_unresolved) {
      throw _batchStateError(
        'bindings.batch_pending',
        'the batcher has no unresolved pending input to retry',
      );
    }
    return _applyPending(
      BatchOperation.retryPending,
      newInputAccepted: null,
      earlierResults: List<Result>.empty(growable: false),
    );
  }

  /// Transfers pending input to the caller and clears unresolved state.
  BatchPendingInput? takePending() {
    final transferred = inspectPending();
    _clearPending();
    return transferred;
  }

  /// Explicitly abandons all pending input and clears unresolved state.
  BatchPendingInput? discardPending() {
    final discarded = inspectPending();
    _clearPending();
    return discarded;
  }

  /// Runs a result-producing lifecycle operation after ordinary pending input.
  List<Result> runResultOperation(
    BatchOperation operation,
    Result Function() callback,
  ) {
    _assertResolved();
    final completed = <Result>[
      ..._applyPending(
        operation,
        newInputAccepted: null,
        earlierResults: List<Result>.empty(growable: false),
      ),
    ];
    final result = _runExternal(operation, completed, callback);
    _recordPublished(result);
    completed.add(result);
    return List<Result>.unmodifiable(completed);
  }

  /// Runs a value-producing lifecycle operation after ordinary pending input.
  BatchValueResult<Value, Result> runValueOperation<Value>(
    BatchOperation operation,
    Value Function() callback, {
    int Function(Value value)? outputPayloadBytes,
  }) {
    _assertResolved();
    final completed = <Result>[
      ..._applyPending(
        operation,
        newInputAccepted: null,
        earlierResults: List<Result>.empty(growable: false),
      ),
    ];
    final value = _runExternal(operation, completed, callback);
    final measured = outputPayloadBytes?.call(value) ?? 0;
    _recordMeasuredOutput(measured);
    return BatchValueResult<Value, Result>(
      completedResults: completed,
      value: value,
    );
  }

  /// Returns the final deterministic counter snapshot.
  BatchMetrics get metrics => BatchMetrics(
    maxBatchBytes: _maxBatchBytes.toString(),
    maxPendingChunks: _maxPendingChunks.toString(),
    inputAttempts: _inputAttempts.toString(),
    inputBytes: _inputBytes.toString(),
    appendAttempts: _appendAttempts.toString(),
    successfulAppends: _successfulAppends.toString(),
    committedBytes: _committedBytes.toString(),
    pendingBytes: _pendingBytes.toString(),
    pendingConstituents: _pending.length.toString(),
    boundaryMetadataBytes: (_pending.length * _logicalBoundaryBytes).toString(),
    scanBytes: _scanBytes.toString(),
    joinCopyBytes: _joinCopyBytes.toString(),
    replayCount: _replayCount.toString(),
    outputPayloadBytes: _outputBytes.toString(),
    publishedResults: _publishedResults.toString(),
  );

  bool _wouldExceed(int nextBytes) =>
      _pendingBytes + nextBytes > _maxBatchBytes ||
      _pending.length + 1 > _maxPendingChunks;

  List<Result> _applyPending(
    BatchOperation operation, {
    required bool? newInputAccepted,
    required List<Result> earlierResults,
  }) {
    if (_pending.isEmpty) {
      _unresolved = false;
      return List<Result>.empty(growable: false);
    }
    return _applyConstituents(
      operation,
      newInputAccepted: newInputAccepted,
      earlierResults: earlierResults,
    );
  }

  List<Result> _applyConstituents(
    BatchOperation operation, {
    required bool? newInputAccepted,
    required List<Result> earlierResults,
  }) {
    final completed = <Result>[];
    while (_pending.isNotEmpty) {
      final chunk = _pending.first;
      final result = _attemptAppend(
        chunk.text,
        chunk.bytes,
        operation,
        earlierResults: earlierResults,
        operationResults: completed,
        newInputAccepted: newInputAccepted,
      );
      _pending.removeFirst();
      _pendingBytes -= chunk.bytes;
      _recordCommittedAppend(chunk.bytes, result);
      completed.add(result);
    }
    _unresolved = false;
    return List<Result>.unmodifiable(completed);
  }

  Result _appendStandalone(
    String chunk,
    int bytes,
    BatchOperation operation,
    List<Result> earlierResults,
  ) {
    final result = _attemptAppend(
      chunk,
      bytes,
      operation,
      earlierResults: earlierResults,
      operationResults: List<Result>.empty(growable: false),
      newInputAccepted: false,
    );
    _recordCommittedAppend(bytes, result);
    return result;
  }

  Result _attemptAppend(
    String chunk,
    int bytes,
    BatchOperation operation, {
    required List<Result> earlierResults,
    required List<Result> operationResults,
    required bool? newInputAccepted,
  }) {
    _appendAttempts += BigInt.one;
    try {
      return _append(chunk, bytes);
    } catch (error, stackTrace) {
      if (_pending.isNotEmpty) {
        _unresolved = true;
      }
      throw BatchOperationException<Result>(
        completedResults: _completedEvidence(earlierResults, operationResults),
        cause: error,
        stackTrace: stackTrace,
        operation: operation,
        pending: inspectPending(),
        newInputAccepted: newInputAccepted,
      );
    }
  }

  Iterable<Result> _completedEvidence(
    List<Result> earlierResults,
    List<Result> operationResults,
  ) sync* {
    yield* earlierResults;
    yield* operationResults;
  }

  Value _runExternal<Value>(
    BatchOperation operation,
    List<Result> completedResults,
    Value Function() callback,
  ) {
    try {
      return callback();
    } catch (error, stackTrace) {
      throw BatchOperationException<Result>(
        completedResults: completedResults,
        cause: error,
        stackTrace: stackTrace,
        operation: operation,
        pending: inspectPending(),
        newInputAccepted: null,
      );
    }
  }

  void _recordCommittedAppend(int bytes, Result result) {
    _successfulAppends += BigInt.one;
    _committedBytes += BigInt.from(bytes);
    _recordPublished(result);
  }

  void _recordPublished(Result result) {
    _recordMeasuredOutput(_measureOutput(result));
    _publishedResults += BigInt.one;
  }

  void _recordMeasuredOutput(int bytes) {
    if (bytes < 0) {
      throw StateError('output payload byte measurements cannot be negative');
    }
    _outputBytes += BigInt.from(bytes);
  }

  void _clearPending() {
    _pending.clear();
    _pendingBytes = 0;
    _unresolved = false;
  }

  void _assertResolved() {
    if (_unresolved) {
      throw _batchStateError(
        'bindings.batch_unresolved',
        'pending input is unresolved; retry, take, or discard it explicitly',
      );
    }
  }
}

/// Package-internal result for lifecycle operations with a non-engine value.
final class BatchValueResult<Value, Result> {
  /// Creates one immutable lifecycle result.
  BatchValueResult({
    required Iterable<Result> completedResults,
    required this.value,
  }) : completedResults = List<Result>.unmodifiable(completedResults);

  /// Append results committed before [value] was produced.
  final List<Result> completedResults;

  /// Lifecycle value produced after pending input committed.
  final Value value;
}

/// Counts UTF-8 bytes without allocating and rejects unpaired UTF-16 units.
int utf8ByteLength(String value) => _scanUtf8ByteLength(value)!;

int _measureUtf8ByteLength(String value, void Function(int bytes) observeScan) {
  final bytes = utf8ByteLength(value);
  observeScan(bytes);
  return bytes;
}

/// Checks whether [value] fits [maxBytes] of UTF-8 without allocation.
///
/// An obvious UTF-16 lower-bound overflow returns before scanning the whole
/// string. Inputs within that bound still reject unpaired UTF-16 units.
bool utf8ByteLengthAtMost(String value, int maxBytes) {
  if (maxBytes < 0) {
    throw RangeError.range(maxBytes, 0, null, 'maxBytes');
  }
  return utf8ByteLengthWithin(value, maxBytes) != null;
}

/// Returns the exact UTF-8 length when [value] fits [maxBytes], otherwise null.
///
/// This package-internal helper performs the same early-exit scan used by
/// native source admission without allocating an encoded buffer.
int? utf8ByteLengthWithin(
  String value,
  int maxBytes, {
  void Function(int bytes)? observeScan,
}) {
  if (maxBytes < 0) {
    throw RangeError.range(maxBytes, 0, null, 'maxBytes');
  }
  if (value.length > maxBytes) {
    observeScan?.call(0);
    return null;
  }
  return _scanUtf8ByteLength(value, maxBytes, observeScan);
}

int? _scanUtf8ByteLength(
  String value, [
  int? maxBytes,
  void Function(int bytes)? observeScan,
]) {
  var bytes = 0;
  for (var index = 0; index < value.length; index += 1) {
    final unit = value.codeUnitAt(index);
    if (unit <= 0x7f) {
      bytes += 1;
    } else if (unit <= 0x7ff) {
      bytes += 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff) {
      if (index + 1 >= value.length) {
        throw const FormatException(
          'input contains an unpaired high surrogate',
        );
      }
      final next = value.codeUnitAt(index + 1);
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
    if (maxBytes != null && bytes > maxBytes) {
      observeScan?.call(bytes);
      return null;
    }
  }
  observeScan?.call(bytes);
  return bytes;
}

MdstreamException _batchStateError(String detailCode, String message) =>
    MdstreamException(
      message,
      status: 1,
      statusName: 'MDSTREAM_INVALID_ARGUMENT',
      detailCode: detailCode,
    );

final class _PendingChunk {
  const _PendingChunk(this.text, this.bytes);

  final String text;
  final int bytes;
}
