part of '../mdstream_flutter.dart';

/// The controller operation that produced a structured error.
enum MdstreamControllerErrorPhase {
  /// Appending source to a local stream.
  append,

  /// Finalizing a local stream.
  finish,

  /// Resetting a local stream.
  reset,

  /// Applying a canonical change to a replica.
  applyChange,

  /// Recovering a replica from a canonical snapshot.
  recoverSnapshot,

  /// Creating an explicit recovery snapshot.
  createRecoverySnapshot,
}

/// One controller failure with its operation and original stack trace.
@immutable
final class MdstreamControllerError {
  /// Creates an immutable controller error.
  const MdstreamControllerError({
    required this.phase,
    required this.error,
    required this.stackTrace,
  });

  /// Operation that failed.
  final MdstreamControllerErrorPhase phase;

  /// Normalized mdstream error.
  final MdstreamException error;

  /// Original Dart stack trace.
  final StackTrace stackTrace;
}

/// Ordered renderer-neutral transition facts published for one operation.
@immutable
final class MdstreamTransitionBatch {
  MdstreamTransitionBatch._({
    required this.revision,
    required Iterable<TransitionFactsView> facts,
  }) : facts = List<TransitionFactsView>.unmodifiable(facts);

  /// Monotonic operation revision, starting at zero before the first batch.
  final int revision;

  /// Ordered facts from every reducer update committed by the operation.
  ///
  /// The list is empty for no-op, failed, artifact-only, and same-floor
  /// operations. Intermediate facts are observations; readable views represent
  /// only the state at the tail of this batch.
  final List<TransitionFactsView> facts;

  static final MdstreamTransitionBatch _initial = MdstreamTransitionBatch._(
    revision: 0,
    facts: const <TransitionFactsView>[],
  );
}

/// Exact invalidations aggregated across one public controller operation.
@immutable
final class MdstreamNotificationImpact {
  /// Creates an immutable notification impact.
  const MdstreamNotificationImpact({
    required this.changedNodeIds,
    required this.removedNodeIds,
    required this.changedResourceIds,
    required this.removedResourceIds,
    required this.sourceChanged,
    required this.projectionChanged,
    required this.lifecycleChanged,
    required this.rootsChanged,
    required this.fullReplace,
  });

  /// Node identities whose views changed or disappeared.
  final List<NodeId> changedNodeIds;

  /// Node identities that disappeared.
  final List<NodeId> removedNodeIds;

  /// Resource identities whose views changed or disappeared.
  final List<ResourceId> changedResourceIds;

  /// Resource identities that disappeared.
  final List<ResourceId> removedResourceIds;

  /// Whether canonical source changed.
  final bool sourceChanged;

  /// Whether canonical projections changed.
  final bool projectionChanged;

  /// Whether document lifecycle changed.
  final bool lifecycleChanged;

  /// Whether root identities or ordering changed.
  final bool rootsChanged;

  /// Whether every previously materialized view must be invalidated.
  final bool fullReplace;

  static const MdstreamNotificationImpact _empty = MdstreamNotificationImpact(
    changedNodeIds: <NodeId>[],
    removedNodeIds: <NodeId>[],
    changedResourceIds: <ResourceId>[],
    removedResourceIds: <ResourceId>[],
    sourceChanged: false,
    projectionChanged: false,
    lifecycleChanged: false,
    rootsChanged: false,
    fullReplace: false,
  );
}

/// Immutable value published by an mdstream Flutter controller.
@immutable
final class MdstreamControllerState {
  /// Creates an immutable controller state.
  const MdstreamControllerState({
    required this.snapshot,
    required this.impact,
    required this.lastError,
  });

  /// Current Rust-backed canonical reducer state.
  final MdstreamStateSnapshot snapshot;

  /// Invalidations aggregated across the latest public operation.
  final MdstreamNotificationImpact impact;

  /// Latest synchronous controller error, if one has not been cleared.
  final MdstreamControllerError? lastError;

  /// Current canonical reducer status.
  ReducerStatusView get status => snapshot.status;

  /// Current document summary, or `null` before initialization.
  DocumentSummaryView? get document => snapshot.document;

  /// Whether the replica requires an explicit recovery snapshot.
  bool get needsSnapshot => status is NeedsSnapshotReducerStatusView;

  /// Whether the current document has been finalized.
  bool get isFinalized => document?.lifecycle == 'finalized';
}

final class _DirectedValueListenable<T> extends ChangeNotifier
    implements ValueListenable<T> {
  _DirectedValueListenable(this._value);

  T _value;
  bool _disposed = false;
  int _activeNotificationDepth = 0;

  @override
  T get value => _value;

  bool replace(T next, {bool force = false}) {
    if (_disposed || (!force && identical(next, _value))) {
      return false;
    }
    _value = next;
    return true;
  }

  void emit() {
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

  void update(T next, {bool force = false}) {
    if (replace(next, force: force)) {
      emit();
    }
  }

  @override
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    if (_activeNotificationDepth == 0) {
      super.dispose();
    }
  }
}

final class _ImpactBuilder {
  final LinkedHashSet<NodeId> changedNodeIds = LinkedHashSet<NodeId>();
  final LinkedHashSet<NodeId> removedNodeIds = LinkedHashSet<NodeId>();
  final LinkedHashSet<ResourceId> changedResourceIds =
      LinkedHashSet<ResourceId>();
  final LinkedHashSet<ResourceId> removedResourceIds =
      LinkedHashSet<ResourceId>();
  bool sourceChanged = false;
  bool projectionChanged = false;
  bool lifecycleChanged = false;
  bool rootsChanged = false;
  bool fullReplace = false;

  void add(ChangeImpactView impact) {
    changedNodeIds.addAll(impact.changedNodeIds);
    removedNodeIds.addAll(impact.removedNodeIds);
    changedResourceIds.addAll(impact.changedResourceIds);
    removedResourceIds.addAll(impact.removedResourceIds);
    sourceChanged = sourceChanged || impact.sourceChanged;
    projectionChanged = projectionChanged || impact.projectionChanged;
    lifecycleChanged = lifecycleChanged || impact.lifecycleChanged;
    rootsChanged = rootsChanged || impact.rootsChanged;
    fullReplace = fullReplace || impact.fullReplace;
  }

  MdstreamNotificationImpact build() => MdstreamNotificationImpact(
    changedNodeIds: List<NodeId>.unmodifiable(changedNodeIds),
    removedNodeIds: List<NodeId>.unmodifiable(removedNodeIds),
    changedResourceIds: List<ResourceId>.unmodifiable(changedResourceIds),
    removedResourceIds: List<ResourceId>.unmodifiable(removedResourceIds),
    sourceChanged: sourceChanged,
    projectionChanged: projectionChanged,
    lifecycleChanged: lifecycleChanged,
    rootsChanged: rootsChanged,
    fullReplace: fullReplace,
  );
}
