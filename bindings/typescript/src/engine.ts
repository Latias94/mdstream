import {
  ProcessorScheduler,
  type ContentProcessor,
  type ProcessorErrorListener,
  type ProcessorRegistration,
  type ProcessorSchedulerLimits,
} from "./processors.js";
import {
  createStoreView,
  readBindingMetrics,
  RustBackedStore,
  type BindingMetricsView,
  type MdstreamStore,
  type MdstreamStoreView,
  type ReducerResult,
} from "./store.js";
import {
  asCanonicalChangeBytes,
  asCanonicalSnapshotBytes,
  MdstreamError,
  type CanonicalChangeBytes,
  type CanonicalSnapshotBytes,
  type DecimalCounter,
} from "./views.js";
import {
  BindingPayloadKind,
  TRANSITION_SCHEMA,
  defaultWasmLoader,
  drainOutput,
  loadWasmBindings,
  readProcessorSchedulerLimits,
  type ValidatedWasmBindings,
  type WasmEngineSession,
  type WasmModuleLoader,
  type WasmOutput,
  type WasmReducerSession,
} from "./wasm.js";

export type { WasmModuleLoader } from "./wasm.js";

export type DecimalInput = string | bigint;

export interface ProtocolLimitOptions {
  readonly maxSourceBytes?: DecimalInput;
  readonly maxNodes?: DecimalInput;
  readonly maxResources?: DecimalInput;
  readonly maxOperations?: DecimalInput;
  readonly maxChangeStructuralItems?: DecimalInput;
  readonly maxDocumentStructuralItems?: DecimalInput;
  readonly maxChildrenPerList?: DecimalInput;
  readonly maxAttributesPerNode?: DecimalInput;
  readonly maxMetadataValueBytes?: DecimalInput;
  readonly maxNodeMetadataBytes?: DecimalInput;
  readonly maxChangeMetadataBytes?: DecimalInput;
  readonly maxDocumentMetadataBytes?: DecimalInput;
  readonly maxTreeDepth?: DecimalInput;
}

export interface CompilerLimitOptions {
  readonly maxMarkdownEvents?: DecimalInput;
  readonly maxMarkdownOverlapWork?: DecimalInput;
  readonly maxDefinitions?: DecimalInput;
  readonly maxDefinitionEdges?: DecimalInput;
  readonly maxDefinitionMetadataBytes?: DecimalInput;
}

export interface EngineLimitOptions {
  readonly maxChangeBytes?: DecimalInput;
  readonly maxTransactionBytes?: DecimalInput;
}

export interface ProcessorLimitOptions {
  readonly maxInputBytes?: DecimalInput;
  readonly maxArtifactBytes?: DecimalInput;
  readonly maxInFlightJobs?: DecimalInput;
  readonly maxInFlightInputBytes?: DecimalInput;
  readonly maxSlots?: DecimalInput;
  readonly maxRetainedArtifacts?: DecimalInput;
  readonly maxRetainedArtifactBytes?: DecimalInput;
  readonly maxErrorBytes?: DecimalInput;
  readonly maxPendingChanges?: DecimalInput;
  readonly maxPendingChangeBytes?: DecimalInput;
}

export interface WireLimitOptions {
  readonly maxCommandBytes?: DecimalInput;
  readonly maxEncodedChangeBytes?: DecimalInput;
  readonly maxEncodedSnapshotBytes?: DecimalInput;
  readonly maxReducerUpdateBytes?: DecimalInput;
  readonly maxProcessorPayloadBytes?: DecimalInput;
  readonly maxArtifactEventBytes?: DecimalInput;
  readonly maxViewBytes?: DecimalInput;
}

export interface CustomBlockOptions {
  readonly namespace: string;
  readonly name: string;
  readonly opaque?: boolean;
  readonly caseInsensitive?: boolean;
}

export interface MdstreamSessionOptions {
  readonly captureTransitions?: boolean;
  readonly protocol?: ProtocolLimitOptions;
  readonly compiler?: CompilerLimitOptions;
  readonly engine?: EngineLimitOptions;
  readonly processor?: ProcessorLimitOptions;
  readonly wire?: WireLimitOptions;
  readonly customBlocks?: readonly CustomBlockOptions[];
}

export interface InitMdstreamOptions {
  readonly loader?: WasmModuleLoader;
}

export interface EngineResult {
  readonly changes: readonly CanonicalChangeBytes[];
  readonly reducerResults: readonly ReducerResult[];
  readonly outputPayloadBytes: DecimalCounter;
}

export interface LosslessBatchOptions {
  readonly maxBatchBytes: number;
  readonly maxPendingChunks: number;
}

