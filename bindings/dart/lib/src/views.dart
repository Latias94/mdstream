// ignore_for_file: public_member_api_docs

import 'dart:convert';

import 'errors.dart';
import 'protocol.dart';

/// Base type for decoded, recursively immutable binding JSON views.
sealed class DecodedBindingView {
  const DecodedBindingView({
    required this.schema,
    required this.kind,
    required this.raw,
  });

  final String schema;
  final String kind;
  final Map<String, Object?> raw;
}

/// Position of a successfully applied protocol change.
final class CoordinateView {
  const CoordinateView({
    required this.epoch,
    required this.sequence,
    required this.changeId,
    required this.sourceCursor,
  });

  final Epoch epoch;
  final Sequence sequence;
  final ChangeId changeId;
  final SourceCursor sourceCursor;
}

/// Extensible reason for requiring an explicit recovery snapshot.
final class RecoveryReasonView {
  const RecoveryReasonView({required this.kind, required this.raw});

  final String kind;
  final Map<String, Object?> raw;
}

/// Current reducer readiness state.
final class ReducerStatusView {
  const ReducerStatusView({required this.kind, this.lastGood, this.reason});

  final String kind;
  final CoordinateView? lastGood;
  final RecoveryReasonView? reason;
}

/// Result of applying one canonical change or snapshot.
final class ApplyOutcomeView {
  const ApplyOutcomeView({
    required this.kind,
    this.coordinate,
    this.current,
    this.receivedEpoch,
    this.receivedSequence,
    this.lastGood,
    this.reason,
  });

  final String kind;
  final CoordinateView? coordinate;
  final CoordinateView? current;
  final Epoch? receivedEpoch;
  final Sequence? receivedSequence;
  final CoordinateView? lastGood;
  final RecoveryReasonView? reason;
}

