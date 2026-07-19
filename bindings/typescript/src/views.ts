import { BindingPayloadKind, TRANSITION_SCHEMA_DRAFT } from "./wasm.js";

declare const mdstreamBrand: unique symbol;

type Brand<Value, Name extends string> = Value & {
  readonly [mdstreamBrand]: Name;
};

export type Epoch = Brand<string, "Epoch">;
export type Sequence = Brand<string, "Sequence">;
export type SourceCursor = Brand<string, "SourceCursor">;
export type RequestGeneration = Brand<string, "RequestGeneration">;
export type NodeId = Brand<string, "NodeId">;
export type ResourceId = Brand<string, "ResourceId">;
export type ChangeId = Brand<string, "ChangeId">;
export type NodeVersion = Brand<string, "NodeVersion">;
export type ResourceVersion = Brand<string, "ResourceVersion">;
export type StructureVersion = Brand<string, "StructureVersion">;
export type ProcessorInputVersion = Brand<string, "ProcessorInputVersion">;
export type DecimalCounter = Brand<string, "DecimalCounter">;
export type ContinuityGeneration = Brand<string, "ContinuityGeneration">;
export type CanonicalChangeBytes = Brand<Uint8Array, "CanonicalChangeBytes">;
export type CanonicalSnapshotBytes = Brand<Uint8Array, "CanonicalSnapshotBytes">;

export interface CoordinateView {
  readonly epoch: Epoch;
  readonly sequence: Sequence;
  readonly changeId: ChangeId;
  readonly sourceCursor: SourceCursor;
}

export type RecoveryReasonView = Readonly<Record<string, unknown>> & {
  readonly kind: string;
};

export type ReducerStatusView =
  | { readonly kind: "uninitialized" }
  | { readonly kind: "ready" }
  | {
      readonly kind: "needs_snapshot";
      readonly lastGood: CoordinateView;
      readonly reason: RecoveryReasonView;
    };

export type ApplyOutcomeView =
  | { readonly kind: "applied"; readonly coordinate: CoordinateView }
  | { readonly kind: "recovered"; readonly coordinate: CoordinateView }
  | { readonly kind: "idempotent" }
  | {
      readonly kind: "stale";
      readonly current: CoordinateView;
      readonly receivedEpoch: Epoch;
      readonly receivedSequence: Sequence;
    }
  | {
      readonly kind: "recovery_required";
      readonly lastGood: CoordinateView;
      readonly reason: RecoveryReasonView;
    };

export interface ChangeImpactView {
  /** Invalidated node keys, including every removed node key. */
  readonly changedNodeIds: readonly NodeId[];
  /** Removed node keys; a subset of `changedNodeIds`. */
  readonly removedNodeIds: readonly NodeId[];
  /** Invalidated resource keys, including every removed resource key. */
  readonly changedResourceIds: readonly ResourceId[];
  /** Removed resource keys; a subset of `changedResourceIds`. */
  readonly removedResourceIds: readonly ResourceId[];
  readonly sourceChanged: boolean;
  readonly projectionChanged: boolean;
  readonly lifecycleChanged: boolean;
  readonly rootsChanged: boolean;
  readonly fullReplace: boolean;
}

export interface ChildListView {
  readonly version: StructureVersion;
  readonly children: readonly NodeId[];
}

export interface DocumentSummaryView {
  readonly coordinate: CoordinateView;
  readonly lifecycle: "open" | "finalized";
  readonly projectionCursor: SourceCursor;
  readonly roots?: ChildListView;
}

export interface TransitionNodeKeyView {
  readonly continuityGeneration: ContinuityGeneration;
  readonly epoch: Epoch;
  readonly nodeId: NodeId;
}

export interface TransitionResourceKeyView {
  readonly continuityGeneration: ContinuityGeneration;
  readonly epoch: Epoch;
  readonly resourceId: ResourceId;
}

export type TransitionChildListOwnerView =
  | { readonly kind: "document" }
  | { readonly kind: "node"; readonly key: TransitionNodeKeyView };

export interface DocumentStateStampView {
  readonly continuityGeneration: ContinuityGeneration;
  readonly coordinate: CoordinateView;
  readonly lifecycle: "open" | "finalized";
  readonly projectionCursor: SourceCursor;
  readonly rootsVersion: StructureVersion;
}

export interface NodeStateStampView {
  readonly version: NodeVersion;
  readonly stability: "provisional" | "stable";
  readonly parent: TransitionChildListOwnerView | null;
  readonly childrenVersion: StructureVersion;
}

export type TextTransitionView =
  | {
      readonly kind: "projection_append";
      readonly range: SourceRangeView;
      readonly text: string;
    }
  | { readonly kind: "replacement" };

export interface NodeTransitionView {
  readonly key: TransitionNodeKeyView;
  readonly before: NodeStateStampView | null;
  readonly after: NodeStateStampView | null;
  readonly text: TextTransitionView | null;
}

export interface StructureTransitionView {
  readonly owner: TransitionChildListOwnerView;
  readonly beforeVersion: StructureVersion;
  readonly afterVersion: StructureVersion;
  readonly start: number;
  readonly removed: readonly TransitionNodeKeyView[];
  readonly inserted: readonly TransitionNodeKeyView[];
}

export interface ResourceTransitionView {
  readonly key: TransitionResourceKeyView;
  readonly beforeVersion: ResourceVersion | null;
  readonly afterVersion: ResourceVersion | null;
  readonly affectedNodes: readonly TransitionNodeKeyView[];
}

export type TransitionFactsView =
  | {
      readonly scope: "continuous";
      readonly before: DocumentStateStampView | null;
      readonly after: DocumentStateStampView;
      readonly nodes: readonly NodeTransitionView[];
      readonly structures: readonly StructureTransitionView[];
      readonly resources: readonly ResourceTransitionView[];
    }
  | {
      readonly scope: "full_replace";
      readonly before: DocumentStateStampView | null;
      readonly after: DocumentStateStampView;
    };

export interface TransitionEnvelopeView {
  readonly schema: typeof TRANSITION_SCHEMA_DRAFT;
  readonly facts: TransitionFactsView;
}

export interface ReducerUpdateView {
  readonly schema: string;
  readonly kind: "reducer_update";
  readonly outcome: ApplyOutcomeView;
  readonly status: ReducerStatusView;
  readonly impact: ChangeImpactView;
  readonly document: DocumentSummaryView | null;
  readonly transition?: TransitionEnvelopeView;
}

export interface SourceRangeView {
  readonly start: SourceCursor;
  readonly end: SourceCursor;
}

export interface PendingSourceView {
  readonly schema: string;
  readonly kind: "pending_source_view";
  readonly range: SourceRangeView;
  readonly text: string;
}

export type TableAlignment = "none" | "left" | "center" | "right";
export type LinkStyle =
  | "inline"
  | "reference"
  | "reference_unknown"
  | "collapsed"
  | "collapsed_unknown"
  | "shortcut"
  | "shortcut_unknown"
  | "autolink"
  | "email";
export type BlockQuoteKind =
  | "plain"
  | "note"
  | "tip"
  | "important"
  | "warning"
  | "caution";
