import {
  asCanonicalChangeBytes,
  asCanonicalSnapshotBytes,
  decodeBindingView,
  decodeMetricsPayload,
  invalidPayload,
  MdstreamError,
  type ArtifactChangeView,
  type ArtifactView,
  type CanonicalChangeBytes,
  type CanonicalSnapshotBytes,
  type ChangeImpactView,
  type DecimalCounter,
  type DocumentSummaryView,
  type Epoch,
  type NodeId,
  type NodeView,
  type PendingSourceView,
  type ProcessorCompletionView,
  type ProcessorFailureCode,
  type ProcessorInputVersion,
  type ProcessorRequestView,
  type ReducerStatusView,
  type ReducerUpdateView,
  type RequestGeneration,
  type ResourceId,
  type ResourceView,
  type TransitionFactsView,
} from "./views.js";
import {
  BindingPayloadKind,
  drainOutput,
  type WasmOutput,
  type WasmReducerSession,
} from "./wasm.js";

export type StoreListener = () => void;
export type TransitionListener = (batch: TransitionBatchView) => void;

export interface ExternalStore<Snapshot> {
  subscribe(listener: StoreListener): () => void;
  getSnapshot(): Snapshot;
}

export interface StoreSnapshot {
  readonly status: ReducerStatusView;
  readonly document: DocumentSummaryView | null;
  readonly impact: ChangeImpactView;
}

export interface TransitionBatchView {
  readonly facts: readonly TransitionFactsView[];
}

export interface ArtifactSlot {
  readonly epoch: Epoch;
  readonly nodeId: NodeId;
  readonly processorId: string;
}

export interface ReducerResult {
  readonly updates: readonly ReducerUpdateView[];
  readonly processorRequests: readonly ProcessorRequestView[];
  readonly processorCompletions: readonly ProcessorCompletionView[];
  readonly artifactChanges: readonly ArtifactChangeView[];
  readonly outputPayloadBytes: DecimalCounter;
}

export interface BindingMetricsView {
  readonly commands: DecimalCounter;
  readonly decodedChangePayloads: DecimalCounter;
  readonly decodedSnapshotPayloads: DecimalCounter;
  readonly changePayloads: DecimalCounter;
  readonly snapshotPayloads: DecimalCounter;
  readonly reducerUpdatePayloads: DecimalCounter;
  readonly processorRequestPayloads: DecimalCounter;
  readonly processorCompletionPayloads: DecimalCounter;
  readonly artifactChangePayloads: DecimalCounter;
  readonly artifactViewPayloads: DecimalCounter;
  readonly materializedNodeViews: DecimalCounter;
  readonly materializedResourceViews: DecimalCounter;
  readonly materializedPendingSourceViews: DecimalCounter;
  readonly encodedPayloadBytes: DecimalCounter;
  readonly pendingProcessorRequests: DecimalCounter;
}

export interface ProcessorMetricsView {
  readonly slots: DecimalCounter;
  readonly inFlightJobs: DecimalCounter;
  readonly inFlightInputBytes: DecimalCounter;
  readonly retainedArtifacts: DecimalCounter;
  readonly retainedArtifactBytes: DecimalCounter;
  readonly pendingChanges: DecimalCounter;
  readonly pendingChangeBytes: DecimalCounter;
  readonly issuedRequests: DecimalCounter;
  readonly acceptedResults: DecimalCounter;
  readonly staleResults: DecimalCounter;
  readonly releasedArtifacts: DecimalCounter;
  readonly storeEntryVisits: DecimalCounter;
  readonly inputMaterializations: DecimalCounter;
}

