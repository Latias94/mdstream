part of '../mdstream_flutter.dart';

/// Flutter key for one canonical node identity within a continuity generation.
@immutable
final class MdstreamNodeKey extends LocalKey {
  /// Creates a continuity- and epoch-qualified node key.
  const MdstreamNodeKey({
    required this.continuityGeneration,
    required this.epoch,
    required this.nodeId,
  });

  /// Controller-local generation advanced by every full replacement.
  final int continuityGeneration;

  /// Document epoch that owns the node.
  final Epoch epoch;

  /// Stable node identity within [epoch].
  final NodeId nodeId;

  @override
  bool operator ==(Object other) =>
      other is MdstreamNodeKey &&
      continuityGeneration == other.continuityGeneration &&
      epoch == other.epoch &&
      nodeId == other.nodeId;

  @override
  int get hashCode => Object.hash(continuityGeneration, epoch, nodeId);

  @override
  String toString() => 'MdstreamNodeKey($continuityGeneration/$epoch/$nodeId)';
}

abstract interface class _ControllerBackend implements _ProcessorBackend {
  void close();
}

final class _GuardedProcessorRegistration implements ProcessorRegistration {
  const _GuardedProcessorRegistration(this._delegate, this._guard);

  final ProcessorRegistration _delegate;
  final VoidCallback _guard;

  @override
  void dispose() {
    _guard();
    _delegate.dispose();
  }
}

final class _EngineBackend implements _ControllerBackend {
  const _EngineBackend(this.engine);

  final MdstreamEngine engine;

  @override
  bool get isClosed => engine.isClosed;

  @override
  MdstreamStateView get state => engine.state;

  @override
  ReducerResult beginProcessor({
    required Epoch expectedEpoch,
    required NodeId nodeId,
    required NodeVersion expectedNodeVersion,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    required bool acceptsProvisional,
    required bool allowProvisional,
  }) => engine.beginProcessorIfCurrent(
    expectedEpoch: expectedEpoch,
    nodeId: nodeId,
    expectedNodeVersion: expectedNodeVersion,
    processorId: processorId,
    processorVersion: processorVersion,
    configurationVersion: configurationVersion,
    acceptsProvisional: acceptsProvisional,
    allowProvisional: allowProvisional,
  );

  @override
  ReducerResult completeProcessorText({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required String text,
  }) => engine.completeProcessorText(
    requestId: requestId,
    protocol: protocol,
    mediaType: mediaType,
    text: text,
  );

  @override
  ReducerResult completeProcessorBinary({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required List<int> bytes,
  }) => engine.completeProcessorBinary(
    requestId: requestId,
    protocol: protocol,
    mediaType: mediaType,
    bytes: bytes,
  );

  @override
  ReducerResult failProcessor({
    required RequestGeneration requestId,
    required String code,
    required String message,
  }) =>
      engine.failProcessor(requestId: requestId, code: code, message: message);

  @override
  ReducerResult cancelProcessor(RequestGeneration requestId) =>
      engine.cancelProcessor(requestId);

  @override
  void close() => engine.close();
}

final class _ReducerBackend implements _ControllerBackend {
  const _ReducerBackend(this.reducer);

  final MdstreamReducer reducer;

  @override
  bool get isClosed => reducer.isClosed;

  @override
  MdstreamStateView get state => reducer.state;

  @override
  ReducerResult beginProcessor({
    required Epoch expectedEpoch,
    required NodeId nodeId,
    required NodeVersion expectedNodeVersion,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    required bool acceptsProvisional,
    required bool allowProvisional,
  }) => reducer.beginProcessorIfCurrent(
    expectedEpoch: expectedEpoch,
    nodeId: nodeId,
    expectedNodeVersion: expectedNodeVersion,
    processorId: processorId,
    processorVersion: processorVersion,
    configurationVersion: configurationVersion,
    acceptsProvisional: acceptsProvisional,
    allowProvisional: allowProvisional,
  );

  @override
  ReducerResult completeProcessorText({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required String text,
  }) => reducer.completeProcessorText(
    requestId: requestId,
    protocol: protocol,
    mediaType: mediaType,
    text: text,
  );

