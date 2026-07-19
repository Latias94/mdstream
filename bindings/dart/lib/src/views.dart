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

/// Continuity-qualified identity for one content node.
final class TransitionNodeKeyView {
  const TransitionNodeKeyView._({
    required this.continuityGeneration,
    required this.epoch,
    required this.nodeId,
  });

  final ContinuityGeneration continuityGeneration;
  final Epoch epoch;
  final NodeId nodeId;
}

/// Continuity-qualified identity for one semantic resource.
final class TransitionResourceKeyView {
  const TransitionResourceKeyView._({
    required this.continuityGeneration,
    required this.epoch,
    required this.resourceId,
  });

  final ContinuityGeneration continuityGeneration;
  final Epoch epoch;
  final ResourceId resourceId;
}

/// Owner of a versioned child list affected by one transition.
sealed class TransitionChildListOwnerView {
  const TransitionChildListOwnerView._(this.kind);

  final String kind;
}

/// The document root child list.
final class DocumentTransitionOwnerView extends TransitionChildListOwnerView {
  const DocumentTransitionOwnerView._() : super._('document');
}

/// A child list owned by a continuity-qualified node.
final class NodeTransitionOwnerView extends TransitionChildListOwnerView {
  const NodeTransitionOwnerView._(this.key) : super._('node');

  final TransitionNodeKeyView key;
}

/// Stable document fields before or after a reducer commit.
final class DocumentStateStampView {
  const DocumentStateStampView._({
    required this.continuityGeneration,
    required this.coordinate,
    required this.lifecycle,
    required this.projectionCursor,
    required this.rootsVersion,
  });

  final ContinuityGeneration continuityGeneration;
  final CoordinateView coordinate;
  final String lifecycle;
  final SourceCursor projectionCursor;
  final StructureVersion rootsVersion;
}

/// Stable node fields before or after a reducer commit.
final class NodeStateStampView {
  const NodeStateStampView._({
    required this.version,
    required this.stability,
    required this.parent,
    required this.childrenVersion,
  });

  final NodeVersion version;
  final String stability;
  final TransitionChildListOwnerView? parent;
  final StructureVersion childrenVersion;
}

/// Owned text delta attached to a node transition.
sealed class TextTransitionView {
  const TextTransitionView._(this.kind);

  final String kind;
}

/// Append-only projected source text retained by the transition record.
final class ProjectionAppendTransitionView extends TextTransitionView {
  const ProjectionAppendTransitionView._({
    required this.range,
    required this.text,
  }) : super._('projection_append');

  final SourceRangeView range;
  final String text;
}

/// A semantic replacement whose full text is available from the tail view.
final class ReplacementTextTransitionView extends TextTransitionView {
  const ReplacementTextTransitionView._() : super._('replacement');
}

/// Before/after stamps for one continuity-qualified node.
final class NodeTransitionView {
  const NodeTransitionView._({
    required this.key,
    required this.before,
    required this.after,
    required this.text,
  });

  final TransitionNodeKeyView key;
  final NodeStateStampView? before;
  final NodeStateStampView? after;
  final TextTransitionView? text;
}

/// One ordered splice against a versioned child list.
final class StructureTransitionView {
  const StructureTransitionView._({
    required this.owner,
    required this.beforeVersion,
    required this.afterVersion,
    required this.start,
    required this.removed,
    required this.inserted,
  });

  final TransitionChildListOwnerView owner;
  final StructureVersion beforeVersion;
  final StructureVersion afterVersion;
  final int start;
  final List<TransitionNodeKeyView> removed;
  final List<TransitionNodeKeyView> inserted;
}

/// Before/after version and affected nodes for one semantic resource.
final class ResourceTransitionView {
  const ResourceTransitionView._({
    required this.key,
    required this.beforeVersion,
    required this.afterVersion,
    required this.affectedNodes,
  });

  final TransitionResourceKeyView key;
  final ResourceVersion? beforeVersion;
  final ResourceVersion? afterVersion;
  final List<TransitionNodeKeyView> affectedNodes;
}

/// Ordered transition facts produced by one reducer update.
sealed class TransitionFactsView {
  const TransitionFactsView._({
    required this.scope,
    required this.before,
    required this.after,
  });

  final String scope;
  final DocumentStateStampView? before;
  final DocumentStateStampView after;
}

/// Incremental facts preserving every changed entity and owned text delta.
final class ContinuousTransitionFactsView extends TransitionFactsView {
  const ContinuousTransitionFactsView._({
    required super.before,
    required super.after,
    required this.nodes,
    required this.structures,
    required this.resources,
  }) : super._(scope: 'continuous');

  final List<NodeTransitionView> nodes;
  final List<StructureTransitionView> structures;
  final List<ResourceTransitionView> resources;
}

/// Coarse facts emitted when advanced recovery replaces continuity.
final class FullReplaceTransitionFactsView extends TransitionFactsView {
  const FullReplaceTransitionFactsView._({
    required super.before,
    required super.after,
  }) : super._(scope: 'full_replace');
}

/// Versioned transition payload embedded in a reducer update.
final class TransitionEnvelopeView {
  const TransitionEnvelopeView._({required this.schema, required this.facts});

  final String schema;
  final TransitionFactsView facts;
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
    required this.transition,
  }) : super(kind: 'reducer_update');

  final ApplyOutcomeView outcome;
  final ReducerStatusView status;
  final ChangeImpactView impact;
  final DocumentSummaryView? document;
  final TransitionEnvelopeView? transition;
}

