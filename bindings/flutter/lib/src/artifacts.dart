part of '../mdstream_flutter.dart';

/// Stable identity and capabilities of a host-side content processor.
@immutable
final class ContentProcessorDescriptor {
  /// Creates a processor descriptor.
  const ContentProcessorDescriptor({
    required this.id,
    required this.version,
    this.acceptsProvisional = false,
  }) : assert(id != ''),
       assert(version != '');

  /// Stable processor identity.
  final String id;

  /// Processor implementation version.
  final String version;

  /// Whether the implementation can process provisional nodes.
  final bool acceptsProvisional;
}

/// Context supplied to one host-side processor invocation.
final class ProcessorContext {
  const ProcessorContext._(this._cancellation);

  final _ProcessorCancellation _cancellation;

  /// Whether the request has been cancelled or invalidated.
  bool get isCancelled => _cancellation.isCancelled;

  /// Host-provided cancellation reason, when available.
  Object? get cancellationReason => _cancellation.reason;

  /// Completes when the request is cancelled or invalidated.
  Future<Object?> get whenCancelled => _cancellation.whenCancelled;
}

/// Host-side processor matched against typed Content IR nodes.
abstract interface class ContentProcessor {
  /// Stable processor metadata.
  ContentProcessorDescriptor get descriptor;

  /// Version of the host configuration that affects output.
  String get configurationVersion;

  /// Whether this registration opts into provisional processing.
  bool get allowProvisional;

  /// Returns whether this processor handles [node].
  bool matches(ContentNodeView node);

  /// Produces a derived artifact for [request].
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  );
}

/// Output produced by a host-side content processor.
sealed class ProcessorOutput {
  const ProcessorOutput();
}

/// Text artifact produced by a content processor.
final class ProcessorTextOutput extends ProcessorOutput {
  /// Creates a text processor output.
  const ProcessorTextOutput({
    required this.protocol,
    required this.mediaType,
    required this.text,
  });

  /// Versioned artifact protocol.
  final String protocol;

  /// Artifact media type.
  final String mediaType;

  /// Text payload.
  final String text;
}

/// Binary artifact produced by a content processor.
final class ProcessorBinaryOutput extends ProcessorOutput {
  /// Creates a binary processor output with an owned byte copy.
  ProcessorBinaryOutput({
    required this.protocol,
    required this.mediaType,
    required List<int> bytes,
  }) : bytes = List<int>.unmodifiable(bytes);

  /// Versioned artifact protocol.
  final String protocol;

  /// Artifact media type.
  final String mediaType;

  /// Binary payload.
  final List<int> bytes;
}

/// Failure categories accepted by the canonical processor host.
enum ProcessorFailureCode {
  /// Processor-defined failure.
  processor('processor'),

  /// Host-language exception or panic.
  panic('panic'),

  /// Invalid processor request.
  invalidRequest('invalid_request'),

  /// Cancelled processor request.
  cancelled('cancelled'),

  /// Unsupported Content IR node.
  unsupportedContent('unsupported_content'),

  /// Required semantic context was unavailable.
  unresolvedContext('unresolved_context'),

  /// Semantic context was invalid.
  invalidContext('invalid_context'),

  /// Processor or artifact resource limit.
  resourceLimit('resource_limit');

  const ProcessorFailureCode(this.wireName);

  /// Stable wire spelling understood by the Rust processor host.
  final String wireName;
}

/// Structured failure returned intentionally by a content processor.
final class ProcessorFailureOutput extends ProcessorOutput {
  /// Creates a failed processor output.
  const ProcessorFailureOutput({required this.code, required this.message});

  /// Stable failure category.
  final ProcessorFailureCode code;

  /// Human-readable failure message.
  final String message;
}

/// Disposable registration for one content processor.
abstract interface class ProcessorRegistration {
  /// Unregisters the processor and cancels its current leases.
  void dispose();
}

/// Stage at which host-side processor execution failed.
enum ProcessorErrorPhase {
  /// Materializing the changed node view.
  view,

  /// Evaluating the processor matcher.
  matches,

  /// Beginning a native processor lease.
  begin,

  /// Running host processor code.
  process,

  /// Completing a native processor lease.
  complete,

  /// Cancelling a native processor lease.
  cancel,
}

/// Observable host-side processor error.
@immutable
final class ProcessorErrorEvent {
  /// Creates an immutable processor error event.
  const ProcessorErrorEvent({
    required this.phase,
    required this.processorId,
    required this.nodeId,
    required this.requestId,
    required this.error,
    required this.stackTrace,
  });

