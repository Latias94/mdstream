import 'dart:convert';
import 'dart:typed_data';

import 'batching.dart';
import 'errors.dart';
import 'ffi.dart';
import 'options.dart';
import 'protocol.dart';
import 'reducer_handle.dart';
import 'views.dart';

/// Ordered engine deltas and the corresponding canonical reducer results.
final class EngineResult {
  /// Creates the result of one native engine command and its reducer effects.
  const EngineResult({
    required this.changes,
    required this.reducerResults,
    required this.outputPayloadBytes,
  });

  /// Canonical protocol changes emitted by the command, in wire order.
  final List<CanonicalChangeBytes> changes;

  /// Reducer results corresponding one-to-one with [changes].
  final List<ReducerResult> reducerResults;

  /// Total encoded bytes emitted by both the engine and reducer.
  final DecimalCounter outputPayloadBytes;

  /// Canonical reducer updates flattened in emission order.
  List<ReducerUpdateView> get updates =>
      List.unmodifiable(reducerResults.expand((result) => result.updates));

  /// Transition facts flattened in emission order.
  List<TransitionFactsView> get transitionFacts => List.unmodifiable(
    reducerResults.expand((result) => result.transitionFacts),
  );

  /// Processor requests flattened in emission order.
  List<ProcessorRequestView> get processorRequests => List.unmodifiable(
    reducerResults.expand((result) => result.processorRequests),
  );

  /// Processor completions flattened in emission order.
  List<ProcessorCompletionView> get processorCompletions => List.unmodifiable(
    reducerResults.expand((result) => result.processorCompletions),
  );

  /// Artifact changes flattened in emission order.
  List<ArtifactChangeView> get artifactChanges => List.unmodifiable(
    reducerResults.expand((result) => result.artifactChanges),
  );
}

/// Dart-side counters for payloads crossing the native engine transport.
final class EngineTransportMetrics {
  /// Creates an immutable snapshot of engine transport counters.
  const EngineTransportMetrics({
    required this.commands,
    required this.changePayloads,
    required this.snapshotPayloads,
    required this.outputPayloadBytes,
  });

  /// Number of commands sent to the native engine.
  final DecimalCounter commands;

  /// Number of canonical change payloads returned by the engine.
  final DecimalCounter changePayloads;

  /// Number of recovery snapshot payloads returned by the engine.
  final DecimalCounter snapshotPayloads;

  /// Total encoded payload bytes returned by the engine.
  final DecimalCounter outputPayloadBytes;
}

/// Result of flushing a batch before requesting a recovery snapshot.
final class BatchedRecoverySnapshot {
  /// Creates a snapshot result with any append committed before the request.
  const BatchedRecoverySnapshot({
    required this.flushed,
    required this.snapshot,
  });

  /// Results committed while flushing pending source before the snapshot.
  final List<EngineResult> flushed;

  /// Canonical snapshot, or `null` when the engine emits no snapshot payload.
  final CanonicalSnapshotBytes? snapshot;
}

/// Lossless, byte-bounded input batching specialized for [MdstreamEngine].
final class MdstreamInputBatcher {
  MdstreamInputBatcher._(
    this._engine,
    this._lease, {
    required int maxBatchBytes,
    required int maxPendingChunks,
  }) : _batcher = BatchInputQueue<EngineResult>(
         maxBatchBytes: maxBatchBytes,
         maxPendingChunks: maxPendingChunks,
         append: (chunk, utf8Bytes) =>
             _engine._appendFromBatcher(_lease, chunk, utf8Bytes),
         outputPayloadBytes: (result) => int.parse(result.outputPayloadBytes),
         inputByteLength: (chunk, observeScan) =>
             _engine._preflightFromBatcher(_lease, chunk, observeScan),
       );

  final MdstreamEngine _engine;
  final _BatchLease _lease;
  final BatchInputQueue<EngineResult> _batcher;
  bool _released = false;

  /// Maximum UTF-8 bytes retained before pending input is flushed.
  int get maxBatchBytes => _batcher.maxBatchBytes;

  /// Maximum non-empty caller chunks retained before lossless pre-flush.
  int get maxPendingChunks => _batcher.maxPendingChunks;

  /// Whether this batcher has released its exclusive engine lease.
  bool get isReleased => _released;

  /// Returns a point-in-time immutable view of batching and output counters.
  BatchMetrics get metrics => _batcher.metrics;

  /// Returns exact pending ownership, or `null` when nothing is retained.
  BatchPendingInput? inspectPending() {
    _assertActive();
    return _batcher.inspectPending();
  }