export interface BatchMetrics {
  readonly maxBatchBytes: DecimalCounter;
  readonly maxPendingChunks: DecimalCounter;
  readonly inputAttempts: DecimalCounter;
  readonly inputBytes: DecimalCounter;
  readonly appendAttempts: DecimalCounter;
  readonly successfulAppends: DecimalCounter;
  readonly committedBytes: DecimalCounter;
  readonly pendingBytes: DecimalCounter;
  readonly pendingConstituents: DecimalCounter;
  /** Logical bytes for live boundary records, at eight bytes per constituent. */
  readonly boundaryMetadataBytes: DecimalCounter;
  readonly scanBytes: DecimalCounter;
  readonly joinCopyBytes: DecimalCounter;
  readonly replayCount: DecimalCounter;
  readonly outputPayloadBytes: DecimalCounter;
  readonly publishedResults: DecimalCounter;
}

export interface BatchPendingInput {
  readonly chunks: readonly string[];
  readonly bytes: DecimalCounter;
  readonly constituents: DecimalCounter;
}

export interface BatchedRecoverySnapshot {
  readonly flushed: readonly EngineResult[];
  readonly snapshot: CanonicalSnapshotBytes | undefined;
}

export type BatchOperation =
  | "push"
  | "flush"
  | "retry_pending"
  | "finish"
  | "reset"
  | "recovery_snapshot";

export class BatchOperationError extends Error {
  readonly completedResults: readonly EngineResult[];
  override readonly cause: unknown;
  readonly operation: BatchOperation;
  readonly pending: BatchPendingInput | undefined;
  readonly newInputAccepted: boolean | undefined;

  constructor(options: {
    readonly completedResults: readonly EngineResult[];
    readonly cause: unknown;
    readonly operation: BatchOperation;
    readonly pending: BatchPendingInput | undefined;
    readonly newInputAccepted: boolean | undefined;
  }) {
    super("batch operation failed with explicitly recoverable input ownership", {
      cause: options.cause,
    });
    this.name = "BatchOperationError";
    this.completedResults = Object.freeze([...options.completedResults]);
    this.cause = options.cause;
    this.operation = options.operation;
    this.pending = options.pending;
    this.newInputAccepted = options.newInputAccepted;
  }
}

const emptyEngineResults = Object.freeze([]) as readonly EngineResult[];

const runtimes = new WeakMap<WasmModuleLoader, Promise<MdstreamRuntime>>();

export async function initMdstream(
  options: InitMdstreamOptions = {},
): Promise<MdstreamRuntime> {
  const loader = options.loader ?? defaultWasmLoader;
  let runtime = runtimes.get(loader);
  if (runtime === undefined) {
    runtime = createRuntime(loader).catch((error: unknown) => {
      runtimes.delete(loader);
      throw error;
    });
    runtimes.set(loader, runtime);
  }
  return runtime;
}

export class MdstreamRuntime {
  readonly #wasm: ValidatedWasmBindings;
  readonly abiVersion: number;
  readonly packageVersion: string;
  readonly bindingSchema: string;
  readonly bindingOptionsSchema: string;
  readonly transitionSchema: typeof TRANSITION_SCHEMA;

  private constructor(wasm: ValidatedWasmBindings) {
    this.#wasm = wasm;
    this.abiVersion = wasm.metadata.abiVersion;
    this.packageVersion = wasm.metadata.packageVersion;
    this.bindingSchema = wasm.metadata.bindingSchema;
    this.bindingOptionsSchema = wasm.metadata.bindingOptionsSchema;
    this.transitionSchema = wasm.metadata.transitionSchema;
  }

  /** @internal */
  static fromWasm(wasm: ValidatedWasmBindings): MdstreamRuntime {
    return new MdstreamRuntime(wasm);
  }

  createStore(options?: MdstreamSessionOptions): MdstreamStore {
    const prepared = prepareSessionOptions(options, this.bindingOptionsSchema);
    try {
      return new RustBackedStore(
        new this.#wasm.MdstreamReducerSession(prepared.encodedJson),
        this.bindingSchema,
        prepared.captureTransitions,
      );
    } catch (error) {
      throw MdstreamError.from(error);
    }
  }

  createEngine(options?: MdstreamSessionOptions): MdstreamEngine {
    const prepared = prepareSessionOptions(options, this.bindingOptionsSchema);
    let engine: WasmEngineSession;
    try {
      engine = new this.#wasm.MdstreamEngineSession(prepared.encodedJson);
    } catch (error) {
      throw MdstreamError.from(error);
    }
    try {
      const reducer = new this.#wasm.MdstreamReducerSession(prepared.encodedJson);
      let store: RustBackedStore | undefined;
      try {
        const schedulerLimits = readProcessorSchedulerLimits(reducer);
        store = new RustBackedStore(
          reducer,
          this.bindingSchema,
          prepared.captureTransitions,
        );
        return MdstreamEngine.fromSessions(engine, store, schedulerLimits);
      } catch (error) {
        releaseFailedReducer(reducer, store);
        throw error;
      }
    } catch (error) {
      engine.free();
      throw MdstreamError.from(error);
    }
  }
}