  /// Processor stage that failed.
  final ProcessorErrorPhase phase;

  /// Stable processor identity.
  final String processorId;

  /// Affected node identity, when known.
  final NodeId? nodeId;

  /// Native request generation, when a lease had begun.
  final RequestGeneration? requestId;

  /// Original Dart or normalized native error.
  final Object error;

  /// Original Dart stack trace.
  final StackTrace stackTrace;
}

/// Readonly derived-artifact views and focused slot notifications.
final class MdstreamArtifacts {
  const MdstreamArtifacts._(this._view, this._listenable);

  final ArtifactView? Function(ArtifactSlot slot) _view;
  final ValueListenable<ArtifactView?> Function(ArtifactSlot slot) _listenable;

  /// Returns the current artifact view for [slot].
  ArtifactView? view(ArtifactSlot slot) => _view(slot);

  /// Returns a stable listenable focused on [slot].
  ValueListenable<ArtifactView?> artifact(ArtifactSlot slot) =>
      _listenable(slot);
}

abstract interface class _ProcessorBackend {
  MdstreamStateView get state;
  bool get isClosed;

  ReducerResult beginProcessor({
    required Epoch expectedEpoch,
    required NodeId nodeId,
    required NodeVersion expectedNodeVersion,
    required String processorId,
    required String processorVersion,
    required String configurationVersion,
    required bool acceptsProvisional,
    required bool allowProvisional,
  });

  ReducerResult completeProcessorText({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required String text,
  });

  ReducerResult completeProcessorBinary({
    required RequestGeneration requestId,
    required String protocol,
    required String mediaType,
    required List<int> bytes,
  });

  ReducerResult failProcessor({
    required RequestGeneration requestId,
    required String code,
    required String message,
  });

  ReducerResult cancelProcessor(RequestGeneration requestId);
}

final class _ProcessorCancellation {
  final Completer<Object?> _completer = Completer<Object?>();
  Object? reason;

  bool get isCancelled => _completer.isCompleted;
  Future<Object?> get whenCancelled => _completer.future;

  void cancel(Object? value) {
    if (isCancelled) {
      return;
    }
    reason = value;
    _completer.complete(value);
  }
}

final class _RegisteredProcessor implements ProcessorRegistration {
  _RegisteredProcessor({
    required this.processor,
    required this.descriptor,
    required this.configurationVersion,
    required this.allowProvisional,
    required void Function(_RegisteredProcessor registration) onDispose,
  }) : _onDispose = onDispose;

  final ContentProcessor processor;
  final ContentProcessorDescriptor descriptor;
  final String configurationVersion;
  final bool allowProvisional;
  final void Function(_RegisteredProcessor registration) _onDispose;
  bool active = true;

  @override
  void dispose() => _onDispose(this);
}

final class _InFlightProcessor {
  const _InFlightProcessor({
    required this.registration,
    required this.request,
    required this.cancellation,
  });

  final _RegisteredProcessor registration;
  final ProcessorRequestView request;
  final _ProcessorCancellation cancellation;
}

final class _ProcessorCandidate {
  _ProcessorCandidate({
    required this.registration,
    required this.expectedEpoch,
    required this.nodeId,
    required this.expectedNodeVersion,
  });

  final _RegisteredProcessor registration;
  final NodeId nodeId;
  Epoch expectedEpoch;
  NodeVersion expectedNodeVersion;
  bool queued = true;
}

final class _CandidateExpectation {
  const _CandidateExpectation({required this.epoch, required this.nodeVersion});

  final Epoch epoch;
  final NodeVersion nodeVersion;
}

enum _BeginDisposition { started, stale, blocked, terminal }

final class _ProcessorScheduler {
  _ProcessorScheduler({required this.backend, required this.onResult});

  static const int _maxDispatchJobs = 32;
  static const int _maxQueuedCandidates = 4096;
  static const int _candidateQueueCompactionFloor = 64;
  static const int _candidateQueueCompactionRatio = 4;

