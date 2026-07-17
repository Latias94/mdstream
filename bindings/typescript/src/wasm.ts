export enum BindingPayloadKind {
  Change = 1,
  Snapshot = 2,
  ReducerUpdate = 3,
  NodeView = 4,
  ResourceView = 5,
  ProcessorRequest = 6,
  ProcessorCompletion = 7,
  ArtifactChange = 8,
  ArtifactView = 9,
}

export interface WasmOutput {
  readonly len: number;
  remaining(): number;
  kind(index: number): number;
  count(kind: number): number;
  take(index: number): Uint8Array;
  free(): void;
}

export interface WasmEngineSession {
  append(chunk: string): WasmOutput;
  finish(): WasmOutput;
  reset(): WasmOutput;
  snapshot(): WasmOutput;
  metrics(): Uint8Array;
  free(): void;
}

export interface WasmReducerSession {
  applyChange(change: Uint8Array): WasmOutput;
  recoverSnapshot(snapshot: Uint8Array): WasmOutput;
  snapshot(): WasmOutput;
  nodeView(nodeId: string): WasmOutput;
  resourceView(resourceId: string): WasmOutput;
  beginProcessor(
    nodeId: string,
    processorId: string,
    processorVersion: string,
    configurationVersion: string,
    acceptsProvisional: boolean,
    allowProvisional: boolean,
  ): WasmOutput;
  artifactView(epoch: string, nodeId: string, processorId: string): WasmOutput;
  completeProcessorText(
    requestId: string,
    protocol: string,
    mediaType: string,
    text: string,
  ): WasmOutput;
  completeProcessorBinary(
    requestId: string,
    protocol: string,
    mediaType: string,
    bytes: Uint8Array,
  ): WasmOutput;
  failProcessor(requestId: string, code: string, message: string): WasmOutput;
  cancelProcessor(requestId: string): WasmOutput;
  status(): string;
  metrics(): Uint8Array;
  processorMetrics(): Uint8Array;
  free(): void;
}

export interface WasmBindings {
  readonly MdstreamEngineSession: new (optionsJson?: string) => WasmEngineSession;
  readonly MdstreamReducerSession: new (optionsJson?: string) => WasmReducerSession;
  abiVersion(): number;
  packageVersion(): string;
  bindingSchema(): string;
  bindingOptionsSchema(): string;
  default?: (input?: unknown) => Promise<unknown>;
}

export type WasmModuleLoader = () => unknown | Promise<unknown>;

export interface TransportPayload {
  readonly kind: BindingPayloadKind;
  readonly bytes: Uint8Array;
}

export interface DrainedOutput {
  readonly payloads: readonly TransportPayload[];
  readonly payloadBytes: bigint;
}

export function drainOutput(output: WasmOutput): DrainedOutput {
  const payloads: TransportPayload[] = [];
  let payloadBytes = 0n;
  try {
    for (let index = 0; index < output.len; index += 1) {
      const kind = output.kind(index);
      if (!Number.isInteger(kind) || kind < 1 || kind > 9) {
        throw new TypeError(`WASM returned unknown binding payload kind ${kind}`);
      }
      const bytes = output.take(index);
      if (!(bytes instanceof Uint8Array)) {
        throw new TypeError("WASM binding payload is not an owned Uint8Array");
      }
      payloadBytes += BigInt(bytes.byteLength);
      payloads.push({ kind: kind as BindingPayloadKind, bytes });
    }
  } finally {
    output.free();
  }
  return { payloads, payloadBytes };
}

export async function loadWasmBindings(loader: WasmModuleLoader): Promise<WasmBindings> {
  const loaded = await loader();
  const namespace = asModuleRecord(loaded);
  const candidate = hasBindings(namespace)
    ? namespace
    : asModuleRecord(namespace.default);

  if (typeof candidate.default === "function") {
    await candidate.default();
  }
  if (!hasBindings(candidate)) {
    throw new TypeError("WASM loader did not return the mdstream binding module");
  }
  if (candidate.abiVersion() !== 1) {
    throw new TypeError(`unsupported mdstream WASM ABI ${candidate.abiVersion()}`);
  }
  return candidate;
}

export const defaultWasmLoader: WasmModuleLoader = async () => {
  const url = new URL("../wasm/mdstream_wasm.js", import.meta.url);
  const loaded = await import(url.href) as Record<string, unknown>;
  const initialize = loaded.default;
  if (typeof initialize !== "function") {
    throw new TypeError("packaged mdstream WASM module has no initializer");
  }
  if (isNodeRuntime()) {
    const fileSystemSpecifier = "node:fs/promises";
    const fileSystem = await import(fileSystemSpecifier) as {
      readFile(path: URL): Promise<Uint8Array>;
    };
    const wasmUrl = new URL("../wasm/mdstream_wasm_bg.wasm", import.meta.url);
    await initialize({ module_or_path: await fileSystem.readFile(wasmUrl) });
  } else {
    await initialize();
  }
  return { ...loaded, default: undefined };
};

function asModuleRecord(value: unknown): Record<string, unknown> {
  if ((typeof value !== "object" && typeof value !== "function") || value === null) {
    return {};
  }
  return value as Record<string, unknown>;
}

function hasBindings(
  value: Record<string, unknown>,
): value is Record<string, unknown> & WasmBindings {
  return (
    typeof value.MdstreamEngineSession === "function" &&
    typeof value.MdstreamReducerSession === "function" &&
    typeof value.abiVersion === "function" &&
    typeof value.bindingSchema === "function" &&
    typeof value.bindingOptionsSchema === "function"
  );
}

function isNodeRuntime(): boolean {
  const host = globalThis as typeof globalThis & {
    readonly process?: { readonly versions?: { readonly node?: string } };
  };
  return typeof host.process?.versions?.node === "string";
}
