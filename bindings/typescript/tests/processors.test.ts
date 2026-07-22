import { describe, expect, it, vi } from "vitest";

import {
  initMdstream,
  type ContentNodeView,
  type MdstreamSessionOptions,
  type ProcessorErrorEvent,
  type ProcessorRegistration,
  type ProcessorOutput,
  type ProcessorRequestView,
} from "../src/index.js";
import { ProcessorScheduler } from "../src/processors.js";
import {
  RustBackedStore,
  type BeginProcessorOptions,
  type ReducerResult,
} from "../src/store.js";
import type {
  Epoch,
  NodeId,
  NodeView,
  ProcessorInputVersion,
} from "../src/views.js";
import { nodeWasmLoader, textDecoder } from "./helpers.js";

const capturedProcessorOptions = {
  captureTransitions: true,
  protocol: {
    maxSourceBytes: "1048576",
    maxNodes: "4096",
    maxResources: "256",
    maxOperations: "4096",
    maxChangeStructuralItems: "4096",
    maxChildrenPerList: "4096",
  },
} satisfies MdstreamSessionOptions;

describe("host-side processor scheduling", () => {
  it("rematches a topology-only input change after rejecting a stale candidate", async () => {
    const epoch = "1" as Epoch;
    const nodeId = "7" as NodeId;
    const childA = "8" as NodeId;
    const childB = "9" as NodeId;
    const nodeVersion = "node:same" as ContentNodeView["version"];
    const inputA = "input:A" as ProcessorInputVersion;
    const inputB = "input:B" as ProcessorInputVersion;
    const view = (inputVersion: ProcessorInputVersion, child: NodeId): NodeView => ({
      schema: "mdstream.bindings/0.4",
      kind: "node_view",
      node: {
        id: nodeId,
        version: nodeVersion,
        stability: "stable",
        source: { start: "0", end: "0" } as ContentNodeView["source"],
        body: { start: "0", end: "0" } as ContentNodeView["body"],
        children: {
          version: `children:${child}` as ContentNodeView["children"]["version"],
          children: [child],
        },
        content: { kind: "paragraph" },
      },
      bodyText: "",
      processorInputVersion: inputVersion,
    });
    const initialView = view(inputA, childA);
    const replacementView = view(inputB, childB);
    let currentView = initialView;
    const conditionalBeginInputs: ProcessorInputVersion[] = [];
    const issuedRequestInputs: ProcessorInputVersion[] = [];
    const emptyResult = {
      updates: [],
      processorRequests: [],
      processorCompletions: [],
      artifactChanges: [],
      outputPayloadBytes: "0",
    } as unknown as ReducerResult;
    const store = {
      getSnapshot: () => ({
        document: {
          coordinate: { epoch },
          roots: { children: [nodeId] },
        },
      }),
      getNodeSnapshot: (requestedNodeId: NodeId) =>
        requestedNodeId === nodeId ? currentView : undefined,
      beginProcessor(options: BeginProcessorOptions): ReducerResult {
        conditionalBeginInputs.push(options.expectedInputVersion);
        if (options.expectedInputVersion !== currentView.processorInputVersion) {
          return emptyResult;
        }
        issuedRequestInputs.push(currentView.processorInputVersion);
        return {
          ...emptyResult,
          processorRequests: [{
            schema: "mdstream.bindings/0.4",
            kind: "processor_request",
            requestId: "1",
            key: {
              epoch,
              nodeId,
              processorId: options.processorId,
              nodeVersion,
              inputVersion: currentView.processorInputVersion,
              processorVersion: options.processorVersion,
              configurationVersion: options.configurationVersion,
              generation: "1",
            },
            input: { node: currentView.node, body: "", resource: null },
          }],
        } as unknown as ReducerResult;
      },
      completeProcessorText: () => emptyResult,
      cancelProcessor: () => emptyResult,
      runDocumentOperation: <Result>(operation: () => Result): Result => operation(),
    } as unknown as RustBackedStore;
    const scheduler = new ProcessorScheduler(store, {
      maxInFlightJobs: 8,
      maxCandidates: 8,
    });
    const processedInputs: ProcessorInputVersion[] = [];
    const matchedViews: Array<{
      readonly nodeVersion: ContentNodeView["version"];
      readonly childrenVersion: ContentNodeView["children"]["version"];
      readonly children: readonly NodeId[];
      readonly inputVersion: ProcessorInputVersion;
    }> = [];
    scheduler.register({
      descriptor: { id: "test.ts.input-freshness", version: "v1" },
      configurationVersion: "default",
      matches(node) {
        matchedViews.push({
          nodeVersion: node.version,
          childrenVersion: node.children.version,
          children: node.children.children,
          inputVersion: currentView.processorInputVersion,
        });
        if (matchedViews.length === 1) {
          currentView = replacementView;
          scheduler.handleStoreEvents({
            updates: [{
              outcome: { kind: "applied" },
              impact: {
                fullReplace: false,
                changedNodeIds: [nodeId],
                removedNodeIds: [],
              },
            }],
            artifactChanges: [],
          } as never);
        }
        return true;
      },
      process(request) {
        processedInputs.push(request.key.inputVersion);
        return {
          kind: "text",
          protocol: "test.ts.input-freshness/1",
          mediaType: "text/plain",
          text: "current",
        };
      },
    });

    await scheduler.whenIdle();

    expect(replacementView.node).toEqual({
      ...initialView.node,
      children: replacementView.node.children,
    });
    expect(replacementView.processorInputVersion).not.toBe(
      initialView.processorInputVersion,
    );
    expect(matchedViews).toEqual([
      {
        nodeVersion,
        childrenVersion: initialView.node.children.version,
        children: [childA],
        inputVersion: inputA,
      },
      {
        nodeVersion,
        childrenVersion: replacementView.node.children.version,
        children: [childB],
        inputVersion: inputB,
      },
    ]);
    expect(conditionalBeginInputs).toEqual([inputA, inputB]);
    expect(issuedRequestInputs).toEqual([inputB]);
    expect(processedInputs).toEqual([inputB]);
    scheduler.close();
  });

  it("scans the current tree once when a processor is registered after content exists", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const requests: ProcessorRequestView[] = [];

    engine.append("- existing processor input");
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.late-registration", version: "v1" },
      configurationVersion: "test.ts.late-registration.default",
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        requests.push(request);
        return {
          kind: "text",
          protocol: "test.ts.late-registration/1",
          mediaType: "text/plain",
          text: "registered late",
        };
      },
    });

    await engine.whenProcessorsIdle();

    expect(requests).toHaveLength(1);
    expect(engine.store.processorMetrics().issuedRequests).toBe("1");
    const request = requests[0]!;
    expect(engine.store.getArtifactSnapshot({
      epoch: request.key.epoch,
      nodeId: request.key.nodeId,
      processorId: request.key.processorId,
    })).toMatchObject({
      state: "ready",
      artifact: { payload: { kind: "text", text: "registered late" } },
    });
    engine.close();
  });

  it("snapshots processor identity and configuration at registration", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const descriptor = {
      id: "test.ts.stable-identity",
      version: "v1",
      acceptsProvisional: false,
    };
    let configurationVersion = "config.v1";
    let descriptorReads = 0;
    let configurationReads = 0;
    let allowProvisionalReads = 0;
    let request: ProcessorRequestView | undefined;

    engine.append("stable identity input");
    engine.finish();
    const registration = engine.registerProcessor({
      get descriptor() {
        descriptorReads += 1;
        return descriptor;
      },
      get configurationVersion() {
        configurationReads += 1;
        return configurationVersion;
      },
      get allowProvisional() {
        allowProvisionalReads += 1;
        return false;
      },
      matches: (node) => node.content.kind === "paragraph",
      process(value) {
        request = value;
        return {
          kind: "text",
          protocol: "test.ts.stable-identity/1",
          mediaType: "text/plain",
          text: "stable identity",
        };
      },
    });
    descriptor.id = "test.ts.mutated-identity";
    descriptor.version = "v2";
    descriptor.acceptsProvisional = true;
    configurationVersion = "config.v2";
    expect(() => engine.registerProcessor({
      descriptor: { id: "test.ts.stable-identity", version: "duplicate" },
      configurationVersion: "duplicate-config",
      matches: () => false,
      process: () => ({
        kind: "failure",
        code: "unsupported_content",
        message: "not reached",
      }),
    })).toThrow("processor test.ts.stable-identity is already registered");

    await engine.whenProcessorsIdle();

    expect(request?.key).toMatchObject({
      processorId: "test.ts.stable-identity",
      processorVersion: "v1",
      configurationVersion: "config.v1",
    });
    expect(descriptorReads).toBe(1);
    expect(configurationReads).toBe(1);
    expect(allowProvisionalReads).toBe(1);

    registration.dispose();
    const replacement = engine.registerProcessor({
      descriptor: { id: "test.ts.stable-identity", version: "v1" },
      configurationVersion: "config.v1",
      matches: () => false,
      process: () => ({
        kind: "failure",
        code: "unsupported_content",
        message: "not reached",
      }),
    });
    replacement.dispose();
    await engine.whenProcessorsIdle();
    engine.close();
  });

  it("rejects invalid processor identities before scanning nodes", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const beginProcessor = vi.spyOn(RustBackedStore.prototype, "beginProcessor");
    engine.append("identity validation input");
    engine.finish();
    const invalidValues = ["", "bad/id", "non-ascii-é", "x".repeat(129)];
    try {
      for (const [field, invalid] of [
        ["id", invalidValues],
        ["version", invalidValues],
        ["configuration", invalidValues],
      ] as const) {
        for (const value of invalid) {
          expect(() => engine.registerProcessor({
            descriptor: {
              id: field === "id" ? value : "test.ts.valid-id",
              version: field === "version" ? value : "v1",
            },
            configurationVersion: field === "configuration" ? value : "default",
            matches: () => true,
            process: () => ({
              kind: "text",
              protocol: "test.ts.identity-validation/1",
              mediaType: "text/plain",
              text: "unreachable",
            }),
          })).toThrow(TypeError);
        }
      }

      await engine.whenProcessorsIdle();
      expect(beginProcessor).not.toHaveBeenCalled();
    } finally {
      beginProcessor.mockRestore();
      engine.close();
    }
  });

  it("does not execute a processor unregistered by its pending notification", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const process = vi.fn<() => ProcessorOutput>(() => ({
      kind: "text",
      protocol: "test.ts.pending-dispose/1",
      mediaType: "text/plain",
      text: "must not be installed",
    }));
    let registration: ProcessorRegistration;

    registration = engine.registerProcessor({
      descriptor: { id: "test.ts.pending-dispose", version: "v1" },
      configurationVersion: "test.ts.pending-dispose.default",
      matches: (node) => node.content.kind === "paragraph",
      process,
    });
    engine.append("pending disposal input");
    engine.finish();
    const document = engine.store.getSnapshot().document!;
    const slot = {
      epoch: document.coordinate.epoch,
      nodeId: document.roots!.children[0]!,
      processorId: "test.ts.pending-dispose",
    };
    const unsubscribe = engine.store.subscribeArtifact(slot, () => {
      if (engine.store.getArtifactSnapshot(slot)?.state === "pending") {
        registration.dispose();
      }
    });

    await engine.whenProcessorsIdle();

    expect(process).not.toHaveBeenCalled();
    expect(engine.store.getArtifactSnapshot(slot)).toBeUndefined();
    expect(engine.store.metrics().pendingProcessorRequests).toBe("0");
    expect(engine.store.processorMetrics().issuedRequests).toBe("1");
    unsubscribe();
    engine.close();
  });

  it("does not execute a request invalidated by reset during its pending notification", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedProcessorOptions);
    const process = vi.fn<() => ProcessorOutput>(() => ({
      kind: "text",
      protocol: "test.ts.pending-reset/1",
      mediaType: "text/plain",
      text: "must not be installed",
    }));

    engine.registerProcessor({
      descriptor: { id: "test.ts.pending-reset", version: "v1" },
      configurationVersion: "test.ts.pending-reset.default",
      matches: (node) => node.content.kind === "paragraph",
      process,
    });
    engine.append("pending reset input");
    engine.finish();
    const document = engine.store.getSnapshot().document!;
    const slot = {
      epoch: document.coordinate.epoch,
      nodeId: document.roots!.children[0]!,
      processorId: "test.ts.pending-reset",
    };
    let reset = false;
    const unsubscribe = engine.store.subscribeArtifact(slot, () => {
      if (!reset && engine.store.getArtifactSnapshot(slot)?.state === "pending") {
        reset = true;
        engine.reset();
      }
    });

    await engine.whenProcessorsIdle();

    expect(reset).toBe(true);
    expect(process).not.toHaveBeenCalled();
    expect(engine.store.getArtifactSnapshot(slot)).toBeUndefined();
    expect(engine.store.metrics().pendingProcessorRequests).toBe("0");
    unsubscribe();
    engine.close();
  });

  it("does not submit a disposed generation after the same processor id is registered again", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    let resolveOld: ((output: ProcessorOutput) => void) | undefined;
    let oldStartedResolve: ((request: ProcessorRequestView) => void) | undefined;
    const oldStarted = new Promise<ProcessorRequestView>((resolve) => {
      oldStartedResolve = resolve;
    });
    const oldRegistration = engine.registerProcessor({
      descriptor: { id: "test.ts.aba", version: "old" },
      configurationVersion: "old-config",
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        oldStartedResolve?.(request);
        return new Promise<ProcessorOutput>((resolve) => {
          resolveOld = resolve;
        });
      },
    });
    engine.append("old generation");
    engine.finish();
    const oldRequest = await oldStarted;

    oldRegistration.dispose();
    let newRequest: ProcessorRequestView | undefined;
    engine.registerProcessor({
      descriptor: { id: "test.ts.aba", version: "new" },
      configurationVersion: "new-config",
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        newRequest = request;
        return {
          kind: "text",
          protocol: "test.ts.aba/1",
          mediaType: "text/plain",
          text: "current generation",
        };
      },
    });
    engine.reset();
    engine.append("new generation");
    engine.finish();
    await vi.waitFor(() => expect(newRequest).toBeDefined());
    const commandsBeforeLateResult = engine.store.metrics().commands;

    resolveOld?.({
      kind: "text",
      protocol: "test.ts.aba/1",
      mediaType: "text/plain",
      text: "stale generation",
    });
    await engine.whenProcessorsIdle();

    expect(newRequest!.requestId).not.toBe(oldRequest.requestId);
    expect(engine.store.metrics().commands).toBe(commandsBeforeLateResult);
    expect(engine.store.getArtifactSnapshot({
      epoch: newRequest!.key.epoch,
      nodeId: newRequest!.key.nodeId,
      processorId: "test.ts.aba",
    })?.artifact?.payload).toEqual({
      kind: "text",
      text: "current generation",
    });
    engine.close();
  });

  it("does not submit a superseded node generation after its replacement completes", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const invocations: {
      request: ProcessorRequestView;
      signal: AbortSignal;
      resolve: (output: ProcessorOutput) => void;
    }[] = [];

    engine.registerProcessor({
      descriptor: {
        id: "test.ts.node-generation",
        version: "v1",
        acceptsProvisional: true,
      },
      configurationVersion: "test.ts.node-generation.default",
      allowProvisional: true,
      matches: (node) => node.content.kind === "paragraph",
      process(request, context) {
        return new Promise<ProcessorOutput>((resolve) => {
          invocations.push({ request, signal: context.signal, resolve });
        });
      },
    });
    engine.append("A");
    await vi.waitFor(() => expect(invocations).toHaveLength(1));
    const first = invocations[0]!;

    engine.append("B");
    await vi.waitFor(() => expect(invocations).toHaveLength(2));
    const second = invocations[1]!;
    expect(first.signal.aborted).toBe(true);
    expect(second.request.requestId).not.toBe(first.request.requestId);

    second.resolve({
      kind: "text",
      protocol: "test.ts.node-generation/1",
      mediaType: "text/plain",
      text: "current generation",
    });
    const slot = {
      epoch: second.request.key.epoch,
      nodeId: second.request.key.nodeId,
      processorId: "test.ts.node-generation",
    };
    await vi.waitFor(() => {
      expect(engine.store.getArtifactSnapshot(slot)?.artifact?.payload).toEqual({
        kind: "text",
        text: "current generation",
      });
    });
    const commandsBeforeLateResult = engine.store.metrics().commands;

    first.resolve({
      kind: "text",
      protocol: "test.ts.node-generation/1",
      mediaType: "text/plain",
      text: "stale generation",
    });
    await engine.whenProcessorsIdle();

    expect(engine.store.metrics().commands).toBe(commandsBeforeLateResult);
    expect(engine.store.getArtifactSnapshot(slot)?.artifact?.payload).toEqual({
      kind: "text",
      text: "current generation",
    });
    engine.close();
  });

  it("rechecks a same-epoch candidate changed synchronously by matches", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const requests: ProcessorRequestView[] = [];
    const errors = vi.fn();
    let reentered = false;
    engine.subscribeProcessorErrors(errors);

    engine.registerProcessor({
      descriptor: {
        id: "test.ts.reentrant-version",
        version: "v1",
        acceptsProvisional: true,
      },
      configurationVersion: "test.ts.reentrant-version.default",
      allowProvisional: true,
      matches(node) {
        if (node.content.kind !== "paragraph") {
          return false;
        }
        const matchedEnd = node.source.end;
        if (!reentered) {
          reentered = true;
          engine.append("b");
        }
        return matchedEnd === "1";
      },
      process(request) {
        requests.push(request);
        return {
          kind: "text",
          protocol: "test.ts.reentrant-version/1",
          mediaType: "text/plain",
          text: "must not be installed",
        };
      },
    });

    engine.append("a");
    await engine.whenProcessorsIdle();

    expect(reentered).toBe(true);
    expect(engine.store.getSnapshot().document?.coordinate.sourceCursor).toBe("2");
    expect(requests).toHaveLength(0);
    expect(engine.store.processorMetrics().issuedRequests).toBe("0");
    expect(errors).not.toHaveBeenCalled();
    engine.close();
  });

  it("rechecks an old-epoch candidate after matches resets the engine", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedProcessorOptions);
    const requests: ProcessorRequestView[] = [];
    const errors = vi.fn();
    let reentered = false;
    engine.subscribeProcessorErrors(errors);

    engine.registerProcessor({
      descriptor: {
        id: "test.ts.reentrant-reset",
        version: "v1",
        acceptsProvisional: true,
      },
      configurationVersion: "test.ts.reentrant-reset.default",
      allowProvisional: true,
      matches(node) {
        if (node.content.kind !== "paragraph") {
          return false;
        }
        if (!reentered) {
          reentered = true;
          engine.reset();
          engine.append("# replacement\n");
        }
        return true;
      },
      process(request) {
        requests.push(request);
        return {
          kind: "text",
          protocol: "test.ts.reentrant-reset/1",
          mediaType: "text/plain",
          text: "must not be installed",
        };
      },
    });

    engine.append("old paragraph");
    await engine.whenProcessorsIdle();

    expect(reentered).toBe(true);
    expect(engine.store.getSnapshot().document?.coordinate.epoch).toBe("2");
    expect(requests).toHaveLength(0);
    expect(engine.store.processorMetrics().issuedRequests).toBe("0");
    expect(errors).not.toHaveBeenCalled();
    engine.close();
  });

  it("queues matching candidates until native dispatch credit is available", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: { maxInFlightJobs: 2n },
    });
    const requests: ProcessorRequestView[] = [];
    const blocked: Array<(output: ProcessorOutput) => void> = [];
    const errors = vi.fn();
    let releaseAll = false;
    let active = 0;
    let peakActive = 0;
    engine.subscribeProcessorErrors(errors);

    engine.append("one\n\ntwo\n\nthree\n\nfour\n\nfive\n");
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.dispatch-credit", version: "v1" },
      configurationVersion: "test.ts.dispatch-credit.default",
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        requests.push(request);
        active += 1;
        peakActive = Math.max(peakActive, active);
        const output: ProcessorOutput = {
          kind: "text",
          protocol: "test.ts.dispatch-credit/1",
          mediaType: "text/plain",
          text: request.input.body,
        };
        if (releaseAll) {
          active -= 1;
          return output;
        }
        return new Promise<ProcessorOutput>((resolve) => {
          blocked.push((value) => {
            active -= 1;
            resolve(value);
          });
        });
      },
    });

    await vi.waitFor(() => expect(requests).toHaveLength(2));
    expect(engine.store.processorMetrics().inFlightJobs).toBe("2");
    releaseAll = true;
    for (const resolve of blocked.splice(0)) {
      resolve({
        kind: "text",
        protocol: "test.ts.dispatch-credit/1",
        mediaType: "text/plain",
        text: "released",
      });
    }
    await engine.whenProcessorsIdle();

    expect(requests).toHaveLength(5);
    expect(peakActive).toBeLessThanOrEqual(2);
    expect(engine.store.processorMetrics().issuedRequests).toBe("5");
    expect(engine.store.processorMetrics().inFlightJobs).toBe("0");
    expect(errors).not.toHaveBeenCalled();
    for (const request of requests) {
      expect(engine.store.getArtifactSnapshot({
        epoch: request.key.epoch,
        nodeId: request.key.nodeId,
        processorId: request.key.processorId,
      })?.state).toBe("ready");
    }
    engine.close();
  });

  it("continues past a permanent resource limit while another job is active", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: {
        maxInputBytes: 1024n,
        maxInFlightJobs: 2n,
      },
    });
    const requests: ProcessorRequestView[] = [];
    const errors: ProcessorErrorEvent[] = [];
    let releaseFirst: (() => void) | undefined;
    engine.subscribeProcessorErrors((event) => errors.push(event));

    engine.append(`first\n\n${"x".repeat(4000)}\n\nthird`);
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.permanent-limit", version: "v1" },
      configurationVersion: "test.ts.permanent-limit.default",
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        requests.push(request);
        const output: ProcessorOutput = {
          kind: "text",
          protocol: "test.ts.permanent-limit/1",
          mediaType: "text/plain",
          text: request.input.body,
        };
        if (request.input.body !== "first") {
          return output;
        }
        return new Promise<ProcessorOutput>((resolve) => {
          releaseFirst = () => resolve(output);
        });
      },
    });

    await vi.waitFor(() => {
      expect(requests.map((request) => request.input.body)).toEqual([
        "first",
        "third",
      ]);
    });
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatchObject({
      phase: "begin",
      error: { detailCode: "processor.resource_limit.input_bytes" },
    });

    releaseFirst?.();
    await engine.whenProcessorsIdle();

    expect(engine.store.processorMetrics().issuedRequests).toBe("2");
    engine.close();
  });

  it("crosses a task boundary between dispatch quanta", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: {
        maxInFlightJobs: 128n,
        maxSlots: 128n,
      },
    });
    const process = vi.fn<() => ProcessorOutput>(() => ({
      kind: "text",
      protocol: "test.ts.dispatch-quantum/1",
      mediaType: "text/plain",
      text: "done",
    }));
    engine.append(
      Array.from({ length: 96 }, (_, index) => `paragraph ${index}`).join("\n\n"),
    );
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.dispatch-quantum", version: "v1" },
      configurationVersion: "test.ts.dispatch-quantum.default",
      matches: (node) => node.content.kind === "paragraph",
      process,
    });
    const taskMarker = new Promise<{
      readonly issuedRequests: string;
      readonly processCalls: number;
    }>((resolve) => {
      setTimeout(() => {
        resolve({
          issuedRequests: engine.store.processorMetrics().issuedRequests,
          processCalls: process.mock.calls.length,
        });
      }, 0);
    });

    await Promise.resolve();

    expect(engine.store.processorMetrics().issuedRequests).toBe("32");
    expect(process).not.toHaveBeenCalled();

    await expect(taskMarker).resolves.toEqual({
      issuedRequests: "32",
      processCalls: 32,
    });

    await engine.whenProcessorsIdle();

    expect(process).toHaveBeenCalledTimes(96);
    expect(engine.store.processorMetrics().issuedRequests).toBe("96");
    engine.close();
  });

  it("lets timer work run before a large tree scan completes", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const candidateCount = 256;
    const engine = runtime.createEngine({
      processor: {
        maxInFlightJobs: BigInt(candidateCount),
        maxSlots: BigInt(candidateCount),
      },
    });
    const matches = vi.fn(
      (node: ContentNodeView) => node.content.kind === "paragraph",
    );
    const process = vi.fn<() => ProcessorOutput>(() => ({
      kind: "text",
      protocol: "test.ts.scan-quantum/1",
      mediaType: "text/plain",
      text: "done",
    }));
    engine.append(
      Array.from(
        { length: candidateCount },
        (_, index) => `paragraph ${index}`,
      ).join("\n\n"),
    );
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.scan-quantum", version: "v1" },
      configurationVersion: "test.ts.scan-quantum.default",
      matches,
      process,
    });
    const matchesAtTimer = await new Promise<number>((resolve) => {
      setTimeout(() => resolve(matches.mock.calls.length), 0);
    });

    await engine.whenProcessorsIdle();

    expect(matches.mock.calls.length).toBeGreaterThan(matchesAtTimer);
    expect(process).toHaveBeenCalledTimes(candidateCount);
    engine.close();
  });

  it("counts each processor match against the scan quantum", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const processorCount = 256;
    const matches = vi.fn(() => false);
    engine.append("one node");
    engine.finish();
    for (let index = 0; index < processorCount; index += 1) {
      engine.registerProcessor({
        descriptor: { id: `test.ts.match-quantum.${index}`, version: "v1" },
        configurationVersion: "test.ts.match-quantum.default",
        matches,
        process: () => ({
          kind: "text",
          protocol: "test.ts.match-quantum/1",
          mediaType: "text/plain",
          text: "unreachable",
        }),
      });
    }

    const matchesAtTimer = await new Promise<number>((resolve) => {
      setTimeout(() => resolve(matches.mock.calls.length), 0);
    });
    await engine.whenProcessorsIdle();

    expect(matchesAtTimer).toBeGreaterThan(0);
    expect(matchesAtTimer).toBeLessThan(processorCount);
    expect(matches.mock.calls.length).toBeGreaterThanOrEqual(processorCount);
    engine.close();
  });

  it("stops a large tree scan after its last processor is unregistered", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const candidateCount = 256;
    const engine = runtime.createEngine({
      processor: { maxSlots: BigInt(candidateCount) },
    });
    const matches = vi.fn(() => false);
    engine.append(
      Array.from(
        { length: candidateCount },
        (_, index) => `paragraph ${index}`,
      ).join("\n\n"),
    );
    engine.finish();
    const registration = engine.registerProcessor({
      descriptor: { id: "test.ts.scan-dispose", version: "v1" },
      configurationVersion: "test.ts.scan-dispose.default",
      matches,
      process: () => ({
        kind: "text",
        protocol: "test.ts.scan-dispose/1",
        mediaType: "text/plain",
        text: "unreachable",
      }),
    });
    const callsAtDispose = await new Promise<number>((resolve) => {
      setTimeout(() => {
        const calls = matches.mock.calls.length;
        registration.dispose();
        resolve(calls);
      }, 0);
    });

    await engine.whenProcessorsIdle();
    await new Promise<void>((resolve) => setTimeout(resolve, 0));

    expect(callsAtDispose).toBeGreaterThan(0);
    expect(callsAtDispose).toBeLessThan(candidateCount);
    expect(matches).toHaveBeenCalledTimes(callsAtDispose);
    engine.close();
  });

  it("removes one unregistered processor from an active shared scan", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({ processor: { maxSlots: 256n } });
    const firstMatches = vi.fn(() => false);
    const secondMatches = vi.fn(() => false);
    engine.append(
      Array.from({ length: 128 }, (_, index) => `paragraph ${index}`).join("\n\n"),
    );
    engine.finish();
    const first = engine.registerProcessor({
      descriptor: { id: "test.ts.shared-dispose.first", version: "v1" },
      configurationVersion: "test.ts.shared-dispose.default",
      matches: firstMatches,
      process: () => ({
        kind: "text",
        protocol: "test.ts.shared-dispose/1",
        mediaType: "text/plain",
        text: "unreachable",
      }),
    });
    engine.registerProcessor({
      descriptor: { id: "test.ts.shared-dispose.second", version: "v1" },
      configurationVersion: "test.ts.shared-dispose.default",
      matches: secondMatches,
      process: () => ({
        kind: "text",
        protocol: "test.ts.shared-dispose/1",
        mediaType: "text/plain",
        text: "unreachable",
      }),
    });
    const callsAtDispose = await new Promise<number>((resolve) => {
      setTimeout(() => {
        const calls = firstMatches.mock.calls.length;
        first.dispose();
        resolve(calls);
      }, 0);
    });

    await engine.whenProcessorsIdle();

    expect(callsAtDispose).toBeGreaterThan(0);
    expect(firstMatches).toHaveBeenCalledTimes(callsAtDispose);
    expect(secondMatches.mock.calls.length).toBeGreaterThan(callsAtDispose);
    engine.close();
  });

  it("retries an input-budget block only after capacity changes", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const candidateCount = 130;
    const engine = runtime.createEngine({
      processor: {
        maxInputBytes: 2048n,
        maxInFlightJobs: 2n,
        maxInFlightInputBytes: 1536n,
        maxSlots: BigInt(candidateCount),
      },
    });
    const beginProcessor = vi.spyOn(RustBackedStore.prototype, "beginProcessor");
    const requests: ProcessorRequestView[] = [];
    let releaseFirst: (() => void) | undefined;
    const bodies = Array.from({ length: candidateCount }, (_, index) => {
      const prefix = `${index.toString().padStart(3, "0")}:`;
      return prefix + "x".repeat(512 - prefix.length);
    });
    engine.append(bodies.join("\n\n"));
    engine.finish();
    try {
      engine.registerProcessor({
        descriptor: { id: "test.ts.input-credit", version: "v1" },
        configurationVersion: "test.ts.input-credit.default",
        matches: (node) => node.content.kind === "paragraph",
        process(request) {
          requests.push(request);
          const output: ProcessorOutput = {
            kind: "text",
            protocol: "test.ts.input-credit/1",
            mediaType: "text/plain",
            text: request.input.body,
          };
          if (requests.length !== 1) {
            return output;
          }
          return new Promise<ProcessorOutput>((resolve) => {
            releaseFirst = () => resolve(output);
          });
        },
      });

      await vi.waitFor(() => expect(releaseFirst).toBeDefined());
      await new Promise<void>((resolve) => setTimeout(resolve, 10));

      expect(beginProcessor).toHaveBeenCalledTimes(2);
      expect(requests).toHaveLength(1);

      releaseFirst?.();
      await engine.whenProcessorsIdle();

      expect(requests).toHaveLength(candidateCount);
      expect(requests.map((request) => request.input.body)).toEqual(bodies);
    } finally {
      releaseFirst?.();
      beginProcessor.mockRestore();
      engine.close();
    }
  });

  it("drops a blocked retry when reset replaces the document", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: {
        maxInputBytes: 2048n,
        maxInFlightJobs: 2n,
        maxInFlightInputBytes: 1536n,
        maxSlots: 3n,
      },
    });
    const oldBodies = ["a".repeat(512), "b".repeat(512)];
    const replacement = "replacement";
    const processedBodies: string[] = [];
    let releaseFirst: (() => void) | undefined;
    engine.append(oldBodies.join("\n\n"));
    engine.finish();
    try {
      engine.registerProcessor({
        descriptor: { id: "test.ts.input-credit-reset", version: "v1" },
        configurationVersion: "test.ts.input-credit-reset.default",
        matches: (node) => node.content.kind === "paragraph",
        process(request) {
          processedBodies.push(request.input.body);
          const output: ProcessorOutput = {
            kind: "text",
            protocol: "test.ts.input-credit-reset/1",
            mediaType: "text/plain",
            text: request.input.body,
          };
          if (processedBodies.length !== 1) {
            return output;
          }
          return new Promise<ProcessorOutput>((resolve) => {
            releaseFirst = () => resolve(output);
          });
        },
      });

      await vi.waitFor(() => expect(releaseFirst).toBeDefined());
      await new Promise<void>((resolve) => setTimeout(resolve, 10));

      engine.reset();
      engine.append(replacement);
      engine.finish();
      releaseFirst?.();
      await engine.whenProcessorsIdle();

      expect(processedBodies).toEqual([oldBodies[0], replacement]);
    } finally {
      releaseFirst?.();
      engine.close();
    }
  });

  it("crosses a task boundary before refilling synchronous capacity", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: { maxInFlightJobs: 1n },
    });
    const process = vi.fn<() => ProcessorOutput>(() => ({
      kind: "text",
      protocol: "test.ts.completion-refill/1",
      mediaType: "text/plain",
      text: "done",
    }));
    engine.append("one\n\ntwo\n\nthree");
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.completion-refill", version: "v1" },
      configurationVersion: "test.ts.completion-refill.default",
      matches: (node) => node.content.kind === "paragraph",
      process,
    });
    const taskMarker = new Promise<{
      readonly issuedRequests: string;
      readonly processCalls: number;
    }>((resolve) => {
      setTimeout(() => {
        resolve({
          issuedRequests: engine.store.processorMetrics().issuedRequests,
          processCalls: process.mock.calls.length,
        });
      }, 0);
    });

    await expect(taskMarker).resolves.toEqual({
      issuedRequests: "1",
      processCalls: 1,
    });

    await engine.whenProcessorsIdle();

    expect(process).toHaveBeenCalledTimes(3);
    expect(engine.store.processorMetrics().issuedRequests).toBe("3");
    engine.close();
  });

  it("invalidates a queued dispatch continuation across reset", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: {
        maxInFlightJobs: 128n,
        maxSlots: 128n,
      },
    });
    const bodies: string[] = [];
    engine.append(
      Array.from({ length: 64 }, (_, index) => `old ${index}`).join("\n\n"),
    );
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.reset-quantum", version: "v1" },
      configurationVersion: "test.ts.reset-quantum.default",
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        bodies.push(request.input.body);
        return {
          kind: "text",
          protocol: "test.ts.reset-quantum/1",
          mediaType: "text/plain",
          text: request.input.body,
        };
      },
    });

    await Promise.resolve();
    expect(engine.store.processorMetrics().issuedRequests).toBe("32");

    engine.reset();
    engine.append("replacement");
    engine.finish();
    await engine.whenProcessorsIdle();

    expect(bodies).toEqual(["replacement"]);
    engine.close();
  });

  it("invalidates a queued tree scan across reset", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const candidateCount = 256;
    const engine = runtime.createEngine({
      processor: {
        maxInFlightJobs: BigInt(candidateCount),
        maxSlots: BigInt(candidateCount),
      },
    });
    const bodies: string[] = [];
    const replacement = `old ${candidateCount - 1}`;
    let replacementNodeId: ContentNodeView["id"] | undefined;
    let replacementMatchCalls = 0;
    engine.append(
      Array.from(
        { length: candidateCount },
        (_, index) => `old ${index}`,
      ).join("\n\n"),
    );
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.reset-scan", version: "v1" },
      configurationVersion: "test.ts.reset-scan.default",
      matches(node) {
        if (node.id === replacementNodeId) {
          replacementMatchCalls += 1;
        }
        return node.content.kind === "paragraph";
      },
      process(request) {
        bodies.push(request.input.body);
        return {
          kind: "text",
          protocol: "test.ts.reset-scan/1",
          mediaType: "text/plain",
          text: request.input.body,
        };
      },
    });
    const oldRequestCount = await new Promise<number>((resolve) => {
      setTimeout(() => {
        const requestCount = bodies.length;
        engine.reset();
        engine.append(replacement);
        engine.finish();
        replacementNodeId = engine.store.getSnapshot().document?.roots
          ?.children[0];
        resolve(requestCount);
      }, 0);
    });

    await engine.whenProcessorsIdle();

    expect(bodies.slice(oldRequestCount)).toEqual([replacement]);
    expect(replacementMatchCalls).toBe(1);
    engine.close();
  });

  it("coalesces candidate queue saturation errors until capacity returns", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: {
        maxInFlightJobs: 1n,
        maxSlots: 2n,
      },
    });
    const errors = vi.fn();
    const beginProcessor = vi.spyOn(RustBackedStore.prototype, "beginProcessor");
    let release: (() => void) | undefined;
    let processCalls = 0;
    try {
      engine.subscribeProcessorErrors(errors);
      engine.append("one\n\ntwo\n\nthree\n\nfour\n\nfive\n\nsix");
      engine.finish();
      engine.registerProcessor({
        descriptor: { id: "test.ts.queue-saturation", version: "v1" },
        configurationVersion: "test.ts.queue-saturation.default",
        matches: (node) => node.content.kind === "paragraph",
        process(request) {
          processCalls += 1;
          const output: ProcessorOutput = {
            kind: "text",
            protocol: "test.ts.queue-saturation/1",
            mediaType: "text/plain",
            text: request.input.body,
          };
          if (processCalls > 1) {
            return output;
          }
          return new Promise<ProcessorOutput>((resolve) => {
            release = () => resolve(output);
          });
        },
      });

      await vi.waitFor(() => expect(release).toBeDefined());

      const saturationErrors = errors.mock.calls
        .map(([event]) => event)
        .filter(({ error }) =>
          error instanceof Error &&
          "detailCode" in error &&
          error.detailCode === "processor.candidate_queue_limit"
        );
      expect(saturationErrors).toHaveLength(1);

      release?.();
      await engine.whenProcessorsIdle();

      const slotErrors = errors.mock.calls
        .map(([event]) => event)
        .filter(({ error }) =>
          error instanceof Error &&
          "detailCode" in error &&
          error.detailCode === "processor.resource_limit.slots"
        );
      expect(beginProcessor).toHaveBeenCalledTimes(6);
      expect(processCalls).toBe(2);
      expect(slotErrors).toHaveLength(4);
    } finally {
      release?.();
      beginProcessor.mockRestore();
      engine.close();
    }
  });

  it("cancels a paused scan when saturation feedback resets the engine", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: {
        maxInFlightJobs: 1n,
        maxSlots: 2n,
      },
    });
    const process = vi.fn<() => ProcessorOutput>(() => ({
      kind: "text",
      protocol: "test.ts.saturation-reset/1",
      mediaType: "text/plain",
      text: "unreachable",
    }));
    let reset = false;
    engine.subscribeProcessorErrors(({ error }) => {
      if (
        !reset &&
        error instanceof Error &&
        "detailCode" in error &&
        error.detailCode === "processor.candidate_queue_limit"
      ) {
        reset = true;
        engine.reset();
        engine.finish();
      }
    });
    engine.append("one\n\ntwo\n\nthree\n\nfour\n\nfive");
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.saturation-reset", version: "v1" },
      configurationVersion: "test.ts.saturation-reset.default",
      matches: (node) => node.content.kind === "paragraph",
      process,
    });

    await engine.whenProcessorsIdle();

    expect(reset).toBe(true);
    expect(process).not.toHaveBeenCalled();
    engine.close();
  });

  it("lets timer-backed jobs make progress while dispatch is saturated", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: { maxInFlightJobs: 1n },
    });
    const requests: ProcessorRequestView[] = [];
    let completedTimers = 0;

    engine.append("one\n\ntwo\n");
    engine.finish();
    engine.registerProcessor({
      descriptor: { id: "test.ts.timer-progress", version: "v1" },
      configurationVersion: "test.ts.timer-progress.default",
      matches: (node) => node.content.kind === "paragraph",
      async process(request) {
        requests.push(request);
        await new Promise<void>((resolve) => {
          setTimeout(() => {
            completedTimers += 1;
            resolve();
          }, 1);
        });
        return {
          kind: "text",
          protocol: "test.ts.timer-progress/1",
          mediaType: "text/plain",
          text: request.input.body,
        };
      },
    });

    await engine.whenProcessorsIdle();

    expect(requests).toHaveLength(2);
    expect(completedTimers).toBe(2);
    engine.close();
  });

  it("compacts invalidated candidates without changing survivor order", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: { maxInFlightJobs: 1n },
    });
    const requests: ProcessorRequestView[] = [];
    let releaseFirst: (() => void) | undefined;
    let firstStartedResolve: (() => void) | undefined;
    const firstStarted = new Promise<void>((resolve) => {
      firstStartedResolve = resolve;
    });

    engine.append("one\n\ntwo\n\nthree");
    engine.registerProcessor({
      descriptor: {
        id: "test.ts.candidate-churn",
        version: "v1",
        acceptsProvisional: true,
      },
      configurationVersion: "test.ts.candidate-churn.default",
      allowProvisional: true,
      matches: (node) => node.content.kind === "paragraph",
      process(request) {
        requests.push(request);
        const output: ProcessorOutput = {
          kind: "text",
          protocol: "test.ts.candidate-churn/1",
          mediaType: "text/plain",
          text: request.input.body,
        };
        if (requests.length !== 1) {
          return output;
        }
        firstStartedResolve?.();
        return new Promise<ProcessorOutput>((resolve) => {
          releaseFirst = () => resolve(output);
        });
      },
    });
    await firstStarted;

    // Each pass invalidates and requeues the same tail node while dispatch is full.
    for (let index = 0; index < 256; index += 1) {
      engine.append("x");
      await Promise.resolve();
    }
    releaseFirst?.();
    await engine.whenProcessorsIdle();

    expect(requests.map((request) => request.input.body)).toEqual([
      "one",
      "two",
      `three${"x".repeat(256)}`,
    ]);
    engine.close();
  });

  it("runs asynchronously, shares changed-node views, and keeps artifacts non-canonical", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const seenByFirst: ContentNodeView[] = [];
    const seenBySecond: ContentNodeView[] = [];
    const requests: ProcessorRequestView[] = [];

    engine.registerProcessor({
      descriptor: { id: "test.ts.first", version: "v1" },
      configurationVersion: "test.ts.first.default",
      matches(node) {
        if (node.content.kind === "paragraph") {
          seenByFirst.push(node);
          return true;
        }
        return false;
      },
      process(request) {
        requests.push(request);
        return {
          kind: "text",
          protocol: "test.ts.first/1",
          mediaType: "text/plain",
          text: "first artifact",
        };
      },
    });
    engine.registerProcessor({
      descriptor: { id: "test.ts.second", version: "v1" },
      configurationVersion: "test.ts.second.default",
      matches(node) {
        if (node.content.kind === "paragraph") {
          seenBySecond.push(node);
          return true;
        }
        return false;
      },
      process() {
        return {
          kind: "binary",
          protocol: "test.ts.second/1",
          mediaType: "application/octet-stream",
          bytes: Uint8Array.of(0, 127, 255),
        };
      },
    });

    engine.append("processor body");
    engine.finish();
    expect(requests).toHaveLength(0);
    await engine.whenProcessorsIdle();

    expect(requests).toHaveLength(1);
    expect(seenByFirst).toHaveLength(1);
    expect(seenBySecond).toHaveLength(1);
    expect(seenByFirst[0]).toBe(seenBySecond[0]);

    const request = requests[0]!;
    const epoch = engine.store.getSnapshot().document!.coordinate.epoch;
    const firstArtifact = engine.store.getArtifactSnapshot({
      epoch,
      nodeId: request.key.nodeId,
      processorId: "test.ts.first",
    });
    const secondArtifact = engine.store.getArtifactSnapshot({
      epoch,
      nodeId: request.key.nodeId,
      processorId: "test.ts.second",
    });
    expect(firstArtifact?.artifact?.payload).toEqual({
      kind: "text",
      text: "first artifact",
    });
    const binaryPayload = secondArtifact?.artifact?.payload;
    expect(binaryPayload?.kind).toBe("binary");
    if (binaryPayload?.kind !== "binary") {
      throw new Error("expected binary processor artifact");
    }
    expect(binaryPayload.bytes.byteLength).toBe(3);
    const callerBytes = binaryPayload.bytes.copyBytes();
    callerBytes[0] = 255;
    expect(binaryPayload.bytes.copyBytes()).toEqual(Uint8Array.of(0, 127, 255));
    expect(engine.store.getArtifactSnapshot({
      epoch,
      nodeId: request.key.nodeId,
      processorId: "test.ts.second",
    })).toBe(secondArtifact);

    const snapshot = engine.createRecoverySnapshot()!;
    const canonical = textDecoder.decode(snapshot);
    expect(canonical).not.toContain("test.ts.first");
    expect(canonical).not.toContain("first artifact");
    engine.close();
  });

  it("maps processor exceptions to structured failures without leaking leases", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const errors = vi.fn();
    engine.subscribeProcessorErrors(() => {
      throw new Error("observer failure");
    });
    engine.subscribeProcessorErrors(errors);
    let request: ProcessorRequestView | undefined;

    engine.registerProcessor({
      descriptor: { id: "test.ts.panic", version: "v1" },
      configurationVersion: "test.ts.panic.default",
      matches: (node) => node.content.kind === "paragraph",
      process(value) {
        request = value;
        throw new Error("processor exploded");
      },
    });
    engine.append("panic body");
    engine.finish();
    await engine.whenProcessorsIdle();

    expect(errors).toHaveBeenCalledWith(
      expect.objectContaining({ phase: "process", processorId: "test.ts.panic" }),
    );
    const epoch = engine.store.getSnapshot().document!.coordinate.epoch;
    const artifact = engine.store.getArtifactSnapshot({
      epoch,
      nodeId: request!.key.nodeId,
      processorId: "test.ts.panic",
    });
    expect(artifact).toMatchObject({
      state: "failed",
      failure: { code: "panic", message: "processor exploded" },
    });
    expect(engine.store.metrics().pendingProcessorRequests).toBe("0");
    expect(engine.store.processorMetrics().inFlightJobs).toBe("0");
    engine.close();
  });

  it("aborts reset work and ignores its late result after native invalidation", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    let resolveOutput: ((output: ProcessorOutput) => void) | undefined;
    let startedResolve: (() => void) | undefined;
    const started = new Promise<void>((resolve) => {
      startedResolve = resolve;
    });
    let request: ProcessorRequestView | undefined;
    let signal: AbortSignal | undefined;

    engine.registerProcessor({
      descriptor: { id: "test.ts.late", version: "v1" },
      configurationVersion: "test.ts.late.default",
      matches: (node) => node.content.kind === "paragraph",
      process(value, context) {
        request = value;
        signal = context.signal;
        startedResolve?.();
        return new Promise<ProcessorOutput>((resolve) => {
          resolveOutput = resolve;
        });
      },
    });
    engine.append("late body");
    engine.finish();
    await started;
    const oldEpoch = engine.store.getSnapshot().document!.coordinate.epoch;

    engine.reset();
    expect(signal?.aborted).toBe(true);
    const commandsAfterReset = engine.store.metrics().commands;
    resolveOutput?.({
      kind: "text",
      protocol: "test.ts.late/1",
      mediaType: "text/plain",
      text: "too late",
    });
    await engine.whenProcessorsIdle();

    expect(engine.store.metrics().commands).toBe(commandsAfterReset);
    expect(engine.store.getArtifactSnapshot({
      epoch: oldEpoch,
      nodeId: request!.key.nodeId,
      processorId: "test.ts.late",
    })).toBeUndefined();
    expect(engine.store.metrics().pendingProcessorRequests).toBe("0");
    expect(engine.store.processorMetrics().inFlightJobs).toBe("0");
    engine.close();
  });
});
