import 'package:mdstream/src/views.dart';

Map<String, Object?> normalizeReducerUpdate(ReducerUpdateView update) => {
  'schema': update.schema,
  'kind': update.kind,
  'outcome': _normalizeOutcome(update.outcome),
  'status': _normalizeStatus(update.status),
  'impact': _normalizeImpact(update.impact),
  'document': update.document == null
      ? null
      : _normalizeDocument(update.document!),
  if (update.transition != null)
    'transition': _normalizeTransition(update.transition!),
};

Map<String, Object?> _normalizeCoordinate(CoordinateView coordinate) => {
  'epoch': coordinate.epoch,
  'sequence': coordinate.sequence,
  'changeId': coordinate.changeId,
  'sourceCursor': coordinate.sourceCursor,
};

Map<String, Object?> _normalizeOutcome(ApplyOutcomeView outcome) {
  return switch (outcome) {
    AppliedOutcomeView(:final coordinate) => {
      'kind': 'applied',
      'coordinate': _normalizeCoordinate(coordinate),
    },
    RecoveredOutcomeView(:final coordinate) => {
      'kind': 'recovered',
      'coordinate': _normalizeCoordinate(coordinate),
    },
    IdempotentOutcomeView() => {'kind': 'idempotent'},
    StaleOutcomeView(
      :final current,
      :final receivedEpoch,
      :final receivedSequence,
    ) =>
      {
        'kind': 'stale',
        'current': _normalizeCoordinate(current),
        'receivedEpoch': receivedEpoch,
        'receivedSequence': receivedSequence,
      },
    RecoveryRequiredOutcomeView(:final lastGood, :final reason) => {
      'kind': 'recovery_required',
      'lastGood': _normalizeCoordinate(lastGood),
      'reason': _camelizeJson(reason.raw),
    },
  };
}

Map<String, Object?> _normalizeStatus(ReducerStatusView status) {
  return switch (status) {
    UninitializedReducerStatusView() => {'kind': 'uninitialized'},
    ReadyReducerStatusView() => {'kind': 'ready'},
    NeedsSnapshotReducerStatusView(:final lastGood, :final reason) => {
      'kind': 'needs_snapshot',
      'lastGood': _normalizeCoordinate(lastGood),
      'reason': _camelizeJson(reason.raw),
    },
  };
}

Map<String, Object?> _normalizeImpact(ChangeImpactView impact) => {
  'changedNodeIds': impact.changedNodeIds,
  'removedNodeIds': impact.removedNodeIds,
  'changedResourceIds': impact.changedResourceIds,
  'removedResourceIds': impact.removedResourceIds,
  'sourceChanged': impact.sourceChanged,
  'projectionChanged': impact.projectionChanged,
  'lifecycleChanged': impact.lifecycleChanged,
  'rootsChanged': impact.rootsChanged,
  'fullReplace': impact.fullReplace,
};

Map<String, Object?> _normalizeDocument(DocumentSummaryView document) => {
  'coordinate': _normalizeCoordinate(document.coordinate),
  'lifecycle': document.lifecycle,
  'projectionCursor': document.projectionCursor,
  if (document.roots != null)
    'roots': {
      'version': document.roots!.version,
      'children': document.roots!.children,
    },
};

Map<String, Object?> _normalizeTransition(TransitionEnvelopeView transition) =>
    {'schema': transition.schema, 'facts': _normalizeFacts(transition.facts)};

Map<String, Object?> _normalizeFacts(TransitionFactsView facts) {
  final common = <String, Object?>{
    'scope': facts.scope,
    'before': facts.before == null
        ? null
        : _normalizeDocumentStamp(facts.before!),
    'after': _normalizeDocumentStamp(facts.after),
  };
  return switch (facts) {
    ContinuousTransitionFactsView() => {
      ...common,
      'nodes': facts.nodes.map(_normalizeNodeTransition).toList(),
      'structures': facts.structures
          .map(_normalizeStructureTransition)
          .toList(),
      'resources': facts.resources.map(_normalizeResourceTransition).toList(),
    },
    FullReplaceTransitionFactsView() => common,
  };
}

Map<String, Object?> _normalizeDocumentStamp(DocumentStateStampView stamp) => {
  'continuityGeneration': stamp.continuityGeneration,
  'coordinate': _normalizeCoordinate(stamp.coordinate),
  'lifecycle': stamp.lifecycle,
  'projectionCursor': stamp.projectionCursor,
  'rootsVersion': stamp.rootsVersion,
};

Map<String, Object?> _normalizeNodeKey(TransitionNodeKeyView key) => {
  'continuityGeneration': key.continuityGeneration,
  'epoch': key.epoch,
  'nodeId': key.nodeId,
};

Map<String, Object?> _normalizeResourceKey(TransitionResourceKeyView key) => {
  'continuityGeneration': key.continuityGeneration,
  'epoch': key.epoch,
  'resourceId': key.resourceId,
};

Map<String, Object?> _normalizeOwner(TransitionChildListOwnerView owner) =>
    switch (owner) {
      DocumentTransitionOwnerView() => {'kind': owner.kind},
      NodeTransitionOwnerView() => {
        'kind': owner.kind,
        'key': _normalizeNodeKey(owner.key),
      },
    };

Map<String, Object?> _normalizeNodeStamp(NodeStateStampView stamp) => {
  'version': stamp.version,
  'stability': stamp.stability,
  'parent': stamp.parent == null ? null : _normalizeOwner(stamp.parent!),
  'childrenVersion': stamp.childrenVersion,
};

Map<String, Object?> _normalizeText(TextTransitionView text) => switch (text) {
  ProjectionAppendTransitionView() => {
    'kind': text.kind,
    'range': {'start': text.range.start, 'end': text.range.end},
    'text': text.text,
  },
  ReplacementTextTransitionView() => {'kind': text.kind},
};

Map<String, Object?> _normalizeNodeTransition(NodeTransitionView transition) =>
    {
      'key': _normalizeNodeKey(transition.key),
      'before': transition.before == null
          ? null
          : _normalizeNodeStamp(transition.before!),
      'after': transition.after == null
          ? null
          : _normalizeNodeStamp(transition.after!),
      'text': transition.text == null ? null : _normalizeText(transition.text!),
    };

Map<String, Object?> _normalizeStructureTransition(
  StructureTransitionView transition,
) => {
  'owner': _normalizeOwner(transition.owner),
  'beforeVersion': transition.beforeVersion,
  'afterVersion': transition.afterVersion,
  'start': transition.start,
  'removed': transition.removed.map(_normalizeNodeKey).toList(),
  'inserted': transition.inserted.map(_normalizeNodeKey).toList(),
};

Map<String, Object?> _normalizeResourceTransition(
  ResourceTransitionView transition,
) => {
  'key': _normalizeResourceKey(transition.key),
  'beforeVersion': transition.beforeVersion,
  'afterVersion': transition.afterVersion,
  'affectedNodes': transition.affectedNodes.map(_normalizeNodeKey).toList(),
};

Object? _camelizeJson(Object? value) {
  if (value is List<Object?>) {
    return value.map(_camelizeJson).toList();
  }
  if (value is Map<String, Object?>) {
    return <String, Object?>{
      for (final MapEntry(:key, :value) in value.entries)
        _camelizeKey(key): _camelizeJson(value),
    };
  }
  return value;
}

String _camelizeKey(String key) => key.replaceAllMapped(
  RegExp(r'_([a-z])'),
  (match) => match.group(1)!.toUpperCase(),
);