  @override
  ReducerResult completeProcessorBinary({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required List<int> bytes,
  }) => reducer.completeProcessorBinary(
    requestId: requestId,
    protocol: protocol,
    mediaType: mediaType,
    bytes: bytes,
  );

  @override
  ReducerResult failProcessor({
    required RequestGeneration requestId,
    required String code,
    required String message,
  }) =>
      reducer.failProcessor(requestId: requestId, code: code, message: message);

  @override
  ReducerResult cancelProcessor(RequestGeneration requestId) =>
      reducer.cancelProcessor(requestId);

  @override
  void close() => reducer.close();
}

abstract class _MdstreamControllerBase extends ChangeNotifier
    implements ValueListenable<MdstreamControllerState> {
  _MdstreamControllerBase(this._backend, {required bool captureTransitions})
    : _value = MdstreamControllerState(
        snapshot: _backend.state.currentState,
        impact: MdstreamNotificationImpact._empty,
        lastError: null,
      ),
      _captureTransitions = captureTransitions,
      _transitions = _DirectedValueListenable<MdstreamTransitionBatch>(
        MdstreamTransitionBatch._initial,
      ) {
    artifacts = MdstreamArtifacts._(artifactView, artifact);
    _processors = _ProcessorScheduler(
      backend: _backend,
      onResult: _consumeProcessorResult,
    );
  }

  final _ControllerBackend _backend;
  final bool _captureTransitions;
  final _DirectedValueListenable<MdstreamTransitionBatch> _transitions;
  MdstreamControllerState _value;
  bool _disposed = false;
  int _activeNotificationDepth = 0;
  int _activeTransitionNotificationDepth = 0;
  int _transitionRevision = 0;
  int _continuityGeneration = 0;

  final Map<NodeId, _DirectedValueListenable<NodeView?>> _nodes =
      <NodeId, _DirectedValueListenable<NodeView?>>{};
  final Map<ResourceId, _DirectedValueListenable<ResourceView?>> _resources =
      <ResourceId, _DirectedValueListenable<ResourceView?>>{};
  final Map<ArtifactSlot, _DirectedValueListenable<ArtifactView?>> _artifacts =
      <ArtifactSlot, _DirectedValueListenable<ArtifactView?>>{};
  _DirectedValueListenable<PendingSourceView?>? _pendingSource;
  final Map<MdstreamNodeKey, MdstreamNodeKey> _nodeKeys =
      <MdstreamNodeKey, MdstreamNodeKey>{};

  late final _ProcessorScheduler _processors;

  /// Readonly derived artifacts and focused artifact notifications.
  late final MdstreamArtifacts artifacts;

  @override
  MdstreamControllerState get value => _value;

  /// Ordered transition facts for the latest public operation.
  ///
  /// The revision stays at zero and this listenable remains silent when the
  /// session was created without transition capture.
  ValueListenable<MdstreamTransitionBatch> get transitions => _transitions;

  /// Most recent host-side processor failure.
  ValueListenable<ProcessorErrorEvent?> get processorErrors =>
      _processors.errors;

  /// Returns the current source suffix not yet represented by Content IR.
  PendingSourceView? pendingSourceView() {
    _assertOpen();
    return _backend.state.pendingSourceView();
  }

  /// Returns a stable listenable for the on-demand pending source view.
  ValueListenable<PendingSourceView?> get pendingSource {
    _assertOpen();
    return _pendingSource ??= _DirectedValueListenable<PendingSourceView?>(
      _backend.state.pendingSourceView(),
    );
  }

  /// Returns the current materialized node view.
  NodeView? nodeView(NodeId id) {
    _assertOpen();
    return _backend.state.nodeView(id);
  }

  /// Returns a stable listenable focused on [id].
  ValueListenable<NodeView?> node(NodeId id) {
    _assertOpen();
    return _nodes.putIfAbsent(
      id,
      () => _DirectedValueListenable<NodeView?>(_backend.state.nodeView(id)),
    );
  }

  /// Returns the continuity- and epoch-qualified Flutter key for [id].
  MdstreamNodeKey nodeKey(NodeId id) {
    _assertOpen();
    final epoch = _value.document?.coordinate.epoch;
    if (epoch == null) {
      throw StateError('mdstream document is not initialized');
    }
    final candidate = MdstreamNodeKey(
      continuityGeneration: _continuityGeneration,
      epoch: epoch,
      nodeId: id,
    );
    return _nodeKeys.putIfAbsent(candidate, () => candidate);
  }

  /// Returns the current materialized semantic resource view.
  ResourceView? resourceView(ResourceId id) {
    _assertOpen();
    return _backend.state.resourceView(id);
  }

  /// Returns a stable listenable focused on a semantic resource.
  ValueListenable<ResourceView?> resource(ResourceId id) {
    _assertOpen();
    return _resources.putIfAbsent(
      id,
      () => _DirectedValueListenable<ResourceView?>(
        _backend.state.resourceView(id),
      ),
    );
  }

  /// Returns the current materialized artifact view.
  ArtifactView? artifactView(ArtifactSlot slot) {
    _assertOpen();
    return _backend.state.artifactView(slot);
  }

  /// Returns a stable listenable focused on an artifact slot.
  ValueListenable<ArtifactView?> artifact(ArtifactSlot slot) {
    _assertOpen();
    return _artifacts.putIfAbsent(
      slot,
      () => _DirectedValueListenable<ArtifactView?>(
        _backend.state.artifactView(slot),
      ),
    );
  }

  /// Registers a host-side content processor.
  ProcessorRegistration registerProcessor(ContentProcessor processor) {
    _assertMutationAllowed();
    return _GuardedProcessorRegistration(
      _processors.register(processor),
      _assertNoTransitionReentry,
    );
  }

  /// Completes after all scheduled processor scans and jobs settle.
  Future<void> whenProcessorsIdle() => _processors.whenIdle();

  T runTransition<T>(
    MdstreamControllerErrorPhase phase,
    T Function() operation,
    Iterable<ReducerResult> Function(T result) results,
  ) {
    _assertMutationAllowed();
    try {
      final result = operation();
      _publish(results(result));
      return result;
    } catch (error, stackTrace) {
      _recordError(phase, error, stackTrace);
      Error.throwWithStackTrace(error, stackTrace);
    }
  }

  void _publish(Iterable<ReducerResult> incoming) {
    if (_disposed) {
      return;
    }
    final results = List<ReducerResult>.of(incoming);
    final previousSnapshot = _value.snapshot;
    final nextSnapshot = _backend.state.currentState;
    final impactBuilder = _ImpactBuilder();
    final artifactSlots = <ArtifactSlot>{};
    List<TransitionFactsView>? transitionFacts;
    for (final result in results) {
      for (final update in result.updates) {
        impactBuilder.add(update.impact);
        if (update.impact.fullReplace) {
          _continuityGeneration += 1;
        }
        final transition = update.transition;
        if (_captureTransitions && transition != null) {
          (transitionFacts ??= <TransitionFactsView>[]).add(transition.facts);
        }
      }
      for (final change in result.artifactChanges) {
        artifactSlots.add(
          ArtifactSlot(
            epoch: change.key.epoch,
            nodeId: change.key.nodeId,
            processorId: change.key.processorId,
          ),
        );
      }
    }
    final impact = impactBuilder.build();
    final canonicalChanged = !identical(previousSnapshot, nextSnapshot);
    final errorCleared = _value.lastError != null;
    final controllerChanged = canonicalChanged || errorCleared;
    if (controllerChanged) {
      _value = MdstreamControllerState(
        snapshot: nextSnapshot,
        impact: impact,
        lastError: null,
      );
    }
    final focusedNotifications = _prepareFocusedViews(impact, artifactSlots);
    _publishTransition(transitionFacts ?? const <TransitionFactsView>[]);
    if (_disposed) {
      return;
    }
    for (final notify in focusedNotifications) {
      if (_disposed) {
        return;
      }
      notify();
    }
    if (controllerChanged) {
      _notifyControllerListeners();
      if (_disposed) {
        return;
      }
    }
    _processors.handleResults(results);
  }

  List<VoidCallback> _prepareFocusedViews(
    MdstreamNotificationImpact impact,
    Set<ArtifactSlot> artifactSlots,
  ) {
    final notifications = <VoidCallback>[];
    if (impact.fullReplace) {
      _nodeKeys.clear();
    }
    final pendingSource = _pendingSource;
    if (pendingSource != null &&
        (impact.sourceChanged ||
            impact.projectionChanged ||
            impact.fullReplace) &&
        pendingSource.replace(
          _backend.state.pendingSourceView(),
          force: impact.fullReplace,
        )) {
      notifications.add(pendingSource.emit);
    }

    final nodeIds = impact.fullReplace
        ? _nodes.keys.toList(growable: false)
        : impact.changedNodeIds;
    for (final id in nodeIds) {
      final listenable = _nodes[id];
      if (listenable != null &&
          listenable.replace(
            _backend.state.nodeView(id),
            force: impact.fullReplace,
          )) {
        notifications.add(listenable.emit);
      }
    }

    final resourceIds = impact.fullReplace
        ? _resources.keys.toList(growable: false)
        : impact.changedResourceIds;
    for (final id in resourceIds) {
      final listenable = _resources[id];
      if (listenable != null &&
          listenable.replace(
            _backend.state.resourceView(id),
            force: impact.fullReplace,
          )) {
        notifications.add(listenable.emit);
      }
    }

    final slots = impact.fullReplace
        ? _artifacts.keys.toList(growable: false)
        : artifactSlots;
    for (final slot in slots) {
      final listenable = _artifacts[slot];
      if (listenable != null &&
          listenable.replace(
            _backend.state.artifactView(slot),
            force: impact.fullReplace,
          )) {
        notifications.add(listenable.emit);
      }
    }
    return notifications;
  }

  void _consumeProcessorResult(ReducerResult result) {
    if (!_disposed) {
      _publish(<ReducerResult>[result]);
    }
  }

  void _recordError(
    MdstreamControllerErrorPhase phase,
    Object error,
    StackTrace stackTrace,
  ) {
    if (_disposed) {
      return;
    }
    _value = MdstreamControllerState(
      snapshot: _value.snapshot,
      impact: MdstreamNotificationImpact._empty,
      lastError: MdstreamControllerError(
        phase: phase,
        error: MdstreamException.fromObject(error),
        stackTrace: stackTrace,
      ),
    );
    _publishTransition(const <TransitionFactsView>[]);
    if (_disposed) {
      return;
    }
    _notifyControllerListeners();
  }

  void _publishTransition(Iterable<TransitionFactsView> facts) {
    if (!_captureTransitions || _disposed) {
      return;
    }
    _transitionRevision += 1;
    _transitions.replace(
      MdstreamTransitionBatch._(revision: _transitionRevision, facts: facts),
      force: true,
    );
    _activeTransitionNotificationDepth += 1;
    try {
      _transitions.emit();
    } finally {
      _activeTransitionNotificationDepth -= 1;
    }
  }

  void _notifyControllerListeners() {
    if (_disposed) {
      return;
    }
    _activeNotificationDepth += 1;
    try {
      notifyListeners();
    } finally {
      _activeNotificationDepth -= 1;
      if (_disposed && _activeNotificationDepth == 0) {
        super.dispose();
      }
    }
  }

  void _assertOpen() {
    if (_disposed || _backend.isClosed) {
      throw StateError('mdstream controller is disposed');
    }
  }

  void _assertMutationAllowed() {
    _assertOpen();
    _assertNoTransitionReentry();
  }

  void _assertNoTransitionReentry() {
    if (_activeTransitionNotificationDepth > 0) {
      throw StateError(
        'mdstream mutation is not allowed during a transition notification',
      );
    }
  }

  @override
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    _processors.close();
    _backend.close();
    _transitions.dispose();
    _pendingSource?.dispose();
    _pendingSource = null;
    for (final listenable in _nodes.values) {
      listenable.dispose();
    }
    for (final listenable in _resources.values) {
      listenable.dispose();
    }
    for (final listenable in _artifacts.values) {
      listenable.dispose();
    }
    _nodes.clear();
    _resources.clear();
    _artifacts.clear();
    _nodeKeys.clear();
    if (_activeNotificationDepth == 0) {
      super.dispose();
    }
  }
}

