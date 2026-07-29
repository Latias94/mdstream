part of '../mdstream_flutter.dart';

/// Stable identity and capabilities of a host-side content processor.
@immutable
final class ContentProcessorDescriptor {
  /// Creates a processor descriptor.
  const ContentProcessorDescriptor({
    required this.id,
    required this.version,
    this.acceptsProvisional = false,
  });

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
  MdstreamProcessorSchedulerLimits get processorSchedulerLimits;
  bool get isClosed;

  ReducerResult beginProcessor({
    required Epoch expectedEpoch,
    required NodeId nodeId,
    required ProcessorInputVersion expectedInputVersion,
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
    required this.expectedInputVersion,
  });

  final _RegisteredProcessor registration;
  final NodeId nodeId;
  Epoch expectedEpoch;
  ProcessorInputVersion expectedInputVersion;
  bool queued = true;
}

final class _CandidateExpectation {
  const _CandidateExpectation({
    required this.epoch,
    required this.inputVersion,
  });

  final Epoch epoch;
  final ProcessorInputVersion inputVersion;
}

enum _ProcessorScanPhase { changed, tree, fallback, done }

final class _ProcessorScanWork {
  _ProcessorScanWork({
    required this.nodeIds,
    required this.registrations,
    required this.pendingRegistrations,
  });

  final List<NodeId> nodeIds;
  List<_RegisteredProcessor> registrations;
  List<_RegisteredProcessor> pendingRegistrations;
  final ListQueue<NodeId> treeQueue = ListQueue<NodeId>();
  final Set<NodeId> visited = <NodeId>{};
  _ProcessorNodeScan? nodeScan;
  _ProcessorScanPhase phase = _ProcessorScanPhase.changed;
  int nodeIndex = 0;
  int fallbackIndex = 0;
  bool treeInitialized = false;
}

final class _ProcessorNodeScan {
  _ProcessorNodeScan({
    required this.nodeId,
    required this.registrations,
    required this.expectedEpoch,
    required this.nodeView,
    required this.viewFailed,
    required this.viewError,
    required this.viewStackTrace,
  });

  final NodeId nodeId;
  List<_RegisteredProcessor> registrations;
  final Epoch? expectedEpoch;
  final NodeView? nodeView;
  final bool viewFailed;
  final Object? viewError;
  final StackTrace? viewStackTrace;
  int registrationIndex = 0;
}

final class _ProcessorNodeScanStep {
  const _ProcessorNodeScanStep({
    required this.complete,
    required this.nodeView,
    required this.blocked,
  });

  final bool complete;
  final NodeView? nodeView;
  final bool blocked;
}

enum _BeginDisposition { started, stale, blocked, terminal }

final RegExp _processorIdentifierPattern = RegExp(r'^[A-Za-z0-9._:+-]{1,128}$');

void _validateProcessorIdentifier(String value, String field) {
  if (!_processorIdentifierPattern.hasMatch(value)) {
    throw ArgumentError.value(
      value,
      field,
      "must be 1-128 ASCII bytes using letters, digits, '.', '_', ':', '+', or '-'",
    );
  }
}

final class _ProcessorScheduler {
  _ProcessorScheduler({
    required this.backend,
    required this.onResult,
    required MdstreamProcessorSchedulerLimits limits,
  }) : _maxDispatchJobs = limits.maxInFlightJobs,
       _maxQueuedCandidates = limits.maxQueuedCandidates;

  static const int _candidateQueueCompactionFloor = 64;
  static const int _candidateQueueCompactionRatio = 4;
  static const int _dispatchQuantum = 32;
  static const int _scanQuantum = 64;
  static const Set<String> _retryableResourceLimitDetailCodes = <String>{
    'processor.resource_limit.in_flight_jobs',
    'processor.resource_limit.in_flight_input_bytes',
  };