/// Half-open source range represented with exact decimal cursors.
final class SourceRangeView {
  const SourceRangeView({required this.start, required this.end});

  final SourceCursor start;
  final SourceCursor end;
}

/// Materialized source suffix not yet covered by the projected document.
final class PendingSourceView extends DecodedBindingView {
  const PendingSourceView._({
    required super.schema,
    required super.raw,
    required this.range,
    required this.text,
  }) : super(kind: 'pending_source_view');

  final SourceRangeView range;
  final String text;
}

enum TableAlignment { none, left, center, right }

enum LinkStyle {
  inline,
  reference,
  referenceUnknown,
  collapsed,
  collapsedUnknown,
  shortcut,
  shortcutUnknown,
  autolink,
  email,
}

enum BlockQuoteKind { plain, note, tip, important, warning, caution }

enum CodeFenceMarker { backtick, tilde }

enum CitationProtocol {
  mdstreamCitation1('mdstream.citation/1');

  const CitationProtocol(this.wireName);
  final String wireName;
}

sealed class SemanticTextView {
  const SemanticTextView(this.kind);
  final String kind;
}

final class SourceSemanticTextView extends SemanticTextView {
  const SourceSemanticTextView() : super('source');
}

final class NormalizedSemanticTextView extends SemanticTextView {
  const NormalizedSemanticTextView(this.value) : super('normalized');
  final String value;
}

sealed class CodeBlockSyntaxView {
  const CodeBlockSyntaxView(this.kind);
  final String kind;
}

final class IndentedCodeBlockSyntaxView extends CodeBlockSyntaxView {
  const IndentedCodeBlockSyntaxView() : super('indented');
}

final class FencedCodeBlockSyntaxView extends CodeBlockSyntaxView {
  const FencedCodeBlockSyntaxView({required this.marker, required this.length})
    : super('fenced');
  final CodeFenceMarker marker;
  final int length;
}

final class ResourceRefView {
  const ResourceRefView({required this.id, required this.version});
  final ResourceId id;
  final ResourceVersion version;
}

sealed class ContentKindView {
  const ContentKindView(this.kind);
  final String kind;
}

final class ParagraphContentView extends ContentKindView {
  const ParagraphContentView() : super('paragraph');
}

final class HeadingContentView extends ContentKindView {
  const HeadingContentView(this.level) : super('heading');
  final int level;
}

final class TextContentView extends ContentKindView {
  const TextContentView(this.text) : super('text');
  final SemanticTextView text;
}

final class EmphasisContentView extends ContentKindView {
  const EmphasisContentView() : super('emphasis');
}

final class StrongContentView extends ContentKindView {
  const StrongContentView() : super('strong');
}

final class StrikethroughContentView extends ContentKindView {
  const StrikethroughContentView() : super('strikethrough');
}

final class LinkContentView extends ContentKindView {
  const LinkContentView({
    required this.target,
    required this.referenceLabel,
    required this.style,
  }) : super('link');
  final ResourceRefView? target;
  final String? referenceLabel;
  final LinkStyle style;
}

final class ImageContentView extends ContentKindView {
  const ImageContentView({
    required this.target,
    required this.referenceLabel,
    required this.style,
    required this.alt,
  }) : super('image');
  final ResourceRefView? target;
  final String? referenceLabel;
  final LinkStyle style;
  final SemanticTextView alt;
}

final class InlineCodeContentView extends ContentKindView {
  const InlineCodeContentView(this.text) : super('inline_code');
  final SemanticTextView text;
}

final class CodeBlockContentView extends ContentKindView {
  const CodeBlockContentView({
    required this.syntax,
    required this.info,
    required this.text,
  }) : super('code_block');
  final CodeBlockSyntaxView syntax;
  final String? info;
  final SemanticTextView text;
}

final class ListContentView extends ContentKindView {
  const ListContentView({
    required this.ordered,
    required this.start,
    required this.tight,
  }) : super('list');
  final bool ordered;
  final int? start;
  final bool tight;
}

final class ListItemContentView extends ContentKindView {
  const ListItemContentView(this.checked) : super('list_item');
  final bool? checked;
}

final class BlockQuoteContentView extends ContentKindView {
  const BlockQuoteContentView(this.style) : super('block_quote');
  final BlockQuoteKind style;
}

final class ThematicBreakContentView extends ContentKindView {
  const ThematicBreakContentView() : super('thematic_break');
}

final class TableContentView extends ContentKindView {
  const TableContentView(this.alignments) : super('table');
  final List<TableAlignment> alignments;
}

final class TableHeadContentView extends ContentKindView {
  const TableHeadContentView() : super('table_head');
}

final class TableBodyContentView extends ContentKindView {
  const TableBodyContentView() : super('table_body');
}

final class TableRowContentView extends ContentKindView {
  const TableRowContentView() : super('table_row');
}

final class TableCellContentView extends ContentKindView {
  const TableCellContentView(this.column) : super('table_cell');
  final int column;
}

final class HtmlContentView extends ContentKindView {
  const HtmlContentView({required this.block, required this.text})
    : super('html');
  final bool block;
  final SemanticTextView text;
}

final class MathContentView extends ContentKindView {
  const MathContentView({required this.display, required this.text})
    : super('math');
  final bool display;
  final SemanticTextView text;
}