export interface MdstreamStoreView extends ExternalStore<StoreSnapshot> {
  subscribeTransitions(listener: TransitionListener): () => void;
  getPendingSourceSnapshot(): PendingSourceView | undefined;
  subscribePendingSource(listener: StoreListener): () => void;
  pendingSource(): ExternalStore<PendingSourceView | undefined>;
  getNodeSnapshot(id: NodeId): NodeView | undefined;
  subscribeNode(id: NodeId, listener: StoreListener): () => void;
  node(id: NodeId): ExternalStore<NodeView | undefined>;
  getResourceSnapshot(id: ResourceId): ResourceView | undefined;
  subscribeResource(id: ResourceId, listener: StoreListener): () => void;
  resource(id: ResourceId): ExternalStore<ResourceView | undefined>;
  getArtifactSnapshot(slot: ArtifactSlot): ArtifactView | undefined;
  subscribeArtifact(slot: ArtifactSlot, listener: StoreListener): () => void;
  artifact(slot: ArtifactSlot): ExternalStore<ArtifactView | undefined>;
  metrics(): BindingMetricsView;
  processorMetrics(): ProcessorMetricsView;
}

export interface MdstreamStore extends MdstreamStoreView {
  applyChange(change: CanonicalChangeBytes): ReducerResult;
  recoverSnapshot(snapshot: CanonicalSnapshotBytes): ReducerResult;
  createRecoverySnapshot(): CanonicalSnapshotBytes | undefined;
  close(): void;
}

/** @internal */
export interface BeginProcessorOptions {
  readonly expectedEpoch: Epoch;
  readonly nodeId: NodeId;
  readonly expectedInputVersion: ProcessorInputVersion;
  readonly processorId: string;
  readonly processorVersion: string;
  readonly configurationVersion: string;
  readonly acceptsProvisional: boolean;
  readonly allowProvisional: boolean;
}

/** @internal */
export interface InternalStoreEvents {
  readonly updates: readonly ReducerUpdateView[];
  readonly artifactChanges: readonly ArtifactChangeView[];
}

interface Notifications {
  root: boolean;
  pendingSource: boolean;
  readonly nodes: Set<string>;
  readonly resources: Set<string>;
  readonly artifacts: Set<string>;
}

interface ConsumedOutput extends ReducerResult {
  readonly snapshots: readonly CanonicalSnapshotBytes[];
  readonly nodeViews: readonly NodeView[];
  readonly resourceViews: readonly ResourceView[];
  readonly pendingSourceViews: readonly PendingSourceView[];
  readonly artifactViews: readonly ArtifactView[];
}

interface StoreOperation {
  readonly notifications: Notifications;
  readonly updates: ReducerUpdateView[];
  readonly artifactChanges: ArtifactChangeView[];
  readonly facts: TransitionFactsView[];
}

const emptyImpact: ChangeImpactView = Object.freeze({
  changedNodeIds: Object.freeze([]),
  removedNodeIds: Object.freeze([]),
  changedResourceIds: Object.freeze([]),
  removedResourceIds: Object.freeze([]),
  sourceChanged: false,
  projectionChanged: false,
  lifecycleChanged: false,
  rootsChanged: false,
  fullReplace: false,
});

const initialSnapshot: StoreSnapshot = Object.freeze({
  status: Object.freeze({ kind: "uninitialized" as const }),
  document: null,
  impact: emptyImpact,
});

const maxCachedMisses = 1_024;

const bindingMetricFields = [
  "commands",
  "decoded_change_payloads",
  "decoded_snapshot_payloads",
  "change_payloads",
  "snapshot_payloads",
  "reducer_update_payloads",
  "processor_request_payloads",
  "processor_completion_payloads",
  "artifact_change_payloads",
  "artifact_view_payloads",
  "materialized_node_views",
  "materialized_resource_views",
  "encoded_payload_bytes",
  "pending_processor_requests",
  "materialized_pending_source_views",
] as const;
const processorMetricFields = [
  "slots",
  "in_flight_jobs",
  "in_flight_input_bytes",
  "retained_artifacts",
  "retained_artifact_bytes",
  "pending_changes",
  "pending_change_bytes",
  "issued_requests",
  "accepted_results",
  "stale_results",
  "released_artifacts",
  "store_entry_visits",
  "input_materializations",
] as const;

/** @internal */
export class RustBackedStore implements MdstreamStore {
  readonly #session: WasmReducerSession;
  readonly #schema: string;
  readonly #captureTransitions: boolean;
  #snapshot = initialSnapshot;
  #operation: StoreOperation | undefined;
  #publishingTransitions = false;
  #closed = false;
  #eventSink: ((events: InternalStoreEvents) => void) | undefined;