/// Exact cache invalidations produced by one reducer operation.
final class ChangeImpactView {
  const ChangeImpactView({
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

  final List<NodeId> changedNodeIds;
  final List<NodeId> removedNodeIds;
  final List<ResourceId> changedResourceIds;
  final List<ResourceId> removedResourceIds;
  final bool sourceChanged;
  final bool projectionChanged;
  final bool lifecycleChanged;
  final bool rootsChanged;
  final bool fullReplace;
}

/// Versioned ordered child identities.
final class ChildListView {
  const ChildListView({required this.version, required this.children});

  final StructureVersion version;
  final List<NodeId> children;
}

/// Document fields needed to update a framework state store.
final class DocumentSummaryView {
  const DocumentSummaryView({
    required this.coordinate,
    required this.lifecycle,
    required this.projectionCursor,
    this.roots,
  });

  final CoordinateView coordinate;
  final String lifecycle;
  final SourceCursor projectionCursor;
  final ChildListView? roots;
}

/// One typed reducer update and its precise invalidation set.
final class ReducerUpdateView extends DecodedBindingView {
  const ReducerUpdateView._({
    required super.schema,
    required super.raw,
    required this.outcome,
    required this.status,
    required this.impact,
    required this.document,
  }) : super(kind: 'reducer_update');

  final ApplyOutcomeView outcome;
  final ReducerStatusView status;
  final ChangeImpactView impact;
  final DocumentSummaryView? document;
}

/// Half-open source range represented with exact decimal cursors.
final class SourceRangeView {
  const SourceRangeView({required this.start, required this.end});

  final SourceCursor start;
  final SourceCursor end;
}

/// Typed stable node envelope with extensible content metadata.
final class ContentNodeView {
  const ContentNodeView({
    required this.id,
    required this.version,
    required this.stability,
    required this.source,
    required this.body,
    required this.children,
    required this.content,
  });

  final NodeId id;
  final NodeVersion version;
  final String stability;
  final SourceRangeView source;
  final SourceRangeView body;
  final ChildListView children;
  final Map<String, Object?> content;
}

/// Materialized node plus its body text.
final class NodeView extends DecodedBindingView {
  const NodeView._({
    required super.schema,
    required super.raw,
    required this.node,
    required this.bodyText,
  }) : super(kind: 'node_view');

  final ContentNodeView node;
  final String bodyText;
}

/// Typed stable semantic resource with extensible content metadata.
final class SemanticResourceView {
  const SemanticResourceView({
    required this.id,
    required this.version,
    required this.content,
  });

  final ResourceId id;
  final ResourceVersion version;
  final Map<String, Object?> content;
}

/// Materialized semantic resource.
final class ResourceView extends DecodedBindingView {
  const ResourceView._({
    required super.schema,
    required super.raw,
    required this.resource,
  }) : super(kind: 'resource_view');

  final SemanticResourceView resource;
}

/// Complete key that prevents stale processor results from being applied.
final class ProcessorKeyView {
  const ProcessorKeyView({
    required this.epoch,
    required this.nodeId,
    required this.processorId,
    required this.nodeVersion,
    required this.inputVersion,
    required this.processorVersion,
    required this.configurationVersion,
    required this.generation,
  });

  final Epoch epoch;
  final NodeId nodeId;
  final String processorId;
  final NodeVersion nodeVersion;
  final ProcessorInputVersion inputVersion;
  final String processorVersion;
  final String configurationVersion;
  final RequestGeneration generation;
}

/// Fully materialized input for one external content processor.
final class ProcessorInputView {
  const ProcessorInputView({
    required this.node,
    required this.body,
    required this.resource,
  });

  final ContentNodeView node;
  final String body;
  final SemanticResourceView? resource;
}

/// Request to run a registered content processor.
final class ProcessorRequestView extends DecodedBindingView {
  const ProcessorRequestView._({
    required super.schema,
    required super.raw,
    required this.requestId,
    required this.key,
    required this.input,
  }) : super(kind: 'processor_request');

  final RequestGeneration requestId;
  final ProcessorKeyView key;
  final ProcessorInputView input;
}

/// Native decision for one processor completion.
final class ProcessorCompletionView extends DecodedBindingView {
  const ProcessorCompletionView._({
    required super.schema,
    required super.raw,
    required this.requestId,
    required this.outcome,
  }) : super(kind: 'processor_completion');

  final RequestGeneration requestId;
  final String outcome;
}

/// State transition for one processor artifact slot.
final class ArtifactChangeKindView {
  const ArtifactChangeKindView({
    required this.kind,
    this.artifactBytes,
    this.code,
    this.reason,
    this.releasedArtifactBytes,
  });

  final String kind;
  final DecimalCounter? artifactBytes;
  final String? code;
  final String? reason;
  final DecimalCounter? releasedArtifactBytes;
}

/// Artifact slot invalidation emitted by the native processor host.
final class ArtifactChangeView extends DecodedBindingView {
  const ArtifactChangeView._({
    required super.schema,
    required super.raw,
    required this.key,
    required this.change,
  }) : super(kind: 'artifact_change');

  final ProcessorKeyView key;
  final ArtifactChangeKindView change;
}

/// Text, binary, or citation payload owned by a processor artifact.
final class ArtifactPayloadView {
  const ArtifactPayloadView({
    required this.kind,
    this.text,
    this.bytes,
    this.key,
    this.destination,
    this.title,
  });

  final String kind;
  final String? text;
  final List<int>? bytes;
  final String? key;
  final String? destination;
  final String? title;
}

/// Protocol-labelled output produced by a content processor.
final class ProcessorArtifactView {
  const ProcessorArtifactView({
    required this.protocol,
    required this.mediaType,
    required this.payload,
  });

  final String protocol;
  final String mediaType;
  final ArtifactPayloadView payload;
}

/// Structured processor failure retained for an artifact slot.
final class ArtifactFailureView {
  const ArtifactFailureView({required this.code, required this.message});

  final String code;
  final String message;
}

/// Current state and optional retained value of one processor artifact slot.
final class ArtifactView extends DecodedBindingView {
  const ArtifactView._({
    required super.schema,
    required super.raw,
    required this.key,
    required this.state,
    required this.artifact,
    required this.failure,
  }) : super(kind: 'artifact_view');

  final ProcessorKeyView key;
  final String state;
  final ProcessorArtifactView? artifact;
  final ArtifactFailureView? failure;
}

const _processorFailureCodes = <String>{
  'processor',
  'panic',
  'invalid_request',
  'cancelled',
  'unsupported_content',
  'unresolved_context',
  'invalid_context',
  'resource_limit',
};

/// Decodes one JSON view emitted by the binding facade.
///
/// Canonical changes and snapshots intentionally remain opaque bytes and are
/// rejected here. All decoded maps and lists are recursively immutable.
DecodedBindingView decodeBindingView(
  BindingPayloadKind payloadKind,
  List<int> bytes,
  String expectedSchema,
) {
  if (payloadKind == BindingPayloadKind.change ||
      payloadKind == BindingPayloadKind.snapshot) {
    throw invalidBindingPayload(
      'canonical byte payloads must not be decoded as binding views',
    );
  }

  final record = _parseJsonRecord(bytes, 'binding payload');
  _requireLiteral(record['schema'], expectedSchema, 'schema');
  _requireLiteral(record['kind'], payloadKind.viewKind, 'kind');

  switch (payloadKind) {
    case BindingPayloadKind.reducerUpdate:
      return _decodeReducerUpdate(record, expectedSchema);
    case BindingPayloadKind.nodeView:
      return _decodeNodeView(record, expectedSchema);
    case BindingPayloadKind.resourceView:
      return _decodeResourceView(record, expectedSchema);
    case BindingPayloadKind.processorRequest:
      return _decodeProcessorRequest(record, expectedSchema);
    case BindingPayloadKind.processorCompletion:
      return _decodeProcessorCompletion(record, expectedSchema);
    case BindingPayloadKind.artifactChange:
      return _decodeArtifactChange(record, expectedSchema);
    case BindingPayloadKind.artifactView:
      return _decodeArtifactView(record, expectedSchema);
    case BindingPayloadKind.change:
    case BindingPayloadKind.snapshot:
      throw invalidBindingPayload(
        'canonical byte payloads must not be decoded as binding views',
      );
  }
}

ReducerUpdateView _decodeReducerUpdate(
  Map<String, Object?> value,
  String schema,
) => ReducerUpdateView._(
  schema: schema,
  raw: value,
  outcome: _decodeOutcome(_requiredRecord(value['outcome'], 'outcome')),
  status: _decodeStatus(_requiredRecord(value['status'], 'status')),
  impact: _decodeImpact(_requiredRecord(value['impact'], 'impact')),
  document: value['document'] == null
      ? null
      : _decodeDocument(_requiredRecord(value['document'], 'document')),
);

ApplyOutcomeView _decodeOutcome(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'outcome.kind');
  switch (kind) {
    case 'applied':
    case 'recovered':
      return ApplyOutcomeView(
        kind: kind,
        coordinate: _decodeCoordinate(
          _requiredRecord(value['coordinate'], 'coordinate'),
        ),
      );
    case 'idempotent':
      return ApplyOutcomeView(kind: kind);
    case 'stale':
      return ApplyOutcomeView(
        kind: kind,
        current: _decodeCoordinate(
          _requiredRecord(value['current'], 'current'),
        ),
        receivedEpoch: requireDecimalString(
          value['received_epoch'],
          'received_epoch',
        ),
        receivedSequence: requireDecimalString(
          value['received_sequence'],
          'received_sequence',
        ),
      );
    case 'recovery_required':
      return ApplyOutcomeView(
        kind: kind,
        lastGood: _decodeCoordinate(
          _requiredRecord(value['last_good'], 'last_good'),
        ),
        reason: _decodeRecoveryReason(
          _requiredRecord(value['reason'], 'reason'),
        ),
      );
    default:
      throw invalidBindingPayload('unknown reducer outcome $kind');
  }
}

ReducerStatusView _decodeStatus(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'status.kind');
  switch (kind) {
    case 'uninitialized':
    case 'ready':
      return ReducerStatusView(kind: kind);
    case 'needs_snapshot':
      return ReducerStatusView(
        kind: kind,
        lastGood: _decodeCoordinate(
          _requiredRecord(value['last_good'], 'last_good'),
        ),
        reason: _decodeRecoveryReason(
          _requiredRecord(value['reason'], 'reason'),
        ),
      );
    default:
      throw invalidBindingPayload('unknown reducer status $kind');
  }
}

