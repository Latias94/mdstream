import {
  RustBackedStore,
  type InternalStoreEvents,
} from "./store.js";
import {
  isProcessorFailureCode,
  MdstreamError,
  type ContentNodeView,
  type Epoch,
  type NodeId,
  type NodeVersion,
  type NodeView,
  type ProcessorFailureCode,
  type ProcessorRequestView,
  type RequestGeneration,
} from "./views.js";

export interface ContentProcessorDescriptor {
  readonly id: string;
  readonly version: string;
  readonly acceptsProvisional?: boolean;
}

export interface ProcessorContext {
  readonly signal: AbortSignal;
}

export type ProcessorOutput =
  | {
      readonly kind: "text";
      readonly protocol: string;
      readonly mediaType: string;
      readonly text: string;
    }
  | {
      readonly kind: "binary";
      readonly protocol: string;
      readonly mediaType: string;
      readonly bytes: Uint8Array;
    }
  | {
      readonly kind: "failure";
      readonly code: ProcessorFailureCode;
      readonly message: string;
    };

export interface ContentProcessor {
  readonly descriptor: ContentProcessorDescriptor;
  readonly configurationVersion: string;
  readonly allowProvisional?: boolean;
  matches(node: ContentNodeView): boolean;
  process(
    request: ProcessorRequestView,
    context: ProcessorContext,
  ): ProcessorOutput | Promise<ProcessorOutput>;
}

export interface ProcessorRegistration {
  dispose(): void;
}

export type ProcessorErrorPhase =
  | "view"
  | "matches"
  | "begin"
  | "process"
  | "complete"
  | "cancel";

export interface ProcessorErrorEvent {
  readonly phase: ProcessorErrorPhase;
  readonly processorId: string;
  readonly nodeId: NodeId | undefined;
  readonly requestId: RequestGeneration | undefined;
  readonly error: unknown;
}

export type ProcessorErrorListener = (event: ProcessorErrorEvent) => void;

interface RegisteredProcessor {
  readonly processor: ContentProcessor;
  readonly descriptor: Readonly<Required<ContentProcessorDescriptor>>;
  readonly configurationVersion: string;
  readonly allowProvisional: boolean;
  active: boolean;
}

interface InFlightProcessor {
  readonly registration: RegisteredProcessor;
  readonly request: ProcessorRequestView;
  readonly controller: AbortController;
}

interface ProcessorCandidate {
  readonly registration: RegisteredProcessor;
  readonly nodeId: NodeId;
  expectedEpoch: Epoch;
  expectedNodeVersion: NodeVersion;
  queued: boolean;
}

interface CandidateExpectation {
  readonly epoch: Epoch;
  readonly nodeVersion: NodeVersion;
}

export interface ProcessorSchedulerLimits {
  readonly maxInFlightJobs: number;
  readonly maxCandidates: number;
}

type BeginDisposition = "started" | "stale" | "blocked" | "terminal";

const candidateQueueCompactionFloor = 64;
const candidateQueueCompactionRatio = 4;

/** @internal */
export class ProcessorScheduler {
  readonly #store: RustBackedStore;
  readonly #processors = new Map<string, RegisteredProcessor>();
  readonly #inFlight = new Map<string, InFlightProcessor>();
  readonly #pendingNodes = new Set<NodeId>();
  readonly #pendingRegistrations = new Set<RegisteredProcessor>();
  readonly #candidateQueue: ProcessorCandidate[] = [];
  readonly #candidates = new Map<
    RegisteredProcessor,
    Map<NodeId, ProcessorCandidate>
  >();
  readonly #rejectedCandidates = new Map<
    RegisteredProcessor,
    Map<NodeId, CandidateExpectation>
  >();
  readonly #jobs = new Set<Promise<void>>();
  readonly #errorListeners = new Set<ProcessorErrorListener>();
  readonly #maxInFlightJobs: number;
  readonly #maxCandidates: number;
  #candidateHead = 0;
  #candidateCount = 0;
  #removedDuringBegin: Map<RequestGeneration, unknown> | undefined;
  #dispatching = false;
  #scanScheduled = false;
  #closed = false;