  readonly #listeners = new Set<StoreListener>();
  readonly #transitionListeners = new Set<TransitionListener>();
  readonly #pendingSourceListeners = new Set<StoreListener>();
  readonly #nodeListeners = new Map<string, Set<StoreListener>>();
  readonly #resourceListeners = new Map<string, Set<StoreListener>>();
  readonly #artifactListeners = new Map<string, Set<StoreListener>>();

  readonly #nodeCache = new Map<string, NodeView>();
  readonly #resourceCache = new Map<string, ResourceView>();
  readonly #artifactCache = new Map<string, ArtifactView>();
  readonly #missingNodes = new BoundedKeySet(maxCachedMisses);
  readonly #missingResources = new BoundedKeySet(maxCachedMisses);
  readonly #missingArtifacts = new BoundedKeySet(maxCachedMisses);
  #pendingSourceCache: PendingSourceView | undefined;
  #pendingSourceLoaded = false;

  constructor(
    session: WasmReducerSession,
    schema: string,
    captureTransitions: boolean,
  ) {
    this.#session = session;
    this.#schema = schema;
    this.#captureTransitions = captureTransitions;
  }

  setEventSink(sink: ((events: InternalStoreEvents) => void) | undefined): void {
    this.#eventSink = sink;
  }

  /** @internal */
  runDocumentOperation<Result>(operation: () => Result): Result {
    this.#assertOpen();
    this.#assertMutationAllowed();
    if (!this.#captureTransitions) {
      return operation();
    }
    if (this.#operation !== undefined) {
      return operation();
    }

    const pending = createStoreOperation();
    this.#operation = pending;
    try {
      return operation();
    } finally {
      this.#operation = undefined;
      this.#finishDocumentOperation(pending);
    }
  }

  /** @internal */
  assertMutationAllowed(): void {
    this.#assertOpen();
    this.#assertMutationAllowed();
  }