RecoveryReasonView _decodeRecoveryReason(Map<String, Object?> value) =>
    RecoveryReasonView(
      kind: _requiredString(value['kind'], 'recovery reason kind'),
      raw: value,
    );

ChangeImpactView _decodeImpact(Map<String, Object?> value) => ChangeImpactView(
  changedNodeIds: _decimalArray(value['changed_node_ids'], 'changed_node_ids'),
  removedNodeIds: _decimalArray(value['removed_node_ids'], 'removed_node_ids'),
  changedResourceIds: _decimalArray(
    value['changed_resource_ids'],
    'changed_resource_ids',
  ),
  removedResourceIds: _decimalArray(
    value['removed_resource_ids'],
    'removed_resource_ids',
  ),
  sourceChanged: _requiredBoolean(value['source_changed'], 'source_changed'),
  projectionChanged: _requiredBoolean(
    value['projection_changed'],
    'projection_changed',
  ),
  lifecycleChanged: _requiredBoolean(
    value['lifecycle_changed'],
    'lifecycle_changed',
  ),
  rootsChanged: _requiredBoolean(value['roots_changed'], 'roots_changed'),
  fullReplace: _requiredBoolean(value['full_replace'], 'full_replace'),
);

DocumentSummaryView _decodeDocument(Map<String, Object?> value) {
  final lifecycle = _requiredString(value['lifecycle'], 'document.lifecycle');
  if (lifecycle != 'open' && lifecycle != 'finalized') {
    throw invalidBindingPayload('unknown document lifecycle $lifecycle');
  }
  return DocumentSummaryView(
    coordinate: _decodeCoordinate(
      _requiredRecord(value['coordinate'], 'document.coordinate'),
    ),
    lifecycle: lifecycle,
    projectionCursor: requireDecimalString(
      value['projection_cursor'],
      'projection_cursor',
    ),
    roots: value.containsKey('roots')
        ? _decodeChildList(_requiredRecord(value['roots'], 'document.roots'))
        : null,
  );
}