/// Local streaming producer with Flutter state notifications.
final class MdstreamController extends _MdstreamControllerBase {
  MdstreamController._(this._engine, {required bool captureTransitions})
    : super(_EngineBackend(_engine), captureTransitions: captureTransitions);

  final MdstreamEngine _engine;

  /// Opens the native library bundled by the Flutter plugin.
  factory MdstreamController.open({MdstreamSessionOptions? options}) =>
      MdstreamController.fromRuntime(
        MdstreamFlutterRuntime.open(),
        options: options,
      );

  /// Creates a local stream from an already validated runtime.
  factory MdstreamController.fromRuntime(
    MdstreamRuntime runtime, {
    MdstreamSessionOptions? options,
  }) => MdstreamController._(
    runtime.createEngine(options: options),
    captureTransitions: options?.captureTransitions ?? false,
  );

  /// Appends one source chunk.
  EngineResult append(String chunk) => runTransition(
    MdstreamControllerErrorPhase.append,
    () => _engine.append(chunk),
    (result) => result.reducerResults,
  );

  /// Finalizes the stream once; repeated calls are no-ops.
  EngineResult finish() => runTransition(
    MdstreamControllerErrorPhase.finish,
    _engine.finish,
    (result) => result.reducerResults,
  );

  /// Starts a new document epoch.
  EngineResult reset() => runTransition(
    MdstreamControllerErrorPhase.reset,
    _engine.reset,
    (result) => result.reducerResults,
  );