  subscribe(listener: StoreListener): () => void {
    this.#assertOpen();
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  getSnapshot(): StoreSnapshot {
    return this.#snapshot;
  }

  subscribeTransitions(listener: TransitionListener): () => void {
    this.#assertOpen();
    this.#transitionListeners.add(listener);
    return () => this.#transitionListeners.delete(listener);
  }

  applyChange(change: CanonicalChangeBytes): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() => this.#session.applyChange(change)))
    );
  }

  recoverSnapshot(snapshot: CanonicalSnapshotBytes): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() => this.#session.recoverSnapshot(snapshot)))
    );
  }

  createRecoverySnapshot(): CanonicalSnapshotBytes | undefined {
    return this.runDocumentOperation(() => {
      this.#assertOpen();
      const output = this.#invoke(() => this.#session.snapshot());
      return output.snapshots[0];
    });
  }

  getPendingSourceSnapshot(): PendingSourceView | undefined {
    if (this.#pendingSourceLoaded) {
      return this.#pendingSourceCache;
    }
    this.#assertOpen();
    const output = this.#invoke(() => this.#session.pendingSourceView());
    if (output.pendingSourceViews.length > 1) {
      throw invalidPayload("pending source view returned more than one payload");
    }
    this.#pendingSourceCache = output.pendingSourceViews[0];
    this.#pendingSourceLoaded = true;
    return this.#pendingSourceCache;
  }

  subscribePendingSource(listener: StoreListener): () => void {
    this.#assertOpen();
    this.#pendingSourceListeners.add(listener);
    return () => this.#pendingSourceListeners.delete(listener);
  }

  pendingSource(): ExternalStore<PendingSourceView | undefined> {
    return Object.freeze({
      subscribe: (listener: StoreListener) => this.subscribePendingSource(listener),
      getSnapshot: () => this.getPendingSourceSnapshot(),
    });
  }

  getNodeSnapshot(id: NodeId): NodeView | undefined {
    const key = id as string;
    const cached = this.#nodeCache.get(key);
    if (cached !== undefined) {
      return cached;
    }
    if (this.#missingNodes.has(key)) {
      return undefined;
    }
    this.#assertOpen();
    try {
      const output = this.#invoke(() => this.#session.nodeView(id));
      const view = output.nodeViews[0];
      if (view === undefined) {
        this.#missingNodes.add(key);
      }
      return view;
    } catch (error) {
      const normalized = MdstreamError.from(error);
      if (normalized.detailCode === "bindings.node_not_found") {
        this.#missingNodes.add(key);
        return undefined;
      }
      throw normalized;
    }
  }

  subscribeNode(id: NodeId, listener: StoreListener): () => void {
    return subscribeKeyed(this.#nodeListeners, id as string, listener, () => this.#assertOpen());
  }

  node(id: NodeId): ExternalStore<NodeView | undefined> {
    return Object.freeze({
      subscribe: (listener: StoreListener) => this.subscribeNode(id, listener),
      getSnapshot: () => this.getNodeSnapshot(id),
    });
  }

  getResourceSnapshot(id: ResourceId): ResourceView | undefined {
    const key = id as string;
    const cached = this.#resourceCache.get(key);
    if (cached !== undefined) {
      return cached;
    }
    if (this.#missingResources.has(key)) {
      return undefined;
    }
    this.#assertOpen();
    try {
      const output = this.#invoke(() => this.#session.resourceView(id));
      const view = output.resourceViews[0];
      if (view === undefined) {
        this.#missingResources.add(key);
      }
      return view;
    } catch (error) {
      const normalized = MdstreamError.from(error);
      if (normalized.detailCode === "bindings.resource_not_found") {
        this.#missingResources.add(key);
        return undefined;
      }
      throw normalized;
    }
  }

  subscribeResource(id: ResourceId, listener: StoreListener): () => void {
    return subscribeKeyed(
      this.#resourceListeners,
      id as string,
      listener,
      () => this.#assertOpen(),
    );
  }

  resource(id: ResourceId): ExternalStore<ResourceView | undefined> {
    return Object.freeze({
      subscribe: (listener: StoreListener) => this.subscribeResource(id, listener),
      getSnapshot: () => this.getResourceSnapshot(id),
    });
  }

  getArtifactSnapshot(slot: ArtifactSlot): ArtifactView | undefined {
    const key = artifactSlotKey(slot);
    const cached = this.#artifactCache.get(key);
    if (cached !== undefined) {
      return cached;
    }
    if (this.#missingArtifacts.has(key)) {
      return undefined;
    }
    this.#assertOpen();
    const output = this.#invoke(() =>
      this.#session.artifactView(slot.epoch, slot.nodeId, slot.processorId),
    );
    const view = output.artifactViews[0];
    if (view === undefined) {
      this.#missingArtifacts.add(key);
    }
    return view;
  }

  subscribeArtifact(slot: ArtifactSlot, listener: StoreListener): () => void {
    return subscribeKeyed(
      this.#artifactListeners,
      artifactSlotKey(slot),
      listener,
      () => this.#assertOpen(),
    );
  }

  artifact(slot: ArtifactSlot): ExternalStore<ArtifactView | undefined> {
    return Object.freeze({
      subscribe: (listener: StoreListener) => this.subscribeArtifact(slot, listener),
      getSnapshot: () => this.getArtifactSnapshot(slot),
    });
  }

  beginProcessor(options: BeginProcessorOptions): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() =>
        this.#session.beginProcessorIfCurrent(
          options.expectedEpoch,
          options.nodeId,
          options.expectedInputVersion,
          options.processorId,
          options.processorVersion,
          options.configurationVersion,
          options.acceptsProvisional,
          options.allowProvisional,
        ),
      ))
    );
  }

  completeProcessorText(
    requestId: RequestGeneration,
    protocol: string,
    mediaType: string,
    text: string,
  ): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() =>
        this.#session.completeProcessorText(requestId, protocol, mediaType, text),
      ))
    );
  }

  completeProcessorBinary(
    requestId: RequestGeneration,
    protocol: string,
    mediaType: string,
    bytes: Uint8Array,
  ): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() =>
        this.#session.completeProcessorBinary(requestId, protocol, mediaType, bytes),
      ))
    );
  }

  failProcessor(
    requestId: RequestGeneration,
    code: ProcessorFailureCode,
    message: string,
  ): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() =>
        this.#session.failProcessor(requestId, code, message),
      ))
    );
  }

  cancelProcessor(requestId: RequestGeneration): ReducerResult {
    return this.runDocumentOperation(() =>
      this.#publicResult(this.#invoke(() => this.#session.cancelProcessor(requestId)))
    );
  }

  metrics(): BindingMetricsView {
    this.#assertOpen();
    return readBindingMetrics(this.#session.metrics());
  }

  processorMetrics(): ProcessorMetricsView {
    this.#assertOpen();
    const metrics = decodeMetricsPayload(
      this.#session.processorMetrics(),
      "processor_metrics",
      processorMetricFields,
    );
    return {
      slots: metric(metrics, "slots"),
      inFlightJobs: metric(metrics, "in_flight_jobs"),
      inFlightInputBytes: metric(metrics, "in_flight_input_bytes"),
      retainedArtifacts: metric(metrics, "retained_artifacts"),
      retainedArtifactBytes: metric(metrics, "retained_artifact_bytes"),
      pendingChanges: metric(metrics, "pending_changes"),
      pendingChangeBytes: metric(metrics, "pending_change_bytes"),
      issuedRequests: metric(metrics, "issued_requests"),
      acceptedResults: metric(metrics, "accepted_results"),
      staleResults: metric(metrics, "stale_results"),
      releasedArtifacts: metric(metrics, "released_artifacts"),
      storeEntryVisits: metric(metrics, "store_entry_visits"),
      inputMaterializations: metric(metrics, "input_materializations"),
    };
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#assertMutationAllowed();
    this.#closed = true;
    this.#eventSink = undefined;
    this.#listeners.clear();
    this.#transitionListeners.clear();
    this.#pendingSourceListeners.clear();
    this.#nodeListeners.clear();
    this.#resourceListeners.clear();
    this.#artifactListeners.clear();
    this.#nodeCache.clear();
    this.#resourceCache.clear();
    this.#artifactCache.clear();
    this.#missingNodes.clear();
    this.#missingResources.clear();
    this.#missingArtifacts.clear();
    this.#pendingSourceCache = undefined;
    this.#pendingSourceLoaded = false;
    this.#session.free();
  }

  #invoke(operation: () => WasmOutput): ConsumedOutput {
    let output: WasmOutput;
    try {
      output = operation();
    } catch (error) {
      throw MdstreamError.from(error);
    }
    return this.#consume(output);
  }

  #consume(output: WasmOutput): ConsumedOutput {
    const drained = drainOutput(output);
    const updates: ReducerUpdateView[] = [];
    const processorRequests: ProcessorRequestView[] = [];
    const processorCompletions: ProcessorCompletionView[] = [];
    const artifactChanges: ArtifactChangeView[] = [];
    const snapshots: CanonicalSnapshotBytes[] = [];
    const nodeViews: NodeView[] = [];
    const resourceViews: ResourceView[] = [];
    const pendingSourceViews: PendingSourceView[] = [];
    const artifactViews: ArtifactView[] = [];

    for (const payload of drained.payloads) {
      if (payload.kind === BindingPayloadKind.Change) {
        asCanonicalChangeBytes(payload.bytes);
        continue;
      }
      if (payload.kind === BindingPayloadKind.Snapshot) {
        snapshots.push(asCanonicalSnapshotBytes(payload.bytes));
        continue;
      }
      const view = decodeBindingView(payload.kind, payload.bytes, this.#schema);
      switch (view.kind) {
        case "reducer_update":
          updates.push(view);
          break;
        case "node_view":
          nodeViews.push(view);
          break;
        case "resource_view":
          resourceViews.push(view);
          break;
        case "pending_source_view":
          pendingSourceViews.push(view);
          break;
        case "processor_request":
          processorRequests.push(view);
          break;
        case "processor_completion":
          processorCompletions.push(view);
          break;
        case "artifact_change":
          artifactChanges.push(view);
          break;
        case "artifact_view":
          artifactViews.push(view);
          break;
      }
    }

    if (
      !this.#captureTransitions &&
      updates.some((update) => update.transition !== undefined)
    ) {
      throw invalidPayload(
        "capture-disabled reducer returned unexpected transition facts",
      );
    }

    const operation = this.#operation;
    const notifications = operation?.notifications ?? createNotifications();
    for (const update of updates) {
      this.#applyUpdate(update, notifications);
    }
    for (const change of artifactChanges) {
      this.#applyArtifactChange(change, notifications);
    }
    for (const view of nodeViews) {
      this.#missingNodes.delete(view.node.id);
      this.#nodeCache.set(view.node.id, view);
    }
    for (const view of resourceViews) {
      this.#missingResources.delete(view.resource.id);
      this.#resourceCache.set(view.resource.id, view);
    }
    for (const view of artifactViews) {
      const key = artifactSlotKey(slotFromProcessorKey(view.key));
      this.#missingArtifacts.delete(key);
      this.#artifactCache.set(key, view);
    }

    if (operation === undefined) {
      this.#notify(notifications);
      if (updates.length > 0 || artifactChanges.length > 0) {
        this.#eventSink?.({ updates, artifactChanges });
      }
    } else {
      operation.updates.push(...updates);
      operation.artifactChanges.push(...artifactChanges);
      for (const update of updates) {
        if (update.transition !== undefined) {
          operation.facts.push(update.transition.facts);
        }
      }
    }

    return {
      updates,
      processorRequests,
      processorCompletions,
      artifactChanges,
      snapshots,
      nodeViews,
      resourceViews,
      pendingSourceViews,
      artifactViews,
      outputPayloadBytes: counter(drained.payloadBytes.toString()),
    };
  }

  #applyUpdate(update: ReducerUpdateView, notifications: Notifications): void {
    const statusChanged = update.status.kind !== this.#snapshot.status.kind;
    const stateChanged =
      update.outcome.kind === "applied" ||
      update.outcome.kind === "recovered" ||
      statusChanged;
    if (!stateChanged) {
      return;
    }

    const previousDocument = this.#snapshot.document;
    const incomingDocument = update.document;
    const document = incomingDocument === null
      ? previousDocument
      : {
          ...incomingDocument,
          ...(incomingDocument.roots === undefined && previousDocument?.roots !== undefined
            ? { roots: previousDocument.roots }
            : {}),
        };
    this.#snapshot = Object.freeze({
      status: update.status,
      document,
      impact: update.impact,
    });
    notifications.root = true;

    if (
      update.impact.sourceChanged ||
      update.impact.projectionChanged ||
      update.impact.fullReplace
    ) {
      this.#pendingSourceCache = undefined;
      this.#pendingSourceLoaded = false;
      notifications.pendingSource = true;
    }

    if (update.impact.fullReplace) {
      this.#nodeCache.clear();
      this.#resourceCache.clear();
      this.#artifactCache.clear();
      this.#missingNodes.clear();
      this.#missingResources.clear();
      this.#missingArtifacts.clear();
      for (const key of this.#nodeListeners.keys()) {
        notifications.nodes.add(key);
      }
      for (const key of this.#resourceListeners.keys()) {
        notifications.resources.add(key);
      }
      for (const key of this.#artifactListeners.keys()) {
        notifications.artifacts.add(key);
      }
    }

    for (const id of update.impact.changedNodeIds) {
      this.#nodeCache.delete(id);
      this.#missingNodes.delete(id);
      notifications.nodes.add(id);
    }
    for (const id of update.impact.removedNodeIds) {
      this.#nodeCache.delete(id);
      this.#missingNodes.add(id);
      notifications.nodes.add(id);
    }
    for (const id of update.impact.changedResourceIds) {
      this.#resourceCache.delete(id);
      this.#missingResources.delete(id);
      notifications.resources.add(id);
    }
    for (const id of update.impact.removedResourceIds) {
      this.#resourceCache.delete(id);
      this.#missingResources.add(id);
      notifications.resources.add(id);
    }
  }

  #applyArtifactChange(change: ArtifactChangeView, notifications: Notifications): void {
    const key = artifactSlotKey(slotFromProcessorKey(change.key));
    this.#artifactCache.delete(key);
    if (change.change.kind === "removed") {
      this.#missingArtifacts.add(key);
    } else {
      this.#missingArtifacts.delete(key);
    }
    notifications.artifacts.add(key);
  }

  #notify(notifications: Notifications): void {
    if (notifications.root) {
      notifyListeners(this.#listeners);
    }
    if (notifications.pendingSource) {
      notifyListeners(this.#pendingSourceListeners);
    }
    notifyKeyed(this.#nodeListeners, notifications.nodes);
    notifyKeyed(this.#resourceListeners, notifications.resources);
    notifyKeyed(this.#artifactListeners, notifications.artifacts);
  }

  #finishDocumentOperation(operation: StoreOperation): void {
    const batch = Object.freeze({
      facts: Object.freeze([...operation.facts]),
    });
    this.#publishingTransitions = true;
    try {
      notifyTransitionListeners(this.#transitionListeners, batch);
    } finally {
      this.#publishingTransitions = false;
    }

    this.#notify(operation.notifications);
    if (operation.updates.length > 0 || operation.artifactChanges.length > 0) {
      this.#eventSink?.({
        updates: operation.updates,
        artifactChanges: operation.artifactChanges,
      });
    }
  }

  #publicResult(output: ConsumedOutput): ReducerResult {
    return {
      updates: output.updates,
      processorRequests: output.processorRequests,
      processorCompletions: output.processorCompletions,
      artifactChanges: output.artifactChanges,
      outputPayloadBytes: output.outputPayloadBytes,
    };
  }

  #assertOpen(): void {
    if (this.#closed) {
      throw new MdstreamError("mdstream store is closed", {
        status: 1,
        statusName: "MDSTREAM_INVALID_ARGUMENT",
        detailCode: "bindings.closed",
      });
    }
  }

  #assertMutationAllowed(): void {
    if (this.#publishingTransitions) {
      throw new MdstreamError(
        "mdstream state cannot be mutated from a transition listener",
        {
          status: 1,
          statusName: "MDSTREAM_INVALID_ARGUMENT",
          detailCode: "bindings.transition_reentry",
        },
      );
    }
  }
}

