import 'dart:convert';

import 'errors.dart';
import 'protocol.dart';

/// Base type for decoded, recursively immutable binding JSON views.
sealed class DecodedBindingView {
  /// Creates a decoded view for one binding [kind] and protocol [schema].
  const DecodedBindingView({
    required this.schema,
    required this.kind,
    required this.raw,
  });

  /// Binding schema that governed decoding of this payload.
  final String schema;

  /// Discriminator of the decoded binding payload.
  final String kind;

  /// Recursively immutable JSON object received from the native binding.
  final Map<String, Object?> raw;
}

/// Position of a successfully applied protocol change.
final class CoordinateView {
  /// Creates an exact coordinate in the canonical change stream.
  const CoordinateView({
    required this.epoch,
    required this.sequence,
    required this.changeId,
    required this.sourceCursor,
  });

  /// Recovery epoch containing the change.
  final Epoch epoch;

  /// Monotonic sequence within [epoch].
  final Sequence sequence;

  /// Stable opaque identity of the canonical change.
  final ChangeId changeId;

  /// Source byte cursor reached after applying the change.
  final SourceCursor sourceCursor;
}

/// Extensible reason for requiring an explicit recovery snapshot.
final class RecoveryReasonView {
  /// Creates an extensible recovery reason from its [kind] and raw fields.
  const RecoveryReasonView({required this.kind, required this.raw});

  /// Stable protocol discriminator for the recovery reason.
  final String kind;

  /// Recursively immutable reason object for forward-compatible inspection.
  final Map<String, Object?> raw;
}

/// Current reducer readiness state.
sealed class ReducerStatusView {
  const ReducerStatusView();
}

/// Reducer state before the first canonical change is applied.
final class UninitializedReducerStatusView extends ReducerStatusView {
  /// Creates the uninitialized reducer status.
  const UninitializedReducerStatusView();
}

/// Reducer state that can accept the next canonical change.
final class ReadyReducerStatusView extends ReducerStatusView {
  /// Creates the ready reducer status.
  const ReadyReducerStatusView();
}

/// Reducer state that requires an explicit recovery snapshot.
final class NeedsSnapshotReducerStatusView extends ReducerStatusView {
  /// Creates a recovery-required reducer status.
  const NeedsSnapshotReducerStatusView({
    required this.lastGood,
    required this.reason,
  });

  /// Last coordinate known to have been applied consistently.
  final CoordinateView lastGood;

  /// Reason continuity could not be preserved.
  final RecoveryReasonView reason;
}

/// Result of applying one canonical change or snapshot.
sealed class ApplyOutcomeView {
  const ApplyOutcomeView();
}

/// Outcome for a canonical change applied in normal continuity.
final class AppliedOutcomeView extends ApplyOutcomeView {
  /// Creates an applied outcome at [coordinate].
  const AppliedOutcomeView(this.coordinate);

  /// Coordinate assigned to the applied change.
  final CoordinateView coordinate;
}

/// Outcome for a recovery snapshot that restored continuity.
final class RecoveredOutcomeView extends ApplyOutcomeView {
  /// Creates a recovered outcome at [coordinate].
  const RecoveredOutcomeView(this.coordinate);

  /// Coordinate restored by the snapshot.
  final CoordinateView coordinate;
}

/// Outcome for a change that was already applied.
final class IdempotentOutcomeView extends ApplyOutcomeView {
  /// Creates an idempotent outcome.
  const IdempotentOutcomeView();
}

/// Outcome for a change older than the reducer's current coordinate.
final class StaleOutcomeView extends ApplyOutcomeView {
  /// Creates a stale outcome.
  const StaleOutcomeView({
    required this.current,
    required this.receivedEpoch,
    required this.receivedSequence,
  });

  /// Coordinate currently retained by the reducer.
  final CoordinateView current;

  /// Epoch carried by the rejected change.
  final Epoch receivedEpoch;

  /// Sequence carried by the rejected change.
  final Sequence receivedSequence;
}

/// Outcome for a change that cannot preserve reducer continuity.
final class RecoveryRequiredOutcomeView extends ApplyOutcomeView {
  /// Creates a recovery-required outcome.
  const RecoveryRequiredOutcomeView({
    required this.lastGood,
    required this.reason,
  });

  /// Last coordinate known to have been applied consistently.
  final CoordinateView lastGood;

  /// Reason an explicit recovery snapshot is required.
  final RecoveryReasonView reason;
}

/// Exact cache invalidations produced by one reducer operation.
final class ChangeImpactView {
  /// Creates the exact invalidation set for one reducer operation.
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

  /// Node identities whose materialized views changed.
  final List<NodeId> changedNodeIds;

  /// Node identities removed from the current document.
  final List<NodeId> removedNodeIds;

  /// Semantic resource identities whose materialized views changed.
  final List<ResourceId> changedResourceIds;

  /// Semantic resource identities removed from the current document.
  final List<ResourceId> removedResourceIds;

  /// Whether the retained source changed.
  final bool sourceChanged;

  /// Whether the projected source boundary changed.
  final bool projectionChanged;

  /// Whether the document lifecycle changed.
  final bool lifecycleChanged;

  /// Whether the document root child list changed.
  final bool rootsChanged;

  /// Whether continuity was replaced and all derived state must be rescanned.
  final bool fullReplace;
}

/// Versioned ordered child identities.
final class ChildListView {
  /// Creates an ordered child list at [version].
  const ChildListView({required this.version, required this.children});

  /// Structural version of this ordered child list.
  final StructureVersion version;

  /// Ordered identities of the child nodes.
  final List<NodeId> children;
}

/// Document fields needed to update a framework state store.
final class DocumentSummaryView {
  /// Creates the document fields required by framework state adapters.
  const DocumentSummaryView({
    required this.coordinate,
    required this.lifecycle,
    required this.projectionCursor,
    this.roots,
  });