  final _ProcessorBackend backend;
  final void Function(ReducerResult result) onResult;
  final Map<String, _RegisteredProcessor> _processors =
      <String, _RegisteredProcessor>{};
  final Map<RequestGeneration, _InFlightProcessor> _inFlight =
      <RequestGeneration, _InFlightProcessor>{};
  final LinkedHashSet<NodeId> _pendingNodes = LinkedHashSet<NodeId>();
  final LinkedHashSet<_RegisteredProcessor> _pendingRegistrations =
      LinkedHashSet<_RegisteredProcessor>();
  final ListQueue<_ProcessorCandidate> _candidateQueue =
      ListQueue<_ProcessorCandidate>();
  final Map<_RegisteredProcessor, Map<NodeId, _ProcessorCandidate>>
  _candidates = <_RegisteredProcessor, Map<NodeId, _ProcessorCandidate>>{};
  final Map<_RegisteredProcessor, Map<NodeId, _CandidateExpectation>>
  _rejectedCandidates =
      <_RegisteredProcessor, Map<NodeId, _CandidateExpectation>>{};
  final Set<Future<void>> _jobs = <Future<void>>{};
  final _DirectedValueListenable<ProcessorErrorEvent?> errors =
      _DirectedValueListenable<ProcessorErrorEvent?>(null);
  Map<RequestGeneration, Object>? _removedDuringBegin;
  int _beginDepth = 0;
  int _candidateCount = 0;
  bool _dispatching = false;
  bool _scanScheduled = false;
  bool _closed = false;

  ProcessorRegistration register(ContentProcessor processor) {
    _assertOpen();
    final descriptor = processor.descriptor;
    if (descriptor.id.isEmpty || descriptor.version.isEmpty) {
      throw ArgumentError('processor id and version must not be empty');
    }
    final configurationVersion = processor.configurationVersion;
    if (configurationVersion.isEmpty) {
      throw ArgumentError('processor configuration version must not be empty');
    }
    if (_processors.containsKey(descriptor.id)) {
      throw ArgumentError('processor ${descriptor.id} is already registered');
    }
    final registration = _RegisteredProcessor(
      processor: processor,
      descriptor: descriptor,
      configurationVersion: configurationVersion,
      allowProvisional: processor.allowProvisional,
      onDispose: _disposeRegistration,
    );
    _processors[descriptor.id] = registration;
    _pendingRegistrations.add(registration);
    _scheduleScan();
    return registration;
  }

  void handleResults(Iterable<ReducerResult> results) {
    if (_closed) {
      return;
    }
    for (final result in results) {
      for (final change in result.artifactChanges) {
        if (change.change.kind == 'removed') {
          final generation = change.key.generation;
          final reason = change.change.reason ?? 'artifact_removed';
          final entry = _inFlight[generation];
          if (entry == null) {
            if (_beginDepth > 0) {
              (_removedDuringBegin ??=
                      <RequestGeneration, Object>{})[generation] =
                  reason;
            }
          } else {
            entry.cancellation.cancel(reason);
            if (identical(_inFlight[generation], entry)) {
              _inFlight.remove(generation);
            }
          }
        }
      }
      if (_processors.isEmpty) {
        continue;
      }
      for (final update in result.updates) {
        if (update.outcome.kind != 'applied' &&
            update.outcome.kind != 'recovered') {
          continue;
        }
        if (update.impact.fullReplace) {
          _clearCandidates();
          _clearRejectedCandidates();
          _pendingNodes.clear();
          _pendingRegistrations.addAll(
            _processors.values.where((registration) => registration.active),
          );
          continue;
        }
        for (final id in update.impact.changedNodeIds) {
          _removeNodeCandidates(id);
          _pendingNodes.add(id);
        }
        for (final id in update.impact.removedNodeIds) {
          _removeNodeCandidates(id);
          _removeRejectedNode(id);
          _pendingNodes.remove(id);
        }
      }
    }
    _scheduleScan();
    _drainCandidates();
  }

  Future<void> whenIdle() async {
    for (;;) {
      await Future<void>.value();
      _drainCandidates();
      if (_scanScheduled ||
          _pendingNodes.isNotEmpty ||
          _pendingRegistrations.isNotEmpty) {
        continue;
      }
      final jobs = List<Future<void>>.of(_jobs);
      if (jobs.isEmpty) {
        if (_candidateCount == 0) {
          return;
        }
        continue;
      }
      await Future.wait(jobs);
    }
  }

