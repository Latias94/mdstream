// ignore_for_file: public_member_api_docs

import 'dart:convert';
import 'dart:typed_data';

import 'batching.dart';
import 'errors.dart';
import 'ffi.dart';
import 'protocol.dart';
import 'reducer_handle.dart';
import 'views.dart';

/// Ordered engine deltas and the corresponding canonical reducer results.
final class EngineResult {
  const EngineResult({
    required this.changes,
    required this.reducerResults,
    required this.outputPayloadBytes,
  });

  final List<CanonicalChangeBytes> changes;
  final List<ReducerResult> reducerResults;
  final DecimalCounter outputPayloadBytes;

  List<ReducerUpdateView> get updates =>
      List.unmodifiable(reducerResults.expand((result) => result.updates));

  List<ProcessorRequestView> get processorRequests => List.unmodifiable(
    reducerResults.expand((result) => result.processorRequests),
  );

  List<ProcessorCompletionView> get processorCompletions => List.unmodifiable(
    reducerResults.expand((result) => result.processorCompletions),
  );

  List<ArtifactChangeView> get artifactChanges => List.unmodifiable(
    reducerResults.expand((result) => result.artifactChanges),
  );
}

/// Dart-side counters for payloads crossing the native engine transport.
final class EngineTransportMetrics {
  const EngineTransportMetrics({
    required this.commands,
    required this.changePayloads,
    required this.snapshotPayloads,
    required this.outputPayloadBytes,
  });

  final DecimalCounter commands;
  final DecimalCounter changePayloads;
  final DecimalCounter snapshotPayloads;
  final DecimalCounter outputPayloadBytes;
}

/// Result of flushing a batch before requesting a recovery snapshot.
final class BatchedRecoverySnapshot {
  const BatchedRecoverySnapshot({
    required this.flushed,
    required this.snapshot,
  });

  final List<EngineResult> flushed;
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

  int get maxBatchBytes => _batcher.maxBatchBytes;
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

  List<EngineResult> push(String chunk) => _batcher.push(chunk);
  EngineResult? flush() => _batcher.flush();
  List<EngineResult> finish() => _batcher.finish();
  List<EngineResult> reset() => _batcher.reset();

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

  bool get isClosed => _engine.isClosed;
  MdstreamStateView get state => _reducer.state;
  EngineTransportMetrics get metrics => EngineTransportMetrics(
    commands: _commands.toString(),
    changePayloads: _changePayloads.toString(),
    snapshotPayloads: _snapshotPayloads.toString(),
    outputPayloadBytes: _outputPayloadBytes.toString(),
  );
  ReducerTransportMetrics get reducerMetrics => _reducer.metrics;

  EngineResult append(String chunk) {
    utf8ByteLength(chunk);
    _commands += 1;
    return _consume(_engine.append(Uint8List.fromList(utf8.encode(chunk))));
  }

  EngineResult finish() {
    _commands += 1;
    return _consume(_engine.execute(_command('finish')));
  }

  EngineResult reset() {
    _commands += 1;
    return _consume(_engine.execute(_command('reset')));
  }

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
      snapshot = CanonicalSnapshotBytes(payload.bytes);
    }
    return snapshot;
  }

  MdstreamInputBatcher createBatcher(int maxBatchBytes) {
    _assertOpen();
    return MdstreamInputBatcher._(this, maxBatchBytes);
  }

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

  ReducerResult failProcessor({
    required RequestGeneration requestId,
    required String code,
    required String message,
  }) => _reducer.failProcessor(
    requestId: requestId,
    code: code,
    message: message,
  );

  ReducerResult cancelProcessor(RequestGeneration requestId) =>
      _reducer.cancelProcessor(requestId);

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
      final change = CanonicalChangeBytes(payload.bytes);
      changes.add(change);
      final reduced = _reducer.applyChange(change);
      reducerResults.add(reduced);
      outputBytes += int.parse(reduced.outputPayloadBytes);
    }
    return EngineResult(
      changes: List.unmodifiable(changes),
      reducerResults: List.unmodifiable(reducerResults),
      outputPayloadBytes: outputBytes.toString(),
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