  /// Coordinate of the document state represented by this summary.
  final CoordinateView coordinate;

  /// Current document lifecycle discriminator.
  final String lifecycle;

  /// Source cursor through which nodes have been projected.
  final SourceCursor projectionCursor;

  /// Root child list when included by the delta payload.
  final ChildListView? roots;
}

/// Continuity-qualified identity for one content node.
final class TransitionNodeKeyView {
  const TransitionNodeKeyView._({
    required this.continuityGeneration,
    required this.epoch,
    required this.nodeId,
  });

  /// Generation that prevents identity reuse across continuity replacement.
  final ContinuityGeneration continuityGeneration;

  /// Recovery epoch containing the node.
  final Epoch epoch;

  /// Stable node identity within the continuity generation.
  final NodeId nodeId;
}

/// Continuity-qualified identity for one semantic resource.
final class TransitionResourceKeyView {
  const TransitionResourceKeyView._({
    required this.continuityGeneration,
    required this.epoch,
    required this.resourceId,
  });

  /// Generation that prevents identity reuse across continuity replacement.
  final ContinuityGeneration continuityGeneration;

  /// Recovery epoch containing the resource.
  final Epoch epoch;

  /// Stable semantic resource identity within the generation.
  final ResourceId resourceId;
}

/// Owner of a versioned child list affected by one transition.
sealed class TransitionChildListOwnerView {
  const TransitionChildListOwnerView._(this.kind);

  /// Owner discriminator used by the transition protocol.
  final String kind;
}

/// The document root child list.
final class DocumentTransitionOwnerView extends TransitionChildListOwnerView {
  const DocumentTransitionOwnerView._() : super._('document');
}

/// A child list owned by a continuity-qualified node.
final class NodeTransitionOwnerView extends TransitionChildListOwnerView {
  const NodeTransitionOwnerView._(this.key) : super._('node');

  /// Continuity-qualified node that owns the child list.
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

  /// Continuity generation represented by this document stamp.
  final ContinuityGeneration continuityGeneration;

  /// Canonical stream coordinate represented by this stamp.
  final CoordinateView coordinate;

  /// Document lifecycle at this stamp.
  final String lifecycle;

  /// Projected source cursor at this stamp.
  final SourceCursor projectionCursor;

  /// Structural version of the document root list at this stamp.
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

  /// Content version of the node at this stamp.
  final NodeVersion version;

  /// Node stability discriminator at this stamp.
  final String stability;

  /// Parent child-list owner, or `null` for a detached node.
  final TransitionChildListOwnerView? parent;

  /// Structural version of the node's child list at this stamp.
  final StructureVersion childrenVersion;
}

/// Owned text delta attached to a node transition.
sealed class TextTransitionView {
  const TextTransitionView._(this.kind);

  /// Text transition discriminator used by the transition protocol.
  final String kind;
}

/// Append-only projected source text retained by the transition record.
final class ProjectionAppendTransitionView extends TextTransitionView {
  const ProjectionAppendTransitionView._({
    required this.range,
    required this.text,
  }) : super._('projection_append');

  /// Half-open source range covered by the appended projected text.
  final SourceRangeView range;

  /// Owned projected source text appended in this transition.
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

  /// Continuity-qualified identity of the affected node.
  final TransitionNodeKeyView key;

  /// Node state before the transition, or `null` for insertion.
  final NodeStateStampView? before;

  /// Node state after the transition, or `null` for removal.
  final NodeStateStampView? after;

  /// Owned text delta when node text changed.
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

  /// Document or node that owns the affected child list.
  final TransitionChildListOwnerView owner;

  /// Structural version before applying the splice.
  final StructureVersion beforeVersion;

  /// Structural version after applying the splice.
  final StructureVersion afterVersion;

  /// Zero-based child index at which the splice begins.
  final int start;

  /// Ordered node keys removed by the splice.
  final List<TransitionNodeKeyView> removed;

  /// Ordered node keys inserted by the splice.
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

  /// Continuity-qualified identity of the affected resource.
  final TransitionResourceKeyView key;

  /// Resource version before the transition, or `null` for insertion.
  final ResourceVersion? beforeVersion;

  /// Resource version after the transition, or `null` for removal.
  final ResourceVersion? afterVersion;

  /// Nodes whose content refers to this resource.
  final List<TransitionNodeKeyView> affectedNodes;
}

/// Ordered transition facts produced by one reducer update.
sealed class TransitionFactsView {
  const TransitionFactsView._({
    required this.scope,
    required this.before,
    required this.after,
  });

  /// Transition scope discriminator: continuous or full replacement.
  final String scope;

  /// Document state before the update, or `null` before initialization.
  final DocumentStateStampView? before;

  /// Document state after the update.
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

  /// Ordered node transitions in this reducer update.
  final List<NodeTransitionView> nodes;

  /// Ordered child-list splices in this reducer update.
  final List<StructureTransitionView> structures;

  /// Ordered semantic resource transitions in this reducer update.
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

  /// Schema governing the nested transition facts.
  final String schema;

  /// Typed transition facts carried by the envelope.
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

  /// Result of applying the command that produced this update.
  final ApplyOutcomeView outcome;

  /// Reducer readiness after the command.
  final ReducerStatusView status;

  /// Exact invalidation set produced by the command.
  final ChangeImpactView impact;

  /// Document summary after the command, when initialized.
  final DocumentSummaryView? document;

  /// Versioned transition facts, when requested by the binding schema.
  final TransitionEnvelopeView? transition;
}

/// Half-open source range represented with exact decimal cursors.
final class SourceRangeView {
  /// Creates a half-open source range from [start] to [end].
  const SourceRangeView({required this.start, required this.end});

  /// Inclusive source byte cursor where the range begins.
  final SourceCursor start;

  /// Exclusive source byte cursor where the range ends.
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