  void close() {
    if (_closed) {
      return;
    }
    _closed = true;
    _scanScheduled = false;
    _pendingNodes.clear();
    _pendingRegistrations.clear();
    _clearCandidates();
    _clearRejectedCandidates();
    for (final registration in _processors.values) {
      registration.active = false;
    }
    _processors.clear();
    for (final entry in List<_InFlightProcessor>.of(_inFlight.values)) {
      entry.cancellation.cancel('controller_disposed');
      _cancel(entry, ProcessorErrorPhase.cancel);
    }
    _inFlight.clear();
    errors.dispose();
  }

  void _scheduleScan() {
    if (_closed ||
        _scanScheduled ||
        (_pendingNodes.isEmpty && _pendingRegistrations.isEmpty)) {
      return;
    }
    _scanScheduled = true;
    scheduleMicrotask(() {
      _scanScheduled = false;
      _scanChangedNodes();
    });
  }

  void _scanChangedNodes() {
    if (_closed) {
      _pendingNodes.clear();
      return;
    }
    final nodeIds = List<NodeId>.of(_pendingNodes);
    _pendingNodes.clear();
    final pendingRegistrations = _pendingRegistrations
        .where((registration) => registration.active)
        .toList(growable: false);
    _pendingRegistrations.clear();
    final pendingSet = pendingRegistrations.toSet();
    final registrations = _processors.values
        .where(
          (registration) =>
              registration.active && !pendingSet.contains(registration),
        )
        .toList(growable: false);
    for (final nodeId in nodeIds) {
      _scanNode(nodeId, registrations);
    }
    if (pendingRegistrations.isNotEmpty) {
      _scanCurrentTree(pendingRegistrations);
    }
    _scheduleScan();
    _drainCandidates();
  }

  void _scanCurrentTree(List<_RegisteredProcessor> registrations) {
    final roots = backend.state.currentState.document?.roots?.children;
    if (roots == null || roots.isEmpty) {
      return;
    }
    final queue = ListQueue<NodeId>.of(roots);
    final visited = <NodeId>{};
    while (queue.isNotEmpty && !_closed) {
      final nodeId = queue.removeFirst();
      if (!visited.add(nodeId)) {
        continue;
      }
      final nodeView = _scanNode(nodeId, registrations);
      if (nodeView != null) {
        queue.addAll(nodeView.node.children.children);
      }
    }
  }

  NodeView? _scanNode(NodeId nodeId, List<_RegisteredProcessor> registrations) {
    if (registrations.isEmpty) {
      return null;
    }
    final expectedEpoch = backend.state.currentState.document?.coordinate.epoch;
    if (expectedEpoch == null) {
      for (final registration in registrations) {
        _removeCandidate(registration, nodeId);
      }
      return null;
    }
    NodeView? nodeView;
    try {
      nodeView = backend.state.nodeView(nodeId);
    } catch (error, stackTrace) {
      for (final registration in registrations) {
        _removeCandidate(registration, nodeId);
        if (registration.active) {
          _emitError(
            phase: ProcessorErrorPhase.view,
            registration: registration,
            nodeId: nodeId,
            error: error,
            stackTrace: stackTrace,
          );
        }
      }
      return null;
    }
    if (nodeView == null) {
      for (final registration in registrations) {
        _removeCandidate(registration, nodeId);
      }
      return null;
    }
    for (final registration in registrations) {
      if (!registration.active) {
        continue;
      }
      final processor = registration.processor;
      if (nodeView.node.stability == 'provisional' &&
          !(registration.descriptor.acceptsProvisional &&
              registration.allowProvisional)) {
        _removeCandidate(registration, nodeId);
        continue;
      }
      bool matches;
      try {
        matches = processor.matches(nodeView.node);
      } catch (error, stackTrace) {
        _removeCandidate(registration, nodeId);
        _emitError(
          phase: ProcessorErrorPhase.matches,
          registration: registration,
          nodeId: nodeId,
          error: error,
          stackTrace: stackTrace,
        );
        continue;
      }
      if (matches && registration.active) {
        _enqueueCandidate(
          registration,
          expectedEpoch,
          nodeId,
          nodeView.node.version,
        );
      } else {
        _removeCandidate(registration, nodeId);
      }
    }
    return nodeView;
  }