CoordinateView _decodeCoordinate(Map<String, Object?> value) => CoordinateView(
  epoch: requireDecimalString(value['epoch'], 'coordinate.epoch'),
  sequence: requireDecimalString(value['sequence'], 'coordinate.sequence'),
  changeId: _requiredString(value['change_id'], 'coordinate.change_id'),
  sourceCursor: requireDecimalString(
    value['source_cursor'],
    'coordinate.source_cursor',
  ),
);

NodeView _decodeNodeView(Map<String, Object?> value, String schema) =>
    NodeView._(
      schema: schema,
      raw: value,
      node: _decodeNode(_requiredRecord(value['node'], 'node')),
      bodyText: _requiredString(value['body_text'], 'body_text'),
    );

ContentNodeView _decodeNode(Map<String, Object?> value) {
  final stability = _requiredString(value['stability'], 'node.stability');
  if (stability != 'provisional' && stability != 'stable') {
    throw invalidBindingPayload('unknown node stability $stability');
  }
  final content = _requiredRecord(value['content'], 'node.content');
  _requiredString(content['kind'], 'node.content.kind');
  return ContentNodeView(
    id: requireDecimalString(value['id'], 'node.id'),
    version: _requiredString(value['version'], 'node.version'),
    stability: stability,
    source: _decodeRange(_requiredRecord(value['source'], 'node.source')),
    body: _decodeRange(_requiredRecord(value['body'], 'node.body')),
    children: _decodeChildList(
      _requiredRecord(value['children'], 'node.children'),
    ),
    content: content,
  );
}

SourceRangeView _decodeRange(Map<String, Object?> value) => SourceRangeView(
  start: requireDecimalString(value['start'], 'range.start'),
  end: requireDecimalString(value['end'], 'range.end'),
);

ChildListView _decodeChildList(Map<String, Object?> value) => ChildListView(
  version: _requiredString(value['version'], 'child_list.version'),
  children: _decimalArray(value['children'], 'child_list.children'),
);

ResourceView _decodeResourceView(Map<String, Object?> value, String schema) =>
    ResourceView._(
      schema: schema,
      raw: value,
      resource: _decodeResource(_requiredRecord(value['resource'], 'resource')),
    );