/** @internal */
export function createStoreView(store: RustBackedStore): MdstreamStoreView {
  return Object.freeze({
    subscribe: (listener: StoreListener) => store.subscribe(listener),
    getSnapshot: () => store.getSnapshot(),
    subscribeTransitions: (listener: TransitionListener) =>
      store.subscribeTransitions(listener),
    getPendingSourceSnapshot: () => store.getPendingSourceSnapshot(),
    subscribePendingSource: (listener: StoreListener) =>
      store.subscribePendingSource(listener),
    pendingSource: () => store.pendingSource(),
    getNodeSnapshot: (id: NodeId) => store.getNodeSnapshot(id),
    subscribeNode: (id: NodeId, listener: StoreListener) =>
      store.subscribeNode(id, listener),
    node: (id: NodeId) => store.node(id),
    getResourceSnapshot: (id: ResourceId) => store.getResourceSnapshot(id),
    subscribeResource: (id: ResourceId, listener: StoreListener) =>
      store.subscribeResource(id, listener),
    resource: (id: ResourceId) => store.resource(id),
    getArtifactSnapshot: (slot: ArtifactSlot) => store.getArtifactSnapshot(slot),
    subscribeArtifact: (slot: ArtifactSlot, listener: StoreListener) =>
      store.subscribeArtifact(slot, listener),
    artifact: (slot: ArtifactSlot) => store.artifact(slot),
    metrics: () => store.metrics(),
    processorMetrics: () => store.processorMetrics(),
  });
}