final class FootnoteDefinitionContentView extends ContentKindView {
  const FootnoteDefinitionContentView({
    required this.label,
    required this.target,
  }) : super('footnote_definition');
  final String label;
  final ResourceRefView target;
}

final class FootnoteReferenceContentView extends ContentKindView {
  const FootnoteReferenceContentView({
    required this.label,
    required this.target,
  }) : super('footnote_reference');
  final String label;
  final ResourceRefView? target;
}

final class CitationDefinitionContentView extends ContentKindView {
  const CitationDefinitionContentView({required this.key, required this.target})
    : super('citation_definition');
  final String key;
  final ResourceRefView target;
}

final class CitationReferenceContentView extends ContentKindView {
  const CitationReferenceContentView({required this.key, required this.target})
    : super('citation_reference');
  final String key;
  final ResourceRefView? target;
}

final class SoftBreakContentView extends ContentKindView {
  const SoftBreakContentView() : super('soft_break');
}

final class HardBreakContentView extends ContentKindView {
  const HardBreakContentView() : super('hard_break');
}

final class CustomContentView extends ContentKindView {
  const CustomContentView({
    required this.namespace,
    required this.name,
    required this.opaque,
    required this.attributes,
  }) : super('custom');
  final String namespace;
  final String name;
  final bool opaque;
  final Map<String, String> attributes;
}

sealed class SemanticResourceKindView {
  const SemanticResourceKindView(this.kind);
  final String kind;
}

final class LinkResourceContentView extends SemanticResourceKindView {
  const LinkResourceContentView({
    required this.destination,
    required this.title,
  }) : super('link');
  final String destination;
  final String? title;
}

final class FootnoteResourceContentView extends SemanticResourceKindView {
  const FootnoteResourceContentView(this.label) : super('footnote');
  final String label;
}

final class CitationResourceContentView extends SemanticResourceKindView {
  const CitationResourceContentView({
    required this.protocol,
    required this.key,
    required this.destination,
    required this.title,
  }) : super('citation');
  final CitationProtocol protocol;
  final String key;
  final String destination;
  final String? title;
}

/// Typed stable node envelope with exhaustive content metadata.
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
  final ContentKindView content;
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

/// Typed stable semantic resource with exhaustive content metadata.
final class SemanticResourceView {
  const SemanticResourceView({
    required this.id,
    required this.version,
    required this.content,
  });

  final ResourceId id;
  final ResourceVersion version;
  final SemanticResourceKindView content;
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
final _opaqueIdentifierPattern = RegExp(r'^[A-Za-z0-9._:-]{1,128}$');

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
    case BindingPayloadKind.pendingSourceView:
      return _decodePendingSourceView(record, expectedSchema);
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
) {
  final hasTransition = value.containsKey('transition');
  _exactKeys(value, {
    'schema',
    'kind',
    'outcome',
    'status',
    'impact',
    'document',
    if (hasTransition) 'transition',
  }, 'reducer update');
  _requireOwnKey(value, 'document', 'document');
  return ReducerUpdateView._(
    schema: schema,
    raw: value,
    outcome: _decodeOutcome(_requiredRecord(value['outcome'], 'outcome')),
    status: _decodeStatus(_requiredRecord(value['status'], 'status')),
    impact: _decodeImpact(_requiredRecord(value['impact'], 'impact')),
    document: value['document'] == null
        ? null
        : _decodeDocument(_requiredRecord(value['document'], 'document')),
    transition: hasTransition
        ? _decodeTransitionEnvelope(
            _requiredRecord(value['transition'], 'transition'),
          )
        : null,
  );
}

TransitionEnvelopeView _decodeTransitionEnvelope(Map<String, Object?> value) {
  _exactKeys(value, {'schema', 'facts'}, 'transition');
  _requireLiteral(value['schema'], transitionSchema, 'transition.schema');
  return TransitionEnvelopeView._(
    schema: transitionSchema,
    facts: _decodeTransitionFacts(
      _requiredRecord(value['facts'], 'transition.facts'),
    ),
  );
}

TransitionFactsView _decodeTransitionFacts(Map<String, Object?> value) {
  final scope = _requiredString(value['scope'], 'transition.facts.scope');
  final before = _requiredNullableRecord(
    value,
    'before',
    'transition.facts.before',
  );
  final after = _decodeDocumentStateStamp(
    _requiredRecord(value['after'], 'transition.facts.after'),
  );
  switch (scope) {
    case 'continuous':
      _exactKeys(value, {
        'scope',
        'before',
        'after',
        'nodes',
        'structures',
        'resources',
      }, 'continuous transition facts');
      return ContinuousTransitionFactsView._(
        before: before == null ? null : _decodeDocumentStateStamp(before),
        after: after,
        nodes: List<NodeTransitionView>.unmodifiable(
          _requiredList(value['nodes'], 'transition.facts.nodes').map(
            (node) =>
                _decodeNodeTransition(_requiredRecord(node, 'transition node')),
          ),
        ),
        structures: List<StructureTransitionView>.unmodifiable(
          _requiredList(value['structures'], 'transition.facts.structures').map(
            (structure) => _decodeStructureTransition(
              _requiredRecord(structure, 'structure transition'),
            ),
          ),
        ),
        resources: List<ResourceTransitionView>.unmodifiable(
          _requiredList(value['resources'], 'transition.facts.resources').map(
            (resource) => _decodeResourceTransition(
              _requiredRecord(resource, 'resource transition'),
            ),
          ),
        ),
      );
    case 'full_replace':
      _exactKeys(value, {
        'scope',
        'before',
        'after',
      }, 'full-replace transition facts');
      return FullReplaceTransitionFactsView._(
        before: before == null ? null : _decodeDocumentStateStamp(before),
        after: after,
      );
    default:
      throw invalidBindingPayload('unknown transition scope $scope');
  }
}

