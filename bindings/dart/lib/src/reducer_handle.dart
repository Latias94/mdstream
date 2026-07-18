// ignore_for_file: public_member_api_docs

import 'dart:collection';
import 'dart:convert';
import 'dart:typed_data';

import 'errors.dart';
import 'ffi.dart';
import 'protocol.dart';
import 'views.dart';

/// Immutable reducer state needed by UI-framework adapters.
final class MdstreamStateSnapshot {
  const MdstreamStateSnapshot({
    required this.status,
    required this.document,
    required this.impact,
  });

  final ReducerStatusView status;
  final DocumentSummaryView? document;
  final ChangeImpactView impact;
}

/// Stable identity of one derived processor artifact slot.
final class ArtifactSlot {
  const ArtifactSlot({
    required this.epoch,
    required this.nodeId,
    required this.processorId,
  });

  final Epoch epoch;
  final NodeId nodeId;
  final String processorId;

  String get _cacheKey =>
      '${epoch.length}:$epoch${nodeId.length}:$nodeId'
      '${processorId.length}:$processorId';

  @override
  bool operator ==(Object other) =>
      other is ArtifactSlot &&
      epoch == other.epoch &&
      nodeId == other.nodeId &&
      processorId == other.processorId;

  @override
  int get hashCode => Object.hash(epoch, nodeId, processorId);
}

/// Typed output from one canonical reducer transition.
final class ReducerResult {
  const ReducerResult({
    required this.updates,
    required this.processorRequests,
    required this.processorCompletions,
    required this.artifactChanges,
    required this.outputPayloadBytes,
  });

  final List<ReducerUpdateView> updates;
  final List<ProcessorRequestView> processorRequests;
  final List<ProcessorCompletionView> processorCompletions;
  final List<ArtifactChangeView> artifactChanges;
  final DecimalCounter outputPayloadBytes;

  List<NodeId> get changedNodeIds => List.unmodifiable(
    LinkedHashSet<NodeId>.from(
      updates.expand((update) => update.impact.changedNodeIds),
    ),
  );

  List<NodeId> get removedNodeIds => List.unmodifiable(
    LinkedHashSet<NodeId>.from(
      updates.expand((update) => update.impact.removedNodeIds),
    ),
  );
}

/// Dart-side counters that prove the binding remains delta-first.
final class ReducerTransportMetrics {
  const ReducerTransportMetrics({
    required this.commands,
    required this.outputPayloadBytes,
    required this.changePayloads,
    required this.snapshotPayloads,
    required this.reducerUpdatePayloads,
    required this.nodeViewPayloads,
    required this.resourceViewPayloads,
    required this.processorRequestPayloads,
    required this.processorCompletionPayloads,
    required this.artifactChangePayloads,
    required this.artifactViewPayloads,
    required this.pendingSourceViewPayloads,
  });

  final DecimalCounter commands;
  final DecimalCounter outputPayloadBytes;
  final DecimalCounter changePayloads;
  final DecimalCounter snapshotPayloads;
  final DecimalCounter reducerUpdatePayloads;
  final DecimalCounter nodeViewPayloads;
  final DecimalCounter resourceViewPayloads;
  final DecimalCounter processorRequestPayloads;
  final DecimalCounter processorCompletionPayloads;
  final DecimalCounter artifactChangePayloads;
  final DecimalCounter artifactViewPayloads;
  final DecimalCounter pendingSourceViewPayloads;

  DecimalCounter payloadCount(BindingPayloadKind kind) => switch (kind) {
    BindingPayloadKind.change => changePayloads,
    BindingPayloadKind.snapshot => snapshotPayloads,
    BindingPayloadKind.reducerUpdate => reducerUpdatePayloads,
    BindingPayloadKind.nodeView => nodeViewPayloads,
    BindingPayloadKind.resourceView => resourceViewPayloads,
    BindingPayloadKind.processorRequest => processorRequestPayloads,
    BindingPayloadKind.processorCompletion => processorCompletionPayloads,
    BindingPayloadKind.artifactChange => artifactChangePayloads,
    BindingPayloadKind.artifactView => artifactViewPayloads,
    BindingPayloadKind.pendingSourceView => pendingSourceViewPayloads,
  };
}

/// Readonly canonical state and lazily materialized content views.
final class MdstreamStateView {
  MdstreamStateView._();

  late final MdstreamReducer _owner;

  MdstreamStateSnapshot get currentState => _owner.currentState;

  NodeView? nodeView(NodeId id) => _owner.nodeView(id);

  ResourceView? resourceView(ResourceId id) => _owner.resourceView(id);