function createStoreOperation(): StoreOperation {
  return {
    notifications: createNotifications(),
    updates: [],
    artifactChanges: [],
    facts: [],
  };
}

function createNotifications(): Notifications {
  return {
    root: false,
    pendingSource: false,
    nodes: new Set(),
    resources: new Set(),
    artifacts: new Set(),
  };
}

export function artifactSlotKey(slot: ArtifactSlot): string {
  return `${slot.epoch.length}:${slot.epoch}${slot.nodeId.length}:${slot.nodeId}${slot.processorId.length}:${slot.processorId}`;
}

/** @internal */
export function readBindingMetrics(
  bytes: Uint8Array,
): BindingMetricsView {
  const metrics = decodeMetricsPayload(bytes, "binding_metrics", bindingMetricFields);
  return {
    commands: metric(metrics, "commands"),
    decodedChangePayloads: metric(metrics, "decoded_change_payloads"),
    decodedSnapshotPayloads: metric(metrics, "decoded_snapshot_payloads"),
    changePayloads: metric(metrics, "change_payloads"),
    snapshotPayloads: metric(metrics, "snapshot_payloads"),
    reducerUpdatePayloads: metric(metrics, "reducer_update_payloads"),
    processorRequestPayloads: metric(metrics, "processor_request_payloads"),
    processorCompletionPayloads: metric(metrics, "processor_completion_payloads"),
    artifactChangePayloads: metric(metrics, "artifact_change_payloads"),
    artifactViewPayloads: metric(metrics, "artifact_view_payloads"),
    materializedNodeViews: metric(metrics, "materialized_node_views"),
    materializedResourceViews: metric(metrics, "materialized_resource_views"),
    encodedPayloadBytes: metric(metrics, "encoded_payload_bytes"),
    pendingProcessorRequests: metric(metrics, "pending_processor_requests"),
    materializedPendingSourceViews: metric(
      metrics,
      "materialized_pending_source_views",
    ),
  };
}