DocumentStateStampView _decodeDocumentStateStamp(Map<String, Object?> value) {
  _exactKeys(value, {
    'continuity_generation',
    'coordinate',
    'lifecycle',
    'projection_cursor',
    'roots_version',
  }, 'transition document stamp');
  final lifecycle = _requiredString(
    value['lifecycle'],
    'transition document stamp lifecycle',
  );
  if (lifecycle != 'open' && lifecycle != 'finalized') {
    throw invalidBindingPayload(
      'unknown transition document lifecycle $lifecycle',
    );
  }
  return DocumentStateStampView._(
    continuityGeneration: decodeDecimalU64(
      value['continuity_generation'],
      'continuity_generation',
    ),
    coordinate: _decodeTransitionCoordinate(
      _requiredRecord(
        value['coordinate'],
        'transition document stamp coordinate',
      ),
    ),
    lifecycle: lifecycle,
    projectionCursor: decodeDecimalU64(
      value['projection_cursor'],
      'projection_cursor',
    ),
    rootsVersion: _opaqueIdentifier(value['roots_version'], 'roots_version'),
  );
}

CoordinateView _decodeTransitionCoordinate(Map<String, Object?> value) {
  _exactKeys(value, {
    'epoch',
    'sequence',
    'change_id',
    'source_cursor',
  }, 'transition document stamp coordinate');
  return CoordinateView(
    epoch: decodeDecimalU64(value['epoch'], 'coordinate.epoch'),
    sequence: decodeDecimalU64(value['sequence'], 'coordinate.sequence'),
    changeId: _opaqueIdentifier(value['change_id'], 'coordinate.change_id'),
    sourceCursor: decodeDecimalU64(
      value['source_cursor'],
      'coordinate.source_cursor',
    ),
  );
}

TransitionNodeKeyView _decodeTransitionNodeKey(Map<String, Object?> value) {
  _exactKeys(value, {
    'continuity_generation',
    'epoch',
    'node_id',
  }, 'transition node key');
  return TransitionNodeKeyView._(
    continuityGeneration: decodeDecimalU64(
      value['continuity_generation'],
      'continuity_generation',
    ),
    epoch: decodeDecimalU64(value['epoch'], 'transition node epoch'),
    nodeId: decodeDecimalU128(value['node_id'], 'transition node id'),
  );
}

TransitionResourceKeyView _decodeTransitionResourceKey(
  Map<String, Object?> value,
) {
  _exactKeys(value, {
    'continuity_generation',
    'epoch',
    'resource_id',
  }, 'transition resource key');
  return TransitionResourceKeyView._(
    continuityGeneration: decodeDecimalU64(
      value['continuity_generation'],
      'continuity_generation',
    ),
    epoch: decodeDecimalU64(value['epoch'], 'transition resource epoch'),
    resourceId: decodeDecimalU128(
      value['resource_id'],
      'transition resource id',
    ),
  );
}

TransitionChildListOwnerView _decodeTransitionOwner(
  Map<String, Object?> value,
) {
  final kind = _requiredString(value['kind'], 'transition owner kind');
  switch (kind) {
    case 'document':
      _exactKeys(value, {'kind'}, 'document transition owner');
      return const DocumentTransitionOwnerView._();
    case 'node':
      _exactKeys(value, {'kind', 'key'}, 'node transition owner');
      return NodeTransitionOwnerView._(
        _decodeTransitionNodeKey(
          _requiredRecord(value['key'], 'transition owner node key'),
        ),
      );
    default:
      throw invalidBindingPayload('unknown transition owner $kind');
  }
}

NodeStateStampView _decodeNodeStateStamp(Map<String, Object?> value) {
  _exactKeys(value, {
    'version',
    'stability',
    'parent',
    'children_version',
  }, 'transition node stamp');
  final stability = _requiredString(
    value['stability'],
    'transition node stability',
  );
  if (stability != 'provisional' && stability != 'stable') {
    throw invalidBindingPayload('unknown transition node stability $stability');
  }
  final parent = _requiredNullableRecord(
    value,
    'parent',
    'transition node parent',
  );
  return NodeStateStampView._(
    version: _opaqueIdentifier(value['version'], 'transition node version'),
    stability: stability,
    parent: parent == null ? null : _decodeTransitionOwner(parent),
    childrenVersion: _opaqueIdentifier(
      value['children_version'],
      'transition children version',
    ),
  );
}

TextTransitionView _decodeTextTransition(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'text transition kind');
  switch (kind) {
    case 'projection_append':
      _exactKeys(value, {
        'kind',
        'range',
        'text',
      }, 'projection-append transition');
      final range = _requiredRecord(value['range'], 'projection-append range');
      _exactKeys(range, {'start', 'end'}, 'projection-append range');
      return ProjectionAppendTransitionView._(
        range: _decodeRange(range),
        text: _requiredString(value['text'], 'projection-append text'),
      );
    case 'replacement':
      _exactKeys(value, {'kind'}, 'replacement transition');
      return const ReplacementTextTransitionView._();
    default:
      throw invalidBindingPayload('unknown text transition $kind');
  }
}