  /// Creates an explicit canonical recovery snapshot.
  CanonicalSnapshotBytes? createRecoverySnapshot() => runTransition(
    MdstreamControllerErrorPhase.createRecoverySnapshot,
    _engine.createRecoverySnapshot,
    (_) => const <ReducerResult>[],
  );
}

/// Canonical replica with explicit gap/fork snapshot recovery.
final class MdstreamReplicaController extends _MdstreamControllerBase {
  MdstreamReplicaController._(this._reducer, {required bool captureTransitions})
    : super(_ReducerBackend(_reducer), captureTransitions: captureTransitions);

  final MdstreamReducer _reducer;

  /// Opens the bundled runtime and creates an independent replica.
  factory MdstreamReplicaController.open({MdstreamSessionOptions? options}) =>
      MdstreamReplicaController.fromRuntime(
        MdstreamFlutterRuntime.open(),
        options: options,
      );

  /// Creates a replica from an already validated runtime.
  factory MdstreamReplicaController.fromRuntime(
    MdstreamRuntime runtime, {
    MdstreamSessionOptions? options,
  }) => MdstreamReplicaController._(
    runtime.createReducer(options: options),
    captureTransitions: options?.captureTransitions ?? false,
  );

  /// Applies one canonical change.
  ReducerResult applyChange(CanonicalChangeBytes change) => runTransition(
    MdstreamControllerErrorPhase.applyChange,
    () => _reducer.applyChange(change),
    (result) => <ReducerResult>[result],
  );

  /// Atomically recovers the replica from a canonical snapshot.
  ReducerResult recoverSnapshot(CanonicalSnapshotBytes snapshot) =>
      runTransition(
        MdstreamControllerErrorPhase.recoverSnapshot,
        () => _reducer.recoverSnapshot(snapshot),
        (result) => <ReducerResult>[result],
      );

  /// Creates an explicit canonical recovery snapshot.
  CanonicalSnapshotBytes? createRecoverySnapshot() => runTransition(
    MdstreamControllerErrorPhase.createRecoverySnapshot,
    _reducer.createRecoverySnapshot,
    (_) => const <ReducerResult>[],
  );
}