  /// Queues one source chunk and returns results from any automatic flushes.
  List<EngineResult> push(String chunk) {
    _assertActive();
    return _batcher.push(chunk);
  }

  /// Commits ordinary pending source and returns all ordered results.
  List<EngineResult> flush() {
    _assertActive();
    return _batcher.flush();
  }

  /// Explicitly retries source retained by an earlier append failure.
  List<EngineResult> retryPending() {
    _assertActive();
    return _batcher.retryPending();
  }

  /// Transfers exact pending chunks to the caller without committing them.
  BatchPendingInput? takePending() {
    _assertActive();
    return _batcher.takePending();
  }

  /// Explicitly abandons pending input without committing it.
  BatchPendingInput? discardPending() {
    _assertActive();
    return _batcher.discardPending();
  }

  /// Flushes pending source and finalizes the underlying stream.
  List<EngineResult> finish() {
    _assertActive();
    return _batcher.runResultOperation(
      BatchOperation.finish,
      () => _engine._finishFromBatcher(_lease),
    );
  }

  /// Flushes pending source and resets the underlying stream.
  List<EngineResult> reset() {
    _assertActive();
    return _batcher.runResultOperation(
      BatchOperation.reset,
      () => _engine._resetFromBatcher(_lease),
    );
  }

  /// Flushes pending source before requesting a canonical recovery snapshot.
  BatchedRecoverySnapshot createRecoverySnapshot() {
    _assertActive();
    final result = _batcher.runValueOperation<CanonicalSnapshotBytes?>(
      BatchOperation.recoverySnapshot,
      () => _engine._snapshotFromBatcher(_lease),
      outputPayloadBytes: (snapshot) => snapshot?.byteLength ?? 0,
    );
    return BatchedRecoverySnapshot(
      flushed: result.completedResults,
      snapshot: result.value,
    );
  }

  /// Releases the exclusive engine lease after pending ownership is resolved.
  void release() {
    _assertActive();
    if (_batcher.inspectPending() != null) {
      throw _batchStateException(
        'bindings.batch_pending',
        'pending input must commit, transfer, or be discarded before release',
      );
    }
    _engine._releaseBatchLease(_lease);
    _released = true;
  }

  void _assertActive() {
    if (_released) {
      throw _batchStateException(
        'bindings.batch_released',
        'mdstream input batcher has released its engine lease',
      );
    }
  }
}

/// High-level streaming engine paired with its canonical native reducer.
final class MdstreamEngine {
  MdstreamEngine._(this._engine, this._reducer, this._schema);

  final NativeEngineHandle _engine;
  final MdstreamReducer _reducer;
  final String _schema;
  int _commands = 0;
  int _changePayloads = 0;
  int _snapshotPayloads = 0;
  int _outputPayloadBytes = 0;
  _BatchLease? _batchLease;

  /// Whether the native engine and paired reducer have been closed.
  bool get isClosed => _engine.isClosed;

  /// Current canonical reducer state.
  MdstreamStateView get state => _reducer.state;

  /// Effective native capacity for framework-side processor scheduling.
  MdstreamProcessorSchedulerLimits get processorSchedulerLimits =>
      _reducer.processorSchedulerLimits;

  /// Returns a point-in-time snapshot of native engine transport counters.
  EngineTransportMetrics get metrics => EngineTransportMetrics(
    commands: decimalCounterFromTrustedInt(_commands),
    changePayloads: decimalCounterFromTrustedInt(_changePayloads),
    snapshotPayloads: decimalCounterFromTrustedInt(_snapshotPayloads),
    outputPayloadBytes: decimalCounterFromTrustedInt(_outputPayloadBytes),
  );

  /// Returns a point-in-time snapshot of reducer transport counters.
  ReducerTransportMetrics get reducerMetrics => _reducer.metrics;

  /// Appends one UTF-8 source chunk and applies every emitted change.
  EngineResult append(String chunk) {
    _assertDirectMutationAllowed();
    return _appendUnchecked(chunk);
  }

  EngineResult _appendFromBatcher(
    _BatchLease lease,
    String chunk,
    int utf8Bytes,
  ) {
    _assertBatchLease(lease);
    _assertCachedAppendAdmission(utf8Bytes);
    return _appendEncoded(chunk);
  }

  EngineResult _appendUnchecked(String chunk) {
    _admittedUtf8ByteLength(chunk);
    return _appendEncoded(chunk);
  }

  EngineResult _appendEncoded(String chunk) {
    _commands += 1;
    return _consume(_engine.append(Uint8List.fromList(utf8.encode(chunk))));
  }