  /// Source range covered by the bounded pending suffix.
  final SourceRangeView range;

  /// Exact pending source text within [range].
  final String text;
}

/// Column alignment encoded by the table Content IR.
enum TableAlignment {
  /// No explicit alignment was declared.
  none,

  /// Aligns cell content to the left edge.
  left,

  /// Centers cell content.
  center,

  /// Aligns cell content to the right edge.
  right,
}

/// Markdown link syntax retained by the Content IR.
enum LinkStyle {
  /// Inline destination syntax.
  inline,

  /// Full reference syntax with a resolved definition.
  reference,

  /// Full reference syntax without a resolved definition.
  referenceUnknown,

  /// Collapsed reference syntax with a resolved definition.
  collapsed,

  /// Collapsed reference syntax without a resolved definition.
  collapsedUnknown,

  /// Shortcut reference syntax with a resolved definition.
  shortcut,

  /// Shortcut reference syntax without a resolved definition.
  shortcutUnknown,

  /// URI autolink syntax.
  autolink,

  /// Email autolink syntax.
  email,
}

/// Semantic variant of a block quote in the Content IR.
enum BlockQuoteKind {
  /// Ordinary Markdown block quote.
  plain,

  /// Note-style alert block quote.
  note,

  /// Tip-style alert block quote.
  tip,

  /// Important-style alert block quote.
  important,

  /// Warning-style alert block quote.
  warning,

  /// Caution-style alert block quote.
  caution,
}

/// Delimiter character used by a fenced code block.
enum CodeFenceMarker {
  /// Backtick fence.
  backtick,

  /// Tilde fence.
  tilde,
}

/// Supported citation resource protocol.
enum CitationProtocol {
  /// Version 1 of the mdstream citation protocol.
  mdstreamCitation1('mdstream.citation/1');

  /// Creates a citation protocol with its stable [wireName].
  const CitationProtocol(this.wireName);

  /// Stable protocol name encoded in Content IR payloads.
  final String wireName;
}

/// Source or normalized text representation in the Content IR.
sealed class SemanticTextView {
  /// Creates a semantic text representation identified by [kind].
  const SemanticTextView(this.kind);

  /// Text representation discriminator encoded by the Content IR.
  final String kind;
}

/// Semantic text read directly from the node's source range.
final class SourceSemanticTextView extends SemanticTextView {
  /// Creates a source-backed semantic text marker.
  const SourceSemanticTextView() : super('source');
}

/// Semantic text normalized independently of the source bytes.
final class NormalizedSemanticTextView extends SemanticTextView {
  /// Creates normalized semantic text containing [value].
  const NormalizedSemanticTextView(this.value) : super('normalized');

  /// Normalized text stored directly in the Content IR.
  final String value;
}

/// Source syntax retained for a code block Content IR node.
sealed class CodeBlockSyntaxView {
  /// Creates a code-block syntax representation identified by [kind].
  const CodeBlockSyntaxView(this.kind);

  /// Code-block syntax discriminator encoded by the Content IR.
  final String kind;
}

/// Indentation-delimited code block syntax.
final class IndentedCodeBlockSyntaxView extends CodeBlockSyntaxView {
  /// Creates an indented code-block syntax marker.
  const IndentedCodeBlockSyntaxView() : super('indented');
}

/// Explicitly fenced code block syntax.
final class FencedCodeBlockSyntaxView extends CodeBlockSyntaxView {
  /// Creates fenced syntax using [marker] repeated [length] times.
  const FencedCodeBlockSyntaxView({required this.marker, required this.length})
    : super('fenced');

  /// Delimiter character used by the source fence.
  final CodeFenceMarker marker;

  /// Number of delimiter characters in the opening fence.
  final int length;
}

/// Version-qualified reference to a semantic resource.
final class ResourceRefView {
  /// Creates a reference to resource [id] at [version].
  const ResourceRefView({required this.id, required this.version});

  /// Stable identity of the referenced semantic resource.
  final ResourceId id;

  /// Resource version required by this Content IR node.
  final ResourceVersion version;
}

/// Base type for exhaustive semantic node content in the Content IR.
sealed class ContentKindView {
  /// Creates node content identified by the stable [kind] discriminator.
  const ContentKindView(this.kind);

  /// Semantic content discriminator encoded by the Content IR.
  final String kind;
}

/// Paragraph container content.
final class ParagraphContentView extends ContentKindView {
  /// Creates paragraph content.
  const ParagraphContentView() : super('paragraph');
}

/// Heading container content.
final class HeadingContentView extends ContentKindView {
  /// Creates heading content at Markdown [level].
  const HeadingContentView(this.level) : super('heading');

  /// Markdown heading level represented by this node.
  final int level;
}

/// Inline text content.
final class TextContentView extends ContentKindView {
  /// Creates inline content backed by semantic [text].
  const TextContentView(this.text) : super('text');

  /// Source-backed or normalized text represented by the node.
  final SemanticTextView text;
}

/// Emphasized inline container content.
final class EmphasisContentView extends ContentKindView {
  /// Creates emphasis content.
  const EmphasisContentView() : super('emphasis');
}

/// Strongly emphasized inline container content.
final class StrongContentView extends ContentKindView {
  /// Creates strong-emphasis content.
  const StrongContentView() : super('strong');
}

/// Struck-through inline container content.
final class StrikethroughContentView extends ContentKindView {
  /// Creates strikethrough content.
  const StrikethroughContentView() : super('strikethrough');
}

/// Link container content with retained Markdown syntax.
final class LinkContentView extends ContentKindView {
  /// Creates link content with its resolved [target] and source [style].
  const LinkContentView({
    required this.target,
    required this.referenceLabel,
    required this.style,
  }) : super('link');

  /// Versioned resource target, or `null` when unresolved.
  final ResourceRefView? target;

  /// Reference label retained from source syntax, when present.
  final String? referenceLabel;

