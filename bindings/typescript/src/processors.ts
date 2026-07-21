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
  type NodeView,
  type ProcessorFailureCode,
  type ProcessorInputVersion,
  type ProcessorRequestView,
  type RequestGeneration,
} from "./views.js";

const RESOURCE_LIMIT_STATUS = Object.freeze({
  status: 11,
  statusName: "MDSTREAM_RESOURCE_LIMIT_EXCEEDED",
} as const);

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
  expectedInputVersion: ProcessorInputVersion;
  queued: boolean;
}

interface CandidateExpectation {
  readonly epoch: Epoch;
  readonly inputVersion: ProcessorInputVersion;
}

type ProcessorScanPhase = "changed" | "tree" | "fallback" | "done";

interface ProcessorScanWork {
  readonly nodeIds: readonly NodeId[];
  registrations: RegisteredProcessor[];
  pendingRegistrations: RegisteredProcessor[];
  readonly treeQueue: Array<NodeId | undefined>;
  readonly visited: Set<NodeId>;
  nodeScan: ProcessorNodeScan | undefined;
  phase: ProcessorScanPhase;
  nodeIndex: number;
  treeIndex: number;
  treeClearedIndex: number;
  fallbackIndex: number;
  treeInitialized: boolean;
}

interface ProcessorNodeScan {
  readonly nodeId: NodeId;
  registrations: RegisteredProcessor[];
  readonly expectedEpoch: Epoch | undefined;
  readonly view:
    | { readonly kind: "ready"; readonly node: NodeView | undefined }
    | { readonly kind: "failed"; readonly error: unknown };
  registrationIndex: number;
}

interface ProcessorNodeScanStep {
  readonly complete: boolean;
  readonly nodeView: NodeView | undefined;
  readonly blocked: boolean;
}

interface ScanUnblocked {
  readonly promise: Promise<void>;
  readonly resolve: () => void;
}

export interface ProcessorSchedulerLimits {
  readonly maxInFlightJobs: number;
  readonly maxCandidates: number;
}

type BeginDisposition = "started" | "stale" | "blocked" | "terminal";

