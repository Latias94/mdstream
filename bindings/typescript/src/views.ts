import { BindingPayloadKind } from "./wasm.js";

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
  readonly changedNodeIds: readonly NodeId[];
  readonly removedNodeIds: readonly NodeId[];
  readonly changedResourceIds: readonly ResourceId[];
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

export interface ReducerUpdateView {
  readonly schema: string;
  readonly kind: "reducer_update";
  readonly outcome: ApplyOutcomeView;
  readonly status: ReducerStatusView;
  readonly impact: ChangeImpactView;
  readonly document: DocumentSummaryView | null;
}

export interface SourceRangeView {
  readonly start: SourceCursor;
  readonly end: SourceCursor;
}

export type ContentKindView = Readonly<Record<string, unknown>> & {
  readonly kind: string;
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
  readonly content: Readonly<Record<string, unknown>> & { readonly kind: string };
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

export type ProcessorFailureCode =
  | "processor"
  | "panic"
  | "invalid_request"
  | "cancelled"
  | "unsupported_content"
  | "unresolved_context"
  | "invalid_context"
  | "resource_limit";

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

/** @internal */
export function decodeBindingView(
  payloadKind: BindingPayloadKind,
  bytes: Uint8Array,
  expectedSchema: string,
): DecodedBindingView {
  const record = parseJsonRecord(bytes, "binding payload");
  requiredLiteral(record.schema, expectedSchema, "schema");
  requiredLiteral(record.kind, expectedViewKind(payloadKind), "kind");

  switch (payloadKind) {
    case BindingPayloadKind.ReducerUpdate:
      return decodeReducerUpdate(record, expectedSchema);
    case BindingPayloadKind.NodeView:
      return decodeNodeView(record, expectedSchema);
    case BindingPayloadKind.ResourceView:
      return decodeResourceView(record, expectedSchema);
    case BindingPayloadKind.ProcessorRequest:
      return decodeProcessorRequest(record, expectedSchema);
    case BindingPayloadKind.ProcessorCompletion:
      return decodeProcessorCompletion(record, expectedSchema);
    case BindingPayloadKind.ArtifactChange:
      return decodeArtifactChange(record, expectedSchema);
    case BindingPayloadKind.ArtifactView:
      return decodeArtifactView(record, expectedSchema);
    case BindingPayloadKind.Change:
    case BindingPayloadKind.Snapshot:
      throw invalidPayload("canonical byte payloads must not be decoded as binding views");
  }
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
  return metrics;
}

export function asNodeId(value: string): NodeId {
  return decimal(value, "node id") as NodeId;
}

export function asResourceId(value: string): ResourceId {
  return decimal(value, "resource id") as ResourceId;
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
  };
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
        receivedEpoch: decimal(requiredString(value.received_epoch, "received_epoch"), "received_epoch") as Epoch,
        receivedSequence: decimal(requiredString(value.received_sequence, "received_sequence"), "received_sequence") as Sequence,
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
    changedNodeIds: decimalArray(value.changed_node_ids, "changed_node_ids") as readonly NodeId[],
    removedNodeIds: decimalArray(value.removed_node_ids, "removed_node_ids") as readonly NodeId[],
    changedResourceIds: decimalArray(value.changed_resource_ids, "changed_resource_ids") as readonly ResourceId[],
    removedResourceIds: decimalArray(value.removed_resource_ids, "removed_resource_ids") as readonly ResourceId[],
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
    projectionCursor: decimal(requiredString(value.projection_cursor, "projection_cursor"), "projection_cursor") as SourceCursor,
    ...(roots === undefined ? {} : { roots }),
  };
}