function slotFromProcessorKey(key: {
  readonly epoch: Epoch;
  readonly nodeId: NodeId;
  readonly processorId: string;
}): ArtifactSlot {
  return {
    epoch: key.epoch,
    nodeId: key.nodeId,
    processorId: key.processorId,
  };
}

function subscribeKeyed(
  listeners: Map<string, Set<StoreListener>>,
  key: string,
  listener: StoreListener,
  assertOpen: () => void,
): () => void {
  assertOpen();
  let group = listeners.get(key);
  if (group === undefined) {
    group = new Set();
    listeners.set(key, group);
  }
  group.add(listener);
  return () => {
    const current = listeners.get(key);
    current?.delete(listener);
    if (current?.size === 0) {
      listeners.delete(key);
    }
  };
}

function notifyKeyed(
  listeners: ReadonlyMap<string, ReadonlySet<StoreListener>>,
  keys: ReadonlySet<string>,
): void {
  for (const key of keys) {
    const group = listeners.get(key);
    if (group !== undefined) {
      notifyListeners(group);
    }
  }
}

function notifyListeners(listeners: ReadonlySet<StoreListener>): void {
  if (listeners.size === 0) {
    return;
  }
  for (const listener of [...listeners]) {
    try {
      listener();
    } catch {
      // Subscriber failures cannot roll back an already-applied Rust update.
    }
  }
}