  int _preflightFromBatcher(
    _BatchLease lease,
    String chunk,
    void Function(int bytes) observeScan,
  ) {
    _assertBatchLease(lease);
    return _admittedUtf8ByteLength(chunk, observeScan: observeScan);
  }

  int _admittedUtf8ByteLength(
    String chunk, {
    void Function(int bytes)? observeScan,
  }) {
    final ceiling = _engine.rawAppendByteCeiling;
    if (ceiling == null) {
      final bytes = utf8ByteLength(chunk);
      observeScan?.call(bytes);
      return bytes;
    }
    final bytes = utf8ByteLengthWithin(
      chunk,
      ceiling,
      observeScan: observeScan,
    );
    if (bytes != null) {
      return bytes;
    }
    throw _rawAdmissionException();
  }

  void _assertCachedAppendAdmission(int utf8Bytes) {
    final ceiling = _engine.rawAppendByteCeiling;
    if (ceiling == null || utf8Bytes <= ceiling) {
      return;
    }
    throw _rawAdmissionException();
  }

  MdstreamException _rawAdmissionException() {
    return MdstreamException(
      'raw append input exceeds the current native source admission ceiling',
      status: BindingStatus.resourceLimitExceeded.value,
      statusName: BindingStatus.resourceLimitExceeded.statusName,
      detailCode: 'bindings.resource_limit',
    );
  }

  /// Finalizes the stream and applies every emitted change.
  EngineResult finish() {
    _assertDirectMutationAllowed();
    return _finishUnchecked();
  }

  EngineResult _finishFromBatcher(_BatchLease lease) {
    _assertBatchLease(lease);
    return _finishUnchecked();
  }

  EngineResult _finishUnchecked() {
    _commands += 1;
    return _consume(_engine.execute(_command('finish')));
  }

  /// Resets the stream and applies every emitted change.
  EngineResult reset() {
    _assertDirectMutationAllowed();
    return _resetUnchecked();
  }

  EngineResult _resetFromBatcher(_BatchLease lease) {
    _assertBatchLease(lease);
    return _resetUnchecked();
  }

  EngineResult _resetUnchecked() {
    _commands += 1;
    return _consume(_engine.execute(_command('reset')));
  }

  /// Requests an opaque canonical snapshot for reducer recovery.
  CanonicalSnapshotBytes? createRecoverySnapshot() {
    _assertDirectMutationAllowed();
    return _snapshotUnchecked();
  }

  CanonicalSnapshotBytes? _snapshotFromBatcher(_BatchLease lease) {
    _assertBatchLease(lease);
    return _snapshotUnchecked();
  }

  CanonicalSnapshotBytes? _snapshotUnchecked() {
    _commands += 1;
    final payloads = _engine.execute(_command('snapshot'));
    CanonicalSnapshotBytes? snapshot;
    for (final payload in payloads) {
      final kind = BindingPayloadKind.fromValue(payload.kind);
      if (kind != BindingPayloadKind.snapshot || snapshot != null) {
        throw _unexpectedEnginePayload(
          'engine snapshot returned ${kind.viewKind}',
        );
      }
      _snapshotPayloads += 1;
      _outputPayloadBytes += payload.bytes.length;
      snapshot = canonicalSnapshotBytesFromOwned(payload.bytes);
    }
    return snapshot;
  }

  /// Creates the engine's only live lossless source batcher.
  MdstreamInputBatcher createBatcher({
    required int maxBatchBytes,
    required int maxPendingChunks,
  }) {
    _assertOpen();
    if (_batchLease != null) {
      throw _batchStateException(
        'bindings.batch_lease_active',
        'mdstream engine already has an active input batcher',
      );
    }
    final lease = _BatchLease();
    _batchLease = lease;
    try {
      return MdstreamInputBatcher._(
        this,
        lease,
        maxBatchBytes: maxBatchBytes,
        maxPendingChunks: maxPendingChunks,
      );
    } catch (_) {
      _batchLease = null;
      rethrow;
    }
  }

  /// Starts processor work for the current version of [nodeId].
  ReducerResult beginProcessor({
    required NodeId nodeId,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    bool acceptsProvisional = false,
    bool allowProvisional = false,
  }) => _reducer.beginProcessor(
    nodeId: nodeId,
    processorId: processorId,
    processorVersion: processorVersion,
    configurationVersion: configurationVersion,
    acceptsProvisional: acceptsProvisional,
    allowProvisional: allowProvisional,
  );