NodeTransitionView _decodeNodeTransition(Map<String, Object?> value) {
  _exactKeys(value, {'key', 'before', 'after', 'text'}, 'node transition');
  final before = _requiredNullableRecord(
    value,
    'before',
    'node transition before',
  );
  final after = _requiredNullableRecord(
    value,
    'after',
    'node transition after',
  );
  final text = _requiredNullableRecord(value, 'text', 'node text transition');
  return NodeTransitionView._(
    key: _decodeTransitionNodeKey(
      _requiredRecord(value['key'], 'node transition key'),
    ),
    before: before == null ? null : _decodeNodeStateStamp(before),
    after: after == null ? null : _decodeNodeStateStamp(after),
    text: text == null ? null : _decodeTextTransition(text),
  );
}

StructureTransitionView _decodeStructureTransition(Map<String, Object?> value) {
  _exactKeys(value, {
    'owner',
    'before_version',
    'after_version',
    'start',
    'removed',
    'inserted',
  }, 'structure transition');
  return StructureTransitionView._(
    owner: _decodeTransitionOwner(
      _requiredRecord(value['owner'], 'structure transition owner'),
    ),
    beforeVersion: _opaqueIdentifier(
      value['before_version'],
      'structure before version',
    ),
    afterVersion: _opaqueIdentifier(
      value['after_version'],
      'structure after version',
    ),
    start: _requiredUnsignedInteger(
      value['start'],
      'structure transition start',
      0xffffffff,
    ),
    removed: _decodeTransitionNodeKeyArray(
      value['removed'],
      'removed transition nodes',
    ),
    inserted: _decodeTransitionNodeKeyArray(
      value['inserted'],
      'inserted transition nodes',
    ),
  );
}

ResourceTransitionView _decodeResourceTransition(Map<String, Object?> value) {
  _exactKeys(value, {
    'key',
    'before_version',
    'after_version',
    'affected_nodes',
  }, 'resource transition');
  return ResourceTransitionView._(
    key: _decodeTransitionResourceKey(
      _requiredRecord(value['key'], 'resource transition key'),
    ),
    beforeVersion: _requiredNullableVersion(
      value,
      'before_version',
      'resource before version',
    ),
    afterVersion: _requiredNullableVersion(
      value,
      'after_version',
      'resource after version',
    ),
    affectedNodes: _decodeTransitionNodeKeyArray(
      value['affected_nodes'],
      'resource affected nodes',
    ),
  );
}