  /// Markdown link syntax retained by this node.
  final LinkStyle style;
}

/// Image content with semantic alternative text.
final class ImageContentView extends ContentKindView {
  /// Creates image content with its target, syntax, and alternative text.
  const ImageContentView({
    required this.target,
    required this.referenceLabel,
    required this.style,
    required this.alt,
  }) : super('image');

  /// Versioned image resource target, or `null` when unresolved.
  final ResourceRefView? target;

  /// Reference label retained from source syntax, when present.
  final String? referenceLabel;

  /// Markdown image-link syntax retained by this node.
  final LinkStyle style;

  /// Semantic alternative text represented by the image node.
  final SemanticTextView alt;
}

/// Inline code span content.
final class InlineCodeContentView extends ContentKindView {
  /// Creates inline code content backed by semantic [text].
  const InlineCodeContentView(this.text) : super('inline_code');

  /// Source-backed or normalized code text.
  final SemanticTextView text;
}

/// Block code content with retained source syntax and info string.
final class CodeBlockContentView extends ContentKindView {
  /// Creates code-block content with [syntax], optional [info], and [text].
  const CodeBlockContentView({
    required this.syntax,
    required this.info,
    required this.text,
  }) : super('code_block');

  /// Source syntax used to delimit the code block.
  final CodeBlockSyntaxView syntax;

  /// Optional code-fence info string.
  final String? info;

  /// Source-backed or normalized code body.
  final SemanticTextView text;
}

/// Ordered or unordered list container content.
final class ListContentView extends ContentKindView {
  /// Creates list content with ordering and tightness metadata.
  const ListContentView({
    required this.ordered,
    required this.start,
    required this.tight,
  }) : super('list');

  /// Whether this is an ordered rather than unordered list.
  final bool ordered;

  /// Explicit starting ordinal for an ordered list, when present.
  final int? start;

  /// Whether the Markdown list is tight.
  final bool tight;
}

/// One list item, optionally carrying task-list state.
final class ListItemContentView extends ContentKindView {
  /// Creates a list item with optional task-list [checked] state.
  const ListItemContentView(this.checked) : super('list_item');

  /// Task-list state, or `null` for a regular list item.
  final bool? checked;
}

/// Block quote or alert container content.
final class BlockQuoteContentView extends ContentKindView {
  /// Creates block-quote content with semantic [style].
  const BlockQuoteContentView(this.style) : super('block_quote');

  /// Plain quote or alert variant retained by the Content IR.
  final BlockQuoteKind style;
}

/// Thematic break content.
final class ThematicBreakContentView extends ContentKindView {
  /// Creates thematic-break content.
  const ThematicBreakContentView() : super('thematic_break');
}

/// Table container content with per-column alignment.
final class TableContentView extends ContentKindView {
  /// Creates table content with ordered column [alignments].
  const TableContentView(this.alignments) : super('table');

  /// Alignment metadata indexed by table column.
  final List<TableAlignment> alignments;
}

/// Table header-section container content.
final class TableHeadContentView extends ContentKindView {
  /// Creates table-head content.
  const TableHeadContentView() : super('table_head');
}

/// Table body-section container content.
final class TableBodyContentView extends ContentKindView {
  /// Creates table-body content.
  const TableBodyContentView() : super('table_body');
}

/// Table row container content.
final class TableRowContentView extends ContentKindView {
  /// Creates table-row content.
  const TableRowContentView() : super('table_row');
}

/// Table cell container content.
final class TableCellContentView extends ContentKindView {
  /// Creates table-cell content at zero-based [column].
  const TableCellContentView(this.column) : super('table_cell');

  /// Zero-based column occupied by this cell.
  final int column;
}

/// Raw HTML content retained by the Content IR.
final class HtmlContentView extends ContentKindView {
  /// Creates block or inline HTML content backed by semantic [text].
  const HtmlContentView({required this.block, required this.text})
    : super('html');

  /// Whether the HTML is block-level rather than inline.
  final bool block;

  /// Source-backed or normalized HTML text.
  final SemanticTextView text;
}

/// Mathematical notation content retained by the Content IR.
final class MathContentView extends ContentKindView {
  /// Creates display or inline math content backed by semantic [text].
  const MathContentView({required this.display, required this.text})
    : super('math');

  /// Whether the expression uses display rather than inline layout.
  final bool display;

  /// Source-backed or normalized mathematical notation.
  final SemanticTextView text;
}

/// Footnote definition content linked to a semantic resource.
final class FootnoteDefinitionContentView extends ContentKindView {
  /// Creates a footnote definition for [label] targeting [target].
  const FootnoteDefinitionContentView({
    required this.label,
    required this.target,
  }) : super('footnote_definition');

  /// Source label identifying the footnote definition.
  final String label;

  /// Versioned semantic resource defined by this node.
  final ResourceRefView target;
}

/// Footnote reference content.
final class FootnoteReferenceContentView extends ContentKindView {
  /// Creates a footnote reference for [label] and optional resolved [target].
  const FootnoteReferenceContentView({
    required this.label,
    required this.target,
  }) : super('footnote_reference');

  /// Source label used by the footnote reference.
  final String label;

  /// Versioned semantic resource, or `null` when unresolved.
  final ResourceRefView? target;
}

/// Citation definition content linked to a semantic resource.
final class CitationDefinitionContentView extends ContentKindView {
  /// Creates a citation definition for [key] targeting [target].
  const CitationDefinitionContentView({required this.key, required this.target})
    : super('citation_definition');

  /// Citation key defined by this node.
  final String key;

  /// Versioned semantic resource defined by this node.
  final ResourceRefView target;
}

/// Citation reference content.
final class CitationReferenceContentView extends ContentKindView {
  /// Creates a citation reference for [key] and optional resolved [target].
  const CitationReferenceContentView({required this.key, required this.target})
    : super('citation_reference');