  PendingSourceView? pendingSourceView() => _owner.pendingSourceView();

  ArtifactView? artifactView(ArtifactSlot slot) => _owner.artifactView(slot);
}

/// Stateful wrapper around the canonical native Rust reducer.
final class MdstreamReducer {
  MdstreamReducer._(this._handle, this._schema)
    : state = MdstreamStateView._(),
      _currentState = _initialState {
    state._owner = this;
  }

  final NativeReducerHandle _handle;
  final String _schema;
  final MdstreamStateView state;

  MdstreamStateSnapshot _currentState;
  final Map<NodeId, NodeView> _nodeCache = {};
  final Map<ResourceId, ResourceView> _resourceCache = {};
  final Map<String, ArtifactView> _artifactCache = {};
  final _BoundedKeySet<NodeId> _missingNodes = _BoundedKeySet(1024);
  final _BoundedKeySet<ResourceId> _missingResources = _BoundedKeySet(1024);
  final _BoundedKeySet<String> _missingArtifacts = _BoundedKeySet(1024);
  PendingSourceView? _pendingSourceCache;
  bool _pendingSourceLoaded = false;
  final Map<BindingPayloadKind, int> _payloadCounts = {
    for (final kind in BindingPayloadKind.values) kind: 0,
  };
  int _commands = 0;
  int _outputPayloadBytes = 0;

  bool get isClosed => _handle.isClosed;

  MdstreamStateSnapshot get currentState => _currentState;

  ReducerResult applyChange(CanonicalChangeBytes change) {
    _commands += 1;
    return _publicResult(
      _consume(_handle.apply(canonicalChangeBytesView(change))),
    );
  }

  ReducerResult recoverSnapshot(CanonicalSnapshotBytes snapshot) {
    _commands += 1;
    return _publicResult(
      _consume(_handle.recover(canonicalSnapshotBytesView(snapshot))),
    );
  }

  CanonicalSnapshotBytes? createRecoverySnapshot() {
    final output = _execute({'schema': _schema, 'kind': 'snapshot'});
    _expectOnly(output, {BindingPayloadKind.snapshot}, 'reducer snapshot');
    if (output.snapshots.length > 1) {
      throw _unexpectedPayload('reducer snapshot returned multiple payloads');
    }
    return output.snapshots.firstOrNull;
  }

  PendingSourceView? pendingSourceView() {
    if (_pendingSourceLoaded) {
      return _pendingSourceCache;
    }
    final output = _execute({'schema': _schema, 'kind': 'pending_source_view'});
    _expectOnly(output, {
      BindingPayloadKind.pendingSourceView,
    }, 'pending source view');
    if (output.pendingSourceViews.length > 1) {
      throw _unexpectedPayload(
        'pending source view returned multiple payloads',
      );
    }
    _pendingSourceCache = output.pendingSourceViews.firstOrNull;
    _pendingSourceLoaded = true;
    return _pendingSourceCache;
  }

  NodeView? nodeView(NodeId id) {
    validateDecimalU128Input(id, 'node_id');
    final cached = _nodeCache[id];
    if (cached != null) {
      return cached;
    }
    if (_missingNodes.contains(id)) {
      return null;
    }
    try {
      final output = _execute({
        'schema': _schema,
        'kind': 'node_view',
        'node_id': id,
      });
      _expectOnly(output, {BindingPayloadKind.nodeView}, 'node view');
      final view = output.nodeViews.firstOrNull;
      if (view == null) {
        _missingNodes.add(id);
        return null;
      }
      if (output.nodeViews.length != 1 || view.node.id != id) {
        throw _unexpectedPayload('node view response does not match node $id');
      }
      _nodeCache[id] = view;
      _missingNodes.remove(id);
      return view;
    } on MdstreamException catch (error) {
      if (error.detailCode == 'bindings.node_not_found' ||
          error.detailCode == 'bindings.document_uninitialized') {
        _missingNodes.add(id);
        return null;
      }
      rethrow;
    }
  }

  ResourceView? resourceView(ResourceId id) {
    validateDecimalU128Input(id, 'resource_id');
    final cached = _resourceCache[id];
    if (cached != null) {
      return cached;
    }
    if (_missingResources.contains(id)) {
      return null;
    }
    try {
      final output = _execute({
        'schema': _schema,
        'kind': 'resource_view',
        'resource_id': id,
      });
      _expectOnly(output, {BindingPayloadKind.resourceView}, 'resource view');
      final view = output.resourceViews.firstOrNull;
      if (view == null) {
        _missingResources.add(id);
        return null;
      }
      if (output.resourceViews.length != 1 || view.resource.id != id) {
        throw _unexpectedPayload(
          'resource view response does not match resource $id',
        );
      }
      _resourceCache[id] = view;
      _missingResources.remove(id);
      return view;
    } on MdstreamException catch (error) {
      if (error.detailCode == 'bindings.resource_not_found') {
        _missingResources.add(id);
        return null;
      }
      rethrow;
    }
  }