List<TransitionNodeKeyView> _decodeTransitionNodeKeyArray(
  Object? value,
  String field,
) => List<TransitionNodeKeyView>.unmodifiable(
  _requiredList(
    value,
    field,
  ).map((entry) => _decodeTransitionNodeKey(_requiredRecord(entry, field))),
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
        receivedEpoch: decodeDecimalU64(
          value['received_epoch'],
          'received_epoch',
        ),
        receivedSequence: decodeDecimalU64(
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
  changedNodeIds: _decimalU128Array(
    value['changed_node_ids'],
    'changed_node_ids',
  ),
  removedNodeIds: _decimalU128Array(
    value['removed_node_ids'],
    'removed_node_ids',
  ),
  changedResourceIds: _decimalU128Array(
    value['changed_resource_ids'],
    'changed_resource_ids',
  ),
  removedResourceIds: _decimalU128Array(
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
    projectionCursor: decodeDecimalU64(
      value['projection_cursor'],
      'projection_cursor',
    ),
    roots: value.containsKey('roots')
        ? _decodeChildList(_requiredRecord(value['roots'], 'document.roots'))
        : null,
  );
}

CoordinateView _decodeCoordinate(Map<String, Object?> value) => CoordinateView(
  epoch: decodeDecimalU64(value['epoch'], 'coordinate.epoch'),
  sequence: decodeDecimalU64(value['sequence'], 'coordinate.sequence'),
  changeId: _requiredString(value['change_id'], 'coordinate.change_id'),
  sourceCursor: decodeDecimalU64(
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
  return ContentNodeView(
    id: decodeDecimalU128(value['id'], 'node.id'),
    version: _requiredString(value['version'], 'node.version'),
    stability: stability,
    source: _decodeRange(_requiredRecord(value['source'], 'node.source')),
    body: _decodeRange(_requiredRecord(value['body'], 'node.body')),
    children: _decodeChildList(
      _requiredRecord(value['children'], 'node.children'),
    ),
    content: _decodeContentKind(
      _requiredRecord(value['content'], 'node.content'),
    ),
  );
}

ContentKindView _decodeContentKind(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'node.content.kind');
  switch (kind) {
    case 'paragraph':
      _exactKeys(value, {'kind'}, 'content paragraph');
      return const ParagraphContentView();
    case 'heading':
      _exactKeys(value, {'kind', 'level'}, 'content heading');
      return HeadingContentView(
        _requiredUnsignedInteger(value['level'], 'heading.level', 255),
      );
    case 'text':
      _exactKeys(value, {'kind', 'text'}, 'content text');
      return TextContentView(
        _decodeSemanticText(_requiredRecord(value['text'], 'text.text')),
      );
    case 'emphasis':
      _exactKeys(value, {'kind'}, 'content emphasis');
      return const EmphasisContentView();
    case 'strong':
      _exactKeys(value, {'kind'}, 'content strong');
      return const StrongContentView();
    case 'strikethrough':
      _exactKeys(value, {'kind'}, 'content strikethrough');
      return const StrikethroughContentView();
    case 'link':
      _exactKeys(value, {
        'kind',
        'target',
        'reference_label',
        'style',
      }, 'content link');
      return LinkContentView(
        target: _nullableResourceRef(value, 'target', 'link.target'),
        referenceLabel: _requiredNullableString(
          value,
          'reference_label',
          'link.reference_label',
        ),
        style: _linkStyle(value['style'], 'link.style'),
      );
    case 'image':
      _exactKeys(value, {
        'kind',
        'target',
        'reference_label',
        'style',
        'alt',
      }, 'content image');
      return ImageContentView(
        target: _nullableResourceRef(value, 'target', 'image.target'),
        referenceLabel: _requiredNullableString(
          value,
          'reference_label',
          'image.reference_label',
        ),
        style: _linkStyle(value['style'], 'image.style'),
        alt: _decodeSemanticText(_requiredRecord(value['alt'], 'image.alt')),
      );
    case 'inline_code':
      _exactKeys(value, {'kind', 'text'}, 'content inline_code');
      return InlineCodeContentView(
        _decodeSemanticText(_requiredRecord(value['text'], 'inline_code.text')),
      );
    case 'code_block':
      _exactKeys(value, {
        'kind',
        'syntax',
        'info',
        'text',
      }, 'content code_block');
      return CodeBlockContentView(
        syntax: _decodeCodeBlockSyntax(
          _requiredRecord(value['syntax'], 'code_block.syntax'),
        ),
        info: _requiredNullableString(value, 'info', 'code_block.info'),
        text: _decodeSemanticText(
          _requiredRecord(value['text'], 'code_block.text'),
        ),
      );
    case 'list':
      _exactKeys(value, {'kind', 'ordered', 'start', 'tight'}, 'content list');
      return ListContentView(
        ordered: _requiredBoolean(value['ordered'], 'list.ordered'),
        start: _requiredNullableInteger(
          value,
          'start',
          'list.start',
          0xffffffff,
        ),
        tight: _requiredBoolean(value['tight'], 'list.tight'),
      );
    case 'list_item':
      _exactKeys(value, {'kind', 'checked'}, 'content list_item');
      return ListItemContentView(
        _requiredNullableBoolean(value, 'checked', 'list_item.checked'),
      );
    case 'block_quote':
      _exactKeys(value, {'kind', 'style'}, 'content block_quote');
      return BlockQuoteContentView(
        _blockQuoteKind(value['style'], 'block_quote.style'),
      );
    case 'thematic_break':
      _exactKeys(value, {'kind'}, 'content thematic_break');
      return const ThematicBreakContentView();
    case 'table':
      _exactKeys(value, {'kind', 'alignments'}, 'content table');
      return TableContentView(
        List<TableAlignment>.unmodifiable(
          _requiredList(
            value['alignments'],
            'table.alignments',
          ).map((alignment) => _tableAlignment(alignment, 'table.alignment')),
        ),
      );
    case 'table_head':
      _exactKeys(value, {'kind'}, 'content table_head');
      return const TableHeadContentView();
    case 'table_body':
      _exactKeys(value, {'kind'}, 'content table_body');
      return const TableBodyContentView();
    case 'table_row':
      _exactKeys(value, {'kind'}, 'content table_row');
      return const TableRowContentView();
    case 'table_cell':
      _exactKeys(value, {'kind', 'column'}, 'content table_cell');
      return TableCellContentView(
        _requiredUnsignedInteger(
          value['column'],
          'table_cell.column',
          0xffffffff,
        ),
      );
    case 'html':
      _exactKeys(value, {'kind', 'block', 'text'}, 'content html');
      return HtmlContentView(
        block: _requiredBoolean(value['block'], 'html.block'),
        text: _decodeSemanticText(_requiredRecord(value['text'], 'html.text')),
      );
    case 'math':
      _exactKeys(value, {'kind', 'display', 'text'}, 'content math');
      return MathContentView(
        display: _requiredBoolean(value['display'], 'math.display'),
        text: _decodeSemanticText(_requiredRecord(value['text'], 'math.text')),
      );
    case 'footnote_definition':
      _exactKeys(value, {
        'kind',
        'label',
        'target',
      }, 'content footnote_definition');
      return FootnoteDefinitionContentView(
        label: _requiredString(value['label'], 'footnote_definition.label'),
        target: _decodeResourceRef(
          _requiredRecord(value['target'], 'footnote_definition.target'),
        ),
      );
    case 'footnote_reference':
      _exactKeys(value, {
        'kind',
        'label',
        'target',
      }, 'content footnote_reference');
      return FootnoteReferenceContentView(
        label: _requiredString(value['label'], 'footnote_reference.label'),
        target: _nullableResourceRef(
          value,
          'target',
          'footnote_reference.target',
        ),
      );
    case 'citation_definition':
      _exactKeys(value, {
        'kind',
        'key',
        'target',
      }, 'content citation_definition');
      return CitationDefinitionContentView(
        key: _requiredString(value['key'], 'citation_definition.key'),
        target: _decodeResourceRef(
          _requiredRecord(value['target'], 'citation_definition.target'),
        ),
      );
    case 'citation_reference':
      _exactKeys(value, {
        'kind',
        'key',
        'target',
      }, 'content citation_reference');
      return CitationReferenceContentView(
        key: _requiredString(value['key'], 'citation_reference.key'),
        target: _nullableResourceRef(
          value,
          'target',
          'citation_reference.target',
        ),
      );
    case 'soft_break':
      _exactKeys(value, {'kind'}, 'content soft_break');
      return const SoftBreakContentView();
    case 'hard_break':
      _exactKeys(value, {'kind'}, 'content hard_break');
      return const HardBreakContentView();
    case 'custom':
      _exactKeys(value, {
        'kind',
        'namespace',
        'name',
        'opaque',
        'attributes',
      }, 'content custom');
      return CustomContentView(
        namespace: _requiredString(value['namespace'], 'custom.namespace'),
        name: _requiredString(value['name'], 'custom.name'),
        opaque: _requiredBoolean(value['opaque'], 'custom.opaque'),
        attributes: _stringRecord(value['attributes'], 'custom.attributes'),
      );
    default:
      throw invalidBindingPayload('unknown content kind $kind');
  }
}

SemanticTextView _decodeSemanticText(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'semantic text kind');
  switch (kind) {
    case 'source':
      _exactKeys(value, {'kind'}, 'semantic text source');
      return const SourceSemanticTextView();
    case 'normalized':
      _exactKeys(value, {'kind', 'value'}, 'semantic text normalized');
      return NormalizedSemanticTextView(
        _requiredString(value['value'], 'semantic text value'),
      );
    default:
      throw invalidBindingPayload('unknown semantic text kind $kind');
  }
}