  /// Citation key used by the reference.
  final String key;

  /// Versioned semantic resource, or `null` when unresolved.
  final ResourceRefView? target;
}

/// Soft line-break content.
final class SoftBreakContentView extends ContentKindView {
  /// Creates soft-break content.
  const SoftBreakContentView() : super('soft_break');
}

/// Hard line-break content.
final class HardBreakContentView extends ContentKindView {
  /// Creates hard-break content.
  const HardBreakContentView() : super('hard_break');
}

/// Namespaced extension content carried by the generic Content IR protocol.
final class CustomContentView extends ContentKindView {
  /// Creates custom content with a namespaced name and immutable attributes.
  const CustomContentView({
    required this.namespace,
    required this.name,
    required this.opaque,
    required this.attributes,
  }) : super('custom');

  /// Extension namespace that owns the custom content protocol.
  final String namespace;

  /// Extension-defined content name within [namespace].
  final String name;

  /// Whether consumers must treat child interpretation as extension-owned.
  final bool opaque;

  /// Extension-defined immutable string attributes.
  final Map<String, String> attributes;
}

/// Base type for semantic resources referenced by Content IR nodes.
sealed class SemanticResourceKindView {
  /// Creates resource content identified by stable [kind].
  const SemanticResourceKindView(this.kind);

  /// Semantic resource discriminator encoded by the Content IR.
  final String kind;
}

/// Resolved link or image destination resource.
final class LinkResourceContentView extends SemanticResourceKindView {
  /// Creates a link resource with [destination] and optional [title].
  const LinkResourceContentView({
    required this.destination,
    required this.title,
  }) : super('link');

  /// Resolved destination string from the Markdown resource.
  final String destination;

  /// Optional resource title.
  final String? title;
}

/// Footnote definition resource.
final class FootnoteResourceContentView extends SemanticResourceKindView {
  /// Creates a footnote resource identified by [label].
  const FootnoteResourceContentView(this.label) : super('footnote');

  /// Canonical footnote label.
  final String label;
}

/// Citation resource resolved under an explicit protocol.
final class CitationResourceContentView extends SemanticResourceKindView {
  /// Creates a citation resource with its protocol and resolved metadata.
  const CitationResourceContentView({
    required this.protocol,
    required this.key,
    required this.destination,
    required this.title,
  }) : super('citation');

  /// Citation protocol used to interpret the resource.
  final CitationProtocol protocol;

  /// Citation key within [protocol].
  final String key;

  /// Resolved citation destination.
  final String destination;

  /// Optional citation title.
  final String? title;
}

/// Typed stable node envelope with exhaustive content metadata.
final class ContentNodeView {
  /// Creates a versioned Content IR node and its structural metadata.
  const ContentNodeView({
    required this.id,
    required this.version,
    required this.stability,
    required this.source,
    required this.body,
    required this.children,
    required this.content,
  });

  /// Stable identity preserved across compatible incremental updates.
  final NodeId id;

  /// Content version used to detect semantic node changes.
  final NodeVersion version;

  /// Streaming stability state of this node.
  final String stability;

  /// Half-open source range covering the complete node syntax.
  final SourceRangeView source;

  /// Half-open source range covering the node's semantic body.
  final SourceRangeView body;

  /// Versioned ordered child identities owned by this node.
  final ChildListView children;

  /// Exhaustive semantic content represented by this node.
  final ContentKindView content;
}

/// Materialized node plus its body text.
final class NodeView extends DecodedBindingView {
  const NodeView._({
    required super.schema,
    required super.raw,
    required this.node,
    required this.bodyText,
    required this.processorInputVersion,
  }) : super(kind: 'node_view');

  /// Versioned Content IR node metadata.
  final ContentNodeView node;

  /// Exact source text selected by the node's body range.
  final String bodyText;

  /// Canonical version of the complete node-local processor input.
  final ProcessorInputVersion processorInputVersion;
}

/// Typed stable semantic resource with exhaustive content metadata.
final class SemanticResourceView {
  /// Creates a versioned semantic resource and its typed [content].
  const SemanticResourceView({
    required this.id,
    required this.version,
    required this.content,
  });

  /// Stable identity of the semantic resource.
  final ResourceId id;

  /// Version used to detect semantic resource changes.
  final ResourceVersion version;

  /// Exhaustive semantic content represented by the resource.
  final SemanticResourceKindView content;
}

/// Materialized semantic resource.
final class ResourceView extends DecodedBindingView {
  const ResourceView._({
    required super.schema,
    required super.raw,
    required this.resource,
  }) : super(kind: 'resource_view');

  /// Versioned semantic resource materialized by this payload.
  final SemanticResourceView resource;
}

/// Complete key that prevents stale processor results from being applied.
final class ProcessorKeyView {
  /// Creates the complete concurrency key for one processor invocation.
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

  /// Document epoch containing the processed node.
  final Epoch epoch;

  /// Stable identity of the processed Content IR node.
  final NodeId nodeId;

  /// Application-defined processor identifier.
  final String processorId;

  /// Content version of the processed node.
  final NodeVersion nodeVersion;

  /// Version of the fully materialized processor input.
  final ProcessorInputVersion inputVersion;

  /// Application-defined processor implementation version.
  final String processorVersion;

  /// Application-defined processor configuration version.
  final String configurationVersion;

  /// Request generation that rejects stale completions.
  final RequestGeneration generation;
}

/// Fully materialized input for one external content processor.
final class ProcessorInputView {
  /// Creates fully materialized processor input for a Content IR node.
  const ProcessorInputView({
    required this.node,
    required this.body,
    required this.resource,
  });

  /// Versioned node metadata supplied to the processor.
  final ContentNodeView node;

  /// Exact source text selected by the node's body range.
  final String body;

  /// Referenced semantic resource when the node has one.
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