  constructor(store: RustBackedStore, limits: ProcessorSchedulerLimits) {
    this.#store = store;
    this.#maxInFlightJobs = Math.max(1, limits.maxInFlightJobs);
    this.#maxCandidates = Math.max(1, limits.maxCandidates);
  }

  register(processor: ContentProcessor): ProcessorRegistration {
    this.#assertOpen();
    // Registration identity is immutable even when the processor exposes getters.
    const suppliedDescriptor = processor.descriptor;
    const descriptor = Object.freeze({
      id: suppliedDescriptor.id,
      version: suppliedDescriptor.version,
      acceptsProvisional: suppliedDescriptor.acceptsProvisional === true,
    });
    if (
      typeof descriptor.id !== "string" ||
      descriptor.id.length === 0 ||
      typeof descriptor.version !== "string" ||
      descriptor.version.length === 0
    ) {
      throw new TypeError("processor id and version must not be empty");
    }
    if (this.#processors.has(descriptor.id)) {
      throw new TypeError(`processor ${descriptor.id} is already registered`);
    }
    const configurationVersion = processor.configurationVersion;
    const allowProvisional = processor.allowProvisional === true;
    if (
      typeof configurationVersion !== "string" ||
      configurationVersion.length === 0
    ) {
      throw new TypeError("processor configuration version must not be empty");
    }
    const registration: RegisteredProcessor = {
      processor,
      descriptor,
      configurationVersion,
      allowProvisional,
      active: true,
    };
    this.#processors.set(descriptor.id, registration);
    this.#pendingRegistrations.add(registration);
    this.#scheduleScan();
    return {
      dispose: () => {
        if (!registration.active) {
          return;
        }
        this.#store.runDocumentOperation(() =>
          this.#disposeRegistration(registration)
        );
      },
    };
  }

  subscribeErrors(listener: ProcessorErrorListener): () => void {
    this.#assertOpen();
    this.#errorListeners.add(listener);
    return () => this.#errorListeners.delete(listener);
  }

  handleStoreEvents(events: InternalStoreEvents): void {
    if (this.#closed) {
      return;
    }
    for (const change of events.artifactChanges) {
      if (change.change.kind === "removed") {
        const entry = this.#inFlight.get(change.key.generation);
        if (entry !== undefined) {
          entry.controller.abort(change.change.reason);
          if (this.#inFlight.get(change.key.generation) === entry) {
            this.#inFlight.delete(change.key.generation);
          }
        } else {
          this.#removedDuringBegin?.set(
            change.key.generation,
            change.change.reason,
          );
        }
      }
    }
    if (this.#processors.size === 0) {
      return;
    }
    for (const update of events.updates) {
      if (update.outcome.kind !== "applied" && update.outcome.kind !== "recovered") {
        continue;
      }
      if (update.impact.fullReplace) {
        this.#clearCandidates();
        this.#clearRejectedCandidates();
        this.#pendingNodes.clear();
        for (const registration of this.#processors.values()) {
          if (registration.active) {
            this.#pendingRegistrations.add(registration);
          }
        }
        continue;
      }
      for (const id of update.impact.changedNodeIds) {
        this.#removeNodeCandidates(id);
        this.#pendingNodes.add(id);
      }
      for (const id of update.impact.removedNodeIds) {
        this.#removeNodeCandidates(id);
        this.#removeRejectedNode(id);
        this.#pendingNodes.delete(id);
      }
    }
    this.#scheduleScan();
    this.#drainCandidates();
  }

  async whenIdle(): Promise<void> {
    for (;;) {
      await Promise.resolve();
      this.#drainCandidates();
      if (
        this.#scanScheduled ||
        this.#pendingNodes.size > 0 ||
        this.#pendingRegistrations.size > 0
      ) {
        continue;
      }
      const jobs = [...this.#jobs];
      if (jobs.length === 0) {
        if (this.#candidateCount === 0) {
          return;
        }
        continue;
      }
      await Promise.allSettled(jobs);
    }
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#scanScheduled = false;
    this.#pendingNodes.clear();
    this.#pendingRegistrations.clear();
    this.#clearCandidates();
    this.#clearRejectedCandidates();
    for (const registration of this.#processors.values()) {
      registration.active = false;
    }
    this.#processors.clear();
    for (const entry of [...this.#inFlight.values()]) {
      entry.controller.abort("scheduler_closed");
      this.#cancel(entry, "cancel");
    }
    this.#inFlight.clear();
    this.#errorListeners.clear();
  }

  #scheduleScan(): void {
    if (
      this.#scanScheduled ||
      (this.#pendingNodes.size === 0 && this.#pendingRegistrations.size === 0) ||
      this.#closed
    ) {
      return;
    }
    this.#scanScheduled = true;
    queueMicrotask(() => {
      this.#scanScheduled = false;
      this.#scanChangedNodes();
    });
  }

  #scanChangedNodes(): void {
    if (this.#closed) {
      this.#pendingNodes.clear();
      return;
    }
    const nodeIds = [...this.#pendingNodes];
    this.#pendingNodes.clear();
    const pendingRegistrations = [...this.#pendingRegistrations]
      .filter((registration) => registration.active);
    this.#pendingRegistrations.clear();
    const pendingSet = new Set(pendingRegistrations);
    // New registrations traverse the tree below, so only existing processors
    // consume the changed-node queue during this pass.
    const registrations = [...this.#processors.values()]
      .filter(
        (registration) => registration.active && !pendingSet.has(registration),
      );

    for (const nodeId of nodeIds) {
      this.#scanNode(nodeId, registrations);
    }
    if (pendingRegistrations.length > 0) {
      const visited = this.#scanCurrentTree(pendingRegistrations);
      for (const nodeId of nodeIds) {
        if (!visited.has(nodeId)) {
          this.#scanNode(nodeId, pendingRegistrations);
        }
      }
    }
    this.#scheduleScan();
    this.#drainCandidates();
  }

  #scanCurrentTree(registrations: readonly RegisteredProcessor[]): Set<NodeId> {
    const roots = this.#store.getSnapshot().document?.roots?.children;
    const visited = new Set<NodeId>();
    if (roots === undefined || roots.length === 0) {
      return visited;
    }
    const queue = [...roots];
    for (let index = 0; index < queue.length && !this.#closed; index += 1) {
      const nodeId = queue[index]!;
      if (visited.has(nodeId)) {
        continue;
      }
      visited.add(nodeId);
      const nodeView = this.#scanNode(nodeId, registrations);
      if (nodeView !== undefined) {
        for (const childId of nodeView.node.children.children) {
          queue.push(childId);
        }
      }
    }
    return visited;
  }

  #scanNode(
    nodeId: NodeId,
    registrations: readonly RegisteredProcessor[],
  ): NodeView | undefined {
    if (registrations.length === 0) {
      return undefined;
    }
    const expectedEpoch = this.#store.getSnapshot().document?.coordinate.epoch;
    if (expectedEpoch === undefined) {
      for (const registration of registrations) {
        this.#removeCandidate(registration, nodeId);
      }
      return undefined;
    }
    let nodeView: NodeView | undefined;
    try {
      nodeView = this.#store.getNodeSnapshot(nodeId);
    } catch (error) {
      for (const registration of registrations) {
        this.#removeCandidate(registration, nodeId);
        if (registration.active) {
          this.#emitError({
            phase: "view",
            processorId: registration.descriptor.id,
            nodeId,
            requestId: undefined,
            error: MdstreamError.from(error),
          });
        }
      }
      return undefined;
    }
    if (nodeView === undefined) {
      for (const registration of registrations) {
        this.#removeCandidate(registration, nodeId);
      }
      return undefined;
    }
    for (const registration of registrations) {
      if (!registration.active) {
        continue;
      }
      const processor = registration.processor;
      if (
        nodeView.node.stability === "provisional" &&
        !(registration.descriptor.acceptsProvisional &&
          registration.allowProvisional)
      ) {
        this.#removeCandidate(registration, nodeId);
        continue;
      }
      let matches: boolean;
      try {
        matches = processor.matches(nodeView.node);
      } catch (error) {
        this.#removeCandidate(registration, nodeId);
        this.#emitError({
          phase: "matches",
          processorId: registration.descriptor.id,
          nodeId,
          requestId: undefined,
          error,
        });
        continue;
      }
      if (matches && registration.active) {
        this.#enqueueCandidate(
          registration,
          expectedEpoch,
          nodeId,
          nodeView.node.version,
        );
      } else {
        this.#removeCandidate(registration, nodeId);
      }
    }
    return nodeView;
  }

  #enqueueCandidate(
    registration: RegisteredProcessor,
    expectedEpoch: Epoch,
    nodeId: NodeId,
    expectedNodeVersion: NodeVersion,
    front = false,
  ): void {
    if (!registration.active || this.#closed) {
      return;
    }
    const rejected = this.#rejectedCandidates.get(registration)?.get(nodeId);
    if (
      rejected?.epoch === expectedEpoch &&
      rejected.nodeVersion === expectedNodeVersion
    ) {
      return;
    }
    let registrationCandidates = this.#candidates.get(registration);
    const existing = registrationCandidates?.get(nodeId);
    if (existing !== undefined) {
      existing.expectedEpoch = expectedEpoch;
      existing.expectedNodeVersion = expectedNodeVersion;
      return;
    }
    if (this.#candidateCount >= this.#maxCandidates) {
      this.#emitError({
        phase: "begin",
        processorId: registration.descriptor.id,
        nodeId,
        requestId: undefined,
        error: new MdstreamError(
          `processor candidate queue limit ${this.#maxCandidates} exceeded`,
          {
            status: 11,
            statusName: "MDSTREAM_RESOURCE_LIMIT_EXCEEDED",
            detailCode: "processor.candidate_queue_limit",
          },
        ),
      });
      return;
    }
    if (registrationCandidates === undefined) {
      registrationCandidates = new Map<NodeId, ProcessorCandidate>();
      this.#candidates.set(registration, registrationCandidates);
    }
    const candidate: ProcessorCandidate = {
      registration,
      nodeId,
      expectedEpoch,
      expectedNodeVersion,
      queued: true,
    };
    registrationCandidates.set(nodeId, candidate);
    if (front) {
      this.#candidateQueue.splice(this.#candidateHead, 0, candidate);
    } else {
      this.#candidateQueue.push(candidate);
    }
    this.#candidateCount += 1;
  }

  #removeCandidate(registration: RegisteredProcessor, nodeId: NodeId): void {
    const registrationCandidates = this.#candidates.get(registration);
    const candidate = registrationCandidates?.get(nodeId);
    if (candidate === undefined) {
      return;
    }
    candidate.queued = false;
    registrationCandidates!.delete(nodeId);
    this.#candidateCount -= 1;
    if (registrationCandidates!.size === 0) {
      this.#candidates.delete(registration);
    }
    this.#compactCandidateQueue();
  }

  #removeNodeCandidates(nodeId: NodeId): void {
    for (const registration of [...this.#candidates.keys()]) {
      this.#removeCandidate(registration, nodeId);
    }
  }

  #removeRegistrationCandidates(registration: RegisteredProcessor): void {
    const candidates = this.#candidates.get(registration);
    if (candidates === undefined) {
      return;
    }
    for (const candidate of candidates.values()) {
      candidate.queued = false;
      this.#candidateCount -= 1;
    }
    this.#candidates.delete(registration);
    this.#compactCandidateQueue();
  }

  #rejectCandidate(candidate: ProcessorCandidate): void {
    let rejected = this.#rejectedCandidates.get(candidate.registration);
    if (rejected === undefined) {
      rejected = new Map<NodeId, CandidateExpectation>();
      this.#rejectedCandidates.set(candidate.registration, rejected);
    }
    rejected.set(candidate.nodeId, {
      epoch: candidate.expectedEpoch,
      nodeVersion: candidate.expectedNodeVersion,
    });
  }

  #removeRejectedCandidate(
    registration: RegisteredProcessor,
    nodeId: NodeId,
  ): void {
    const rejected = this.#rejectedCandidates.get(registration);
    rejected?.delete(nodeId);
    if (rejected?.size === 0) {
      this.#rejectedCandidates.delete(registration);
    }
  }

  #removeRejectedNode(nodeId: NodeId): void {
    for (const registration of [...this.#rejectedCandidates.keys()]) {
      this.#removeRejectedCandidate(registration, nodeId);
    }
  }

  #clearRejectedCandidates(): void {
    this.#rejectedCandidates.clear();
  }

  #clearCandidates(): void {
    for (const candidates of this.#candidates.values()) {
      for (const candidate of candidates.values()) {
        candidate.queued = false;
      }
    }
    this.#candidates.clear();
    this.#candidateQueue.length = 0;
    this.#candidateHead = 0;
    this.#candidateCount = 0;
  }

  #takeCandidate(): ProcessorCandidate | undefined {
    while (this.#candidateHead < this.#candidateQueue.length) {
      const candidate = this.#candidateQueue[this.#candidateHead++]!;
      if (!candidate.queued) {
        continue;
      }
      candidate.queued = false;
      const registrationCandidates = this.#candidates.get(candidate.registration);
      registrationCandidates?.delete(candidate.nodeId);
      if (registrationCandidates?.size === 0) {
        this.#candidates.delete(candidate.registration);
      }
      this.#candidateCount -= 1;
      this.#compactCandidateQueue();
      return candidate;
    }
    this.#compactCandidateQueue();
    return undefined;
  }

  #compactCandidateQueue(): void {
    if (this.#candidateCount === 0) {
      this.#candidateQueue.length = 0;
      this.#candidateHead = 0;
      return;
    }
    if (
      this.#candidateQueue.length <= candidateQueueCompactionFloor ||
      this.#candidateQueue.length <=
        this.#candidateCount * candidateQueueCompactionRatio
    ) {
      return;
    }
    let write = 0;
    for (
      let read = this.#candidateHead;
      read < this.#candidateQueue.length;
      read += 1
    ) {
      const candidate = this.#candidateQueue[read]!;
      if (candidate.queued) {
        this.#candidateQueue[write] = candidate;
        write += 1;
      }
    }
    this.#candidateQueue.length = write;
    this.#candidateHead = 0;
  }

  #drainCandidates(): void {
    if (this.#dispatching || this.#closed) {
      return;
    }
    this.#dispatching = true;
    try {
      while (
        this.#candidateCount > 0 &&
        this.#inFlight.size < this.#maxInFlightJobs
      ) {
        const candidate = this.#takeCandidate();
        if (candidate === undefined) {
          break;
        }
        if (!candidate.registration.active) {
          continue;
        }
        if (this.#begin(candidate) === "blocked") {
          break;
        }
      }
    } finally {
      this.#dispatching = false;
    }
  }

  #begin(candidate: ProcessorCandidate): BeginDisposition {
    const {
      registration,
      expectedEpoch,
      nodeId,
      expectedNodeVersion,
    } = candidate;
    let request: ProcessorRequestView | undefined;
    const parentRemovals = this.#removedDuringBegin;
    const removals = new Map<RequestGeneration, unknown>();
    this.#removedDuringBegin = removals;
    try {
      const requests = this.#store.beginProcessor({
        expectedEpoch,
        nodeId,
        expectedNodeVersion,
        processorId: registration.descriptor.id,
        processorVersion: registration.descriptor.version,
        configurationVersion: registration.configurationVersion,
        acceptsProvisional: registration.descriptor.acceptsProvisional,
        allowProvisional: registration.allowProvisional,
      }).processorRequests;
      if (requests.length === 0) {
        this.#rejectCandidate(candidate);
        this.#pendingNodes.add(nodeId);
        this.#scheduleScan();
        return "stale";
      }
      if (requests.length !== 1) {
        throw new Error(
          "Rust processor host returned no unique processor request",
        );
      }
      request = requests[0]!;
      this.#removeRejectedCandidate(registration, nodeId);
    } catch (error) {
      const normalized = MdstreamError.from(error);
      if (normalized.status === 11 && this.#inFlight.size > 0) {
        this.#enqueueCandidate(
          registration,
          expectedEpoch,
          nodeId,
          expectedNodeVersion,
          true,
        );
        return "blocked";
      }
      this.#emitError({
        phase: "begin",
        processorId: registration.descriptor.id,
        nodeId,
        requestId: undefined,
        error: normalized,
      });
      return "terminal";
    } finally {
      this.#removedDuringBegin = parentRemovals;
      if (parentRemovals !== undefined) {
        for (const [generation, reason] of removals) {
          parentRemovals.set(generation, reason);
        }
      }
    }

    const entry: InFlightProcessor = {
      registration,
      request,
      controller: new AbortController(),
    };
    if (removals.has(request.requestId)) {
      entry.controller.abort(removals.get(request.requestId));
      return "stale";
    }
    if (this.#closed) {
      entry.controller.abort("scheduler_closed");
      this.#cancel(entry, "cancel");
      return "terminal";
    }
    if (!registration.active) {
      entry.controller.abort("processor_unregistered");
      this.#cancel(entry, "cancel");
      return "terminal";
    }
    this.#inFlight.set(request.requestId, entry);
    const job = Promise.resolve()
      .then(() => this.#run(entry))
      .finally(() => {
        if (this.#inFlight.get(request.requestId) === entry) {
          this.#inFlight.delete(request.requestId);
        }
        this.#jobs.delete(job);
        this.#drainCandidates();
        this.#scheduleScan();
      });
    this.#jobs.add(job);
    return "started";
  }

  async #run(entry: InFlightProcessor): Promise<void> {
    if (!this.#isCurrent(entry)) {
      return;
    }
    try {
      const output = await entry.registration.processor.process(entry.request, {
        signal: entry.controller.signal,
      });
      if (!this.#isCurrent(entry)) {
        return;
      }
      this.#complete(entry, normalizeOutput(output));
    } catch (error) {
      this.#processorFailed(entry, error);
    }
  }

  #complete(entry: InFlightProcessor, output: ProcessorOutput): void {
    if (!this.#isCurrent(entry)) {
      return;
    }
    try {
      switch (output.kind) {
        case "text":
          this.#store.completeProcessorText(
            entry.request.requestId,
            output.protocol,
            output.mediaType,
            output.text,
          );
          break;
        case "binary":
          this.#store.completeProcessorBinary(
            entry.request.requestId,
            output.protocol,
            output.mediaType,
            output.bytes,
          );
          break;
        case "failure":
          this.#store.failProcessor(entry.request.requestId, output.code, output.message);
          break;
      }
    } catch (error) {
      this.#handleCompletionFailure(entry, error);
    }
  }

  #processorFailed(entry: InFlightProcessor, error: unknown): void {
    if (!this.#isCurrent(entry)) {
      return;
    }
    this.#emitError({
      phase: "process",
      processorId: entry.registration.descriptor.id,
      nodeId: entry.request.key.nodeId,
      requestId: entry.request.requestId,
      error,
    });
    if (!this.#isCurrent(entry)) {
      return;
    }
    try {
      this.#store.failProcessor(
        entry.request.requestId,
        entry.controller.signal.aborted ? "cancelled" : "panic",
        processorErrorMessage(error),
      );
    } catch (completionError) {
      this.#handleCompletionFailure(entry, completionError);
    }
  }

  #handleCompletionFailure(entry: InFlightProcessor, error: unknown): void {
    this.#emitError({
      phase: "complete",
      processorId: entry.registration.descriptor.id,
      nodeId: entry.request.key.nodeId,
      requestId: entry.request.requestId,
      error: MdstreamError.from(error),
    });
    if (this.#inFlight.get(entry.request.requestId) === entry) {
      this.#cancel(entry, "cancel");
    }
  }

  #disposeRegistration(registration: RegisteredProcessor): void {
    if (!registration.active) {
      return;
    }
    registration.active = false;
    if (this.#processors.get(registration.descriptor.id) === registration) {
      this.#processors.delete(registration.descriptor.id);
    }
    this.#pendingRegistrations.delete(registration);
    this.#removeRegistrationCandidates(registration);
    this.#rejectedCandidates.delete(registration);
    for (const entry of [...this.#inFlight.values()]) {
      if (entry.registration === registration) {
        entry.controller.abort("processor_unregistered");
        this.#inFlight.delete(entry.request.requestId);
        this.#cancel(entry, "cancel");
      }
    }
    this.#drainCandidates();
  }

  #cancel(entry: InFlightProcessor, phase: ProcessorErrorPhase): void {
    try {
      this.#store.cancelProcessor(entry.request.requestId);
    } catch (error) {
      this.#emitError({
        phase,
        processorId: entry.registration.descriptor.id,
        nodeId: entry.request.key.nodeId,
        requestId: entry.request.requestId,
        error: MdstreamError.from(error),
      });
    }
  }

  #emitError(event: ProcessorErrorEvent): void {
    if (this.#closed) {
      return;
    }
    for (const listener of [...this.#errorListeners]) {
      try {
        listener(event);
      } catch {
        // Observers cannot interrupt processor lease settlement.
      }
    }
  }

  #assertOpen(): void {
    if (this.#closed) {
      throw new TypeError("processor scheduler is closed");
    }
  }

  #isCurrent(entry: InFlightProcessor): boolean {
    return (
      !this.#closed &&
      entry.registration.active &&
      this.#inFlight.get(entry.request.requestId) === entry
    );
  }
}