  void _enqueueCandidate(
    _RegisteredProcessor registration,
    Epoch expectedEpoch,
    NodeId nodeId,
    NodeVersion expectedNodeVersion, {
    bool front = false,
  }) {
    if (_closed || !registration.active) {
      return;
    }
    final rejected = _rejectedCandidates[registration]?[nodeId];
    if (rejected?.epoch == expectedEpoch &&
        rejected?.nodeVersion == expectedNodeVersion) {
      return;
    }
    final registrationCandidates = _candidates.putIfAbsent(
      registration,
      () => <NodeId, _ProcessorCandidate>{},
    );
    final existing = registrationCandidates[nodeId];
    if (existing != null) {
      existing.expectedEpoch = expectedEpoch;
      existing.expectedNodeVersion = expectedNodeVersion;
      return;
    }
    if (_candidateCount >= _maxQueuedCandidates) {
      _emitError(
        phase: ProcessorErrorPhase.begin,
        registration: registration,
        nodeId: nodeId,
        error: MdstreamException(
          'processor candidate queue limit $_maxQueuedCandidates exceeded',
          status: BindingStatus.resourceLimitExceeded.value,
          statusName: BindingStatus.resourceLimitExceeded.statusName,
          detailCode: 'processor.candidate_queue_limit',
        ),
        stackTrace: StackTrace.current,
      );
      if (registrationCandidates.isEmpty) {
        _candidates.remove(registration);
      }
      return;
    }
    final candidate = _ProcessorCandidate(
      registration: registration,
      expectedEpoch: expectedEpoch,
      nodeId: nodeId,
      expectedNodeVersion: expectedNodeVersion,
    );
    registrationCandidates[nodeId] = candidate;
    if (front) {
      _candidateQueue.addFirst(candidate);
    } else {
      _candidateQueue.addLast(candidate);
    }
    _candidateCount += 1;
  }

  void _removeCandidate(_RegisteredProcessor registration, NodeId nodeId) {
    final registrationCandidates = _candidates[registration];
    final candidate = registrationCandidates?.remove(nodeId);
    if (candidate == null) {
      return;
    }
    candidate.queued = false;
    _candidateCount -= 1;
    if (registrationCandidates!.isEmpty) {
      _candidates.remove(registration);
    }
    _compactCandidateQueue();
  }

  void _removeNodeCandidates(NodeId nodeId) {
    for (final registration in _candidates.keys.toList(growable: false)) {
      _removeCandidate(registration, nodeId);
    }
  }

  void _removeRegistrationCandidates(_RegisteredProcessor registration) {
    final candidates = _candidates.remove(registration);
    if (candidates == null) {
      return;
    }
    for (final candidate in candidates.values) {
      candidate.queued = false;
      _candidateCount -= 1;
    }
    _compactCandidateQueue();
  }

  void _clearCandidates() {
    for (final candidates in _candidates.values) {
      for (final candidate in candidates.values) {
        candidate.queued = false;
      }
    }
    _candidates.clear();
    _candidateQueue.clear();
    _candidateCount = 0;
  }

  void _rejectCandidate(_ProcessorCandidate candidate) {
    final rejected = _rejectedCandidates.putIfAbsent(
      candidate.registration,
      () => <NodeId, _CandidateExpectation>{},
    );
    rejected[candidate.nodeId] = _CandidateExpectation(
      epoch: candidate.expectedEpoch,
      nodeVersion: candidate.expectedNodeVersion,
    );
  }

  void _removeRejectedNode(NodeId nodeId) {
    for (final registration in _rejectedCandidates.keys.toList(
      growable: false,
    )) {
      final rejected = _rejectedCandidates[registration]!;
      rejected.remove(nodeId);
      if (rejected.isEmpty) {
        _rejectedCandidates.remove(registration);
      }
    }
  }

  void _removeRejectedCandidate(
    _RegisteredProcessor registration,
    NodeId nodeId,
  ) {
    final rejected = _rejectedCandidates[registration];
    rejected?.remove(nodeId);
    if (rejected?.isEmpty ?? false) {
      _rejectedCandidates.remove(registration);
    }
  }

  void _clearRejectedCandidates() => _rejectedCandidates.clear();

  _ProcessorCandidate? _takeCandidate() {
    while (_candidateQueue.isNotEmpty) {
      final candidate = _candidateQueue.removeFirst();
      if (!candidate.queued) {
        continue;
      }
      candidate.queued = false;
      final registrationCandidates = _candidates[candidate.registration];
      registrationCandidates?.remove(candidate.nodeId);
      if (registrationCandidates?.isEmpty ?? false) {
        _candidates.remove(candidate.registration);
      }
      _candidateCount -= 1;
      _compactCandidateQueue();
      return candidate;
    }
    _compactCandidateQueue();
    return null;
  }

