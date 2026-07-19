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
  TRANSITION_SCHEMA_DRAFT,
  defaultWasmLoader,
  drainOutput,
  loadWasmBindings,
  type WasmBindings,
  type WasmEngineSession,
  type WasmModuleLoader,
  type WasmOutput,
} from "./wasm.js";

export type { WasmModuleLoader } from "./wasm.js";

export type DecimalInput = string | bigint;

export interface ProtocolLimitOptions {
  readonly maxSourceBytes?: DecimalInput;
  readonly maxNodes?: DecimalInput;
  readonly maxResources?: DecimalInput;
  readonly maxDefinitions?: DecimalInput;
  readonly maxDefinitionEdges?: DecimalInput;
  readonly maxOperations?: DecimalInput;
  readonly maxChangeStructuralItems?: DecimalInput;
  readonly maxDocumentStructuralItems?: DecimalInput;
  readonly maxChildrenPerList?: DecimalInput;
  readonly maxAttributesPerNode?: DecimalInput;
  readonly maxMetadataValueBytes?: DecimalInput;
  readonly maxNodeMetadataBytes?: DecimalInput;
  readonly maxChangeMetadataBytes?: DecimalInput;
  readonly maxDocumentMetadataBytes?: DecimalInput;
  readonly maxDefinitionMetadataBytes?: DecimalInput;
  readonly maxTreeDepth?: DecimalInput;
  readonly maxMarkdownEvents?: DecimalInput;
  readonly maxMarkdownOverlapWork?: DecimalInput;
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

export interface BatchMetrics {
  readonly maxBatchBytes: DecimalCounter;
  readonly inputChunks: DecimalCounter;
  readonly inputBytes: DecimalCounter;
  readonly forwardedBytes: DecimalCounter;
  readonly pendingBytes: DecimalCounter;
  readonly joinCopyBytes: DecimalCounter;
  readonly outputPayloadBytes: DecimalCounter;
  readonly batchCount: DecimalCounter;
  readonly wasmAppendCalls: DecimalCounter;
}

export interface BatchedRecoverySnapshot {
  readonly flushed: readonly EngineResult[];
  readonly snapshot: CanonicalSnapshotBytes | undefined;
}

export class BatchOperationError extends Error {
  readonly completedResults: readonly EngineResult[];

  constructor(completedResults: readonly EngineResult[], cause: unknown) {
    super("batch operation failed after committing earlier results", { cause });
    this.name = "BatchOperationError";
    this.completedResults = Object.freeze([...completedResults]);
  }
}

const emptyEngineResults = Object.freeze([]) as readonly EngineResult[];
type DocumentOperationRunner = <Result>(operation: () => Result) => Result;
const documentOperationRunners = new WeakMap<MdstreamEngine, DocumentOperationRunner>();

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
  readonly #wasm: WasmBindings;
  readonly abiVersion: number;
  readonly packageVersion: string;
  readonly bindingSchema: string;
  readonly bindingOptionsSchema: string;
  readonly transitionSchema: string;

  private constructor(wasm: WasmBindings) {
    this.#wasm = wasm;
    this.abiVersion = wasm.abiVersion();
    this.packageVersion = wasm.packageVersion();
    this.bindingSchema = wasm.bindingSchema();
    this.bindingOptionsSchema = wasm.bindingOptionsSchema();
    this.transitionSchema = TRANSITION_SCHEMA_DRAFT;
  }