function notifyTransitionListeners(
  listeners: ReadonlySet<TransitionListener>,
  batch: TransitionBatchView,
): void {
  for (const listener of [...listeners]) {
    try {
      listener(batch);
    } catch {
      // One host observer cannot prevent later observers from seeing the same batch.
    }
  }
}

class BoundedKeySet {
  readonly #limit: number;
  readonly #values = new Set<string>();

  constructor(limit: number) {
    this.#limit = limit;
  }

  has(value: string): boolean {
    return this.#values.has(value);
  }

  add(value: string): void {
    if (this.#values.has(value)) {
      return;
    }
    this.#values.add(value);
    if (this.#values.size > this.#limit) {
      const oldest = this.#values.values().next().value;
      if (oldest !== undefined) {
        this.#values.delete(oldest);
      }
    }
  }

  delete(value: string): void {
    this.#values.delete(value);
  }

  clear(): void {
    this.#values.clear();
  }
}

function counter(value: string): DecimalCounter {
  return value as DecimalCounter;
}

function metric(
  metrics: Readonly<Record<string, DecimalCounter>>,
  name: string,
): DecimalCounter {
  const value = metrics[name];
  if (value === undefined) {
    throw invalidPayload(`metrics payload omitted ${name}`);
  }
  return value;
}