  void _compactCandidateQueue() {
    if (_candidateCount == 0) {
      _candidateQueue.clear();
      return;
    }
    if (_candidateQueue.length <= _candidateQueueCompactionFloor ||
        _candidateQueue.length <=
            _candidateCount * _candidateQueueCompactionRatio) {
      return;
    }
    final retained = _candidateQueue
        .where((candidate) => candidate.queued)
        .toList(growable: false);
    _candidateQueue
      ..clear()
      ..addAll(retained);
  }

  void _drainCandidates() {
    if (_dispatching || _closed) {
      return;
    }
    _dispatching = true;
    try {
      while (_candidateCount > 0 && _inFlight.length < _maxDispatchJobs) {
        final candidate = _takeCandidate();
        if (candidate == null) {
          break;
        }
        if (!candidate.registration.active) {
          continue;
        }
        if (_begin(candidate) == _BeginDisposition.blocked) {
          break;
        }
      }
    } finally {
      _dispatching = false;
    }
  }

  _BeginDisposition _begin(_ProcessorCandidate candidate) {
    final registration = candidate.registration;
    final expectedEpoch = candidate.expectedEpoch;
    final nodeId = candidate.nodeId;
    final expectedNodeVersion = candidate.expectedNodeVersion;
    final processor = registration.processor;
    ProcessorRequestView request;
    final parentRemovals = _removedDuringBegin;
    Map<RequestGeneration, Object>? removals;
    _removedDuringBegin = null;
    _beginDepth += 1;
    try {
      final result = backend.beginProcessor(
        expectedEpoch: expectedEpoch,
        nodeId: nodeId,
        expectedNodeVersion: expectedNodeVersion,
        processorId: registration.descriptor.id,
        processorVersion: registration.descriptor.version,
        configurationVersion: registration.configurationVersion,
        acceptsProvisional: registration.descriptor.acceptsProvisional,
        allowProvisional: registration.allowProvisional,
      );
      onResult(result);
      if (result.processorRequests.isEmpty) {
        _rejectCandidate(candidate);
        _pendingNodes.add(nodeId);
        _scheduleScan();
        return _BeginDisposition.stale;
      }
      if (result.processorRequests.length != 1) {
        throw StateError('native processor host returned no unique request');
      }
      request = result.processorRequests.single;
      _removeRejectedCandidate(registration, nodeId);
    } catch (error, stackTrace) {
      final normalized = MdstreamException.fromObject(error);
      if (normalized.status == BindingStatus.resourceLimitExceeded.value &&
          _inFlight.isNotEmpty) {
        _enqueueCandidate(
          registration,
          expectedEpoch,
          nodeId,
          expectedNodeVersion,
          front: true,
        );
        return _BeginDisposition.blocked;
      }
      _emitError(
        phase: ProcessorErrorPhase.begin,
        registration: registration,
        nodeId: nodeId,
        error: normalized,
        stackTrace: stackTrace,
      );
      return _BeginDisposition.terminal;
    } finally {
      removals = _removedDuringBegin;
      _beginDepth -= 1;
      _removedDuringBegin = parentRemovals;
      if (_beginDepth > 0 && removals != null) {
        (_removedDuringBegin ??= <RequestGeneration, Object>{}).addAll(
          removals,
        );
      }
    }

    if (removals?.containsKey(request.requestId) ?? false) {
      return _BeginDisposition.stale;
    }
    final entry = _InFlightProcessor(
      registration: registration,
      request: request,
      cancellation: _ProcessorCancellation(),
    );
    if (_closed || !registration.active || backend.isClosed) {
      entry.cancellation.cancel('processor_inactive');
      _cancel(entry, ProcessorErrorPhase.cancel);
      return _BeginDisposition.terminal;
    }
    _inFlight[request.requestId] = entry;
    late final Future<void> job;
    job =
        Future<ProcessorOutput>.sync(
              () => processor.process(
                request,
                ProcessorContext._(entry.cancellation),
              ),
            )
            .then(
              (output) => _complete(entry, output),
              onError: (Object error, StackTrace stackTrace) =>
                  _processorFailed(entry, error, stackTrace),
            )
            .whenComplete(() {
              if (identical(_inFlight[request.requestId], entry)) {
                _inFlight.remove(request.requestId);
              }
              _jobs.remove(job);
              _drainCandidates();
              _scheduleScan();
            });
    _jobs.add(job);
    return _BeginDisposition.started;
  }