  /// Native request identity used for completion or cancellation.
  final RequestGeneration requestId;

  /// Complete concurrency key for rejecting stale processor results.
  final ProcessorKeyView key;

  /// Fully materialized immutable processor input.
  final ProcessorInputView input;
}

/// Closed set of native decisions for one processor completion.
enum ProcessorCompletionOutcome {
  /// The current processor request accepted the submitted completion.
  applied('applied'),

  /// The submitted completion no longer matched the current request.
  stale('stale');

  /// Creates a completion outcome with its stable [wireName].
  const ProcessorCompletionOutcome(this.wireName);

  /// Stable wire spelling emitted by the native processor host.
  final String wireName;
}

/// Native decision for one processor completion.
final class ProcessorCompletionView extends DecodedBindingView {
  const ProcessorCompletionView._({
    required super.schema,
    required super.raw,
    required this.requestId,
    required this.outcome,
  }) : super(kind: 'processor_completion');

  /// Native request identity whose completion was evaluated.
  final RequestGeneration requestId;

  /// Native acceptance outcome for the submitted completion.
  final ProcessorCompletionOutcome outcome;
}

/// State transition for one processor artifact slot.
sealed class ArtifactChangeKindView {
  const ArtifactChangeKindView();
}

/// Transition that reserves an artifact slot for active processor work.
final class PendingArtifactChangeView extends ArtifactChangeKindView {
  /// Creates a pending artifact transition.
  const PendingArtifactChangeView();
}

/// Transition that installs a completed artifact.
final class ReadyArtifactChangeView extends ArtifactChangeKindView {
  /// Creates a ready artifact transition retaining [artifactBytes].
  const ReadyArtifactChangeView(this.artifactBytes);

  /// Retained artifact byte size for a ready transition.
  final DecimalCounter artifactBytes;
}

/// Transition that retains a structured processor failure.
final class FailedArtifactChangeView extends ArtifactChangeKindView {
  /// Creates a failed artifact transition with [code].
  const FailedArtifactChangeView(this.code);

  /// Structured processor error code for a failed transition.
  final String code;
}

/// Transition that removes an artifact slot and releases retained bytes.
final class RemovedArtifactChangeView extends ArtifactChangeKindView {
  /// Creates a removed artifact transition.
  const RemovedArtifactChangeView({
    required this.reason,
    required this.releasedArtifactBytes,
  });

  /// Removal reason for a removed transition.
  final String reason;

  /// Retained bytes released by a removed transition.
  final DecimalCounter releasedArtifactBytes;
}

/// Artifact slot invalidation emitted by the native processor host.
final class ArtifactChangeView extends DecodedBindingView {
  const ArtifactChangeView._({
    required super.schema,
    required super.raw,
    required this.key,
    required this.change,
  }) : super(kind: 'artifact_change');

  /// Complete processor key identifying the affected artifact slot.
  final ProcessorKeyView key;

  /// State transition applied to the artifact slot.
  final ArtifactChangeKindView change;
}

/// Text, binary, or citation payload owned by a processor artifact.
sealed class ArtifactPayloadView {
  const ArtifactPayloadView();
}

/// UTF-8 text retained by a processor artifact.
final class TextArtifactPayloadView extends ArtifactPayloadView {
  /// Creates a text artifact payload.
  const TextArtifactPayloadView(this.text);

  /// Retained text value.
  final String text;
}

/// Opaque binary octets retained by a processor artifact.
final class BinaryArtifactPayloadView extends ArtifactPayloadView {
  /// Creates a binary artifact payload with an owned immutable copy.
  BinaryArtifactPayloadView(List<int> bytes)
    : bytes = List<int>.unmodifiable(bytes);

  /// Immutable retained octets.
  final List<int> bytes;
}

/// Resolved citation retained by a processor artifact.
final class CitationArtifactPayloadView extends ArtifactPayloadView {
  /// Creates a resolved citation artifact payload.
  const CitationArtifactPayloadView({
    required this.key,
    required this.destination,
    required this.title,
  });

  /// Citation key.
  final String key;

  /// Resolved citation destination.
  final String destination;

  /// Optional citation title supplied by the processor.
  final String? title;
}

/// Protocol-labelled output produced by a content processor.
final class ProcessorArtifactView {
  /// Creates a protocol-labelled artifact with its [payload].
  const ProcessorArtifactView({
    required this.protocol,
    required this.mediaType,
    required this.payload,
  });

  /// Content protocol understood by the artifact consumer.
  final String protocol;

  /// Media type of the processor output.
  final String mediaType;

  /// Typed value produced by the processor.
  final ArtifactPayloadView payload;
}

/// Structured processor failure retained for an artifact slot.
final class ArtifactFailureView {
  /// Creates a retained processor failure with [code] and [message].
  const ArtifactFailureView({required this.code, required this.message});

  /// Stable structured processor failure code.
  final String code;

  /// Human-readable processor failure description.
  final String message;
}

/// Closed set of states for one processor artifact slot.
enum ArtifactState {
  /// Processor work is active and no retained result is available yet.
  pending('pending'),

  /// A processor artifact is retained for the current request key.
  ready('ready'),

  /// A structured processor failure is retained for the current request key.
  failed('failed');

  /// Creates an artifact state with its stable [wireName].
  const ArtifactState(this.wireName);

  /// Stable wire spelling emitted by the native processor host.
  final String wireName;
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

  /// Complete processor key identifying this artifact slot.
  final ProcessorKeyView key;

  /// Current artifact slot state discriminator.
  final ArtifactState state;

  /// Retained artifact when the slot is ready.
  final ProcessorArtifactView? artifact;