CodeBlockSyntaxView _decodeCodeBlockSyntax(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'code block syntax kind');
  switch (kind) {
    case 'indented':
      _exactKeys(value, {'kind'}, 'indented code block syntax');
      return const IndentedCodeBlockSyntaxView();
    case 'fenced':
      _exactKeys(value, {
        'kind',
        'marker',
        'length',
      }, 'fenced code block syntax');
      return FencedCodeBlockSyntaxView(
        marker: _codeFenceMarker(value['marker'], 'code block fence marker'),
        length: _requiredUnsignedInteger(
          value['length'],
          'code block fence length',
          0xffffffff,
        ),
      );
    default:
      throw invalidBindingPayload('unknown code block syntax $kind');
  }
}

ResourceRefView _decodeResourceRef(Map<String, Object?> value) {
  _exactKeys(value, {'id', 'version'}, 'resource reference');
  return ResourceRefView(
    id: decodeDecimalU128(value['id'], 'resource reference id'),
    version: _requiredString(value['version'], 'resource reference version'),
  );
}

SourceRangeView _decodeRange(Map<String, Object?> value) => SourceRangeView(
  start: decodeDecimalU64(value['start'], 'range.start'),
  end: decodeDecimalU64(value['end'], 'range.end'),
);

PendingSourceView _decodePendingSourceView(
  Map<String, Object?> value,
  String schema,
) => PendingSourceView._(
  schema: schema,
  raw: value,
  range: _decodeRange(_requiredRecord(value['range'], 'pending source range')),
  text: _requiredString(value['text'], 'pending source text'),
);

ChildListView _decodeChildList(Map<String, Object?> value) => ChildListView(
  version: _requiredString(value['version'], 'child_list.version'),
  children: _decimalU128Array(value['children'], 'child_list.children'),
);

ResourceView _decodeResourceView(Map<String, Object?> value, String schema) =>
    ResourceView._(
      schema: schema,
      raw: value,
      resource: _decodeResource(_requiredRecord(value['resource'], 'resource')),
    );

SemanticResourceView _decodeResource(Map<String, Object?> value) {
  return SemanticResourceView(
    id: decodeDecimalU128(value['id'], 'resource.id'),
    version: _requiredString(value['version'], 'resource.version'),
    content: _decodeSemanticResourceKind(
      _requiredRecord(value['content'], 'resource.content'),
    ),
  );
}

SemanticResourceKindView _decodeSemanticResourceKind(
  Map<String, Object?> value,
) {
  final kind = _requiredString(value['kind'], 'resource.content.kind');
  switch (kind) {
    case 'link':
      _exactKeys(value, {'kind', 'destination', 'title'}, 'link resource');
      return LinkResourceContentView(
        destination: _requiredString(
          value['destination'],
          'link resource destination',
        ),
        title: _requiredNullableString(value, 'title', 'link resource title'),
      );
    case 'footnote':
      _exactKeys(value, {'kind', 'label'}, 'footnote resource');
      return FootnoteResourceContentView(
        _requiredString(value['label'], 'footnote resource label'),
      );
    case 'citation':
      _exactKeys(value, {
        'kind',
        'protocol',
        'key',
        'destination',
        'title',
      }, 'citation resource');
      _requireLiteral(
        value['protocol'],
        'mdstream.citation/1',
        'citation protocol',
      );
      return CitationResourceContentView(
        protocol: CitationProtocol.mdstreamCitation1,
        key: _requiredString(value['key'], 'citation resource key'),
        destination: _requiredString(
          value['destination'],
          'citation resource destination',
        ),
        title: _requiredNullableString(
          value,
          'title',
          'citation resource title',
        ),
      );
    default:
      throw invalidBindingPayload('unknown semantic resource kind $kind');
  }
}

ProcessorRequestView _decodeProcessorRequest(
  Map<String, Object?> value,
  String schema,
) {
  final input = _requiredRecord(value['input'], 'processor input');
  return ProcessorRequestView._(
    schema: schema,
    raw: value,
    requestId: decodeDecimalU64(value['request_id'], 'request_id'),
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
    requestId: decodeDecimalU64(value['request_id'], 'request_id'),
    outcome: outcome,
  );
}