export type CodeFenceMarker = "backtick" | "tilde";
export type CitationProtocol = "mdstream.citation/1";

export type SemanticTextView =
  | { readonly kind: "source" }
  | { readonly kind: "normalized"; readonly value: string };

export type CodeBlockSyntaxView =
  | { readonly kind: "indented" }
  | {
      readonly kind: "fenced";
      readonly marker: CodeFenceMarker;
      readonly length: number;
    };

export interface ResourceRefView {
  readonly id: ResourceId;
  readonly version: ResourceVersion;
}

type EmptyContentKindView =
  | { readonly kind: "paragraph" }
  | { readonly kind: "emphasis" }
  | { readonly kind: "strong" }
  | { readonly kind: "strikethrough" }
  | { readonly kind: "thematic_break" }
  | { readonly kind: "table_head" }
  | { readonly kind: "table_body" }
  | { readonly kind: "table_row" }
  | { readonly kind: "soft_break" }
  | { readonly kind: "hard_break" };

export type ContentKindView =
  | EmptyContentKindView
  | { readonly kind: "heading"; readonly level: number }
  | { readonly kind: "text"; readonly text: SemanticTextView }
  | {
      readonly kind: "link";
      readonly target: ResourceRefView | null;
      readonly referenceLabel: string | null;
      readonly style: LinkStyle;
    }
  | {
      readonly kind: "image";
      readonly target: ResourceRefView | null;
      readonly referenceLabel: string | null;
      readonly style: LinkStyle;
      readonly alt: SemanticTextView;
    }
  | { readonly kind: "inline_code"; readonly text: SemanticTextView }
  | {
      readonly kind: "code_block";
      readonly syntax: CodeBlockSyntaxView;
      readonly info: string | null;
      readonly text: SemanticTextView;
    }
  | {
      readonly kind: "list";
      readonly ordered: boolean;
      readonly start: number | null;
      readonly tight: boolean;
    }
  | { readonly kind: "list_item"; readonly checked: boolean | null }
  | { readonly kind: "block_quote"; readonly style: BlockQuoteKind }
  | { readonly kind: "table"; readonly alignments: readonly TableAlignment[] }
  | { readonly kind: "table_cell"; readonly column: number }
  | {
      readonly kind: "html";
      readonly block: boolean;
      readonly text: SemanticTextView;
    }
  | {
      readonly kind: "math";
      readonly display: boolean;
      readonly text: SemanticTextView;
    }
  | {
      readonly kind: "footnote_definition";
      readonly label: string;
      readonly target: ResourceRefView;
    }
  | {
      readonly kind: "footnote_reference";
      readonly label: string;
      readonly target: ResourceRefView | null;
    }
  | {
      readonly kind: "citation_definition";
      readonly key: string;
      readonly target: ResourceRefView;
    }
  | {
      readonly kind: "citation_reference";
      readonly key: string;
      readonly target: ResourceRefView | null;
    }
  | {
      readonly kind: "custom";
      readonly namespace: string;
      readonly name: string;
      readonly opaque: boolean;
      readonly attributes: Readonly<Record<string, string>>;
    };

export type SemanticResourceKindView =
  | {
      readonly kind: "link";
      readonly destination: string;
      readonly title: string | null;
    }
  | { readonly kind: "footnote"; readonly label: string }
  | {
      readonly kind: "citation";
      readonly protocol: CitationProtocol;
      readonly key: string;
      readonly destination: string;
      readonly title: string | null;
    };

export interface ContentNodeView {
  readonly id: NodeId;
  readonly version: NodeVersion;
  readonly stability: "provisional" | "stable";
  readonly source: SourceRangeView;
  readonly body: SourceRangeView;
  readonly children: ChildListView;
  readonly content: ContentKindView;
}

export interface NodeView {
  readonly schema: string;
  readonly kind: "node_view";
  readonly node: ContentNodeView;
  readonly bodyText: string;
}

export interface SemanticResourceView {
  readonly id: ResourceId;
  readonly version: ResourceVersion;
  readonly content: SemanticResourceKindView;
}

export interface ResourceView {
  readonly schema: string;
  readonly kind: "resource_view";
  readonly resource: SemanticResourceView;
}

export interface ProcessorKeyView {
  readonly epoch: Epoch;
  readonly nodeId: NodeId;
  readonly processorId: string;
  readonly nodeVersion: NodeVersion;
  readonly inputVersion: ProcessorInputVersion;
  readonly processorVersion: string;
  readonly configurationVersion: string;
  readonly generation: RequestGeneration;
}

export interface ProcessorRequestView {
  readonly schema: string;
  readonly kind: "processor_request";
  readonly requestId: RequestGeneration;
  readonly key: ProcessorKeyView;
  readonly input: {
    readonly node: ContentNodeView;
    readonly body: string;
    readonly resource: SemanticResourceView | null;
  };
}

export interface ProcessorCompletionView {
  readonly schema: string;
  readonly kind: "processor_completion";
  readonly requestId: RequestGeneration;
  readonly outcome: "applied" | "stale";
}

export const PROCESSOR_FAILURE_CODES = [
  "processor",
  "panic",
  "invalid_request",
  "cancelled",
  "unsupported_content",
  "unresolved_context",
  "invalid_context",
  "resource_limit",
] as const;

export type ProcessorFailureCode = (typeof PROCESSOR_FAILURE_CODES)[number];

/** @internal */
export function isProcessorFailureCode(value: unknown): value is ProcessorFailureCode {
  return (
    typeof value === "string" &&
    (PROCESSOR_FAILURE_CODES as readonly string[]).includes(value)
  );
}

export type ArtifactChangeKindView =
  | { readonly kind: "pending" }
  | { readonly kind: "ready"; readonly artifactBytes: DecimalCounter }
  | { readonly kind: "failed"; readonly code: ProcessorFailureCode }
  | {
      readonly kind: "removed";
      readonly reason: string;
      readonly releasedArtifactBytes: DecimalCounter;
    };

export interface ArtifactChangeView {
  readonly schema: string;
  readonly kind: "artifact_change";
  readonly key: ProcessorKeyView;
  readonly change: ArtifactChangeKindView;
}

export type ArtifactPayloadView =
  | { readonly kind: "text"; readonly text: string }
  | { readonly kind: "binary"; readonly bytes: Uint8Array }
  | {
      readonly kind: "citation";
      readonly key: string;
      readonly destination: string;
      readonly title: string | null;
    };

export interface ProcessorArtifactView {
  readonly protocol: string;
  readonly mediaType: string;
  readonly payload: ArtifactPayloadView;
}

export interface ArtifactView {
  readonly schema: string;
  readonly kind: "artifact_view";
  readonly key: ProcessorKeyView;
  readonly state: "pending" | "ready" | "failed";
  readonly artifact: ProcessorArtifactView | null;
  readonly failure: {
    readonly code: ProcessorFailureCode;
    readonly message: string;
  } | null;
}

export type DecodedBindingView =
  | ReducerUpdateView
  | NodeView
  | ResourceView
  | PendingSourceView
  | ProcessorRequestView
  | ProcessorCompletionView
  | ArtifactChangeView
  | ArtifactView;

export class MdstreamError extends Error {
  readonly status: number;
  readonly statusName: string;
  readonly detailCode: string;
  readonly schema: string | undefined;