export class MdstreamEngine {
  readonly store: MdstreamStoreView;
  readonly #engine: WasmEngineSession;
  readonly #rustStore: RustBackedStore;
  readonly #scheduler: ProcessorScheduler;
  #activeBatchLease: object | undefined;
  #closed = false;

  private constructor(
    engine: WasmEngineSession,
    store: RustBackedStore,
    schedulerLimits: ProcessorSchedulerLimits,
  ) {
    this.#engine = engine;
    this.#rustStore = store;
    this.store = createStoreView(store);
    this.#scheduler = new ProcessorScheduler(store, schedulerLimits);
    store.setEventSink((events) => this.#scheduler.handleStoreEvents(events));
  }

  /** @internal */
  static fromSessions(
    engine: WasmEngineSession,
    store: RustBackedStore,
    schedulerLimits: ProcessorSchedulerLimits,
  ): MdstreamEngine {
    return new MdstreamEngine(engine, store, schedulerLimits);
  }

  append(chunk: string): EngineResult {
    return this.#runDirectMutation(() => this.#append(chunk));
  }

  finish(): EngineResult {
    return this.#runDirectMutation(() => this.#finish());
  }

  reset(): EngineResult {
    return this.#runDirectMutation(() => this.#reset());
  }

  createRecoverySnapshot(): CanonicalSnapshotBytes | undefined {
    return this.#runDirectMutation(() => this.#recoverySnapshot());
  }

  registerProcessor(processor: ContentProcessor): ProcessorRegistration {
    return this.#runDirectMutation(() => this.#scheduler.register(processor));
  }

  subscribeProcessorErrors(listener: ProcessorErrorListener): () => void {
    return this.#scheduler.subscribeErrors(listener);
  }

  whenProcessorsIdle(): Promise<void> {
    return this.#scheduler.whenIdle();
  }

  createBatcher(options: LosslessBatchOptions): LosslessInputBatcher {
    const normalized = normalizeBatchOptions(options);
    return new EngineInputBatcher(
      this.#acquireBatchLease(),
      normalized.maxBatchBytes,
      normalized.maxPendingChunks,
    );
  }

  metrics(): BindingMetricsView {
    this.#assertOpen();
    return readBindingMetrics(this.#engine.metrics());
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#assertNoBatchLease();
    this.#rustStore.assertMutationAllowed();
    this.#closed = true;
    this.#scheduler.close();
    this.#rustStore.setEventSink(undefined);
    this.#engine.free();
    this.#rustStore.close();
  }

  #runDirectMutation<Result>(operation: () => Result): Result {
    this.#assertOpen();
    this.#assertNoBatchLease();
    return this.#rustStore.runDocumentOperation(operation);
  }

  #append(chunk: string): EngineResult {
    assertRawAppendAdmission(this.#engine, chunk);
    return this.#consume(() => this.#engine.append(chunk));
  }

  #finish(): EngineResult {
    return this.#consume(() => this.#engine.finish());
  }