  /// Starts processor work only when the expected epoch and input version match.
  ReducerResult beginProcessorIfCurrent({
    required Epoch expectedEpoch,
    required NodeId nodeId,
    required ProcessorInputVersion expectedInputVersion,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    bool acceptsProvisional = false,
    bool allowProvisional = false,
  }) => _reducer.beginProcessorIfCurrent(
    expectedEpoch: expectedEpoch,
    nodeId: nodeId,
    expectedInputVersion: expectedInputVersion,
    processorId: processorId,
    processorVersion: processorVersion,
    configurationVersion: configurationVersion,
    acceptsProvisional: acceptsProvisional,
    allowProvisional: allowProvisional,
  );

  /// Completes a processor request with a UTF-8 text artifact.
  ReducerResult completeProcessorText({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required String text,
  }) => _reducer.completeProcessorText(
    requestId: requestId,
    protocol: protocol,
    mediaType: mediaType,
    text: text,
  );

  /// Completes a processor request with an opaque binary artifact.
  ReducerResult completeProcessorBinary({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required List<int> bytes,
  }) => _reducer.completeProcessorBinary(
    requestId: requestId,
    protocol: protocol,
    mediaType: mediaType,
    bytes: bytes,
  );

  /// Marks a processor request as failed with a stable error code.
  ReducerResult failProcessor({
    required RequestGeneration requestId,
    required String code,
    required String message,
  }) => _reducer.failProcessor(
    requestId: requestId,
    code: code,
    message: message,
  );

  /// Cancels the processor request identified by [requestId].
  ReducerResult cancelProcessor(RequestGeneration requestId) =>
      _reducer.cancelProcessor(requestId);

  /// Releases the native engine and reducer handles.
  void close() {
    if (_engine.isClosed) {
      return;
    }
    if (_batchLease != null) {
      throw _batchStateException(
        'bindings.batch_lease_active',
        'release the active input batcher before closing its engine',
      );
    }
    _engine.close();
    _reducer.close();
  }

  EngineResult _consume(List<NativePayload> payloads) {
    final changes = <CanonicalChangeBytes>[];
    final reducerResults = <ReducerResult>[];
    var outputBytes = 0;
    for (final payload in payloads) {
      final kind = BindingPayloadKind.fromValue(payload.kind);
      if (kind != BindingPayloadKind.change) {
        throw _unexpectedEnginePayload(
          'engine transition returned ${kind.viewKind}',
        );
      }
      _changePayloads += 1;
      _outputPayloadBytes += payload.bytes.length;
      outputBytes += payload.bytes.length;
      final change = canonicalChangeBytesFromOwned(payload.bytes);
      changes.add(change);
      final reduced = _reducer.applyChange(change);
      reducerResults.add(reduced);
      outputBytes += int.parse(reduced.outputPayloadBytes);
    }
    return EngineResult(
      changes: List.unmodifiable(changes),
      reducerResults: List.unmodifiable(reducerResults),
      outputPayloadBytes: decimalCounterFromTrustedInt(outputBytes),
    );
  }

  Uint8List _command(String kind) => Uint8List.fromList(
    utf8.encode(jsonEncode({'schema': _schema, 'kind': kind})),
  );

  void _assertOpen() {
    if (isClosed) {
      throw MdstreamException(
        'mdstream engine is closed',
        status: BindingStatus.invalidArgument.value,
        statusName: BindingStatus.invalidArgument.statusName,
        detailCode: 'bindings.closed',
      );
    }
  }

  void _assertDirectMutationAllowed() {
    _assertOpen();
    if (_batchLease != null) {
      throw _batchStateException(
        'bindings.batch_lease_active',
        'direct engine mutation is blocked by an active input batcher',
      );
    }
  }

  void _assertBatchLease(_BatchLease lease) {
    _assertOpen();
    if (!identical(_batchLease, lease)) {
      throw _batchStateException(
        'bindings.batch_released',
        'the input batcher no longer owns this engine',
      );
    }
  }

  void _releaseBatchLease(_BatchLease lease) {
    _assertBatchLease(lease);
    _batchLease = null;
  }
}

final class _BatchLease {}

/// Pairs native engine and reducer handles behind the high-level engine API.
MdstreamEngine createNativeEngine(
  NativeEngineHandle engine,
  MdstreamReducer reducer,
  String schema,
) => MdstreamEngine._(engine, reducer, schema);

MdstreamException _unexpectedEnginePayload(String message) => MdstreamException(
  message,
  status: BindingStatus.internalError.value,
  statusName: BindingStatus.internalError.statusName,
  detailCode: 'bindings.unexpected_payload',
);

MdstreamException _batchStateException(String detailCode, String message) =>
    MdstreamException(
      message,
      status: BindingStatus.invalidArgument.value,
      statusName: BindingStatus.invalidArgument.statusName,
      detailCode: detailCode,
    );