function normalizeOutput(value: unknown): ProcessorOutput {
  if (typeof value !== "object" || value === null || !("kind" in value)) {
    return invalidProcessorOutput("processor returned no structured result");
  }
  const output = value as Record<string, unknown>;
  switch (output.kind) {
    case "text":
      if (
        typeof output.protocol === "string" &&
        typeof output.mediaType === "string" &&
        typeof output.text === "string"
      ) {
        return {
          kind: "text",
          protocol: output.protocol,
          mediaType: output.mediaType,
          text: output.text,
        };
      }
      break;
    case "binary":
      if (
        typeof output.protocol === "string" &&
        typeof output.mediaType === "string" &&
        output.bytes instanceof Uint8Array
      ) {
        return {
          kind: "binary",
          protocol: output.protocol,
          mediaType: output.mediaType,
          bytes: output.bytes,
        };
      }
      break;
    case "failure":
      if (
        isProcessorFailureCode(output.code) &&
        typeof output.message === "string"
      ) {
        return { kind: "failure", code: output.code, message: output.message };
      }
      break;
  }
  return invalidProcessorOutput("processor returned a malformed result");
}

function invalidProcessorOutput(message: string): ProcessorOutput {
  return { kind: "failure", code: "invalid_request", message };
}

function processorErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return typeof error === "string" ? error : "processor threw a non-Error value";
}
