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
  MdstreamInputBatcher._(this._engine, int maxBatchBytes)
    : _batcher = LosslessInputBatcher<EngineResult>(
        maxBatchBytes: maxBatchBytes,
        append: _engine.append,
        finish: _engine.finish,
        reset: _engine.reset,
        outputPayloadBytes: (result) => int.parse(result.outputPayloadBytes),
      );

  final MdstreamEngine _engine;
  final LosslessInputBatcher<EngineResult> _batcher;
  int _snapshotOutputBytes = 0;

  /// Maximum UTF-8 bytes retained before pending input is flushed.
  int get maxBatchBytes => _batcher.maxBatchBytes;

  /// Returns a point-in-time immutable view of batching and output counters.
  BatchMetrics get metrics {
    final metrics = _batcher.metrics;
    return BatchMetrics(
      maxBatchBytes: metrics.maxBatchBytes,
      inputChunks: metrics.inputChunks,
      inputBytes: metrics.inputBytes,
      forwardedBytes: metrics.forwardedBytes,
      pendingBytes: metrics.pendingBytes,
      joinCopyBytes: metrics.joinCopyBytes,
      outputPayloadBytes:
          (BigInt.parse(metrics.outputPayloadBytes) +
                  BigInt.from(_snapshotOutputBytes))
              .toString(),
      batchCount: metrics.batchCount,
      appendCalls: metrics.appendCalls,
    );
  }

  /// Queues one source chunk and returns results from any automatic flushes.
  List<EngineResult> push(String chunk) => _batcher.push(chunk);

  /// Forwards pending source and returns its result, if any.
  EngineResult? flush() => _batcher.flush();

  /// Flushes pending source and finalizes the underlying stream.
  List<EngineResult> finish() => _batcher.finish();

  /// Flushes pending source and resets the underlying stream.
  List<EngineResult> reset() => _batcher.reset();

  /// Flushes pending source before requesting a canonical recovery snapshot.
  BatchedRecoverySnapshot createRecoverySnapshot() {
    final flushed = <EngineResult>[];
    final result = flush();
    if (result != null) {
      flushed.add(result);
    }
    try {
      final snapshot = _engine.createRecoverySnapshot();
      _snapshotOutputBytes += snapshot?.byteLength ?? 0;
      return BatchedRecoverySnapshot(
        flushed: List.unmodifiable(flushed),
        snapshot: snapshot,
      );
    } catch (error, stackTrace) {
      if (flushed.isEmpty) {
        rethrow;
      }
      throw BatchOperationException<EngineResult>(
        completedResults: List.unmodifiable(flushed),
        cause: error,
        stackTrace: stackTrace,
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
    utf8ByteLength(chunk);
    _commands += 1;
    return _consume(_engine.append(Uint8List.fromList(utf8.encode(chunk))));
  }

  /// Finalizes the stream and applies every emitted change.
  EngineResult finish() {
    _commands += 1;
    return _consume(_engine.execute(_command('finish')));
  }

  /// Resets the stream and applies every emitted change.
  EngineResult reset() {
    _commands += 1;
    return _consume(_engine.execute(_command('reset')));
  }

  /// Requests an opaque canonical snapshot for reducer recovery.
  CanonicalSnapshotBytes? createRecoverySnapshot() {
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

  /// Creates a lossless source batcher bounded by [maxBatchBytes].
  MdstreamInputBatcher createBatcher(int maxBatchBytes) {
    _assertOpen();
    return MdstreamInputBatcher._(this, maxBatchBytes);
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
}

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