ProcessorKeyView _decodeProcessorKey(Map<String, Object?> value) =>
    ProcessorKeyView(
      epoch: decodeDecimalU64(value['epoch'], 'processor key epoch'),
      nodeId: decodeDecimalU128(value['node_id'], 'processor key node_id'),
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
      generation: decodeDecimalU64(
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
        artifactBytes: decodeDecimalU64(
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
        releasedArtifactBytes: decodeDecimalU64(
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

ResourceRefView? _nullableResourceRef(
  Map<String, Object?> value,
  String key,
  String field,
) {
  _requireOwnKey(value, key, field);
  return value[key] == null
      ? null
      : _decodeResourceRef(_requiredRecord(value[key], field));
}

Map<String, Object?>? _requiredNullableRecord(
  Map<String, Object?> value,
  String key,
  String field,
) {
  _requireOwnKey(value, key, field);
  return value[key] == null ? null : _requiredRecord(value[key], field);
}

ResourceVersion? _requiredNullableVersion(
  Map<String, Object?> value,
  String key,
  String field,
) {
  _requireOwnKey(value, key, field);
  return value[key] == null ? null : _opaqueIdentifier(value[key], field);
}

String? _requiredNullableString(
  Map<String, Object?> value,
  String key,
  String field,
) {
  _requireOwnKey(value, key, field);
  return value[key] == null ? null : _requiredString(value[key], field);
}

bool? _requiredNullableBoolean(
  Map<String, Object?> value,
  String key,
  String field,
) {
  _requireOwnKey(value, key, field);
  return value[key] == null ? null : _requiredBoolean(value[key], field);
}

int? _requiredNullableInteger(
  Map<String, Object?> value,
  String key,
  String field,
  int maximum,
) {
  _requireOwnKey(value, key, field);
  return value[key] == null
      ? null
      : _requiredUnsignedInteger(value[key], field, maximum);
}

int _requiredUnsignedInteger(Object? value, String field, int maximum) {
  if (value is! int || value < 0 || value > maximum) {
    throw invalidBindingPayload(
      '$field must be an unsigned integer no greater than $maximum',
    );
  }
  return value;
}

Map<String, String> _stringRecord(Object? value, String field) {
  final source = _requiredRecord(value, field);
  final result = <String, String>{};
  for (final MapEntry(:key, :value) in source.entries) {
    result[key] = _requiredString(value, '$field.$key');
  }
  return Map<String, String>.unmodifiable(result);
}

TableAlignment _tableAlignment(Object? value, String field) {
  final alignment = _requiredString(value, field);
  return switch (alignment) {
    'none' => TableAlignment.none,
    'left' => TableAlignment.left,
    'center' => TableAlignment.center,
    'right' => TableAlignment.right,
    _ => throw invalidBindingPayload('unknown table alignment $alignment'),
  };
}

LinkStyle _linkStyle(Object? value, String field) {
  final style = _requiredString(value, field);
  return switch (style) {
    'inline' => LinkStyle.inline,
    'reference' => LinkStyle.reference,
    'reference_unknown' => LinkStyle.referenceUnknown,
    'collapsed' => LinkStyle.collapsed,
    'collapsed_unknown' => LinkStyle.collapsedUnknown,
    'shortcut' => LinkStyle.shortcut,
    'shortcut_unknown' => LinkStyle.shortcutUnknown,
    'autolink' => LinkStyle.autolink,
    'email' => LinkStyle.email,
    _ => throw invalidBindingPayload('unknown link style $style'),
  };
}

BlockQuoteKind _blockQuoteKind(Object? value, String field) {
  final kind = _requiredString(value, field);
  return switch (kind) {
    'plain' => BlockQuoteKind.plain,
    'note' => BlockQuoteKind.note,
    'tip' => BlockQuoteKind.tip,
    'important' => BlockQuoteKind.important,
    'warning' => BlockQuoteKind.warning,
    'caution' => BlockQuoteKind.caution,
    _ => throw invalidBindingPayload('unknown block quote kind $kind'),
  };
}

CodeFenceMarker _codeFenceMarker(Object? value, String field) {
  final marker = _requiredString(value, field);
  return switch (marker) {
    'backtick' => CodeFenceMarker.backtick,
    'tilde' => CodeFenceMarker.tilde,
    _ => throw invalidBindingPayload('unknown code fence marker $marker'),
  };
}

void _exactKeys(
  Map<String, Object?> value,
  Set<String> expected,
  String field,
) {
  for (final key in value.keys) {
    if (!expected.contains(key)) {
      throw invalidBindingPayload('$field contains unknown field $key');
    }
  }
}

void _requireOwnKey(Map<String, Object?> value, String key, String field) {
  if (!value.containsKey(key)) {
    throw invalidBindingPayload('$field is required');
  }
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

String _opaqueIdentifier(Object? value, String field) {
  final identifier = _requiredString(value, field);
  if (!_opaqueIdentifierPattern.hasMatch(identifier)) {
    throw invalidBindingPayload(
      '$field must be a 1-128 byte ASCII opaque identifier',
    );
  }
  return identifier;
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

List<String> _decimalU128Array(Object? value, String field) =>
    List<String>.unmodifiable(
      _requiredList(
        value,
        field,
      ).map((entry) => decodeDecimalU128(entry, field)),
    );