const candidateQueueCompactionFloor = 64;
const candidateQueueCompactionRatio = 4;
const scanQueueCompactionFloor = 64;
const scanQueueCompactionRatio = 4;
const dispatchQuantum = 32;
const scanQuantum = 64;
const retryableResourceLimitDetailCodes = new Set([
  "processor.resource_limit.in_flight_jobs",
  "processor.resource_limit.in_flight_input_bytes",
]);
const processorIdentifierPattern = /^[A-Za-z0-9._:+-]{1,128}$/;

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
  #candidateQueueSaturated = false;
  #removedDuringBegin: Map<RequestGeneration, unknown> | undefined;
  #dispatching = false;
  #dispatchBlocked = false;
  #scheduledDispatchRevision: number | undefined;
  #dispatchRevision = 0;
  #scheduledScanRevision: number | undefined;
  #scheduledScanContinuationRevision: number | undefined;
  #scanRevision = 0;
  #scanWork: ProcessorScanWork | undefined;
  #scanBlocked = false;
  #scanUnblocked: ScanUnblocked | undefined;
  #closed = false;

  constructor(store: RustBackedStore, limits: ProcessorSchedulerLimits) {
    this.#store = store;
    this.#maxInFlightJobs = limits.maxInFlightJobs;
    this.#maxCandidates = limits.maxCandidates;
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
    validateProcessorIdentifier(descriptor.id, "processor id");
    validateProcessorIdentifier(descriptor.version, "processor version");
    if (this.#processors.has(descriptor.id)) {
      throw new TypeError(`processor ${descriptor.id} is already registered`);
    }
    const configurationVersion = processor.configurationVersion;
    const allowProvisional = processor.allowProvisional === true;
    validateProcessorIdentifier(
      configurationVersion,
      "processor configuration version",
    );
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
    let capacityChanged = false;
    for (const change of events.artifactChanges) {
      if (change.change.kind === "removed") {
        const entry = this.#inFlight.get(change.key.generation);
        if (entry !== undefined) {
          entry.controller.abort(change.change.reason);
          if (this.#inFlight.get(change.key.generation) === entry) {
            this.#inFlight.delete(change.key.generation);
            this.#dispatchBlocked = false;
            capacityChanged = true;
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
        this.#invalidateDispatch();
        this.#invalidateScan();
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
    if (capacityChanged) {
      this.#scheduleDispatch();
    } else {
      this.#drainCandidates();
    }
  }

  async whenIdle(): Promise<void> {
    for (;;) {
      await Promise.resolve();
      const scanUnblocked = this.#scanUnblocked;
      if (this.#scanBlocked && scanUnblocked !== undefined) {
        await scanUnblocked.promise;
        continue;
      }
      if (
        this.#scheduledDispatchRevision !== undefined ||
        this.#scheduledScanRevision !== undefined
      ) {
        await new Promise<void>((resolve) => {
          setTimeout(resolve, 0);
        });
        continue;
      }
      this.#drainCandidates();
      if (
        this.#scheduledScanRevision !== undefined ||
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
    this.#invalidateDispatch();
    this.#invalidateScan();
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
      this.#scheduledScanRevision !== undefined ||
      (this.#pendingNodes.size === 0 && this.#pendingRegistrations.size === 0) ||
      this.#closed
    ) {
      return;
    }
    const revision = this.#scanRevision;
    this.#scheduledScanRevision = revision;
    queueMicrotask(() => this.#runScan(revision));
  }

  #createScanWork(): ProcessorScanWork {
    this.#candidateQueueSaturated = false;
    const nodeIds = [...this.#pendingNodes];
    this.#pendingNodes.clear();
    const pendingRegistrations = [...this.#pendingRegistrations]
      .filter((registration) => registration.active);
    this.#pendingRegistrations.clear();
    const pendingSet = new Set(pendingRegistrations);
    const registrations = [...this.#processors.values()]
      .filter(
        (registration) => registration.active && !pendingSet.has(registration),
      );
    return {
      nodeIds,
      registrations,
      pendingRegistrations,
      treeQueue: [],
      visited: new Set<NodeId>(),
      nodeScan: undefined,
      phase: "changed",
      nodeIndex: 0,
      treeIndex: 0,
      treeClearedIndex: 0,
      fallbackIndex: 0,
      treeInitialized: false,
    };
  }

  #runScan(revision: number): void {
    if (!this.#isCurrentScan(revision)) {
      return;
    }
    const work = this.#scanWork ?? this.#createScanWork();
    this.#scanWork = work;
    let remaining = scanQuantum;
    while (remaining > 0 && work.phase !== "done") {
      if (!this.#isCurrentScan(revision)) {
        return;
      }
      switch (work.phase) {
        case "changed": {
          if (
            !this.#hasActiveRegistrations(work.registrations) ||
            (work.nodeScan === undefined &&
              work.nodeIndex >= work.nodeIds.length)
          ) {
            work.nodeScan = undefined;
            work.phase = this.#hasActiveRegistrations(work.pendingRegistrations)
              ? "tree"
              : "done";
            continue;
          }
          if (work.nodeScan === undefined) {
            const nodeId = work.nodeIds[work.nodeIndex]!;
            work.nodeIndex += 1;
            work.nodeScan = this.#createNodeScan(nodeId, work.registrations);
          } else {
            const step = this.#stepNodeScan(work.nodeScan);
            if (step.blocked) {
              if (!this.#isCurrentScan(revision)) {
                return;
              }
              this.#blockScan();
              remaining = 0;
              break;
            }
            if (step.complete) {
              work.nodeScan = undefined;
            }
          }
          remaining -= 1;
          break;
        }
        case "tree": {
          if (!this.#hasActiveRegistrations(work.pendingRegistrations)) {
            work.nodeScan = undefined;
            work.treeQueue.length = 0;
            work.visited.clear();
            work.phase = "done";
            continue;
          }
          if (!work.treeInitialized) {
            const roots = this.#store.getSnapshot().document?.roots?.children;
            if (roots !== undefined) {
              work.treeQueue.push(...roots);
            }
            work.treeInitialized = true;
          }
          if (work.nodeScan !== undefined) {
            const step = this.#stepNodeScan(work.nodeScan);
            if (step.blocked) {
              if (!this.#isCurrentScan(revision)) {
                return;
              }
              this.#blockScan();
              remaining = 0;
              break;
            }
            if (step.complete) {
              if (step.nodeView !== undefined) {
                work.treeQueue.push(...step.nodeView.node.children.children);
              }
              work.nodeScan = undefined;
            }
            remaining -= 1;
            break;
          }
          if (work.treeIndex >= work.treeQueue.length) {
            work.treeQueue.length = 0;
            work.treeIndex = 0;
            work.treeClearedIndex = 0;
            work.phase = "fallback";
            continue;
          }
          const nodeId = work.treeQueue[work.treeIndex]!;
          work.treeIndex += 1;
          remaining -= 1;
          if (!work.visited.has(nodeId)) {
            work.visited.add(nodeId);
            work.nodeScan = this.#createNodeScan(
              nodeId,
              work.pendingRegistrations,
            );
          }
          break;
        }
        case "fallback": {
          if (
            !this.#hasActiveRegistrations(work.pendingRegistrations) ||
            (work.nodeScan === undefined &&
              work.fallbackIndex >= work.nodeIds.length)
          ) {
            work.nodeScan = undefined;
            work.phase = "done";
            continue;
          }
          if (work.nodeScan !== undefined) {
            const step = this.#stepNodeScan(work.nodeScan);
            if (step.blocked) {
              if (!this.#isCurrentScan(revision)) {
                return;
              }
              this.#blockScan();
              remaining = 0;
              break;
            }
            if (step.complete) {
              work.nodeScan = undefined;
            }
            remaining -= 1;
            break;
          }
          const nodeId = work.nodeIds[work.fallbackIndex]!;
          work.fallbackIndex += 1;
          remaining -= 1;
          if (!work.visited.has(nodeId)) {
            work.nodeScan = this.#createNodeScan(
              nodeId,
              work.pendingRegistrations,
            );
          }
          break;
        }
      }
    }
    if (!this.#isCurrentScan(revision)) {
      return;
    }
    this.#compactScanTreeQueue(work);
    if (work.phase === "done") {
      this.#scanWork = undefined;
      this.#scheduledScanRevision = undefined;
      this.#scheduleScan();
      this.#drainCandidates();
      return;
    }
    this.#drainCandidates();
    if (!this.#isCurrentScan(revision)) {
      return;
    }
    if (!this.#scanBlocked) {
      this.#scheduleScanContinuation(revision);
    }
  }

  #isCurrentScan(revision: number): boolean {
    return !this.#closed &&
      revision === this.#scanRevision &&
      this.#scheduledScanRevision === revision;
  }

  #hasActiveRegistrations(
    registrations: readonly RegisteredProcessor[],
  ): boolean {
    return registrations.some((registration) => registration.active);
  }

  #compactScanTreeQueue(work: ProcessorScanWork): void {
    if (work.treeIndex === 0) {
      return;
    }
    for (
      let index = work.treeClearedIndex;
      index < work.treeIndex;
      index += 1
    ) {
      work.treeQueue[index] = undefined;
    }
    work.treeClearedIndex = work.treeIndex;
    const remaining = work.treeQueue.length - work.treeIndex;
    if (remaining === 0) {
      work.treeQueue.length = 0;
      work.treeIndex = 0;
      work.treeClearedIndex = 0;
      return;
    }
    if (
      work.treeIndex < scanQueueCompactionFloor ||
      work.treeQueue.length <= remaining * scanQueueCompactionRatio
    ) {
      return;
    }
    work.treeQueue.splice(0, work.treeIndex);
    work.treeIndex = 0;
    work.treeClearedIndex = 0;
  }

  #invalidateScan(): void {
    this.#scanRevision += 1;
    this.#scheduledScanRevision = undefined;
    this.#scheduledScanContinuationRevision = undefined;
    this.#scanWork = undefined;
    this.#unblockScan();
  }

  #blockScan(): void {
    this.#scanBlocked = true;
    if (this.#scanUnblocked !== undefined) {
      return;
    }
    let resolve!: () => void;
    const promise = new Promise<void>((complete) => {
      resolve = complete;
    });
    this.#scanUnblocked = { promise, resolve };
  }

  #unblockScan(): void {
    this.#scanBlocked = false;
    const scanUnblocked = this.#scanUnblocked;
    this.#scanUnblocked = undefined;
    scanUnblocked?.resolve();
  }

  #scheduleScanContinuation(revision: number): void {
    if (this.#scheduledScanContinuationRevision !== undefined) {
      return;
    }
    this.#scheduledScanContinuationRevision = revision;
    setTimeout(() => {
      if (this.#scheduledScanContinuationRevision !== revision) {
        return;
      }
      this.#scheduledScanContinuationRevision = undefined;
      this.#runScan(revision);
    }, 0);
  }

  #createNodeScan(
    nodeId: NodeId,
    registrations: RegisteredProcessor[],
  ): ProcessorNodeScan {
    const expectedEpoch = this.#store.getSnapshot().document?.coordinate.epoch;
    let view: ProcessorNodeScan["view"];
    try {
      view = {
        kind: "ready",
        node: expectedEpoch === undefined
          ? undefined
          : this.#store.getNodeSnapshot(nodeId),
      };
    } catch (error) {
      view = { kind: "failed", error: MdstreamError.from(error) };
    }
    return {
      nodeId,
      registrations,
      expectedEpoch,
      view,
      registrationIndex: 0,
    };
  }

  #stepNodeScan(scan: ProcessorNodeScan): ProcessorNodeScanStep {
    const registration = scan.registrations[scan.registrationIndex];
    if (registration === undefined) {
      return {
        complete: true,
        nodeView: scan.view.kind === "ready" ? scan.view.node : undefined,
        blocked: false,
      };
    }
    if (registration.active) {
      if (scan.expectedEpoch === undefined) {
        this.#removeCandidate(registration, scan.nodeId);
      } else if (scan.view.kind === "failed") {
        this.#removeCandidate(registration, scan.nodeId);
        this.#emitError({
          phase: "view",
          processorId: registration.descriptor.id,
          nodeId: scan.nodeId,
          requestId: undefined,
          error: scan.view.error,
        });
      } else {
        const nodeView = scan.view.node;
        if (nodeView === undefined) {
          this.#removeCandidate(registration, scan.nodeId);
          this.#advanceNodeScan(scan, registration);
          return this.#nodeScanStep(scan);
        }
        const processor = registration.processor;
        if (
          nodeView.node.stability === "provisional" &&
          !(registration.descriptor.acceptsProvisional &&
            registration.allowProvisional)
        ) {
          this.#removeCandidate(registration, scan.nodeId);
        } else {
          let matches: boolean;
          try {
            matches = processor.matches(nodeView.node);
          } catch (error) {
            this.#removeCandidate(registration, scan.nodeId);
            this.#emitError({
              phase: "matches",
              processorId: registration.descriptor.id,
              nodeId: scan.nodeId,
              requestId: undefined,
              error,
            });
            this.#advanceNodeScan(scan, registration);
            return this.#nodeScanStep(scan);
          }
          if (matches && registration.active) {
            if (!this.#enqueueCandidate(
              registration,
              scan.expectedEpoch,
              scan.nodeId,
              nodeView.processorInputVersion,
            )) {
              return {
                complete: false,
                nodeView,
                blocked: true,
              };
            }
          } else {
            this.#removeCandidate(registration, scan.nodeId);
          }
        }
      }
    }
    this.#advanceNodeScan(scan, registration);
    return this.#nodeScanStep(scan);
  }

  #advanceNodeScan(
    scan: ProcessorNodeScan,
    registration: RegisteredProcessor,
  ): void {
    if (scan.registrations[scan.registrationIndex] === registration) {
      scan.registrationIndex += 1;
    }
  }

  #nodeScanStep(scan: ProcessorNodeScan): ProcessorNodeScanStep {
    return {
      complete: scan.registrationIndex >= scan.registrations.length,
      nodeView: scan.view.kind === "ready" ? scan.view.node : undefined,
      blocked: false,
    };
  }

  #enqueueCandidate(
    registration: RegisteredProcessor,
    expectedEpoch: Epoch,
    nodeId: NodeId,
    expectedInputVersion: ProcessorInputVersion,
    front = false,
  ): boolean {
    if (!registration.active || this.#closed) {
      return true;
    }
    const rejected = this.#rejectedCandidates.get(registration)?.get(nodeId);
    if (
      rejected?.epoch === expectedEpoch &&
      rejected.inputVersion === expectedInputVersion
    ) {
      return true;
    }
    let registrationCandidates = this.#candidates.get(registration);
    const existing = registrationCandidates?.get(nodeId);
    if (existing !== undefined) {
      existing.expectedEpoch = expectedEpoch;
      existing.expectedInputVersion = expectedInputVersion;
      return true;
    }
    if (this.#candidateCount >= this.#maxCandidates) {
      if (!this.#candidateQueueSaturated) {
        this.#candidateQueueSaturated = true;
        this.#emitError({
          phase: "begin",
          processorId: registration.descriptor.id,
          nodeId,
          requestId: undefined,
          error: new MdstreamError(
            `processor candidate queue limit ${this.#maxCandidates} exceeded`,
            {
              ...RESOURCE_LIMIT_STATUS,
              detailCode: "processor.candidate_queue_limit",
            },
          ),
        });
      }
      return !registration.active || this.#closed;
    }
    if (registrationCandidates === undefined) {
      registrationCandidates = new Map<NodeId, ProcessorCandidate>();
      this.#candidates.set(registration, registrationCandidates);
    }
    const candidate: ProcessorCandidate = {
      registration,
      nodeId,
      expectedEpoch,
      expectedInputVersion,
      queued: true,
    };
    registrationCandidates.set(nodeId, candidate);
    if (front) {
      this.#candidateQueue.splice(this.#candidateHead, 0, candidate);
    } else {
      this.#candidateQueue.push(candidate);
    }
    this.#candidateCount += 1;
    return true;
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
    this.#markCandidateCapacityAvailable();
    if (registrationCandidates!.size === 0) {
      this.#candidates.delete(registration);
    }
    this.#compactCandidateQueue();
  }

  #removeNodeCandidates(nodeId: NodeId): void {
    for (const registration of this.#candidates.keys()) {
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
    this.#markCandidateCapacityAvailable();
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
      inputVersion: candidate.expectedInputVersion,
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
    for (const registration of this.#rejectedCandidates.keys()) {
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
    this.#candidateQueueSaturated = false;
    this.#dispatchBlocked = false;
  }

  #takeCandidate(): ProcessorCandidate | null | undefined {
    if (this.#candidateHead >= this.#candidateQueue.length) {
      this.#compactCandidateQueue();
      return undefined;
    }
    const candidate = this.#candidateQueue[this.#candidateHead++]!;
    if (!candidate.queued) {
      this.#compactCandidateQueue();
      return null;
    }
    candidate.queued = false;
    const registrationCandidates = this.#candidates.get(candidate.registration);
    registrationCandidates?.delete(candidate.nodeId);
    if (registrationCandidates?.size === 0) {
      this.#candidates.delete(candidate.registration);
    }
    this.#candidateCount -= 1;
    this.#markCandidateCapacityAvailable();
    this.#compactCandidateQueue();
    return candidate;
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
    if (
      this.#dispatching ||
      this.#dispatchBlocked ||
      this.#scheduledDispatchRevision !== undefined ||
      this.#closed
    ) {
      return;
    }
    this.#dispatching = true;
    let dequeued = 0;
    try {
      while (
        dequeued < dispatchQuantum &&
        this.#candidateCount > 0 &&
        this.#inFlight.size < this.#maxInFlightJobs
      ) {
        const candidate = this.#takeCandidate();
        if (candidate === undefined) {
          break;
        }
        dequeued += 1;
        if (candidate === null) {
          continue;
        }
        if (!candidate.registration.active) {
          continue;
        }
        if (this.#begin(candidate) === "blocked") {
          this.#dispatchBlocked = true;
          break;
        }
      }
    } finally {
      this.#dispatching = false;
    }
    if (
      !this.#dispatchBlocked &&
      dequeued === dispatchQuantum &&
      this.#candidateCount > 0 &&
      this.#inFlight.size < this.#maxInFlightJobs
    ) {
      this.#scheduleDispatch();
    }
  }

  #scheduleDispatch(): void {
    if (
      this.#dispatchBlocked ||
      this.#scheduledDispatchRevision !== undefined ||
      this.#candidateCount === 0 ||
      this.#inFlight.size >= this.#maxInFlightJobs ||
      this.#closed
    ) {
      return;
    }
    const revision = this.#dispatchRevision;
    this.#scheduledDispatchRevision = revision;
    setTimeout(() => {
      if (this.#scheduledDispatchRevision !== revision) {
        return;
      }
      this.#scheduledDispatchRevision = undefined;
      if (this.#closed || revision !== this.#dispatchRevision) {
        return;
      }
      this.#drainCandidates();
    }, 0);
  }

  #invalidateDispatch(): void {
    this.#dispatchRevision += 1;
    this.#scheduledDispatchRevision = undefined;
    this.#dispatchBlocked = false;
  }

  #markCandidateCapacityAvailable(): void {
    if (this.#candidateCount < this.#maxCandidates) {
      if (this.#scanBlocked) {
        this.#unblockScan();
        const revision = this.#scheduledScanRevision;
        if (revision !== undefined) {
          this.#scheduleScanContinuation(revision);
        }
      }
    }
  }

  #begin(candidate: ProcessorCandidate): BeginDisposition {
    const {
      registration,
      expectedEpoch,
      nodeId,
      expectedInputVersion,
    } = candidate;
    let request: ProcessorRequestView | undefined;
    const parentRemovals = this.#removedDuringBegin;
    const removals = new Map<RequestGeneration, unknown>();
    this.#removedDuringBegin = removals;
    try {
      const requests = this.#store.beginProcessor({
        expectedEpoch,
        nodeId,
        expectedInputVersion,
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
      if (
        normalized.status === RESOURCE_LIMIT_STATUS.status &&
        retryableResourceLimitDetailCodes.has(normalized.detailCode) &&
        this.#inFlight.size > 0
      ) {
        this.#enqueueCandidate(
          registration,
          expectedEpoch,
          nodeId,
          expectedInputVersion,
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
          this.#dispatchBlocked = false;
        }
        this.#jobs.delete(job);
        this.#scheduleDispatch();
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
    this.#removeRegistrationFromScan(registration);
    this.#removeRegistrationCandidates(registration);
    this.#rejectedCandidates.delete(registration);
    let capacityChanged = false;
    for (const entry of [...this.#inFlight.values()]) {
      if (entry.registration === registration) {
        entry.controller.abort("processor_unregistered");
        this.#inFlight.delete(entry.request.requestId);
        capacityChanged = true;
        this.#cancel(entry, "cancel");
      }
    }
    if (capacityChanged) {
      this.#dispatchBlocked = false;
    }
    if (this.#processors.size === 0) {
      this.#invalidateScan();
      this.#pendingNodes.clear();
      this.#pendingRegistrations.clear();
    }
    this.#drainCandidates();
  }

  #removeRegistrationFromScan(registration: RegisteredProcessor): void {
    const work = this.#scanWork;
    if (work === undefined) {
      return;
    }
    work.registrations = work.registrations.filter(
      (candidate) => candidate !== registration,
    );
    work.pendingRegistrations = work.pendingRegistrations.filter(
      (candidate) => candidate !== registration,
    );
    const scan = work.nodeScan;
    if (scan !== undefined) {
      const index = scan.registrations.indexOf(registration);
      if (index >= 0) {
        scan.registrations = scan.registrations.filter(
          (candidate) => candidate !== registration,
        );
        if (index < scan.registrationIndex) {
          scan.registrationIndex -= 1;
        }
      }
    }
    if (
      work.phase === "changed" &&
      !this.#hasActiveRegistrations(work.registrations)
    ) {
      work.nodeScan = undefined;
      work.phase = this.#hasActiveRegistrations(work.pendingRegistrations)
        ? "tree"
        : "done";
    } else if (
      (work.phase === "tree" || work.phase === "fallback") &&
      !this.#hasActiveRegistrations(work.pendingRegistrations)
    ) {
      work.nodeScan = undefined;
      work.treeQueue.length = 0;
      work.visited.clear();
      work.phase = "done";
    }
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

function validateProcessorIdentifier(value: unknown, field: string): asserts value is string {
  if (typeof value !== "string" || !processorIdentifierPattern.test(value)) {
    throw new TypeError(
      `${field} must be 1-128 ASCII bytes using letters, digits, '.', '_', ':', '+', or '-'`,
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