  constructor(
    message: string,
    options: {
      readonly status: number;
      readonly statusName: string;
      readonly detailCode: string;
      readonly schema?: string;
      readonly cause?: unknown;
    },
  ) {
    super(message, { cause: options.cause });
    this.name = "MdstreamError";
    this.status = options.status;
    this.statusName = options.statusName;
    this.detailCode = options.detailCode;
    this.schema = options.schema;
  }

  static from(value: unknown): MdstreamError {
    if (value instanceof MdstreamError) {
      return value;
    }
    if (isRecord(value)) {
      const message = optionalString(value.message) ?? "mdstream operation failed";
      const status = typeof value.status === "number" ? value.status : 12;
      const statusName = optionalString(value.status_name) ?? "MDSTREAM_INTERNAL_ERROR";
      const detailCode = optionalString(value.detail_code) ?? "bindings.javascript_error";
      const schema = optionalString(value.schema);
      return new MdstreamError(message, {
        status,
        statusName,
        detailCode,
        ...(schema === undefined ? {} : { schema }),
        cause: value,
      });
    }
    return new MdstreamError(errorMessage(value), {
      status: 12,
      statusName: "MDSTREAM_INTERNAL_ERROR",
      detailCode: "bindings.javascript_error",
      cause: value,
    });
  }
}

const decoder = new TextDecoder("utf-8", { fatal: true });
const decimalPattern = /^(0|[1-9][0-9]*)$/;
const opaqueIdentifierPattern = /^[A-Za-z0-9._:-]{1,128}$/;
const maxU64 = "18446744073709551615";
const maxU128 = "340282366920938463463374607431768211455";

/** @internal */
export function decodeBindingView(
  payloadKind: BindingPayloadKind,
  bytes: Uint8Array,
  expectedSchema: string,
): DecodedBindingView {
  const record = parseJsonRecord(bytes, "binding payload");
  requiredLiteral(record.schema, expectedSchema, "schema");
  requiredLiteral(record.kind, expectedViewKind(payloadKind), "kind");

  let decoded: DecodedBindingView;
  switch (payloadKind) {
    case BindingPayloadKind.ReducerUpdate:
      decoded = decodeReducerUpdate(record, expectedSchema);
      break;
    case BindingPayloadKind.NodeView:
      decoded = decodeNodeView(record, expectedSchema);
      break;
    case BindingPayloadKind.ResourceView:
      decoded = decodeResourceView(record, expectedSchema);
      break;
    case BindingPayloadKind.PendingSourceView:
      decoded = decodePendingSourceView(record, expectedSchema);
      break;
    case BindingPayloadKind.ProcessorRequest:
      decoded = decodeProcessorRequest(record, expectedSchema);
      break;
    case BindingPayloadKind.ProcessorCompletion:
      decoded = decodeProcessorCompletion(record, expectedSchema);
      break;
    case BindingPayloadKind.ArtifactChange:
      decoded = decodeArtifactChange(record, expectedSchema);
      break;
    case BindingPayloadKind.ArtifactView:
      decoded = decodeArtifactView(record, expectedSchema);
      break;
    case BindingPayloadKind.Change:
    case BindingPayloadKind.Snapshot:
      throw invalidPayload("canonical byte payloads must not be decoded as binding views");
  }
  return deepFreeze(decoded);
}