SemanticResourceView _decodeResource(Map<String, Object?> value) {
  final content = _requiredRecord(value['content'], 'resource.content');
  _requiredString(content['kind'], 'resource.content.kind');
  return SemanticResourceView(
    id: requireDecimalString(value['id'], 'resource.id'),
    version: _requiredString(value['version'], 'resource.version'),
    content: content,
  );
}

ProcessorRequestView _decodeProcessorRequest(
  Map<String, Object?> value,
  String schema,
) {
  final input = _requiredRecord(value['input'], 'processor input');
  return ProcessorRequestView._(
    schema: schema,
    raw: value,
    requestId: requireDecimalString(value['request_id'], 'request_id'),
    key: _decodeProcessorKey(_requiredRecord(value['key'], 'processor key')),
    input: ProcessorInputView(
      node: _decodeNode(_requiredRecord(input['node'], 'processor input node')),
      body: _requiredString(input['body'], 'processor input body'),
      resource: input['resource'] == null
          ? null
          : _decodeResource(
              _requiredRecord(input['resource'], 'processor input resource'),
            ),
    ),
  );
}

ProcessorCompletionView _decodeProcessorCompletion(
  Map<String, Object?> value,
  String schema,
) {
  final outcome = _requiredString(
    value['outcome'],
    'processor completion outcome',
  );
  if (outcome != 'applied' && outcome != 'stale') {
    throw invalidBindingPayload(
      'unknown processor completion outcome $outcome',
    );
  }
  return ProcessorCompletionView._(
    schema: schema,
    raw: value,
    requestId: requireDecimalString(value['request_id'], 'request_id'),
    outcome: outcome,
  );
}

ProcessorKeyView _decodeProcessorKey(Map<String, Object?> value) =>
    ProcessorKeyView(
      epoch: requireDecimalString(value['epoch'], 'processor key epoch'),
      nodeId: requireDecimalString(value['node_id'], 'processor key node_id'),
      processorId: _requiredString(
        value['processor_id'],
        'processor key processor_id',
      ),
      nodeVersion: _requiredString(
        value['node_version'],
        'processor key node_version',
      ),
      inputVersion: _requiredString(
        value['input_version'],
        'processor key input_version',
      ),
      processorVersion: _requiredString(
        value['processor_version'],
        'processor key processor_version',
      ),
      configurationVersion: _requiredString(
        value['configuration_version'],
        'processor key configuration_version',
      ),
      generation: requireDecimalString(
        value['generation'],
        'processor key generation',
      ),
    );

ArtifactChangeView _decodeArtifactChange(
  Map<String, Object?> value,
  String schema,
) {
  final change = _requiredRecord(value['change'], 'artifact change');
  final kind = _requiredString(change['kind'], 'artifact change kind');
  final ArtifactChangeKindView decoded;
  switch (kind) {
    case 'pending':
      decoded = ArtifactChangeKindView(kind: kind);
    case 'ready':
      decoded = ArtifactChangeKindView(
        kind: kind,
        artifactBytes: requireDecimalString(
          change['artifact_bytes'],
          'artifact_bytes',
        ),
      );
    case 'failed':
      decoded = ArtifactChangeKindView(
        kind: kind,
        code: _failureCode(change['code']),
      );
    case 'removed':
      decoded = ArtifactChangeKindView(
        kind: kind,
        reason: _requiredString(change['reason'], 'artifact removal reason'),
        releasedArtifactBytes: requireDecimalString(
          change['released_artifact_bytes'],
          'released_artifact_bytes',
        ),
      );
    default:
      throw invalidBindingPayload('unknown artifact change $kind');
  }
  return ArtifactChangeView._(
    schema: schema,
    raw: value,
    key: _decodeProcessorKey(_requiredRecord(value['key'], 'artifact key')),
    change: decoded,
  );
}

ArtifactView _decodeArtifactView(Map<String, Object?> value, String schema) {
  final state = _requiredString(value['state'], 'artifact state');
  if (state != 'pending' && state != 'ready' && state != 'failed') {
    throw invalidBindingPayload('unknown artifact state $state');
  }
  return ArtifactView._(
    schema: schema,
    raw: value,
    key: _decodeProcessorKey(_requiredRecord(value['key'], 'artifact key')),
    state: state,
    artifact: value['artifact'] == null
        ? null
        : _decodeArtifact(_requiredRecord(value['artifact'], 'artifact')),
    failure: value['failure'] == null
        ? null
        : _decodeFailure(_requiredRecord(value['failure'], 'artifact failure')),
  );
}