  /// Retained structured failure when the slot failed.
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
    continuityGeneration: decodeContinuityGeneration(
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
    projectionCursor: decodeSourceCursor(
      value['projection_cursor'],
      'projection_cursor',
    ),
    rootsVersion: decodeStructureVersion(
      value['roots_version'],
      'roots_version',
    ),
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
    epoch: decodeEpoch(value['epoch'], 'coordinate.epoch'),
    sequence: decodeSequence(value['sequence'], 'coordinate.sequence'),
    changeId: decodeChangeId(value['change_id'], 'coordinate.change_id'),
    sourceCursor: decodeSourceCursor(
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
    continuityGeneration: decodeContinuityGeneration(
      value['continuity_generation'],
      'continuity_generation',
    ),
    epoch: decodeEpoch(value['epoch'], 'transition node epoch'),
    nodeId: decodeNodeId(value['node_id'], 'transition node id'),
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
    continuityGeneration: decodeContinuityGeneration(
      value['continuity_generation'],
      'continuity_generation',
    ),
    epoch: decodeEpoch(value['epoch'], 'transition resource epoch'),
    resourceId: decodeResourceId(
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
    version: decodeNodeVersion(value['version'], 'transition node version'),
    stability: stability,
    parent: parent == null ? null : _decodeTransitionOwner(parent),
    childrenVersion: decodeStructureVersion(
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
    beforeVersion: decodeStructureVersion(
      value['before_version'],
      'structure before version',
    ),
    afterVersion: decodeStructureVersion(
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

const _appliedOutcomeKeys = <String>{'kind', 'coordinate'};
const _recoveredOutcomeKeys = <String>{'kind', 'coordinate'};
const _idempotentOutcomeKeys = <String>{'kind'};
const _staleOutcomeKeys = <String>{
  'kind',
  'current',
  'received_epoch',
  'received_sequence',
};
const _recoveryRequiredOutcomeKeys = <String>{'kind', 'last_good', 'reason'};
const _uninitializedStatusKeys = <String>{'kind'};
const _readyStatusKeys = <String>{'kind'};
const _needsSnapshotStatusKeys = <String>{'kind', 'last_good', 'reason'};

ApplyOutcomeView _decodeOutcome(Map<String, Object?> value) {
  final kind = _requiredString(value['kind'], 'outcome.kind');
  switch (kind) {
    case 'applied':
      _exactKeys(value, _appliedOutcomeKeys, 'applied outcome');
      return AppliedOutcomeView(
        _decodeCoordinate(_requiredRecord(value['coordinate'], 'coordinate')),
      );
    case 'recovered':
      _exactKeys(value, _recoveredOutcomeKeys, 'recovered outcome');
      return RecoveredOutcomeView(
        _decodeCoordinate(_requiredRecord(value['coordinate'], 'coordinate')),
      );
    case 'idempotent':
      _exactKeys(value, _idempotentOutcomeKeys, 'idempotent outcome');
      return const IdempotentOutcomeView();
    case 'stale':
      _exactKeys(value, _staleOutcomeKeys, 'stale outcome');
      return StaleOutcomeView(
        current: _decodeCoordinate(
          _requiredRecord(value['current'], 'current'),
        ),
        receivedEpoch: decodeEpoch(value['received_epoch'], 'received_epoch'),
        receivedSequence: decodeSequence(
          value['received_sequence'],
          'received_sequence',
        ),
      );
    case 'recovery_required':
      _exactKeys(
        value,
        _recoveryRequiredOutcomeKeys,
        'recovery-required outcome',
      );
      return RecoveryRequiredOutcomeView(
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
      _exactKeys(value, _uninitializedStatusKeys, 'uninitialized status');
      return const UninitializedReducerStatusView();
    case 'ready':
      _exactKeys(value, _readyStatusKeys, 'ready status');
      return const ReadyReducerStatusView();
    case 'needs_snapshot':
      _exactKeys(value, _needsSnapshotStatusKeys, 'needs-snapshot status');
      return NeedsSnapshotReducerStatusView(
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
  changedNodeIds: _nodeIdArray(value['changed_node_ids'], 'changed_node_ids'),
  removedNodeIds: _nodeIdArray(value['removed_node_ids'], 'removed_node_ids'),
  changedResourceIds: _resourceIdArray(
    value['changed_resource_ids'],
    'changed_resource_ids',
  ),
  removedResourceIds: _resourceIdArray(
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
    projectionCursor: decodeSourceCursor(
      value['projection_cursor'],
      'projection_cursor',
    ),
    roots: value.containsKey('roots')
        ? _decodeChildList(_requiredRecord(value['roots'], 'document.roots'))
        : null,
  );
}

CoordinateView _decodeCoordinate(Map<String, Object?> value) => CoordinateView(
  epoch: decodeEpoch(value['epoch'], 'coordinate.epoch'),
  sequence: decodeSequence(value['sequence'], 'coordinate.sequence'),
  changeId: decodeChangeId(value['change_id'], 'coordinate.change_id'),
  sourceCursor: decodeSourceCursor(
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
      processorInputVersion: decodeProcessorInputVersion(
        value['processor_input_version'],
        'processor_input_version',
      ),
    );

ContentNodeView _decodeNode(Map<String, Object?> value) {
  final stability = _requiredString(value['stability'], 'node.stability');
  if (stability != 'provisional' && stability != 'stable') {
    throw invalidBindingPayload('unknown node stability $stability');
  }
  return ContentNodeView(
    id: decodeNodeId(value['id'], 'node.id'),
    version: decodeNodeVersion(value['version'], 'node.version'),
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
    id: decodeResourceId(value['id'], 'resource reference id'),
    version: decodeResourceVersion(
      value['version'],
      'resource reference version',
    ),
  );
}

SourceRangeView _decodeRange(Map<String, Object?> value) => SourceRangeView(
  start: decodeSourceCursor(value['start'], 'range.start'),
  end: decodeSourceCursor(value['end'], 'range.end'),
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
  version: decodeStructureVersion(value['version'], 'child_list.version'),
  children: _nodeIdArray(value['children'], 'child_list.children'),
);

ResourceView _decodeResourceView(Map<String, Object?> value, String schema) =>
    ResourceView._(
      schema: schema,
      raw: value,
      resource: _decodeResource(_requiredRecord(value['resource'], 'resource')),
    );

SemanticResourceView _decodeResource(Map<String, Object?> value) {
  return SemanticResourceView(
    id: decodeResourceId(value['id'], 'resource.id'),
    version: decodeResourceVersion(value['version'], 'resource.version'),
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
    requestId: decodeRequestGeneration(value['request_id'], 'request_id'),
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
  final wireOutcome = _requiredString(
    value['outcome'],
    'processor completion outcome',
  );
  final outcome = switch (wireOutcome) {
    'applied' => ProcessorCompletionOutcome.applied,
    'stale' => ProcessorCompletionOutcome.stale,
    _ => throw invalidBindingPayload(
      'unknown processor completion outcome $wireOutcome',
    ),
  };
  return ProcessorCompletionView._(
    schema: schema,
    raw: value,
    requestId: decodeRequestGeneration(value['request_id'], 'request_id'),
    outcome: outcome,
  );
}

ProcessorKeyView _decodeProcessorKey(Map<String, Object?> value) =>
    ProcessorKeyView(
      epoch: decodeEpoch(value['epoch'], 'processor key epoch'),
      nodeId: decodeNodeId(value['node_id'], 'processor key node_id'),
      processorId: _requiredString(
        value['processor_id'],
        'processor key processor_id',
      ),
      nodeVersion: decodeNodeVersion(
        value['node_version'],
        'processor key node_version',
      ),
      inputVersion: decodeProcessorInputVersion(
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
      generation: decodeRequestGeneration(
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
      _exactKeys(change, const <String>{'kind'}, 'pending artifact change');
      decoded = const PendingArtifactChangeView();
    case 'ready':
      _exactKeys(change, const <String>{
        'kind',
        'artifact_bytes',
      }, 'ready artifact change');
      decoded = ReadyArtifactChangeView(
        decodeDecimalCounter(change['artifact_bytes'], 'artifact_bytes'),
      );
    case 'failed':
      _exactKeys(change, const <String>{
        'kind',
        'code',
      }, 'failed artifact change');
      decoded = FailedArtifactChangeView(_failureCode(change['code']));
    case 'removed':
      _exactKeys(change, const <String>{
        'kind',
        'reason',
        'released_artifact_bytes',
      }, 'removed artifact change');
      decoded = RemovedArtifactChangeView(
        reason: _requiredString(change['reason'], 'artifact removal reason'),
        releasedArtifactBytes: decodeDecimalCounter(
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
  final wireState = _requiredString(value['state'], 'artifact state');
  final state = switch (wireState) {
    'pending' => ArtifactState.pending,
    'ready' => ArtifactState.ready,
    'failed' => ArtifactState.failed,
    _ => throw invalidBindingPayload('unknown artifact state $wireState'),
  };
  final artifact = _requiredNullableRecord(value, 'artifact', 'artifact');
  final failure = _requiredNullableRecord(value, 'failure', 'artifact failure');
  switch (state) {
    case ArtifactState.pending:
      if (artifact != null || failure != null) {
        throw invalidBindingPayload(
          'pending artifact state cannot retain an artifact or failure',
        );
      }
    case ArtifactState.ready:
      if (artifact == null || failure != null) {
        throw invalidBindingPayload(
          'ready artifact state must retain only an artifact',
        );
      }
    case ArtifactState.failed:
      if (artifact != null || failure == null) {
        throw invalidBindingPayload(
          'failed artifact state must retain only a failure',
        );
      }
  }
  return ArtifactView._(
    schema: schema,
    raw: value,
    key: _decodeProcessorKey(_requiredRecord(value['key'], 'artifact key')),
    state: state,
    artifact: artifact == null ? null : _decodeArtifact(artifact),
    failure: failure == null ? null : _decodeFailure(failure),
  );
}

ProcessorArtifactView _decodeArtifact(Map<String, Object?> value) {
  final payload = _requiredRecord(value['payload'], 'artifact payload');
  final kind = _requiredString(payload['kind'], 'artifact payload kind');
  final ArtifactPayloadView decoded;
  switch (kind) {
    case 'text':
      _exactKeys(payload, const <String>{
        'kind',
        'text',
      }, 'text artifact payload');
      decoded = TextArtifactPayloadView(
        _requiredString(payload['text'], 'artifact text'),
      );
    case 'binary':
      _exactKeys(payload, const <String>{
        'kind',
        'bytes',
      }, 'binary artifact payload');
      final octets = _requiredList(payload['bytes'], 'artifact bytes')
          .map((value) {
            if (value is! int || value < 0 || value > 255) {
              throw invalidBindingPayload('artifact bytes must contain octets');
            }
            return value;
          })
          .toList(growable: false);
      decoded = BinaryArtifactPayloadView(octets);
    case 'citation':
      _exactKeys(payload, const <String>{
        'kind',
        'key',
        'destination',
        'title',
      }, 'citation artifact payload');
      decoded = CitationArtifactPayloadView(
        key: _requiredString(payload['key'], 'citation key'),
        destination: _requiredString(
          payload['destination'],
          'citation destination',
        ),
        title: _requiredNullableString(payload, 'title', 'citation title'),
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
  return value[key] == null ? null : decodeResourceVersion(value[key], field);
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

List<NodeId> _nodeIdArray(Object? value, String field) =>
    List<NodeId>.unmodifiable(
      _requiredList(value, field).map((entry) => decodeNodeId(entry, field)),
    );

List<ResourceId> _resourceIdArray(Object? value, String field) =>
    List<ResourceId>.unmodifiable(
      _requiredList(
        value,
        field,
      ).map((entry) => decodeResourceId(entry, field)),
    );