  void _complete(_InFlightProcessor entry, ProcessorOutput output) {
    if (!_isCurrent(entry)) {
      return;
    }
    try {
      final result = switch (output) {
        ProcessorTextOutput(:final protocol, :final mediaType, :final text) =>
          backend.completeProcessorText(
            requestId: entry.request.requestId,
            protocol: protocol,
            mediaType: mediaType,
            text: text,
          ),
        ProcessorBinaryOutput(
          :final protocol,
          :final mediaType,
          :final bytes,
        ) =>
          backend.completeProcessorBinary(
            requestId: entry.request.requestId,
            protocol: protocol,
            mediaType: mediaType,
            bytes: bytes,
          ),
        ProcessorFailureOutput(:final code, :final message) =>
          backend.failProcessor(
            requestId: entry.request.requestId,
            code: code.wireName,
            message: message,
          ),
      };
      onResult(result);
    } catch (error, stackTrace) {
      _emitError(
        phase: ProcessorErrorPhase.complete,
        registration: entry.registration,
        nodeId: entry.request.key.nodeId,
        requestId: entry.request.requestId,
        error: error,
        stackTrace: stackTrace,
      );
      if (_isCurrent(entry)) {
        _cancel(entry, ProcessorErrorPhase.cancel);
      }
    }
  }

  void _processorFailed(
    _InFlightProcessor entry,
    Object error,
    StackTrace stackTrace,
  ) {
    if (!_isCurrent(entry)) {
      return;
    }
    _emitError(
      phase: ProcessorErrorPhase.process,
      registration: entry.registration,
      nodeId: entry.request.key.nodeId,
      requestId: entry.request.requestId,
      error: error,
      stackTrace: stackTrace,
    );
    if (!_isCurrent(entry)) {
      return;
    }
    try {
      onResult(
        backend.failProcessor(
          requestId: entry.request.requestId,
          code: entry.cancellation.isCancelled ? 'cancelled' : 'panic',
          message: error.toString(),
        ),
      );
    } catch (completionError, completionStackTrace) {
      _emitError(
        phase: ProcessorErrorPhase.complete,
        registration: entry.registration,
        nodeId: entry.request.key.nodeId,
        requestId: entry.request.requestId,
        error: completionError,
        stackTrace: completionStackTrace,
      );
      if (_isCurrent(entry)) {
        _cancel(entry, ProcessorErrorPhase.cancel);
      }
    }
  }

  bool _isCurrent(_InFlightProcessor entry) =>
      !_closed &&
      entry.registration.active &&
      identical(_inFlight[entry.request.requestId], entry);

  void _disposeRegistration(_RegisteredProcessor registration) {
    if (!registration.active) {
      return;
    }
    registration.active = false;
    _processors.remove(registration.descriptor.id);
    _pendingRegistrations.remove(registration);
    _removeRegistrationCandidates(registration);
    _rejectedCandidates.remove(registration);
    for (final entry in List<_InFlightProcessor>.of(_inFlight.values)) {
      if (identical(entry.registration, registration)) {
        entry.cancellation.cancel('processor_unregistered');
        _cancel(entry, ProcessorErrorPhase.cancel);
        _inFlight.remove(entry.request.requestId);
      }
    }
    _drainCandidates();
  }

  void _cancel(_InFlightProcessor entry, ProcessorErrorPhase phase) {
    if (backend.isClosed) {
      return;
    }
    try {
      onResult(backend.cancelProcessor(entry.request.requestId));
    } catch (error, stackTrace) {
      if (!_closed) {
        _emitError(
          phase: phase,
          registration: entry.registration,
          nodeId: entry.request.key.nodeId,
          requestId: entry.request.requestId,
          error: error,
          stackTrace: stackTrace,
        );
      }
    }
  }

  void _emitError({
    required ProcessorErrorPhase phase,
    required _RegisteredProcessor registration,
    required NodeId? nodeId,
    RequestGeneration? requestId,
    required Object error,
    required StackTrace stackTrace,
  }) {
    if (_closed) {
      return;
    }
    errors.update(
      ProcessorErrorEvent(
        phase: phase,
        processorId: registration.descriptor.id,
        nodeId: nodeId,
        requestId: requestId,
        error: error,
        stackTrace: stackTrace,
      ),
      force: true,
    );
  }

  void _assertOpen() {
    if (_closed) {
      throw StateError('mdstream processor scheduler is closed');
    }
  }
}