  ArtifactView? artifactView(ArtifactSlot slot) {
    validateDecimalU64Input(slot.epoch, 'epoch');
    validateDecimalU128Input(slot.nodeId, 'node_id');
    final key = slot._cacheKey;
    final cached = _artifactCache[key];
    if (cached != null) {
      return cached;
    }
    if (_missingArtifacts.contains(key)) {
      return null;
    }
    final output = _execute({
      'schema': _schema,
      'kind': 'artifact_view',
      'epoch': slot.epoch,
      'node_id': slot.nodeId,
      'processor_id': slot.processorId,
    });
    _expectOnly(output, {BindingPayloadKind.artifactView}, 'artifact view');
    final view = output.artifactViews.firstOrNull;
    if (view == null) {
      _missingArtifacts.add(key);
      return null;
    }
    if (output.artifactViews.length != 1 ||
        view.key.epoch != slot.epoch ||
        view.key.nodeId != slot.nodeId ||
        view.key.processorId != slot.processorId) {
      throw _unexpectedPayload(
        'artifact view response does not match its slot',
      );
    }
    _artifactCache[key] = view;
    _missingArtifacts.remove(key);
    return view;
  }

  ReducerResult beginProcessor({
    required NodeId nodeId,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    bool acceptsProvisional = false,
    bool allowProvisional = false,
  }) => _publicResult(
    _execute({
      'schema': _schema,
      'kind': 'begin_processor',
      'node_id': validateDecimalU128Input(nodeId, 'node_id'),
      'processor_id': processorId,
      'processor_version': processorVersion,
      'configuration_version': configurationVersion,
      'accepts_provisional': acceptsProvisional,
      'allow_provisional': allowProvisional,
    }),
  );

  ReducerResult beginProcessorIfCurrent({
    required Epoch expectedEpoch,
    required NodeId nodeId,
    required NodeVersion expectedNodeVersion,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    bool acceptsProvisional = false,
    bool allowProvisional = false,
  }) => _publicResult(
    _execute({
      'schema': _schema,
      'kind': 'begin_processor_if_current',
      'expected_epoch': validateDecimalU64Input(
        expectedEpoch,
        'expected_epoch',
      ),
      'node_id': validateDecimalU128Input(nodeId, 'node_id'),
      'expected_node_version': expectedNodeVersion,
      'processor_id': processorId,
      'processor_version': processorVersion,
      'configuration_version': configurationVersion,
      'accepts_provisional': acceptsProvisional,
      'allow_provisional': allowProvisional,
    }),
  );

  ReducerResult completeProcessorText({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required String text,
  }) => _completeProcessor(requestId, {
    'kind': 'text',
    'protocol': protocol,
    'media_type': mediaType,
    'text': text,
  });

  ReducerResult completeProcessorBinary({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required List<int> bytes,
  }) => _completeProcessor(requestId, {
    'kind': 'binary',
    'protocol': protocol,
    'media_type': mediaType,
    'bytes': _validatedOctets(bytes),
  });

  ReducerResult failProcessor({
    required RequestGeneration requestId,
    required String code,
    required String message,
  }) => _completeProcessor(requestId, {
    'kind': 'failure',
    'code': code,
    'message': message,
  });

  ReducerResult cancelProcessor(RequestGeneration requestId) => _publicResult(
    _execute({
      'schema': _schema,
      'kind': 'cancel_processor',
      'request_id': validateDecimalU64Input(requestId, 'request_id'),
    }),
  );

  ReducerTransportMetrics get metrics => ReducerTransportMetrics(
    commands: _commands.toString(),
    outputPayloadBytes: _outputPayloadBytes.toString(),
    changePayloads: _count(BindingPayloadKind.change),
    snapshotPayloads: _count(BindingPayloadKind.snapshot),
    reducerUpdatePayloads: _count(BindingPayloadKind.reducerUpdate),
    nodeViewPayloads: _count(BindingPayloadKind.nodeView),
    resourceViewPayloads: _count(BindingPayloadKind.resourceView),
    processorRequestPayloads: _count(BindingPayloadKind.processorRequest),
    processorCompletionPayloads: _count(BindingPayloadKind.processorCompletion),
    artifactChangePayloads: _count(BindingPayloadKind.artifactChange),
    artifactViewPayloads: _count(BindingPayloadKind.artifactView),
    pendingSourceViewPayloads: _count(BindingPayloadKind.pendingSourceView),
  );