  #reset(): EngineResult {
    return this.#consume(() => this.#engine.reset());
  }

  #recoverySnapshot(): CanonicalSnapshotBytes | undefined {
    let output: WasmOutput;
    try {
      output = this.#engine.snapshot();
    } catch (error) {
      throw MdstreamError.from(error);
    }
    const drained = drainOutput(output);
    let snapshot: CanonicalSnapshotBytes | undefined;
    for (const payload of drained.payloads) {
      if (payload.kind !== BindingPayloadKind.Snapshot || snapshot !== undefined) {
        throw new MdstreamError("engine snapshot returned an unexpected payload", {
          status: 12,
          statusName: "MDSTREAM_INTERNAL_ERROR",
          detailCode: "bindings.unexpected_payload",
        });
      }
      snapshot = asCanonicalSnapshotBytes(payload.bytes);
    }
    return snapshot;
  }

  #acquireBatchLease(): EngineBatchLease {
    this.#assertOpen();
    this.#assertNoBatchLease();
    this.#rustStore.assertMutationAllowed();
    const token = {};
    this.#activeBatchLease = token;
    const assertActive = () => {
      this.#assertOpen();
      if (this.#activeBatchLease !== token) {
        throw batchStateError(
          "the mdstream batching lease has been released",
          "bindings.batch_released",
        );
      }
    };
    return Object.freeze({
      run: <Result>(operation: () => Result): Result => {
        assertActive();
        return this.#rustStore.runCoherentDocumentOperation(operation);
      },
      preflightAppend: (
        chunk: string,
        observeScan: (bytes: number) => void,
      ): number => {
        assertActive();
        return admittedUtf8ByteLength(this.#engine, chunk, observeScan);
      },
      append: (chunk: string, admittedBytes: number): EngineResult => {
        assertActive();
        assertRawAppendByteLengthAdmission(this.#engine, admittedBytes);
        return this.#consume(() => this.#engine.append(chunk));
      },
      finish: (): EngineResult => {
        assertActive();
        return this.#finish();
      },
      reset: (): EngineResult => {
        assertActive();
        return this.#reset();
      },
      recoverySnapshot: (): CanonicalSnapshotBytes | undefined => {
        assertActive();
        return this.#recoverySnapshot();
      },
      assertActive,
      assertMutationAllowed: (): void => {
        assertActive();
        this.#rustStore.assertMutationAllowed();
      },
      release: (): void => {
        assertActive();
        this.#activeBatchLease = undefined;
      },
    });
  }

  #assertNoBatchLease(): void {
    if (this.#activeBatchLease !== undefined) {
      throw batchStateError(
        "mdstream engine mutation is owned by an active batching lease",
        "bindings.batch_lease_active",
      );
    }
  }

  #consume(operation: () => WasmOutput): EngineResult {
    let output: WasmOutput;
    try {
      output = operation();
    } catch (error) {
      throw MdstreamError.from(error);
    }
    const drained = drainOutput(output);
    const changes: CanonicalChangeBytes[] = [];
    const reducerResults: ReducerResult[] = [];
    let outputPayloadBytes = drained.payloadBytes;
    for (const payload of drained.payloads) {
      if (payload.kind !== BindingPayloadKind.Change) {
        throw new MdstreamError("engine operation returned a non-change payload", {
          status: 12,
          statusName: "MDSTREAM_INTERNAL_ERROR",
          detailCode: "bindings.unexpected_payload",
        });
      }
      const change = asCanonicalChangeBytes(payload.bytes);
      changes.push(change);
      const reducerResult = this.#rustStore.applyChange(change);
      reducerResults.push(reducerResult);
      outputPayloadBytes += BigInt(reducerResult.outputPayloadBytes);
    }
    return {
      changes,
      reducerResults,
      outputPayloadBytes: counter(outputPayloadBytes),
    };
  }

  #assertOpen(): void {
    if (this.#closed) {
      throw new MdstreamError("mdstream engine is closed", {
        status: 1,
        statusName: "MDSTREAM_INVALID_ARGUMENT",
        detailCode: "bindings.closed",
      });
    }
  }
}

export interface LosslessInputBatcher {
  readonly maxBatchBytes: number;
  readonly maxPendingChunks: number;
  push(chunk: string): readonly EngineResult[];
  flush(): readonly EngineResult[];
  retryPending(): readonly EngineResult[];
  finish(): readonly EngineResult[];
  reset(): readonly EngineResult[];
  createRecoverySnapshot(): BatchedRecoverySnapshot;
  inspectPending(): BatchPendingInput | undefined;
  takePending(): BatchPendingInput | undefined;
  discardPending(): BatchPendingInput | undefined;
  release(): void;
  metrics(): BatchMetrics;
}

interface EngineBatchLease {
  run<Result>(operation: () => Result): Result;
  preflightAppend(chunk: string, observeScan: (bytes: number) => void): number;
  append(chunk: string, admittedBytes: number): EngineResult;
  finish(): EngineResult;
  reset(): EngineResult;
  recoverySnapshot(): CanonicalSnapshotBytes | undefined;
  assertActive(): void;
  assertMutationAllowed(): void;
  release(): void;
}

interface PendingChunk {
  readonly text: string;
  readonly bytes: number;
}

const logicalBoundaryMetadataBytes = 8n;

class PendingChunks {
  readonly #chunks: PendingChunk[] = [];
  #bytes = 0;

  get bytes(): number {
    return this.#bytes;
  }

  get constituents(): number {
    return this.#chunks.length;
  }

  get isEmpty(): boolean {
    return this.#chunks.length === 0;
  }

  wouldExceed(bytes: number, maxBytes: number, maxChunks: number): boolean {
    return this.#chunks.length >= maxChunks || bytes > maxBytes - this.#bytes;
  }

  accept(text: string, bytes: number): void {
    if (bytes === 0) {
      return;
    }
    this.#chunks.push({ text, bytes });
    this.#bytes += bytes;
  }