/** @internal */
export function decodeMetricsPayload(
  bytes: Uint8Array,
  expectedKind: "binding_metrics" | "processor_metrics",
  fields: readonly string[],
): Readonly<Record<string, DecimalCounter>> {
  const expectedLength = 6 + fields.length * 8;
  const kind = expectedKind === "binding_metrics" ? 1 : 2;
  if (
    bytes.byteLength !== expectedLength ||
    bytes[0] !== 0x4d ||
    bytes[1] !== 0x44 ||
    bytes[2] !== 0x4d ||
    bytes[3] !== 1 ||
    bytes[4] !== kind ||
    bytes[5] !== fields.length
  ) {
    throw invalidPayload(`${expectedKind} frame has an unsupported layout`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const metrics: Record<string, DecimalCounter> = {};
  for (let index = 0; index < fields.length; index += 1) {
    metrics[fields[index]!] = view.getBigUint64(6 + index * 8, true).toString() as DecimalCounter;
  }
  return Object.freeze(metrics);
}

export function asEpoch(value: string): Epoch {
  return inputDecimal(value, "epoch", maxU64) as Epoch;
}

export function asNodeId(value: string): NodeId {
  return inputDecimal(value, "node id", maxU128) as NodeId;
}

export function asResourceId(value: string): ResourceId {
  return inputDecimal(value, "resource id", maxU128) as ResourceId;
}

export function asCanonicalChangeBytes(value: Uint8Array): CanonicalChangeBytes {
  return value as CanonicalChangeBytes;
}

export function asCanonicalSnapshotBytes(value: Uint8Array): CanonicalSnapshotBytes {
  return value as CanonicalSnapshotBytes;
}

function decodeReducerUpdate(
  value: Record<string, unknown>,
  schema: string,
): ReducerUpdateView {
  const hasTransition = Object.hasOwn(value, "transition");
  exactKeys(
    value,
    [
      "schema",
      "kind",
      "outcome",
      "status",
      "impact",
      "document",
      ...(hasTransition ? ["transition"] : []),
    ],
    "reducer update",
  );
  const transition = hasTransition
    ? decodeTransitionEnvelope(requiredRecord(value.transition, "transition"))
    : undefined;
  return {
    schema,
    kind: "reducer_update",
    outcome: decodeOutcome(requiredRecord(value.outcome, "outcome")),
    status: decodeStatus(requiredRecord(value.status, "status")),
    impact: decodeImpact(requiredRecord(value.impact, "impact")),
    document:
      value.document === null
        ? null
        : decodeDocument(requiredRecord(value.document, "document")),
    ...(transition === undefined ? {} : { transition }),
  };
}

function decodeTransitionEnvelope(
  value: Record<string, unknown>,
): TransitionEnvelopeView {
  exactKeys(value, ["schema", "facts"], "transition");
  requiredLiteral(value.schema, TRANSITION_SCHEMA_DRAFT, "transition.schema");
  return {
    schema: TRANSITION_SCHEMA_DRAFT,
    facts: decodeTransitionFacts(requiredRecord(value.facts, "transition.facts")),
  };
}

function decodeTransitionFacts(value: Record<string, unknown>): TransitionFactsView {
  const scope = requiredString(value.scope, "transition.facts.scope");
  const before = requiredNullableRecord(value, "before", "transition.facts.before");
  const after = decodeDocumentStateStamp(
    requiredRecord(value.after, "transition.facts.after"),
  );
  switch (scope) {
    case "continuous":
      exactKeys(
        value,
        ["scope", "before", "after", "nodes", "structures", "resources"],
        "continuous transition facts",
      );
      return {
        scope,
        before: before === null ? null : decodeDocumentStateStamp(before),
        after,
        nodes: requiredArray(value.nodes, "transition.facts.nodes").map(
          (node) => decodeNodeTransition(requiredRecord(node, "transition node")),
        ),
        structures: requiredArray(
          value.structures,
          "transition.facts.structures",
        ).map((structure) =>
          decodeStructureTransition(requiredRecord(structure, "structure transition"))
        ),
        resources: requiredArray(
          value.resources,
          "transition.facts.resources",
        ).map((resource) =>
          decodeResourceTransition(requiredRecord(resource, "resource transition"))
        ),
      };
    case "full_replace":
      exactKeys(
        value,
        ["scope", "before", "after"],
        "full-replace transition facts",
      );
      return {
        scope,
        before: before === null ? null : decodeDocumentStateStamp(before),
        after,
      };
    default:
      throw invalidPayload(`unknown transition scope ${scope}`);
  }
}

function decodeDocumentStateStamp(
  value: Record<string, unknown>,
): DocumentStateStampView {
  exactKeys(
    value,
    [
      "continuity_generation",
      "coordinate",
      "lifecycle",
      "projection_cursor",
      "roots_version",
    ],
    "transition document stamp",
  );
  const lifecycle = requiredString(
    value.lifecycle,
    "transition document stamp lifecycle",
  );
  if (lifecycle !== "open" && lifecycle !== "finalized") {
    throw invalidPayload(`unknown transition document lifecycle ${lifecycle}`);
  }
  const coordinate = requiredRecord(
    value.coordinate,
    "transition document stamp coordinate",
  );
  exactKeys(
    coordinate,
    ["epoch", "sequence", "change_id", "source_cursor"],
    "transition document stamp coordinate",
  );
  return {
    continuityGeneration: decimalU64(
      requiredString(value.continuity_generation, "continuity_generation"),
      "continuity_generation",
    ) as ContinuityGeneration,
    coordinate: decodeCoordinate(coordinate),
    lifecycle,
    projectionCursor: decimalU64(
      requiredString(value.projection_cursor, "projection_cursor"),
      "projection_cursor",
    ) as SourceCursor,
    rootsVersion: opaqueIdentifier(
      value.roots_version,
      "roots_version",
    ) as StructureVersion,
  };
}

function decodeTransitionNodeKey(
  value: Record<string, unknown>,
): TransitionNodeKeyView {
  exactKeys(
    value,
    ["continuity_generation", "epoch", "node_id"],
    "transition node key",
  );
  return {
    continuityGeneration: decimalU64(
      requiredString(value.continuity_generation, "continuity_generation"),
      "continuity_generation",
    ) as ContinuityGeneration,
    epoch: decimalU64(
      requiredString(value.epoch, "transition node epoch"),
      "transition node epoch",
    ) as Epoch,
    nodeId: decimalU128(
      requiredString(value.node_id, "transition node id"),
      "transition node id",
    ) as NodeId,
  };
}

function decodeTransitionResourceKey(
  value: Record<string, unknown>,
): TransitionResourceKeyView {
  exactKeys(
    value,
    ["continuity_generation", "epoch", "resource_id"],
    "transition resource key",
  );
  return {
    continuityGeneration: decimalU64(
      requiredString(value.continuity_generation, "continuity_generation"),
      "continuity_generation",
    ) as ContinuityGeneration,
    epoch: decimalU64(
      requiredString(value.epoch, "transition resource epoch"),
      "transition resource epoch",
    ) as Epoch,
    resourceId: decimalU128(
      requiredString(value.resource_id, "transition resource id"),
      "transition resource id",
    ) as ResourceId,
  };
}

function decodeTransitionOwner(
  value: Record<string, unknown>,
): TransitionChildListOwnerView {
  const kind = requiredString(value.kind, "transition owner kind");
  switch (kind) {
    case "document":
      exactKeys(value, ["kind"], "document transition owner");
      return { kind };
    case "node":
      exactKeys(value, ["kind", "key"], "node transition owner");
      return {
        kind,
        key: decodeTransitionNodeKey(
          requiredRecord(value.key, "transition owner node key"),
        ),
      };
    default:
      throw invalidPayload(`unknown transition owner ${kind}`);
  }
}

function decodeNodeStateStamp(value: Record<string, unknown>): NodeStateStampView {
  exactKeys(
    value,
    ["version", "stability", "parent", "children_version"],
    "transition node stamp",
  );
  const stability = requiredString(value.stability, "transition node stability");
  if (stability !== "provisional" && stability !== "stable") {
    throw invalidPayload(`unknown transition node stability ${stability}`);
  }
  const parent = requiredNullableRecord(value, "parent", "transition node parent");
  return {
    version: opaqueIdentifier(value.version, "transition node version") as NodeVersion,
    stability,
    parent: parent === null ? null : decodeTransitionOwner(parent),
    childrenVersion: opaqueIdentifier(
      value.children_version,
      "transition children version",
    ) as StructureVersion,
  };
}

function decodeTextTransition(value: Record<string, unknown>): TextTransitionView {
  const kind = requiredString(value.kind, "text transition kind");
  switch (kind) {
    case "projection_append": {
      exactKeys(value, ["kind", "range", "text"], "projection-append transition");
      const range = requiredRecord(value.range, "projection-append range");
      exactKeys(range, ["start", "end"], "projection-append range");
      return {
        kind,
        range: decodeRange(range),
        text: requiredString(value.text, "projection-append text"),
      };
    }
    case "replacement":
      exactKeys(value, ["kind"], "replacement transition");
      return { kind };
    default:
      throw invalidPayload(`unknown text transition ${kind}`);
  }
}

function decodeNodeTransition(value: Record<string, unknown>): NodeTransitionView {
  exactKeys(value, ["key", "before", "after", "text"], "node transition");
  const before = requiredNullableRecord(value, "before", "node transition before");
  const after = requiredNullableRecord(value, "after", "node transition after");
  const text = requiredNullableRecord(value, "text", "node text transition");
  return {
    key: decodeTransitionNodeKey(requiredRecord(value.key, "node transition key")),
    before: before === null ? null : decodeNodeStateStamp(before),
    after: after === null ? null : decodeNodeStateStamp(after),
    text: text === null ? null : decodeTextTransition(text),
  };
}

function decodeStructureTransition(
  value: Record<string, unknown>,
): StructureTransitionView {
  exactKeys(
    value,
    [
      "owner",
      "before_version",
      "after_version",
      "start",
      "removed",
      "inserted",
    ],
    "structure transition",
  );
  return {
    owner: decodeTransitionOwner(
      requiredRecord(value.owner, "structure transition owner"),
    ),
    beforeVersion: opaqueIdentifier(
      value.before_version,
      "structure before version",
    ) as StructureVersion,
    afterVersion: opaqueIdentifier(
      value.after_version,
      "structure after version",
    ) as StructureVersion,
    start: requiredInteger(value.start, "structure transition start", 0xffff_ffff),
    removed: decodeTransitionNodeKeyArray(value.removed, "removed transition nodes"),
    inserted: decodeTransitionNodeKeyArray(value.inserted, "inserted transition nodes"),
  };
}

function decodeResourceTransition(
  value: Record<string, unknown>,
): ResourceTransitionView {
  exactKeys(
    value,
    ["key", "before_version", "after_version", "affected_nodes"],
    "resource transition",
  );
  return {
    key: decodeTransitionResourceKey(
      requiredRecord(value.key, "resource transition key"),
    ),
    beforeVersion: requiredNullableVersion(
      value,
      "before_version",
      "resource before version",
    ),
    afterVersion: requiredNullableVersion(
      value,
      "after_version",
      "resource after version",
    ),
    affectedNodes: decodeTransitionNodeKeyArray(
      value.affected_nodes,
      "resource affected nodes",
    ),
  };
}

function decodeTransitionNodeKeyArray(
  value: unknown,
  field: string,
): readonly TransitionNodeKeyView[] {
  return requiredArray(value, field).map((entry) =>
    decodeTransitionNodeKey(requiredRecord(entry, field))
  );
}

function decodeOutcome(value: Record<string, unknown>): ApplyOutcomeView {
  const kind = requiredString(value.kind, "outcome.kind");
  switch (kind) {
    case "applied":
    case "recovered":
      return { kind, coordinate: decodeCoordinate(requiredRecord(value.coordinate, "coordinate")) };
    case "idempotent":
      return { kind };
    case "stale":
      return {
        kind,
        current: decodeCoordinate(requiredRecord(value.current, "current")),
        receivedEpoch: decimalU64(requiredString(value.received_epoch, "received_epoch"), "received_epoch") as Epoch,
        receivedSequence: decimalU64(requiredString(value.received_sequence, "received_sequence"), "received_sequence") as Sequence,
      };
    case "recovery_required":
      return {
        kind,
        lastGood: decodeCoordinate(requiredRecord(value.last_good, "last_good")),
        reason: decodeRecoveryReason(requiredRecord(value.reason, "reason")),
      };
    default:
      throw invalidPayload(`unknown reducer outcome ${kind}`);
  }
}

function decodeStatus(value: Record<string, unknown>): ReducerStatusView {
  const kind = requiredString(value.kind, "status.kind");
  switch (kind) {
    case "uninitialized":
    case "ready":
      return { kind };
    case "needs_snapshot":
      return {
        kind,
        lastGood: decodeCoordinate(requiredRecord(value.last_good, "last_good")),
        reason: decodeRecoveryReason(requiredRecord(value.reason, "reason")),
      };
    default:
      throw invalidPayload(`unknown reducer status ${kind}`);
  }
}

function decodeRecoveryReason(value: Record<string, unknown>): RecoveryReasonView {
  requiredString(value.kind, "recovery reason kind");
  return value as RecoveryReasonView;
}

function decodeImpact(value: Record<string, unknown>): ChangeImpactView {
  return {
    changedNodeIds: decimalU128Array(value.changed_node_ids, "changed_node_ids") as readonly NodeId[],
    removedNodeIds: decimalU128Array(value.removed_node_ids, "removed_node_ids") as readonly NodeId[],
    changedResourceIds: decimalU128Array(value.changed_resource_ids, "changed_resource_ids") as readonly ResourceId[],
    removedResourceIds: decimalU128Array(value.removed_resource_ids, "removed_resource_ids") as readonly ResourceId[],
    sourceChanged: requiredBoolean(value.source_changed, "source_changed"),
    projectionChanged: requiredBoolean(value.projection_changed, "projection_changed"),
    lifecycleChanged: requiredBoolean(value.lifecycle_changed, "lifecycle_changed"),
    rootsChanged: requiredBoolean(value.roots_changed, "roots_changed"),
    fullReplace: requiredBoolean(value.full_replace, "full_replace"),
  };
}

function decodeDocument(value: Record<string, unknown>): DocumentSummaryView {
  const lifecycle = requiredString(value.lifecycle, "document.lifecycle");
  if (lifecycle !== "open" && lifecycle !== "finalized") {
    throw invalidPayload(`unknown document lifecycle ${lifecycle}`);
  }
  const roots = value.roots === undefined
    ? undefined
    : decodeChildList(requiredRecord(value.roots, "document.roots"));
  return {
    coordinate: decodeCoordinate(requiredRecord(value.coordinate, "document.coordinate")),
    lifecycle,
    projectionCursor: decimalU64(requiredString(value.projection_cursor, "projection_cursor"), "projection_cursor") as SourceCursor,
    ...(roots === undefined ? {} : { roots }),
  };
}

function decodeCoordinate(value: Record<string, unknown>): CoordinateView {
  return {
    epoch: decimalU64(requiredString(value.epoch, "coordinate.epoch"), "coordinate.epoch") as Epoch,
    sequence: decimalU64(requiredString(value.sequence, "coordinate.sequence"), "coordinate.sequence") as Sequence,
    changeId: opaqueIdentifier(value.change_id, "coordinate.change_id") as ChangeId,
    sourceCursor: decimalU64(requiredString(value.source_cursor, "coordinate.source_cursor"), "coordinate.source_cursor") as SourceCursor,
  };
}

function decodeNodeView(value: Record<string, unknown>, schema: string): NodeView {
  return {
    schema,
    kind: "node_view",
    node: decodeNode(requiredRecord(value.node, "node")),
    bodyText: requiredString(value.body_text, "body_text"),
  };
}

function decodeNode(value: Record<string, unknown>): ContentNodeView {
  const stability = requiredString(value.stability, "node.stability");
  if (stability !== "provisional" && stability !== "stable") {
    throw invalidPayload(`unknown node stability ${stability}`);
  }
  return {
    id: decimalU128(requiredString(value.id, "node.id"), "node.id") as NodeId,
    version: opaqueIdentifier(value.version, "node.version") as NodeVersion,
    stability,
    source: decodeRange(requiredRecord(value.source, "node.source")),
    body: decodeRange(requiredRecord(value.body, "node.body")),
    children: decodeChildList(requiredRecord(value.children, "node.children")),
    content: decodeContentKind(requiredRecord(value.content, "node.content")),
  };
}

function decodeContentKind(value: Record<string, unknown>): ContentKindView {
  const kind = requiredString(value.kind, "node.content.kind");
  switch (kind) {
    case "paragraph":
    case "emphasis":
    case "strong":
    case "strikethrough":
    case "thematic_break":
    case "table_head":
    case "table_body":
    case "table_row":
    case "soft_break":
    case "hard_break":
      exactKeys(value, ["kind"], `content ${kind}`);
      return { kind };
    case "heading":
      exactKeys(value, ["kind", "level"], "content heading");
      return { kind, level: requiredInteger(value.level, "heading.level", 255) };
    case "text":
      exactKeys(value, ["kind", "text"], "content text");
      return {
        kind,
        text: decodeSemanticText(requiredRecord(value.text, "text.text")),
      };
    case "link":
      exactKeys(value, ["kind", "target", "reference_label", "style"], "content link");
      return {
        kind,
        target: nullableResourceRef(value, "target", "link.target"),
        referenceLabel: requiredNullableString(value, "reference_label", "link.reference_label"),
        style: linkStyle(value.style, "link.style"),
      };
    case "image":
      exactKeys(value, ["kind", "target", "reference_label", "style", "alt"], "content image");
      return {
        kind,
        target: nullableResourceRef(value, "target", "image.target"),
        referenceLabel: requiredNullableString(value, "reference_label", "image.reference_label"),
        style: linkStyle(value.style, "image.style"),
        alt: decodeSemanticText(requiredRecord(value.alt, "image.alt")),
      };
    case "inline_code":
      exactKeys(value, ["kind", "text"], "content inline_code");
      return {
        kind,
        text: decodeSemanticText(requiredRecord(value.text, "inline_code.text")),
      };
    case "code_block":
      exactKeys(value, ["kind", "syntax", "info", "text"], "content code_block");
      return {
        kind,
        syntax: decodeCodeBlockSyntax(requiredRecord(value.syntax, "code_block.syntax")),
        info: requiredNullableString(value, "info", "code_block.info"),
        text: decodeSemanticText(requiredRecord(value.text, "code_block.text")),
      };
    case "list":
      exactKeys(value, ["kind", "ordered", "start", "tight"], "content list");
      return {
        kind,
        ordered: requiredBoolean(value.ordered, "list.ordered"),
        start: requiredNullableInteger(value, "start", "list.start", 0xffff_ffff),
        tight: requiredBoolean(value.tight, "list.tight"),
      };
    case "list_item":
      exactKeys(value, ["kind", "checked"], "content list_item");
      return {
        kind,
        checked: requiredNullableBoolean(value, "checked", "list_item.checked"),
      };
    case "block_quote":
      exactKeys(value, ["kind", "style"], "content block_quote");
      return { kind, style: blockQuoteKind(value.style, "block_quote.style") };
    case "table":
      exactKeys(value, ["kind", "alignments"], "content table");
      return {
        kind,
        alignments: requiredArray(value.alignments, "table.alignments").map(
          (alignment) => tableAlignment(alignment, "table.alignment"),
        ),
      };
    case "table_cell":
      exactKeys(value, ["kind", "column"], "content table_cell");
      return {
        kind,
        column: requiredInteger(value.column, "table_cell.column", 0xffff_ffff),
      };
    case "html":
      exactKeys(value, ["kind", "block", "text"], "content html");
      return {
        kind,
        block: requiredBoolean(value.block, "html.block"),
        text: decodeSemanticText(requiredRecord(value.text, "html.text")),
      };
    case "math":
      exactKeys(value, ["kind", "display", "text"], "content math");
      return {
        kind,
        display: requiredBoolean(value.display, "math.display"),
        text: decodeSemanticText(requiredRecord(value.text, "math.text")),
      };
    case "footnote_definition":
      exactKeys(value, ["kind", "label", "target"], "content footnote_definition");
      return {
        kind,
        label: requiredString(value.label, "footnote_definition.label"),
        target: decodeResourceRef(requiredRecord(value.target, "footnote_definition.target")),
      };
    case "footnote_reference":
      exactKeys(value, ["kind", "label", "target"], "content footnote_reference");
      return {
        kind,
        label: requiredString(value.label, "footnote_reference.label"),
        target: nullableResourceRef(value, "target", "footnote_reference.target"),
      };
    case "citation_definition":
      exactKeys(value, ["kind", "key", "target"], "content citation_definition");
      return {
        kind,
        key: requiredString(value.key, "citation_definition.key"),
        target: decodeResourceRef(requiredRecord(value.target, "citation_definition.target")),
      };
    case "citation_reference":
      exactKeys(value, ["kind", "key", "target"], "content citation_reference");
      return {
        kind,
        key: requiredString(value.key, "citation_reference.key"),
        target: nullableResourceRef(value, "target", "citation_reference.target"),
      };
    case "custom":
      exactKeys(value, ["kind", "namespace", "name", "opaque", "attributes"], "content custom");
      return {
        kind,
        namespace: requiredString(value.namespace, "custom.namespace"),
        name: requiredString(value.name, "custom.name"),
        opaque: requiredBoolean(value.opaque, "custom.opaque"),
        attributes: stringRecord(value.attributes, "custom.attributes"),
      };
    default:
      throw invalidPayload(`unknown content kind ${kind}`);
  }
}

function decodeSemanticText(value: Record<string, unknown>): SemanticTextView {
  const kind = requiredString(value.kind, "semantic text kind");
  switch (kind) {
    case "source":
      exactKeys(value, ["kind"], "semantic text source");
      return { kind };
    case "normalized":
      exactKeys(value, ["kind", "value"], "semantic text normalized");
      return { kind, value: requiredString(value.value, "semantic text value") };
    default:
      throw invalidPayload(`unknown semantic text kind ${kind}`);
  }
}

function decodeCodeBlockSyntax(value: Record<string, unknown>): CodeBlockSyntaxView {
  const kind = requiredString(value.kind, "code block syntax kind");
  switch (kind) {
    case "indented":
      exactKeys(value, ["kind"], "indented code block syntax");
      return { kind };
    case "fenced":
      exactKeys(value, ["kind", "marker", "length"], "fenced code block syntax");
      return {
        kind,
        marker: codeFenceMarker(value.marker, "code block fence marker"),
        length: requiredInteger(value.length, "code block fence length", 0xffff_ffff),
      };
    default:
      throw invalidPayload(`unknown code block syntax ${kind}`);
  }
}

function decodeResourceRef(value: Record<string, unknown>): ResourceRefView {
  exactKeys(value, ["id", "version"], "resource reference");
  return {
    id: decimalU128(requiredString(value.id, "resource reference id"), "resource reference id") as ResourceId,
    version: opaqueIdentifier(
      value.version,
      "resource reference version",
    ) as ResourceVersion,
  };
}

function decodeRange(value: Record<string, unknown>): SourceRangeView {
  return {
    start: decimalU64(requiredString(value.start, "range.start"), "range.start") as SourceCursor,
    end: decimalU64(requiredString(value.end, "range.end"), "range.end") as SourceCursor,
  };
}

function decodeChildList(value: Record<string, unknown>): ChildListView {
  return {
    version: opaqueIdentifier(value.version, "child_list.version") as StructureVersion,
    children: decimalU128Array(value.children, "child_list.children") as readonly NodeId[],
  };
}

function decodeResourceView(value: Record<string, unknown>, schema: string): ResourceView {
  return {
    schema,
    kind: "resource_view",
    resource: decodeResource(requiredRecord(value.resource, "resource")),
  };
}

function decodePendingSourceView(
  value: Record<string, unknown>,
  schema: string,
): PendingSourceView {
  return {
    schema,
    kind: "pending_source_view",
    range: decodeRange(requiredRecord(value.range, "pending source range")),
    text: requiredString(value.text, "pending source text"),
  };
}

function decodeResource(value: Record<string, unknown>): SemanticResourceView {
  return {
    id: decimalU128(requiredString(value.id, "resource.id"), "resource.id") as ResourceId,
    version: opaqueIdentifier(value.version, "resource.version") as ResourceVersion,
    content: decodeSemanticResourceKind(
      requiredRecord(value.content, "resource.content"),
    ),
  };
}

function decodeSemanticResourceKind(
  value: Record<string, unknown>,
): SemanticResourceKindView {
  const kind = requiredString(value.kind, "resource.content.kind");
  switch (kind) {
    case "link":
      exactKeys(value, ["kind", "destination", "title"], "link resource");
      return {
        kind,
        destination: requiredString(value.destination, "link resource destination"),
        title: requiredNullableString(value, "title", "link resource title"),
      };
    case "footnote":
      exactKeys(value, ["kind", "label"], "footnote resource");
      return {
        kind,
        label: requiredString(value.label, "footnote resource label"),
      };
    case "citation": {
      exactKeys(
        value,
        ["kind", "protocol", "key", "destination", "title"],
        "citation resource",
      );
      requiredLiteral(value.protocol, "mdstream.citation/1", "citation protocol");
      return {
        kind,
        protocol: "mdstream.citation/1",
        key: requiredString(value.key, "citation resource key"),
        destination: requiredString(
          value.destination,
          "citation resource destination",
        ),
        title: requiredNullableString(value, "title", "citation resource title"),
      };
    }
    default:
      throw invalidPayload(`unknown semantic resource kind ${kind}`);
  }
}

function decodeProcessorRequest(
  value: Record<string, unknown>,
  schema: string,
): ProcessorRequestView {
  const input = requiredRecord(value.input, "processor input");
  return {
    schema,
    kind: "processor_request",
    requestId: decimalU64(requiredString(value.request_id, "request_id"), "request_id") as RequestGeneration,
    key: decodeProcessorKey(requiredRecord(value.key, "processor key")),
    input: {
      node: decodeNode(requiredRecord(input.node, "processor input node")),
      body: requiredString(input.body, "processor input body"),
      resource: input.resource === null
        ? null
        : decodeResource(requiredRecord(input.resource, "processor input resource")),
    },
  };
}

function decodeProcessorCompletion(
  value: Record<string, unknown>,
  schema: string,
): ProcessorCompletionView {
  const outcome = requiredString(value.outcome, "processor completion outcome");
  if (outcome !== "applied" && outcome !== "stale") {
    throw invalidPayload(`unknown processor completion outcome ${outcome}`);
  }
  return {
    schema,
    kind: "processor_completion",
    requestId: decimalU64(requiredString(value.request_id, "request_id"), "request_id") as RequestGeneration,
    outcome,
  };
}

function decodeProcessorKey(value: Record<string, unknown>): ProcessorKeyView {
  return {
    epoch: decimalU64(requiredString(value.epoch, "processor key epoch"), "processor key epoch") as Epoch,
    nodeId: decimalU128(requiredString(value.node_id, "processor key node_id"), "processor key node_id") as NodeId,
    processorId: requiredString(value.processor_id, "processor key processor_id"),
    nodeVersion: opaqueIdentifier(
      value.node_version,
      "processor key node_version",
    ) as NodeVersion,
    inputVersion: opaqueIdentifier(
      value.input_version,
      "processor key input_version",
    ) as ProcessorInputVersion,
    processorVersion: requiredString(value.processor_version, "processor key processor_version"),
    configurationVersion: requiredString(value.configuration_version, "processor key configuration_version"),
    generation: decimalU64(requiredString(value.generation, "processor key generation"), "processor key generation") as RequestGeneration,
  };
}

function decodeArtifactChange(
  value: Record<string, unknown>,
  schema: string,
): ArtifactChangeView {
  const change = requiredRecord(value.change, "artifact change");
  const kind = requiredString(change.kind, "artifact change kind");
  let decoded: ArtifactChangeKindView;
  switch (kind) {
    case "pending":
      decoded = { kind };
      break;
    case "ready":
      decoded = {
        kind,
        artifactBytes: decimalU64(requiredString(change.artifact_bytes, "artifact_bytes"), "artifact_bytes") as DecimalCounter,
      };
      break;
    case "failed":
      decoded = { kind, code: failureCode(change.code) };
      break;
    case "removed":
      decoded = {
        kind,
        reason: requiredString(change.reason, "artifact removal reason"),
        releasedArtifactBytes: decimalU64(requiredString(change.released_artifact_bytes, "released_artifact_bytes"), "released_artifact_bytes") as DecimalCounter,
      };
      break;
    default:
      throw invalidPayload(`unknown artifact change ${kind}`);
  }
  return {
    schema,
    kind: "artifact_change",
    key: decodeProcessorKey(requiredRecord(value.key, "artifact key")),
    change: decoded,
  };
}

function decodeArtifactView(value: Record<string, unknown>, schema: string): ArtifactView {
  const state = requiredString(value.state, "artifact state");
  if (state !== "pending" && state !== "ready" && state !== "failed") {
    throw invalidPayload(`unknown artifact state ${state}`);
  }
  return {
    schema,
    kind: "artifact_view",
    key: decodeProcessorKey(requiredRecord(value.key, "artifact key")),
    state,
    artifact: value.artifact === null
      ? null
      : decodeArtifact(requiredRecord(value.artifact, "artifact")),
    failure: value.failure === null
      ? null
      : decodeFailure(requiredRecord(value.failure, "artifact failure")),
  };
}

function decodeArtifact(value: Record<string, unknown>): ProcessorArtifactView {
  const payload = requiredRecord(value.payload, "artifact payload");
  const kind = requiredString(payload.kind, "artifact payload kind");
  let decoded: ArtifactPayloadView;
  switch (kind) {
    case "text":
      decoded = { kind, text: requiredString(payload.text, "artifact text") };
      break;
    case "binary": {
      const values = requiredArray(payload.bytes, "artifact bytes");
      decoded = {
        kind,
        bytes: Uint8Array.from(values, (entry) => {
          if (
            !Number.isInteger(entry) ||
            (entry as number) < 0 ||
            (entry as number) > 255
          ) {
            throw invalidPayload("artifact bytes must contain octets");
          }
          return entry as number;
        }),
      };
      break;
    }
    case "citation":
      decoded = {
        kind,
        key: requiredString(payload.key, "citation key"),
        destination: requiredString(payload.destination, "citation destination"),
        title: payload.title === null ? null : requiredString(payload.title, "citation title"),
      };
      break;
    default:
      throw invalidPayload(`unknown artifact payload ${kind}`);
  }
  return {
    protocol: requiredString(value.protocol, "artifact protocol"),
    mediaType: requiredString(value.media_type, "artifact media_type"),
    payload: decoded,
  };
}

function decodeFailure(value: Record<string, unknown>): ArtifactView["failure"] {
  return {
    code: failureCode(value.code),
    message: requiredString(value.message, "failure message"),
  };
}

function failureCode(value: unknown): ProcessorFailureCode {
  const code = requiredString(value, "processor failure code");
  if (!isProcessorFailureCode(code)) {
    throw invalidPayload(`unknown processor failure code ${code}`);
  }
  return code;
}

function expectedViewKind(kind: BindingPayloadKind): string {
  switch (kind) {
    case BindingPayloadKind.ReducerUpdate:
      return "reducer_update";
    case BindingPayloadKind.NodeView:
      return "node_view";
    case BindingPayloadKind.ResourceView:
      return "resource_view";
    case BindingPayloadKind.PendingSourceView:
      return "pending_source_view";
    case BindingPayloadKind.ProcessorRequest:
      return "processor_request";
    case BindingPayloadKind.ProcessorCompletion:
      return "processor_completion";
    case BindingPayloadKind.ArtifactChange:
      return "artifact_change";
    case BindingPayloadKind.ArtifactView:
      return "artifact_view";
    case BindingPayloadKind.Change:
      return "change";
    case BindingPayloadKind.Snapshot:
      return "snapshot";
  }
}

function decimalU64(value: string, field: string): string {
  return outputDecimal(value, field, maxU64);
}

function decimalU128(value: string, field: string): string {
  return outputDecimal(value, field, maxU128);
}

function outputDecimal(value: string, field: string, maximum: string): string {
  if (!decimalPattern.test(value) || decimalExceeds(value, maximum)) {
    throw invalidPayload(`${field} must be a canonical unsigned decimal string`);
  }
  return value;
}

function inputDecimal(value: string, field: string, maximum: string): string {
  if (!decimalPattern.test(value) || decimalExceeds(value, maximum)) {
    throw new MdstreamError(
      `${field} must be a canonical unsigned decimal string within its supported range`,
      {
        status: 1,
        statusName: "MDSTREAM_INVALID_ARGUMENT",
        detailCode: "bindings.decimal_id",
      },
    );
  }
  return value;
}

function decimalExceeds(value: string, maximum: string): boolean {
  return (
    value.length > maximum.length ||
    (value.length === maximum.length && value > maximum)
  );
}

function decimalU128Array(value: unknown, field: string): readonly string[] {
  return requiredArray(value, field).map((entry) =>
    decimalU128(requiredString(entry, field), field),
  );
}

function nullableResourceRef(
  value: Record<string, unknown>,
  key: string,
  field: string,
): ResourceRefView | null {
  requireOwnKey(value, key, field);
  return value[key] === null
    ? null
    : decodeResourceRef(requiredRecord(value[key], field));
}

function requiredNullableRecord(
  value: Record<string, unknown>,
  key: string,
  field: string,
): Record<string, unknown> | null {
  requireOwnKey(value, key, field);
  return value[key] === null ? null : requiredRecord(value[key], field);
}

function requiredNullableVersion(
  value: Record<string, unknown>,
  key: string,
  field: string,
): ResourceVersion | null {
  requireOwnKey(value, key, field);
  return value[key] === null
    ? null
    : opaqueIdentifier(value[key], field) as ResourceVersion;
}

function requiredNullableString(
  value: Record<string, unknown>,
  key: string,
  field: string,
): string | null {
  requireOwnKey(value, key, field);
  return value[key] === null ? null : requiredString(value[key], field);
}

function requiredNullableBoolean(
  value: Record<string, unknown>,
  key: string,
  field: string,
): boolean | null {
  requireOwnKey(value, key, field);
  return value[key] === null ? null : requiredBoolean(value[key], field);
}

function requiredNullableInteger(
  value: Record<string, unknown>,
  key: string,
  field: string,
  maximum: number,
): number | null {
  requireOwnKey(value, key, field);
  return value[key] === null
    ? null
    : requiredInteger(value[key], field, maximum);
}

function requiredInteger(value: unknown, field: string, maximum: number): number {
  if (!Number.isInteger(value) || (value as number) < 0 || (value as number) > maximum) {
    throw invalidPayload(`${field} must be an unsigned integer no greater than ${maximum}`);
  }
  return value as number;
}

function stringRecord(value: unknown, field: string): Readonly<Record<string, string>> {
  const source = requiredRecord(value, field);
  const result: Record<string, string> = {};
  for (const [key, entry] of Object.entries(source)) {
    result[key] = requiredString(entry, `${field}.${key}`);
  }
  return result;
}

function tableAlignment(value: unknown, field: string): TableAlignment {
  const alignment = requiredString(value, field);
  if (alignment !== "none" && alignment !== "left" && alignment !== "center" && alignment !== "right") {
    throw invalidPayload(`unknown table alignment ${alignment}`);
  }
  return alignment;
}

function linkStyle(value: unknown, field: string): LinkStyle {
  const style = requiredString(value, field);
  switch (style) {
    case "inline":
    case "reference":
    case "reference_unknown":
    case "collapsed":
    case "collapsed_unknown":
    case "shortcut":
    case "shortcut_unknown":
    case "autolink":
    case "email":
      return style;
    default:
      throw invalidPayload(`unknown link style ${style}`);
  }
}

function blockQuoteKind(value: unknown, field: string): BlockQuoteKind {
  const kind = requiredString(value, field);
  switch (kind) {
    case "plain":
    case "note":
    case "tip":
    case "important":
    case "warning":
    case "caution":
      return kind;
    default:
      throw invalidPayload(`unknown block quote kind ${kind}`);
  }
}

function codeFenceMarker(value: unknown, field: string): CodeFenceMarker {
  const marker = requiredString(value, field);
  if (marker !== "backtick" && marker !== "tilde") {
    throw invalidPayload(`unknown code fence marker ${marker}`);
  }
  return marker;
}

function exactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
  field: string,
): void {
  const allowed = new Set(expected);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw invalidPayload(`${field} contains unknown field ${key}`);
    }
  }
}