  void close() {
    if (_handle.isClosed) {
      return;
    }
    _handle.close();
    _nodeCache.clear();
    _resourceCache.clear();
    _artifactCache.clear();
    _missingNodes.clear();
    _missingResources.clear();
    _missingArtifacts.clear();
    _invalidatePendingSource();
  }

  ReducerResult _completeProcessor(
    RequestGeneration requestId,
    Map<String, Object> outcome,
  ) => _publicResult(
    _execute({
      'schema': _schema,
      'kind': 'complete_processor',
      'request_id': validateDecimalU64Input(requestId, 'request_id'),
      'outcome': outcome,
    }),
  );

  _DecodedReducerOutput _execute(Map<String, Object> command) {
    _commands += 1;
    final bytes = Uint8List.fromList(utf8.encode(jsonEncode(command)));
    return _consume(_handle.execute(bytes));
  }

  _DecodedReducerOutput _consume(List<NativePayload> payloads) {
    final output = _DecodedReducerOutput();
    for (final payload in payloads) {
      final kind = BindingPayloadKind.fromValue(payload.kind);
      _payloadCounts[kind] = (_payloadCounts[kind] ?? 0) + 1;
      _outputPayloadBytes += payload.bytes.length;
      output.payloadKinds.add(kind);
      output.outputPayloadBytes += payload.bytes.length;
      switch (kind) {
        case BindingPayloadKind.change:
          throw _unexpectedPayload('reducer returned a canonical change');
        case BindingPayloadKind.snapshot:
          output.snapshots.add(canonicalSnapshotBytesFromOwned(payload.bytes));
        case BindingPayloadKind.reducerUpdate:
          output.updates.add(
            decodeBindingView(kind, payload.bytes, _schema)
                as ReducerUpdateView,
          );
        case BindingPayloadKind.nodeView:
          output.nodeViews.add(
            decodeBindingView(kind, payload.bytes, _schema) as NodeView,
          );
        case BindingPayloadKind.resourceView:
          output.resourceViews.add(
            decodeBindingView(kind, payload.bytes, _schema) as ResourceView,
          );
        case BindingPayloadKind.processorRequest:
          output.processorRequests.add(
            decodeBindingView(kind, payload.bytes, _schema)
                as ProcessorRequestView,
          );
        case BindingPayloadKind.processorCompletion:
          output.processorCompletions.add(
            decodeBindingView(kind, payload.bytes, _schema)
                as ProcessorCompletionView,
          );
        case BindingPayloadKind.artifactChange:
          output.artifactChanges.add(
            decodeBindingView(kind, payload.bytes, _schema)
                as ArtifactChangeView,
          );
        case BindingPayloadKind.artifactView:
          output.artifactViews.add(
            decodeBindingView(kind, payload.bytes, _schema) as ArtifactView,
          );
        case BindingPayloadKind.pendingSourceView:
          output.pendingSourceViews.add(
            decodeBindingView(kind, payload.bytes, _schema)
                as PendingSourceView,
          );
      }
    }
    for (final update in output.updates) {
      _applyUpdate(update);
    }
    for (final change in output.artifactChanges) {
      final slot = ArtifactSlot(
        epoch: change.key.epoch,
        nodeId: change.key.nodeId,
        processorId: change.key.processorId,
      );
      _artifactCache.remove(slot._cacheKey);
      _missingArtifacts.remove(slot._cacheKey);
      if (change.change.kind == 'removed') {
        _missingArtifacts.add(slot._cacheKey);
      }
    }
    return output;
  }