function decodeCoordinate(value: Record<string, unknown>): CoordinateView {
  return {
    epoch: decimal(requiredString(value.epoch, "coordinate.epoch"), "coordinate.epoch") as Epoch,
    sequence: decimal(requiredString(value.sequence, "coordinate.sequence"), "coordinate.sequence") as Sequence,
    changeId: requiredString(value.change_id, "coordinate.change_id") as ChangeId,
    sourceCursor: decimal(requiredString(value.source_cursor, "coordinate.source_cursor"), "coordinate.source_cursor") as SourceCursor,
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
  const content = requiredRecord(value.content, "node.content");
  requiredString(content.kind, "node.content.kind");
  return {
    id: decimal(requiredString(value.id, "node.id"), "node.id") as NodeId,
    version: requiredString(value.version, "node.version") as NodeVersion,
    stability,
    source: decodeRange(requiredRecord(value.source, "node.source")),
    body: decodeRange(requiredRecord(value.body, "node.body")),
    children: decodeChildList(requiredRecord(value.children, "node.children")),
    content: content as ContentKindView,
  };
}

function decodeRange(value: Record<string, unknown>): SourceRangeView {
  return {
    start: decimal(requiredString(value.start, "range.start"), "range.start") as SourceCursor,
    end: decimal(requiredString(value.end, "range.end"), "range.end") as SourceCursor,
  };
}

function decodeChildList(value: Record<string, unknown>): ChildListView {
  return {
    version: requiredString(value.version, "child_list.version") as StructureVersion,
    children: decimalArray(value.children, "child_list.children") as readonly NodeId[],
  };
}

function decodeResourceView(value: Record<string, unknown>, schema: string): ResourceView {
  return {
    schema,
    kind: "resource_view",
    resource: decodeResource(requiredRecord(value.resource, "resource")),
  };
}

function decodeResource(value: Record<string, unknown>): SemanticResourceView {
  const content = requiredRecord(value.content, "resource.content");
  requiredString(content.kind, "resource.content.kind");
  return {
    id: decimal(requiredString(value.id, "resource.id"), "resource.id") as ResourceId,
    version: requiredString(value.version, "resource.version") as ResourceVersion,
    content: content as SemanticResourceView["content"],
  };
}

function decodeProcessorRequest(
  value: Record<string, unknown>,
  schema: string,
): ProcessorRequestView {
  const input = requiredRecord(value.input, "processor input");
  return {
    schema,
    kind: "processor_request",
    requestId: decimal(requiredString(value.request_id, "request_id"), "request_id") as RequestGeneration,
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
    requestId: decimal(requiredString(value.request_id, "request_id"), "request_id") as RequestGeneration,
    outcome,
  };
}

function decodeProcessorKey(value: Record<string, unknown>): ProcessorKeyView {
  return {
    epoch: decimal(requiredString(value.epoch, "processor key epoch"), "processor key epoch") as Epoch,
    nodeId: decimal(requiredString(value.node_id, "processor key node_id"), "processor key node_id") as NodeId,
    processorId: requiredString(value.processor_id, "processor key processor_id"),
    nodeVersion: requiredString(value.node_version, "processor key node_version") as NodeVersion,
    inputVersion: requiredString(value.input_version, "processor key input_version") as ProcessorInputVersion,
    processorVersion: requiredString(value.processor_version, "processor key processor_version"),
    configurationVersion: requiredString(value.configuration_version, "processor key configuration_version"),
    generation: decimal(requiredString(value.generation, "processor key generation"), "processor key generation") as RequestGeneration,
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
        artifactBytes: decimal(requiredString(change.artifact_bytes, "artifact_bytes"), "artifact_bytes") as DecimalCounter,
      };
      break;
    case "failed":
      decoded = { kind, code: failureCode(change.code) };
      break;
    case "removed":
      decoded = {
        kind,
        reason: requiredString(change.reason, "artifact removal reason"),
        releasedArtifactBytes: decimal(requiredString(change.released_artifact_bytes, "released_artifact_bytes"), "released_artifact_bytes") as DecimalCounter,
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
      const values = requiredArray(payload.bytes, "artifact bytes").map((entry) => {
        if (!Number.isInteger(entry) || (entry as number) < 0 || (entry as number) > 255) {
          throw invalidPayload("artifact bytes must contain octets");
        }
        return entry as number;
      });
      decoded = { kind, bytes: Uint8Array.from(values) };
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
  switch (code) {
    case "processor":
    case "panic":
    case "invalid_request":
    case "cancelled":
    case "unsupported_content":
    case "unresolved_context":
    case "invalid_context":
    case "resource_limit":
      return code;
    default:
      throw invalidPayload(`unknown processor failure code ${code}`);
  }
}

function expectedViewKind(kind: BindingPayloadKind): string {
  switch (kind) {
    case BindingPayloadKind.ReducerUpdate:
      return "reducer_update";
    case BindingPayloadKind.NodeView:
      return "node_view";
    case BindingPayloadKind.ResourceView:
      return "resource_view";
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

function decimal(value: string, field: string): string {
  if (!decimalPattern.test(value)) {
    throw invalidPayload(`${field} must be a canonical unsigned decimal string`);
  }
  return value;
}

function decimalArray(value: unknown, field: string): readonly string[] {
  return requiredArray(value, field).map((entry) =>
    decimal(requiredString(entry, field), field),
  );
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

function invalidPayload(message: string, cause?: unknown): MdstreamError {
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