ProcessorArtifactView _decodeArtifact(Map<String, Object?> value) {
  final payload = _requiredRecord(value['payload'], 'artifact payload');
  final kind = _requiredString(payload['kind'], 'artifact payload kind');
  final ArtifactPayloadView decoded;
  switch (kind) {
    case 'text':
      decoded = ArtifactPayloadView(
        kind: kind,
        text: _requiredString(payload['text'], 'artifact text'),
      );
    case 'binary':
      final octets = _requiredList(payload['bytes'], 'artifact bytes')
          .map((value) {
            if (value is! int || value < 0 || value > 255) {
              throw invalidBindingPayload('artifact bytes must contain octets');
            }
            return value;
          })
          .toList(growable: false);
      decoded = ArtifactPayloadView(
        kind: kind,
        bytes: List<int>.unmodifiable(octets),
      );
    case 'citation':
      decoded = ArtifactPayloadView(
        kind: kind,
        key: _requiredString(payload['key'], 'citation key'),
        destination: _requiredString(
          payload['destination'],
          'citation destination',
        ),
        title: payload['title'] == null
            ? null
            : _requiredString(payload['title'], 'citation title'),
      );
    default:
      throw invalidBindingPayload('unknown artifact payload $kind');
  }
  return ProcessorArtifactView(
    protocol: _requiredString(value['protocol'], 'artifact protocol'),
    mediaType: _requiredString(value['media_type'], 'artifact media_type'),
    payload: decoded,
  );
}

ArtifactFailureView _decodeFailure(Map<String, Object?> value) =>
    ArtifactFailureView(
      code: _failureCode(value['code']),
      message: _requiredString(value['message'], 'failure message'),
    );

String _failureCode(Object? value) {
  final code = _requiredString(value, 'processor failure code');
  if (!_processorFailureCodes.contains(code)) {
    throw invalidBindingPayload('unknown processor failure code $code');
  }
  return code;
}

Map<String, Object?> _parseJsonRecord(List<int> bytes, String field) {
  try {
    final decoded = jsonDecode(utf8.decode(bytes, allowMalformed: false));
    final frozen = _freezeJson(decoded);
    return _requiredRecord(frozen, field);
  } catch (error) {
    if (error is MdstreamException) {
      rethrow;
    }
    throw invalidBindingPayload('$field is not valid UTF-8 JSON', error);
  }
}

Object? _freezeJson(Object? value) {
  if (value == null || value is String || value is bool || value is num) {
    return value;
  }
  if (value is List) {
    return List<Object?>.unmodifiable(value.map(_freezeJson));
  }
  if (value is Map) {
    final result = <String, Object?>{};
    for (final entry in value.entries) {
      if (entry.key is! String) {
        throw invalidBindingPayload('JSON object keys must be strings');
      }
      result[entry.key as String] = _freezeJson(entry.value);
    }
    return Map<String, Object?>.unmodifiable(result);
  }
  throw invalidBindingPayload('JSON contains an unsupported value');
}

Map<String, Object?> _requiredRecord(Object? value, String field) {
  if (value is! Map<String, Object?>) {
    throw invalidBindingPayload('$field must be an object');
  }
  return value;
}

List<Object?> _requiredList(Object? value, String field) {
  if (value is! List<Object?>) {
    throw invalidBindingPayload('$field must be an array');
  }
  return value;
}

String _requiredString(Object? value, String field) {
  if (value is! String) {
    throw invalidBindingPayload('$field must be a string');
  }
  return value;
}

bool _requiredBoolean(Object? value, String field) {
  if (value is! bool) {
    throw invalidBindingPayload('$field must be a boolean');
  }
  return value;
}

void _requireLiteral(Object? value, String expected, String field) {
  if (value != expected) {
    throw invalidBindingPayload('$field must be $expected');
  }
}

List<String> _decimalArray(Object? value, String field) =>
    List<String>.unmodifiable(
      _requiredList(
        value,
        field,
      ).map((entry) => requireDecimalString(entry, field)),
    );
