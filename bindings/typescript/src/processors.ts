import {
  RustBackedStore,
  type InternalStoreEvents,
} from "./store.js";
import {
  isProcessorFailureCode,
  MdstreamError,
  type ContentNodeView,
  type NodeId,
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
  active: boolean;
}

interface InFlightProcessor {
  readonly registration: RegisteredProcessor;
  readonly request: ProcessorRequestView;
  readonly controller: AbortController;
}

/** @internal */
export class ProcessorScheduler {
  readonly #store: RustBackedStore;
  readonly #processors = new Map<string, RegisteredProcessor>();
  readonly #inFlight = new Map<string, InFlightProcessor>();
  readonly #pendingNodes = new Set<NodeId>();
  readonly #jobs = new Set<Promise<void>>();
  readonly #errorListeners = new Set<ProcessorErrorListener>();
  #scanScheduled = false;
  #closed = false;

  constructor(store: RustBackedStore) {
    this.#store = store;
  }

  register(processor: ContentProcessor): ProcessorRegistration {
    this.#assertOpen();
    const id = processor.descriptor.id;
    if (this.#processors.has(id)) {
      throw new TypeError(`processor ${id} is already registered`);
    }
    const registration: RegisteredProcessor = { processor, active: true };
    this.#processors.set(id, registration);
    return {
      dispose: () => this.#disposeRegistration(registration),
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
        this.#inFlight.get(change.key.generation)?.controller.abort(change.change.reason);
      }
    }
    if (this.#processors.size === 0) {
      return;
    }
    for (const update of events.updates) {
      if (update.outcome.kind !== "applied" && update.outcome.kind !== "recovered") {
        continue;
      }
      for (const id of update.impact.removedNodeIds) {
        this.#pendingNodes.delete(id);
      }
      for (const id of update.impact.changedNodeIds) {
        this.#pendingNodes.add(id);
      }
    }
    this.#scheduleScan();
  }

  async whenIdle(): Promise<void> {
    for (;;) {
      await Promise.resolve();
      if (this.#scanScheduled || this.#pendingNodes.size > 0) {
        continue;
      }
      const jobs = [...this.#jobs];
      if (jobs.length === 0) {
        return;
      }
      await Promise.allSettled(jobs);
    }
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#pendingNodes.clear();
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
    if (this.#scanScheduled || this.#pendingNodes.size === 0 || this.#closed) {
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
    const registrations = [...this.#processors.values()];

    for (const nodeId of nodeIds) {
      let nodeView: NodeView | undefined;
      try {
        nodeView = this.#store.getNodeSnapshot(nodeId);
      } catch (error) {
        for (const registration of registrations) {
          if (registration.active) {
            this.#emitError({
              phase: "view",
              processorId: registration.processor.descriptor.id,
              nodeId,
              requestId: undefined,
              error: MdstreamError.from(error),
            });
          }
        }
        continue;
      }
      if (nodeView === undefined) {
        continue;
      }
      for (const registration of registrations) {
        if (!registration.active) {
          continue;
        }
        const processor = registration.processor;
        if (
          nodeView.node.stability === "provisional" &&
          !(processor.descriptor.acceptsProvisional === true &&
            processor.allowProvisional === true)
        ) {
          continue;
        }
        let matches: boolean;
        try {
          matches = processor.matches(nodeView.node);
        } catch (error) {
          this.#emitError({
            phase: "matches",
            processorId: processor.descriptor.id,
            nodeId,
            requestId: undefined,
            error,
          });
          continue;
        }
        if (!matches) {
          continue;
        }
        this.#begin(registration, nodeId);
      }
    }
    this.#scheduleScan();
  }

  #begin(registration: RegisteredProcessor, nodeId: NodeId): void {
    const processor = registration.processor;
    let request: ProcessorRequestView | undefined;
    try {
      request = this.#store.beginProcessor({
        nodeId,
        processorId: processor.descriptor.id,
        processorVersion: processor.descriptor.version,
        configurationVersion: processor.configurationVersion,
        acceptsProvisional: processor.descriptor.acceptsProvisional === true,
        allowProvisional: processor.allowProvisional === true,
      }).processorRequests[0];
      if (request === undefined) {
        throw new Error("Rust processor host returned no processor request");
      }
    } catch (error) {
      this.#emitError({
        phase: "begin",
        processorId: processor.descriptor.id,
        nodeId,
        requestId: undefined,
        error: MdstreamError.from(error),
      });
      return;
    }

    const entry: InFlightProcessor = {
      registration,
      request,
      controller: new AbortController(),
    };
    this.#inFlight.set(request.requestId, entry);
    const job = Promise.resolve()
      .then(() => processor.process(request, { signal: entry.controller.signal }))
      .then(
        (output) => this.#complete(entry, normalizeOutput(output)),
        (error) => this.#processorFailed(entry, error),
      )
      .finally(() => {
        if (this.#inFlight.get(request.requestId) === entry) {
          this.#inFlight.delete(request.requestId);
        }
        this.#jobs.delete(job);
      });
    this.#jobs.add(job);
  }

  #complete(entry: InFlightProcessor, output: ProcessorOutput): void {
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
      this.#emitError({
        phase: "complete",
        processorId: entry.registration.processor.descriptor.id,
        nodeId: entry.request.key.nodeId,
        requestId: entry.request.requestId,
        error: MdstreamError.from(error),
      });
      this.#cancel(entry, "cancel");
    }
  }

  #processorFailed(entry: InFlightProcessor, error: unknown): void {
    this.#emitError({
      phase: "process",
      processorId: entry.registration.processor.descriptor.id,
      nodeId: entry.request.key.nodeId,
      requestId: entry.request.requestId,
      error,
    });
    try {
      this.#store.failProcessor(
        entry.request.requestId,
        entry.controller.signal.aborted ? "cancelled" : "panic",
        processorErrorMessage(error),
      );
    } catch (completionError) {
      this.#emitError({
        phase: "complete",
        processorId: entry.registration.processor.descriptor.id,
        nodeId: entry.request.key.nodeId,
        requestId: entry.request.requestId,
        error: MdstreamError.from(completionError),
      });
      this.#cancel(entry, "cancel");
    }
  }

  #disposeRegistration(registration: RegisteredProcessor): void {
    if (!registration.active) {
      return;
    }
    registration.active = false;
    this.#processors.delete(registration.processor.descriptor.id);
    for (const entry of [...this.#inFlight.values()]) {
      if (entry.registration === registration) {
        entry.controller.abort("processor_unregistered");
        this.#cancel(entry, "cancel");
        this.#inFlight.delete(entry.request.requestId);
      }
    }
  }

  #cancel(entry: InFlightProcessor, phase: ProcessorErrorPhase): void {
    try {
      this.#store.cancelProcessor(entry.request.requestId);
    } catch (error) {
      this.#emitError({
        phase,
        processorId: entry.registration.processor.descriptor.id,
        nodeId: entry.request.key.nodeId,
        requestId: entry.request.requestId,
        error: MdstreamError.from(error),
      });
    }
  }

  #emitError(event: ProcessorErrorEvent): void {
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