function requireOwnKey(
  value: Record<string, unknown>,
  key: string,
  field: string,
): void {
  if (!Object.hasOwn(value, key)) {
    throw invalidPayload(`${field} is required`);
  }
}

function requiredRecord(value: unknown, field: string): Record<string, unknown> {
  if (!isRecord(value)) {
    throw invalidPayload(`${field} must be an object`);
  }
  return value;
}

function parseJsonRecord(bytes: Uint8Array, field: string): Record<string, unknown> {
  let value: unknown;
  try {
    value = JSON.parse(decoder.decode(bytes));
  } catch (error) {
    throw invalidPayload(`${field} is not valid UTF-8 JSON`, error);
  }
  return requiredRecord(value, field);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requiredArray(value: unknown, field: string): readonly unknown[] {
  if (!Array.isArray(value)) {
    throw invalidPayload(`${field} must be an array`);
  }
  return value;
}

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw invalidPayload(`${field} must be a string`);
  }
  return value;
}

function opaqueIdentifier(value: unknown, field: string): string {
  const identifier = requiredString(value, field);
  if (!opaqueIdentifierPattern.test(identifier)) {
    throw invalidPayload(
      `${field} must be a 1-128 byte ASCII opaque identifier`,
    );
  }
  return identifier;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function requiredBoolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") {
    throw invalidPayload(`${field} must be a boolean`);
  }
  return value;
}

function requiredLiteral(value: unknown, expected: string, field: string): void {
  if (value !== expected) {
    throw invalidPayload(`${field} must be ${expected}`);
  }
}

function deepFreeze<T>(value: T): T {
  if (typeof value !== "object" || value === null || ArrayBuffer.isView(value)) {
    return value;
  }
  for (const entry of Object.values(value)) {
    deepFreeze(entry);
  }
  return Object.freeze(value);
}

/** @internal */
export function invalidPayload(message: string, cause?: unknown): MdstreamError {
  return new MdstreamError(message, {
    status: 12,
    statusName: "MDSTREAM_INTERNAL_ERROR",
    detailCode: "bindings.invalid_payload",
    ...(cause === undefined ? {} : { cause }),
  });
}

function errorMessage(value: unknown): string {
  if (value instanceof Error) {
    return value.message;
  }
  return typeof value === "string" ? value : "mdstream operation failed";
}