  final _ProcessorBackend backend;
  final void Function(ReducerResult result) onResult;
  final int _maxDispatchJobs;
  final int _maxQueuedCandidates;
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
  int _dispatchRevision = 0;
  int? _scheduledDispatchRevision;
  int _scanRevision = 0;
  int? _scheduledScanRevision;
  int? _scheduledScanContinuationRevision;
  _ProcessorScanWork? _scanWork;
  Completer<void>? _scanUnblocked;
  bool _dispatching = false;
  bool _dispatchBlocked = false;
  bool _scanBlocked = false;
  bool _candidateQueueSaturated = false;
  bool _closed = false;

  ProcessorRegistration register(ContentProcessor processor) {
    _assertOpen();
    final descriptor = processor.descriptor;
    _validateProcessorIdentifier(descriptor.id, 'processor.id');
    _validateProcessorIdentifier(descriptor.version, 'processor.version');
    final configurationVersion = processor.configurationVersion;
    _validateProcessorIdentifier(
      configurationVersion,
      'processor.configuration_version',
    );
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
    var capacityChanged = false;
    for (final result in results) {
      for (final change in result.artifactChanges) {
        final removal = change.change;
        if (removal is RemovedArtifactChangeView) {
          final generation = change.key.generation;
          final reason = removal.reason;
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
              _dispatchBlocked = false;
              capacityChanged = true;
            }
          }
        }
      }
      if (_processors.isEmpty) {
        continue;
      }
      for (final update in result.updates) {
        if (update.outcome is! AppliedOutcomeView &&
            update.outcome is! RecoveredOutcomeView) {
          continue;
        }
        if (update.impact.fullReplace) {
          _invalidateScheduledDispatch();
          _invalidateScheduledScan();
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
    if (capacityChanged) {
      _scheduleDispatch();
    } else {
      _drainCandidates();
    }
  }

  Future<void> whenIdle() async {
    for (;;) {
      final scanUnblocked = _scanUnblocked;
      if (_scanBlocked && scanUnblocked != null) {
        await scanUnblocked.future;
        continue;
      }
      if (_scheduledDispatchRevision != null ||
          _scheduledScanRevision != null) {
        await Future<void>.delayed(Duration.zero);
      } else {
        await Future<void>.value();
      }
      _drainCandidates();
      if (_scheduledScanRevision != null ||
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
    _invalidateScheduledDispatch();
    _invalidateScheduledScan();
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
        _scheduledScanRevision != null ||
        (_pendingNodes.isEmpty && _pendingRegistrations.isEmpty)) {
      return;
    }
    final revision = _scanRevision;
    _scheduledScanRevision = revision;
    scheduleMicrotask(() => _runScan(revision));
  }

  _ProcessorScanWork _createScanWork() {
    _candidateQueueSaturated = false;
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
    return _ProcessorScanWork(
      nodeIds: nodeIds,
      registrations: registrations,
      pendingRegistrations: pendingRegistrations,
    );
  }

  void _runScan(int revision) {
    if (!_isCurrentScan(revision)) {
      return;
    }
    final work = _scanWork ?? _createScanWork();
    _scanWork = work;
    var remaining = _scanQuantum;
    while (remaining > 0 && work.phase != _ProcessorScanPhase.done) {
      if (!_isCurrentScan(revision)) {
        return;
      }
      switch (work.phase) {
        case _ProcessorScanPhase.changed:
          if (!_hasActiveRegistrations(work.registrations) ||
              (work.nodeScan == null &&
                  work.nodeIndex >= work.nodeIds.length)) {
            work.nodeScan = null;
            work.phase = _hasActiveRegistrations(work.pendingRegistrations)
                ? _ProcessorScanPhase.tree
                : _ProcessorScanPhase.done;
            continue;
          }
          final nodeScan = work.nodeScan;
          if (nodeScan == null) {
            final nodeId = work.nodeIds[work.nodeIndex];
            work.nodeIndex += 1;
            work.nodeScan = _createNodeScan(nodeId, work.registrations);
          } else {
            final step = _stepNodeScan(nodeScan);
            if (step.blocked) {
              if (!_isCurrentScan(revision)) {
                return;
              }
              _blockScan();
              remaining = 0;
              break;
            }
            if (step.complete) {
              work.nodeScan = null;
            }
          }
          remaining -= 1;
        case _ProcessorScanPhase.tree:
          if (!_hasActiveRegistrations(work.pendingRegistrations)) {
            work.nodeScan = null;
            work.treeQueue.clear();
            work.visited.clear();
            work.phase = _ProcessorScanPhase.done;
            continue;
          }
          if (!work.treeInitialized) {
            final roots = backend.state.currentState.document?.roots?.children;
            if (roots != null) {
              work.treeQueue.addAll(roots);
            }
            work.treeInitialized = true;
          }
          final nodeScan = work.nodeScan;
          if (nodeScan != null) {
            final step = _stepNodeScan(nodeScan);
            if (step.blocked) {
              if (!_isCurrentScan(revision)) {
                return;
              }
              _blockScan();
              remaining = 0;
              break;
            }
            if (step.complete) {
              final nodeView = step.nodeView;
              if (nodeView != null) {
                work.treeQueue.addAll(nodeView.node.children.children);
              }
              work.nodeScan = null;
            }
            remaining -= 1;
            break;
          }
          if (work.treeQueue.isEmpty) {
            work.phase = _ProcessorScanPhase.fallback;
            continue;
          }
          final nodeId = work.treeQueue.removeFirst();
          remaining -= 1;
          if (work.visited.add(nodeId)) {
            work.nodeScan = _createNodeScan(nodeId, work.pendingRegistrations);
          }
        case _ProcessorScanPhase.fallback:
          if (!_hasActiveRegistrations(work.pendingRegistrations) ||
              (work.nodeScan == null &&
                  work.fallbackIndex >= work.nodeIds.length)) {
            work.nodeScan = null;
            work.phase = _ProcessorScanPhase.done;
            continue;
          }
          final nodeScan = work.nodeScan;
          if (nodeScan != null) {
            final step = _stepNodeScan(nodeScan);
            if (step.blocked) {
              if (!_isCurrentScan(revision)) {
                return;
              }
              _blockScan();
              remaining = 0;
              break;
            }
            if (step.complete) {
              work.nodeScan = null;
            }
            remaining -= 1;
            break;
          }
          final nodeId = work.nodeIds[work.fallbackIndex];
          work.fallbackIndex += 1;
          remaining -= 1;
          if (!work.visited.contains(nodeId)) {
            work.nodeScan = _createNodeScan(nodeId, work.pendingRegistrations);
          }
        case _ProcessorScanPhase.done:
          break;
      }
    }
    if (!_isCurrentScan(revision)) {
      return;
    }
    if (work.phase == _ProcessorScanPhase.done) {
      _scanWork = null;
      _scheduledScanRevision = null;
      _scheduleScan();
      _drainCandidates();
      return;
    }
    _drainCandidates();
    if (!_isCurrentScan(revision)) {
      return;
    }
    if (!_scanBlocked) {
      _scheduleScanContinuation(revision);
    }
  }

  bool _isCurrentScan(int revision) =>
      !_closed &&
      revision == _scanRevision &&
      _scheduledScanRevision == revision;

  bool _hasActiveRegistrations(List<_RegisteredProcessor> registrations) =>
      registrations.any((registration) => registration.active);

  void _invalidateScheduledScan() {
    _scanRevision += 1;
    _scheduledScanRevision = null;
    _scheduledScanContinuationRevision = null;
    _scanWork = null;
    _scanBlocked = false;
    final scanUnblocked = _scanUnblocked;
    _scanUnblocked = null;
    if (scanUnblocked != null && !scanUnblocked.isCompleted) {
      scanUnblocked.complete();
    }
  }

  void _blockScan() {
    _scanBlocked = true;
    _scanUnblocked ??= Completer<void>();
  }

  void _scheduleScanContinuation(int revision) {
    if (_scheduledScanContinuationRevision != null) {
      return;
    }
    _scheduledScanContinuationRevision = revision;
    Timer.run(() {
      if (_scheduledScanContinuationRevision != revision) {
        return;
      }
      _scheduledScanContinuationRevision = null;
      _runScan(revision);
    });
  }

  _ProcessorNodeScan _createNodeScan(
    NodeId nodeId,
    List<_RegisteredProcessor> registrations,
  ) {
    final expectedEpoch = backend.state.currentState.document?.coordinate.epoch;
    NodeView? nodeView;
    Object? viewError;
    StackTrace? viewStackTrace;
    var viewFailed = false;
    if (expectedEpoch != null) {
      try {
        nodeView = backend.state.nodeView(nodeId);
      } catch (error, stackTrace) {
        viewFailed = true;
        viewError = error;
        viewStackTrace = stackTrace;
      }
    }
    return _ProcessorNodeScan(
      nodeId: nodeId,
      registrations: registrations,
      expectedEpoch: expectedEpoch,
      nodeView: nodeView,
      viewFailed: viewFailed,
      viewError: viewError,
      viewStackTrace: viewStackTrace,
    );
  }

  _ProcessorNodeScanStep _stepNodeScan(_ProcessorNodeScan scan) {
    final registration = scan.registrationIndex < scan.registrations.length
        ? scan.registrations[scan.registrationIndex]
        : null;
    if (registration == null) {
      return _ProcessorNodeScanStep(
        complete: true,
        nodeView: scan.nodeView,
        blocked: false,
      );
    }
    if (registration.active) {
      if (scan.expectedEpoch == null) {
        _removeCandidate(registration, scan.nodeId);
      } else if (scan.viewFailed) {
        _removeCandidate(registration, scan.nodeId);
        _emitError(
          phase: ProcessorErrorPhase.view,
          registration: registration,
          nodeId: scan.nodeId,
          error: scan.viewError!,
          stackTrace: scan.viewStackTrace!,
        );
      } else {
        final nodeView = scan.nodeView;
        if (nodeView == null) {
          _removeCandidate(registration, scan.nodeId);
          _advanceNodeScan(scan, registration);
          return _nodeScanStep(scan);
        }
        final processor = registration.processor;
        if (nodeView.node.stability == 'provisional' &&
            !(registration.descriptor.acceptsProvisional &&
                registration.allowProvisional)) {
          _removeCandidate(registration, scan.nodeId);
        } else {
          bool matches;
          try {
            matches = processor.matches(nodeView.node);
          } catch (error, stackTrace) {
            _removeCandidate(registration, scan.nodeId);
            _emitError(
              phase: ProcessorErrorPhase.matches,
              registration: registration,
              nodeId: scan.nodeId,
              error: error,
              stackTrace: stackTrace,
            );
            _advanceNodeScan(scan, registration);
            return _nodeScanStep(scan);
          }
          if (matches && registration.active) {
            if (!_enqueueCandidate(
              registration,
              scan.expectedEpoch!,
              scan.nodeId,
              nodeView.processorInputVersion,
            )) {
              return _ProcessorNodeScanStep(
                complete: false,
                nodeView: nodeView,
                blocked: true,
              );
            }
          } else {
            _removeCandidate(registration, scan.nodeId);
          }
        }
      }
    }
    _advanceNodeScan(scan, registration);
    return _nodeScanStep(scan);
  }

  void _advanceNodeScan(
    _ProcessorNodeScan scan,
    _RegisteredProcessor registration,
  ) {
    if (scan.registrationIndex < scan.registrations.length &&
        identical(scan.registrations[scan.registrationIndex], registration)) {
      scan.registrationIndex += 1;
    }
  }

  _ProcessorNodeScanStep _nodeScanStep(_ProcessorNodeScan scan) {
    return _ProcessorNodeScanStep(
      complete: scan.registrationIndex >= scan.registrations.length,
      nodeView: scan.nodeView,
      blocked: false,
    );
  }

  bool _enqueueCandidate(
    _RegisteredProcessor registration,
    Epoch expectedEpoch,
    NodeId nodeId,
    ProcessorInputVersion expectedInputVersion, {
    bool front = false,
  }) {
    if (_closed || !registration.active) {
      return true;
    }
    final rejected = _rejectedCandidates[registration]?[nodeId];
    if (rejected?.epoch == expectedEpoch &&
        rejected?.inputVersion == expectedInputVersion) {
      return true;
    }
    final registrationCandidates = _candidates[registration];
    final existing = registrationCandidates?[nodeId];
    if (existing != null) {
      existing.expectedEpoch = expectedEpoch;
      existing.expectedInputVersion = expectedInputVersion;
      return true;
    }
    if (_candidateCount >= _maxQueuedCandidates) {
      if (!_candidateQueueSaturated) {
        _candidateQueueSaturated = true;
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
      }
      return _closed || !registration.active;
    }
    final candidates =
        registrationCandidates ?? <NodeId, _ProcessorCandidate>{};
    if (registrationCandidates == null) {
      _candidates[registration] = candidates;
    }
    final candidate = _ProcessorCandidate(
      registration: registration,
      expectedEpoch: expectedEpoch,
      nodeId: nodeId,
      expectedInputVersion: expectedInputVersion,
    );
    candidates[nodeId] = candidate;
    if (front) {
      _candidateQueue.addFirst(candidate);
    } else {
      _candidateQueue.addLast(candidate);
    }
    _candidateCount += 1;
    return true;
  }

  void _removeCandidate(_RegisteredProcessor registration, NodeId nodeId) {
    final registrationCandidates = _candidates[registration];
    final candidate = registrationCandidates?.remove(nodeId);
    if (candidate == null) {
      return;
    }
    candidate.queued = false;
    _candidateCount -= 1;
    _markCandidateCapacityAvailable();
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
    _markCandidateCapacityAvailable();
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
    _candidateQueueSaturated = false;
    _dispatchBlocked = false;
  }

  void _rejectCandidate(_ProcessorCandidate candidate) {
    final rejected = _rejectedCandidates.putIfAbsent(
      candidate.registration,
      () => <NodeId, _CandidateExpectation>{},
    );
    rejected[candidate.nodeId] = _CandidateExpectation(
      epoch: candidate.expectedEpoch,
      inputVersion: candidate.expectedInputVersion,
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
      _markCandidateCapacityAvailable();
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
    if (_dispatching ||
        _dispatchBlocked ||
        _scheduledDispatchRevision != null ||
        _closed) {
      return;
    }
    _dispatching = true;
    var attempts = 0;
    try {
      while (_candidateCount > 0 &&
          _inFlight.length < _maxDispatchJobs &&
          attempts < _dispatchQuantum) {
        final candidate = _takeCandidate();
        if (candidate == null) {
          break;
        }
        attempts += 1;
        if (!candidate.registration.active) {
          continue;
        }
        if (_begin(candidate) == _BeginDisposition.blocked) {
          _dispatchBlocked = true;
          break;
        }
      }
    } finally {
      _dispatching = false;
    }
    if (!_dispatchBlocked &&
        _candidateCount > 0 &&
        _inFlight.length < _maxDispatchJobs) {
      _scheduleDispatch();
    }
  }

  void _scheduleDispatch() {
    if (_closed ||
        _dispatchBlocked ||
        _scheduledDispatchRevision != null ||
        _candidateCount == 0 ||
        _inFlight.length >= _maxDispatchJobs) {
      return;
    }
    final revision = _dispatchRevision;
    _scheduledDispatchRevision = revision;
    Timer.run(() {
      if (_scheduledDispatchRevision != revision) {
        return;
      }
      _scheduledDispatchRevision = null;
      _drainCandidates();
    });
  }

  void _invalidateScheduledDispatch() {
    _dispatchRevision += 1;
    _scheduledDispatchRevision = null;
    _dispatchBlocked = false;
  }

  void _markCandidateCapacityAvailable() {
    if (_candidateCount < _maxQueuedCandidates) {
      if (_scanBlocked) {
        _scanBlocked = false;
        final scanUnblocked = _scanUnblocked;
        _scanUnblocked = null;
        if (scanUnblocked != null && !scanUnblocked.isCompleted) {
          scanUnblocked.complete();
        }
        final revision = _scheduledScanRevision;
        if (revision != null) {
          _scheduleScanContinuation(revision);
        }
      }
    }
  }

  _BeginDisposition _begin(_ProcessorCandidate candidate) {
    final registration = candidate.registration;
    final expectedEpoch = candidate.expectedEpoch;
    final nodeId = candidate.nodeId;
    final expectedInputVersion = candidate.expectedInputVersion;
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
        expectedInputVersion: expectedInputVersion,
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
          _retryableResourceLimitDetailCodes.contains(normalized.detailCode) &&
          _inFlight.isNotEmpty) {
        _enqueueCandidate(
          registration,
          expectedEpoch,
          nodeId,
          expectedInputVersion,
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
                _dispatchBlocked = false;
              }
              _jobs.remove(job);
              _scheduleDispatch();
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
    _removeRegistrationFromScan(registration);
    _removeRegistrationCandidates(registration);
    _rejectedCandidates.remove(registration);
    var capacityChanged = false;
    for (final entry in List<_InFlightProcessor>.of(_inFlight.values)) {
      if (identical(entry.registration, registration)) {
        entry.cancellation.cancel('processor_unregistered');
        _cancel(entry, ProcessorErrorPhase.cancel);
        _inFlight.remove(entry.request.requestId);
        capacityChanged = true;
      }
    }
    if (capacityChanged) {
      _dispatchBlocked = false;
    }
    if (_processors.isEmpty) {
      _invalidateScheduledScan();
      _pendingNodes.clear();
      _pendingRegistrations.clear();
    }
    _drainCandidates();
  }

  void _removeRegistrationFromScan(_RegisteredProcessor registration) {
    final work = _scanWork;
    if (work == null) {
      return;
    }
    work.registrations = work.registrations
        .where((candidate) => !identical(candidate, registration))
        .toList(growable: false);
    work.pendingRegistrations = work.pendingRegistrations
        .where((candidate) => !identical(candidate, registration))
        .toList(growable: false);
    final scan = work.nodeScan;
    if (scan != null) {
      final index = scan.registrations.indexWhere(
        (candidate) => identical(candidate, registration),
      );
      if (index >= 0) {
        scan.registrations = scan.registrations
            .where((candidate) => !identical(candidate, registration))
            .toList(growable: false);
        if (index < scan.registrationIndex) {
          scan.registrationIndex -= 1;
        }
      }
    }
    if (work.phase == _ProcessorScanPhase.changed &&
        !_hasActiveRegistrations(work.registrations)) {
      work.nodeScan = null;
      work.phase = _hasActiveRegistrations(work.pendingRegistrations)
          ? _ProcessorScanPhase.tree
          : _ProcessorScanPhase.done;
    } else if ((work.phase == _ProcessorScanPhase.tree ||
            work.phase == _ProcessorScanPhase.fallback) &&
        !_hasActiveRegistrations(work.pendingRegistrations)) {
      work.nodeScan = null;
      work.treeQueue.clear();
      work.visited.clear();
      work.phase = _ProcessorScanPhase.done;
    }
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