  void _applyUpdate(ReducerUpdateView update) {
    final statusChanged = update.status.kind != _currentState.status.kind;
    final stateChanged =
        update.outcome.kind == 'applied' ||
        update.outcome.kind == 'recovered' ||
        statusChanged;
    if (!stateChanged) {
      return;
    }
    final impact = update.impact;
    if (impact.sourceChanged ||
        impact.projectionChanged ||
        impact.fullReplace) {
      _invalidatePendingSource();
    }
    if (impact.fullReplace) {
      _nodeCache.clear();
      _resourceCache.clear();
      _artifactCache.clear();
      _missingNodes.clear();
      _missingResources.clear();
      _missingArtifacts.clear();
    }
    for (final id in impact.changedNodeIds) {
      _nodeCache.remove(id);
      _missingNodes.remove(id);
    }
    for (final id in impact.removedNodeIds) {
      _missingNodes.add(id);
    }
    for (final id in impact.changedResourceIds) {
      _resourceCache.remove(id);
      _missingResources.remove(id);
    }
    for (final id in impact.removedResourceIds) {
      _missingResources.add(id);
    }

    final previousDocument = _currentState.document;
    final incomingDocument = update.document;
    final document =
        update.outcome.kind == 'recovery_required' && previousDocument != null
        ? previousDocument
        : incomingDocument == null
        ? previousDocument
        : incomingDocument.roots != null || previousDocument == null
        ? incomingDocument
        : DocumentSummaryView(
            coordinate: incomingDocument.coordinate,
            lifecycle: incomingDocument.lifecycle,
            projectionCursor: incomingDocument.projectionCursor,
            roots: previousDocument.roots,
          );
    _currentState = MdstreamStateSnapshot(
      status: update.status,
      document: document,
      impact: impact,
    );
  }

  ReducerResult _publicResult(_DecodedReducerOutput output) {
    _expectOnly(output, {
      BindingPayloadKind.reducerUpdate,
      BindingPayloadKind.processorRequest,
      BindingPayloadKind.processorCompletion,
      BindingPayloadKind.artifactChange,
    }, 'reducer transition');
    return ReducerResult(
      updates: List.unmodifiable(output.updates),
      processorRequests: List.unmodifiable(output.processorRequests),
      processorCompletions: List.unmodifiable(output.processorCompletions),
      artifactChanges: List.unmodifiable(output.artifactChanges),
      outputPayloadBytes: output.outputPayloadBytes.toString(),
    );
  }

  void _expectOnly(
    _DecodedReducerOutput output,
    Set<BindingPayloadKind> allowed,
    String operation,
  ) {
    final unexpected = output.payloadKinds.where(
      (kind) => !allowed.contains(kind),
    );
    if (unexpected.isNotEmpty) {
      throw _unexpectedPayload(
        '$operation returned ${unexpected.map((kind) => kind.viewKind).join(', ')}',
      );
    }
  }

  String _count(BindingPayloadKind kind) =>
      (_payloadCounts[kind] ?? 0).toString();

  void _invalidatePendingSource() {
    _pendingSourceCache = null;
    _pendingSourceLoaded = false;
  }
}

MdstreamReducer createNativeReducer(
  NativeReducerHandle handle,
  String schema,
) => MdstreamReducer._(handle, schema);

final class _DecodedReducerOutput {
  final List<BindingPayloadKind> payloadKinds = [];
  final List<CanonicalSnapshotBytes> snapshots = [];
  final List<ReducerUpdateView> updates = [];
  final List<NodeView> nodeViews = [];
  final List<ResourceView> resourceViews = [];
  final List<ProcessorRequestView> processorRequests = [];
  final List<ProcessorCompletionView> processorCompletions = [];
  final List<ArtifactChangeView> artifactChanges = [];
  final List<ArtifactView> artifactViews = [];
  final List<PendingSourceView> pendingSourceViews = [];
  int outputPayloadBytes = 0;
}

final class _BoundedKeySet<T> {
  _BoundedKeySet(this._maximum);

  final int _maximum;
  final LinkedHashSet<T> _values = LinkedHashSet();

  bool contains(T value) => _values.contains(value);

  void add(T value) {
    _values.remove(value);
    _values.add(value);
    while (_values.length > _maximum) {
      _values.remove(_values.first);
    }
  }

  void remove(T value) => _values.remove(value);
  void clear() => _values.clear();
}

const _emptyImpact = ChangeImpactView(
  changedNodeIds: [],
  removedNodeIds: [],
  changedResourceIds: [],
  removedResourceIds: [],
  sourceChanged: false,
  projectionChanged: false,
  lifecycleChanged: false,
  rootsChanged: false,
  fullReplace: false,
);

const _initialState = MdstreamStateSnapshot(
  status: ReducerStatusView(kind: 'uninitialized'),
  document: null,
  impact: _emptyImpact,
);

MdstreamException _unexpectedPayload(String message) => MdstreamException(
  message,
  status: BindingStatus.internalError.value,
  statusName: BindingStatus.internalError.statusName,
  detailCode: 'bindings.unexpected_payload',
);

List<int> _validatedOctets(List<int> bytes) {
  for (final byte in bytes) {
    if (byte < 0 || byte > 255) {
      throw RangeError.range(byte, 0, 255, 'byte');
    }
  }
  return Uint8List.fromList(bytes).toList(growable: false);
}

extension<T> on List<T> {
  T? get firstOrNull => isEmpty ? null : first;
}