  front(): PendingChunk | undefined {
    return this.#chunks[0];
  }

  commitFront(expected: PendingChunk): void {
    if (this.#chunks[0] !== expected) {
      throw new Error("pending input ownership changed during append");
    }
    this.#chunks.shift();
    this.#bytes -= expected.bytes;
  }

  clear(): void {
    this.#chunks.length = 0;
    this.#bytes = 0;
  }

  snapshot(): BatchPendingInput | undefined {
    if (this.#chunks.length === 0) {
      return undefined;
    }
    return Object.freeze({
      chunks: Object.freeze(this.#chunks.map(({ text }) => text)),
      bytes: counter(BigInt(this.#bytes)),
      constituents: counter(BigInt(this.#chunks.length)),
    });
  }

  joinedForEvaluation(): string | undefined {
    if (this.#chunks.length === 0) {
      return undefined;
    }
    return this.#chunks.length === 1
      ? this.#chunks[0]?.text
      : this.#chunks.map(({ text }) => text).join("");
  }
}

interface PendingApplyObserver<Result> {
  onAttempt(): void;
  onCommitted(chunk: PendingChunk, result: Result): void;
}

class PendingApplyFailure<Result> extends Error {
  readonly completedResults: readonly Result[];
  override readonly cause: unknown;

  constructor(completedResults: readonly Result[], cause: unknown) {
    super("pending constituent append failed", { cause });
    this.completedResults = Object.freeze([...completedResults]);
    this.cause = cause;
  }
}

function applyPendingConstituents<Result>(
  pending: PendingChunks,
  append: (chunk: string, bytes: number) => Result,
  observer: PendingApplyObserver<Result>,
): readonly Result[] {
  const completed: Result[] = [];
  while (true) {
    const chunk = pending.front();
    if (chunk === undefined) {
      return completed.length === 0 ? Object.freeze([]) : Object.freeze(completed);
    }
    observer.onAttempt();
    let result: Result;
    try {
      result = append(chunk.text, chunk.bytes);
    } catch (error) {
      throw new PendingApplyFailure(completed, error);
    }
    pending.commitFront(chunk);
    observer.onCommitted(chunk, result);
    completed.push(result);
  }
}

class EngineInputBatcher implements LosslessInputBatcher {
  readonly #lease: EngineBatchLease;
  readonly #pending = new PendingChunks();
  readonly #maxBatchBytes: number;
  readonly #maxPendingChunks: number;
  #released = false;
  #unresolved = false;
  #inputAttempts = 0n;
  #inputBytes = 0n;
  #appendAttempts = 0n;
  #successfulAppends = 0n;
  #committedBytes = 0n;
  #scanBytes = 0n;
  #joinCopyBytes = 0n;
  #replayCount = 0n;
  #outputPayloadBytes = 0n;
  #publishedResults = 0n;

  constructor(
    lease: EngineBatchLease,
    maxBatchBytes: number,
    maxPendingChunks: number,
  ) {
    this.#lease = lease;
    this.#maxBatchBytes = maxBatchBytes;
    this.#maxPendingChunks = maxPendingChunks;
  }

  get maxBatchBytes(): number {
    return this.#maxBatchBytes;
  }

  get maxPendingChunks(): number {
    return this.#maxPendingChunks;
  }

  push(chunk: string): readonly EngineResult[] {
    return this.#lease.run(() => {
      this.#assertUsable();
      this.#inputAttempts += 1n;
      this.#assertResolved();
      const bytes = this.#lease.preflightAppend(chunk, (scannedBytes) => {
        this.#scanBytes += BigInt(scannedBytes);
      });
      this.#inputBytes += BigInt(bytes);
      if (bytes === 0) {
        return emptyEngineResults;
      }

      const results: EngineResult[] = [];
      if (
        !this.#pending.isEmpty &&
        this.#pending.wouldExceed(
          bytes,
          this.#maxBatchBytes,
          this.#maxPendingChunks,
        )
      ) {
        results.push(...this.#applyPending("push", false));
      }

      if (bytes > this.#maxBatchBytes) {
        try {
          results.push(this.#appendStandalone(chunk, bytes));
        } catch (error) {
          this.#throwBatchOperationFailure(results, error, "push", false);
        }
        return Object.freeze(results);
      }

      this.#pending.accept(chunk, bytes);
      if (this.#pending.bytes === this.#maxBatchBytes) {
        results.push(...this.#applyPending("push", true, results));
      }
      return results.length === 0 ? emptyEngineResults : Object.freeze(results);
    });
  }

  flush(): readonly EngineResult[] {
    return this.#lease.run(() => {
      this.#assertUsable();
      this.#assertResolved();
      return this.#applyPending("flush", undefined);
    });
  }

  retryPending(): readonly EngineResult[] {
    return this.#lease.run(() => {
      this.#assertUsable();
      if (!this.#unresolved) {
        throw batchStateError(
          "retryPending requires unresolved pending input",
          "bindings.batch_pending",
        );
      }
      return this.#applyPending("retry_pending", undefined);
    });
  }

  finish(): readonly EngineResult[] {
    return this.#lifecycle("finish", () => this.#lease.finish());
  }

  reset(): readonly EngineResult[] {
    return this.#lifecycle("reset", () => this.#lease.reset());
  }

  createRecoverySnapshot(): BatchedRecoverySnapshot {
    return this.#lease.run(() => {
      this.#assertUsable();
      this.#assertResolved();
      const flushed = this.#applyPending("recovery_snapshot", undefined);
      let snapshot: CanonicalSnapshotBytes | undefined;
      try {
        snapshot = this.#lease.recoverySnapshot();
      } catch (error) {
        this.#throwBatchOperationFailure(
          flushed,
          error,
          "recovery_snapshot",
          undefined,
        );
      }
      if (snapshot !== undefined) {
        this.#outputPayloadBytes += BigInt(snapshot.byteLength);
      }
      return Object.freeze({ flushed, snapshot });
    });
  }

  inspectPending(): BatchPendingInput | undefined {
    this.#assertUsable();
    this.#lease.assertActive();
    return this.#pending.snapshot();
  }

  takePending(): BatchPendingInput | undefined {
    this.#assertUsable();
    this.#lease.assertMutationAllowed();
    const pending = this.#pending.snapshot();
    this.#pending.clear();
    this.#unresolved = false;
    return pending;
  }

  discardPending(): BatchPendingInput | undefined {
    this.#assertUsable();
    this.#lease.assertMutationAllowed();
    const discarded = this.#pending.snapshot();
    this.#pending.clear();
    this.#unresolved = false;
    return discarded;
  }

  release(): void {
    this.#assertUsable();
    this.#lease.assertMutationAllowed();
    if (!this.#pending.isEmpty || this.#unresolved) {
      throw batchStateError(
        "pending input must commit, transfer, or be explicitly discarded before release",
        "bindings.batch_pending",
      );
    }
    this.#lease.release();
    this.#released = true;
  }

  metrics(): BatchMetrics {
    return Object.freeze({
      maxBatchBytes: counter(BigInt(this.#maxBatchBytes)),
      maxPendingChunks: counter(BigInt(this.#maxPendingChunks)),
      inputAttempts: counter(this.#inputAttempts),
      inputBytes: counter(this.#inputBytes),
      appendAttempts: counter(this.#appendAttempts),
      successfulAppends: counter(this.#successfulAppends),
      committedBytes: counter(this.#committedBytes),
      pendingBytes: counter(BigInt(this.#pending.bytes)),
      pendingConstituents: counter(BigInt(this.#pending.constituents)),
      boundaryMetadataBytes: counter(
        BigInt(this.#pending.constituents) * logicalBoundaryMetadataBytes,
      ),
      scanBytes: counter(this.#scanBytes),
      joinCopyBytes: counter(this.#joinCopyBytes),
      replayCount: counter(this.#replayCount),
      outputPayloadBytes: counter(this.#outputPayloadBytes),
      publishedResults: counter(this.#publishedResults),
    });
  }

  #lifecycle(
    operation: "finish" | "reset",
    callback: () => EngineResult,
  ): readonly EngineResult[] {
    return this.#lease.run(() => {
      this.#assertUsable();
      this.#assertResolved();
      const results = [...this.#applyPending(operation, undefined)];
      let result: EngineResult;
      try {
        result = callback();
      } catch (error) {
        this.#throwBatchOperationFailure(results, error, operation, undefined);
      }
      this.#recordPublishedResult(result);
      results.push(result);
      return Object.freeze(results);
    });
  }

  #applyPending(
    operation: BatchOperation,
    newInputAccepted: boolean | undefined,
    earlierResults: readonly EngineResult[] = emptyEngineResults,
  ): readonly EngineResult[] {
    try {
      const results = applyPendingConstituents(
        this.#pending,
        (chunk, bytes) => this.#lease.append(chunk, bytes),
        {
          onAttempt: () => {
            this.#appendAttempts += 1n;
          },
          onCommitted: (chunk, result) => {
            this.#recordCommittedAppend(chunk.bytes, result);
          },
        },
      );
      this.#unresolved = false;
      return results;
    } catch (error) {
      if (!(error instanceof PendingApplyFailure)) {
        throw error;
      }
      this.#unresolved = true;
      throw new BatchOperationError({
        completedResults: [...earlierResults, ...error.completedResults],
        cause: error.cause,
        operation,
        pending: this.#pending.snapshot(),
        newInputAccepted,
      });
    }
  }

  #appendStandalone(chunk: string, bytes: number): EngineResult {
    this.#appendAttempts += 1n;
    const result = this.#lease.append(chunk, bytes);
    this.#recordCommittedAppend(bytes, result);
    return result;
  }

  #recordCommittedAppend(bytes: number, result: EngineResult): void {
    this.#successfulAppends += 1n;
    this.#committedBytes += BigInt(bytes);
    this.#recordPublishedResult(result);
  }

  #recordPublishedResult(result: EngineResult): void {
    this.#outputPayloadBytes += BigInt(result.outputPayloadBytes);
    this.#publishedResults += 1n;
  }

  #throwBatchOperationFailure(
    completedResults: readonly EngineResult[],
    cause: unknown,
    operation: BatchOperation,
    newInputAccepted: boolean | undefined,
  ): never {
    throw new BatchOperationError({
      completedResults,
      cause,
      operation,
      pending: this.#pending.snapshot(),
      newInputAccepted,
    });
  }

  #assertUsable(): void {
    if (this.#released) {
      throw batchStateError(
        "the mdstream batching lease has been released",
        "bindings.batch_released",
      );
    }
  }

  #assertResolved(): void {
    if (this.#unresolved) {
      throw batchStateError(
        "pending input must be retried, transferred, or discarded first",
        "bindings.batch_unresolved",
      );
    }
  }
}

/** @internal */
export interface BatchCandidateMetrics {
  readonly appendAttempts: DecimalCounter;
  readonly encodedResultBytes: DecimalCounter;
  readonly scanBytes: DecimalCounter;
  readonly joinCopyBytes: DecimalCounter;
  readonly replayCount: DecimalCounter;
}

/** @internal Test-only KTD3 evaluator; not exported from the package entry point. */
export function evaluateBatchCandidateForTest(
  chunks: readonly string[],
  candidate: "joined-first" | "constituent-first",
  operations: {
    readonly append: (chunk: string) => EngineResult;
    readonly finish: () => EngineResult;
  },
): BatchCandidateMetrics {
  const pending = new PendingChunks();
  let scanBytes = 0n;
  for (const chunk of chunks) {
    const bytes = utf8ByteLength(chunk);
    scanBytes += BigInt(bytes);
    pending.accept(chunk, bytes);
  }

  let appendAttempts = 0n;
  let encodedResultBytes = 0n;
  let joinCopyBytes = 0n;
  const record = (result: EngineResult) => {
    encodedResultBytes += BigInt(result.outputPayloadBytes);
  };

  if (candidate === "constituent-first") {
    applyPendingConstituents(pending, (chunk) => operations.append(chunk), {
      onAttempt: () => {
        appendAttempts += 1n;
      },
      onCommitted: (_chunk, result) => record(result),
    });
  } else {
    const joined = pending.joinedForEvaluation();
    if (joined !== undefined) {
      appendAttempts += 1n;
      if (pending.constituents > 1) {
        joinCopyBytes += BigInt(pending.bytes);
      }
      record(operations.append(joined));
      pending.clear();
    }
  }
  record(operations.finish());

  return Object.freeze({
    appendAttempts: counter(appendAttempts),
    encodedResultBytes: counter(encodedResultBytes),
    scanBytes: counter(scanBytes),
    joinCopyBytes: counter(joinCopyBytes),
    replayCount: counter(0n),
  });
}

export function utf8ByteLength(value: string): number {
  const bytes = scanUtf8ByteLength(value);
  if (bytes === undefined) {
    throw new RangeError("input UTF-8 byte length exceeds the JavaScript safe integer range");
  }
  return bytes;
}

function assertRawAppendAdmission(engine: WasmEngineSession, chunk: string): void {
  admittedUtf8ByteLength(engine, chunk);
}

function admittedUtf8ByteLength(
  engine: WasmEngineSession,
  chunk: string,
  observeScan: (bytes: number) => void = () => {},
): number {
  const limit = rawAppendByteCeiling(engine);
  if (chunk.length > limit) {
    observeScan(0);
    throw rawAdmissionError();
  }
  const scan = scanUtf8(chunk, limit);
  observeScan(scan.bytes);
  if (!scan.withinLimit) {
    throw rawAdmissionError();
  }
  return scan.bytes;
}

function assertRawAppendByteLengthAdmission(
  engine: WasmEngineSession,
  admittedBytes: number,
): void {
  if (admittedBytes > rawAppendByteCeiling(engine)) {
    throw rawAdmissionError();
  }
}

function rawAppendByteCeiling(engine: WasmEngineSession): number {
  let ceiling: unknown;
  try {
    ceiling = engine.rawAppendByteCeiling();
  } catch (error) {
    throw MdstreamError.from(error);
  }
  if (
    typeof ceiling !== "number" ||
    !Number.isSafeInteger(ceiling) ||
    ceiling < 0
  ) {
    throw new TypeError(
      "mdstream WASM rawAppendByteCeiling must return a non-negative safe integer",
    );
  }
  return ceiling;
}

function scanUtf8ByteLength(value: string, limit?: number): number | undefined {
  const scan = scanUtf8(value, limit ?? Number.MAX_SAFE_INTEGER);
  return scan.withinLimit ? scan.bytes : undefined;
}

function scanUtf8(
  value: string,
  maxBytes: number,
): { readonly bytes: number; readonly withinLimit: boolean } {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit <= 0x7f) {
      bytes += 1;
    } else if (unit <= 0x7ff) {
      bytes += 2;
    } else if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (index + 1 >= value.length || next < 0xdc00 || next > 0xdfff) {
        throw new TypeError("input contains an unpaired UTF-16 high surrogate");
      }
      bytes += 4;
      index += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      throw new TypeError("input contains an unpaired UTF-16 low surrogate");
    } else {
      bytes += 3;
    }
    if (bytes > maxBytes) {
      return { bytes, withinLimit: false };
    }
  }
  return { bytes, withinLimit: true };
}

function rawAdmissionError(): MdstreamError {
  return new MdstreamError(
    "raw append input exceeds the current native source admission ceiling",
    {
      status: 11,
      statusName: "MDSTREAM_RESOURCE_LIMIT_EXCEEDED",
      detailCode: "bindings.resource_limit",
    },
  );
}

async function createRuntime(loader: WasmModuleLoader): Promise<MdstreamRuntime> {
  try {
    return MdstreamRuntime.fromWasm(await loadWasmBindings(loader));
  } catch (error) {
    throw MdstreamError.from(error);
  }
}

interface PreparedSessionOptions {
  readonly encodedJson: string | undefined;
  readonly captureTransitions: boolean;
}

function prepareSessionOptions(
  options: MdstreamSessionOptions | undefined,
  schema: string,
): PreparedSessionOptions {
  if (options === undefined) {
    return {
      encodedJson: undefined,
      captureTransitions: false,
    };
  }
  const normalized = normalizeOptions(options) as Record<string, unknown>;
  return {
    encodedJson: JSON.stringify({ schema, ...normalized }),
    captureTransitions: normalized.capture_transitions === true,
  };
}

function normalizeBatchOptions(
  options: LosslessBatchOptions,
): LosslessBatchOptions {
  if (options === null || typeof options !== "object") {
    throw new TypeError("lossless batch options must be an object");
  }
  assertPositiveSafeInteger(options.maxBatchBytes, "maxBatchBytes");
  assertPositiveSafeInteger(options.maxPendingChunks, "maxPendingChunks");
  return Object.freeze({
    maxBatchBytes: options.maxBatchBytes,
    maxPendingChunks: options.maxPendingChunks,
  });
}

function assertPositiveSafeInteger(value: number, name: string): void {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`${name} must be a positive safe integer`);
  }
}

function batchStateError(message: string, detailCode: string): MdstreamError {
  return new MdstreamError(message, {
    status: 1,
    statusName: "MDSTREAM_INVALID_ARGUMENT",
    detailCode,
  });
}

function releaseFailedReducer(
  reducer: WasmReducerSession,
  store: RustBackedStore | undefined,
): void {
  try {
    if (store === undefined) {
      reducer.free();
    } else {
      store.close();
    }
  } catch {
    // Preserve the construction failure that triggered cleanup.
  }
}

function normalizeOptions(value: unknown): unknown {
  if (typeof value === "bigint") {
    if (value < 0n) {
      throw new RangeError("mdstream decimal options cannot be negative");
    }
    return value.toString();
  }
  if (typeof value === "number") {
    throw new TypeError("mdstream integer options must use bigint or decimal strings");
  }
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map(normalizeOptions);
  }
  if (typeof value === "object") {
    const normalized: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value)) {
      if (entry !== undefined) {
        normalized[toSnakeCase(key)] = normalizeOptions(entry);
      }
    }
    return normalized;
  }
  throw new TypeError("mdstream options contain an unsupported value");
}

function toSnakeCase(value: string): string {
  return value.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

function counter(value: bigint): DecimalCounter {
  return value.toString() as DecimalCounter;
}