  /** @internal */
  static fromWasm(wasm: WasmBindings): MdstreamRuntime {
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
      const store = new RustBackedStore(
        new this.#wasm.MdstreamReducerSession(prepared.encodedJson),
        this.bindingSchema,
        prepared.captureTransitions,
      );
      return MdstreamEngine.fromSessions(
        engine,
        store,
        prepared.schedulerLimits,
      );
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
    documentOperationRunners.set(
      this,
      (operation) => this.#rustStore.runDocumentOperation(operation),
    );
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
    return this.#rustStore.runDocumentOperation(() => {
      this.#assertOpen();
      utf8ByteLength(chunk);
      return this.#consume(() => this.#engine.append(chunk));
    });
  }

  finish(): EngineResult {
    return this.#rustStore.runDocumentOperation(() => {
      this.#assertOpen();
      return this.#consume(() => this.#engine.finish());
    });
  }

  reset(): EngineResult {
    return this.#rustStore.runDocumentOperation(() => {
      this.#assertOpen();
      return this.#consume(() => this.#engine.reset());
    });
  }

  createRecoverySnapshot(): CanonicalSnapshotBytes | undefined {
    this.#assertOpen();
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

  registerProcessor(processor: ContentProcessor): ProcessorRegistration {
    return this.#rustStore.runDocumentOperation(() =>
      this.#scheduler.register(processor)
    );
  }

  subscribeProcessorErrors(listener: ProcessorErrorListener): () => void {
    return this.#scheduler.subscribeErrors(listener);
  }

  whenProcessorsIdle(): Promise<void> {
    return this.#scheduler.whenIdle();
  }

  createBatcher(maxBatchBytes: number): LosslessInputBatcher {
    this.#assertOpen();
    return new LosslessInputBatcher(this, maxBatchBytes);
  }

  metrics(): BindingMetricsView {
    this.#assertOpen();
    return readBindingMetrics(this.#engine.metrics());
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#rustStore.assertMutationAllowed();
    this.#closed = true;
    this.#scheduler.close();
    this.#rustStore.setEventSink(undefined);
    this.#engine.free();
    this.#rustStore.close();
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

export class LosslessInputBatcher {
  readonly #engine: MdstreamEngine;
  readonly #maxBatchBytes: number;
  readonly #runOperation: DocumentOperationRunner;
  readonly #chunks: string[] = [];
  #pendingBytes = 0;
  #inputChunks = 0n;
  #inputBytes = 0n;
  #forwardedBytes = 0n;
  #joinCopyBytes = 0n;
  #outputPayloadBytes = 0n;
  #batchCount = 0n;

  constructor(engine: MdstreamEngine, maxBatchBytes: number) {
    if (!Number.isSafeInteger(maxBatchBytes) || maxBatchBytes <= 0) {
      throw new RangeError("maxBatchBytes must be a positive safe integer");
    }
    this.#engine = engine;
    this.#maxBatchBytes = maxBatchBytes;
    const runOperation = documentOperationRunners.get(engine);
    if (runOperation === undefined) {
      throw new TypeError("batchers require an mdstream-created engine");
    }
    this.#runOperation = runOperation;
  }

  push(chunk: string): readonly EngineResult[] {
    return this.#runOperation(() => this.#push(chunk));
  }

  #push(chunk: string): readonly EngineResult[] {
    let results: EngineResult[] | undefined;
    const bytes = utf8ByteLength(chunk);
    this.#inputChunks += 1n;
    this.#inputBytes += BigInt(bytes);
    if (bytes === 0) {
      return emptyEngineResults;
    }
    if (this.#pendingBytes > 0 && this.#pendingBytes + bytes > this.#maxBatchBytes) {
      const flushed = this.flush();
      if (flushed !== undefined) {
        (results ??= []).push(flushed);
      }
    }
    if (bytes > this.#maxBatchBytes) {
      const forwarded = this.#afterCommitted(
        results ?? emptyEngineResults,
        () => this.#forward(chunk, bytes),
      );
      (results ??= []).push(forwarded);
      return Object.freeze(results);
    }
    this.#chunks.push(chunk);
    this.#pendingBytes += bytes;
    if (this.#pendingBytes === this.#maxBatchBytes) {
      const flushed = this.flush();
      if (flushed !== undefined) {
        (results ??= []).push(flushed);
      }
    }
    return results === undefined ? emptyEngineResults : Object.freeze(results);
  }

  flush(): EngineResult | undefined {
    return this.#runOperation(() => this.#flush());
  }

  #flush(): EngineResult | undefined {
    if (this.#chunks.length === 0) {
      return undefined;
    }
    const bytes = this.#pendingBytes;
    const joined =
      this.#chunks.length === 1 ? this.#chunks[0] : this.#chunks.join("");
    if (joined === undefined) {
      return undefined;
    }
    if (this.#chunks.length > 1) {
      this.#joinCopyBytes += BigInt(bytes);
    }
    const result = this.#engine.append(joined);
    this.#chunks.length = 0;
    this.#pendingBytes = 0;
    this.#recordForward(bytes, result);
    return result;
  }

  finish(): readonly EngineResult[] {
    return this.#runOperation(() => {
      const results = this.#flushResults();
      const result = this.#afterCommitted(results, () => this.#engine.finish());
      this.#outputPayloadBytes += BigInt(result.outputPayloadBytes);
      results.push(result);
      return Object.freeze(results);
    });
  }

  reset(): readonly EngineResult[] {
    return this.#runOperation(() => {
      const results = this.#flushResults();
      const result = this.#afterCommitted(results, () => this.#engine.reset());
      this.#outputPayloadBytes += BigInt(result.outputPayloadBytes);
      results.push(result);
      return Object.freeze(results);
    });
  }

  createRecoverySnapshot(): BatchedRecoverySnapshot {
    return this.#runOperation(() => {
      const flushed = this.#flushResults();
      const snapshot = this.#afterCommitted(
        flushed,
        () => this.#engine.createRecoverySnapshot(),
      );
      if (snapshot !== undefined) {
        this.#outputPayloadBytes += BigInt(snapshot.byteLength);
      }
      return Object.freeze({ flushed: Object.freeze(flushed), snapshot });
    });
  }

  metrics(): BatchMetrics {
    return {
      maxBatchBytes: counter(BigInt(this.#maxBatchBytes)),
      inputChunks: counter(this.#inputChunks),
      inputBytes: counter(this.#inputBytes),
      forwardedBytes: counter(this.#forwardedBytes),
      pendingBytes: counter(BigInt(this.#pendingBytes)),
      joinCopyBytes: counter(this.#joinCopyBytes),
      outputPayloadBytes: counter(this.#outputPayloadBytes),
      batchCount: counter(this.#batchCount),
      wasmAppendCalls: counter(this.#batchCount),
    };
  }

  #forward(chunk: string, bytes: number): EngineResult {
    const result = this.#engine.append(chunk);
    this.#recordForward(bytes, result);
    return result;
  }

  #recordForward(bytes: number, result: EngineResult): void {
    this.#forwardedBytes += BigInt(bytes);
    this.#outputPayloadBytes += BigInt(result.outputPayloadBytes);
    this.#batchCount += 1n;
  }

  #flushResults(): EngineResult[] {
    const results: EngineResult[] = [];
    const flushed = this.#flush();
    if (flushed !== undefined) {
      results.push(flushed);
    }
    return results;
  }

  #afterCommitted<Result>(
    completedResults: readonly EngineResult[],
    operation: () => Result,
  ): Result {
    try {
      return operation();
    } catch (error) {
      if (completedResults.length === 0) {
        throw error;
      }
      throw new BatchOperationError(completedResults, error);
    }
  }
}

export function utf8ByteLength(value: string): number {
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
    if (!Number.isSafeInteger(bytes)) {
      throw new RangeError("input UTF-8 byte length exceeds the JavaScript safe integer range");
    }
  }
  return bytes;
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
  readonly schedulerLimits: ProcessorSchedulerLimits;
}

function prepareSessionOptions(
  options: MdstreamSessionOptions | undefined,
  schema: string,
): PreparedSessionOptions {
  if (options === undefined) {
    return {
      encodedJson: undefined,
      captureTransitions: false,
      schedulerLimits: { maxInFlightJobs: 32, maxCandidates: 256 },
    };
  }
  const normalized = normalizeOptions(options) as Record<string, unknown>;
  const processor = recordOrEmpty(normalized.processor);
  return {
    encodedJson: JSON.stringify({ schema, ...normalized }),
    captureTransitions: normalized.capture_transitions === true,
    schedulerLimits: {
      maxInFlightJobs: schedulingLimit(processor.max_in_flight_jobs, 32),
      maxCandidates: schedulingLimit(processor.max_slots, 256),
    },
  };
}

function schedulingLimit(value: unknown, fallback: number): number {
  if (value !== undefined && typeof value !== "string") {
    throw new TypeError("mdstream processor limits must be decimal strings");
  }
  const parsed = value === undefined ? BigInt(fallback) : BigInt(value);
  return parsed > BigInt(Number.MAX_SAFE_INTEGER)
    ? Number.MAX_SAFE_INTEGER
    : Number(parsed);
}

function recordOrEmpty(value: unknown): Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Readonly<Record<string, unknown>>
    : {};
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
