import { describe, expect, it, vi } from "vitest";

import {
  initMdstream,
  type ContentNodeView,
  type ProcessorOutput,
  type ProcessorRequestView,
} from "../src/index.js";
import { nodeWasmLoader, textDecoder } from "./helpers.js";

describe("host-side processor scheduling", () => {
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
    expect(secondArtifact?.artifact?.payload).toEqual({
      kind: "binary",
      bytes: Uint8Array.of(0, 127, 255),
    });

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

  it("aborts reset work and still submits the late result for Rust freshness checks", async () => {
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
    resolveOutput?.({
      kind: "text",
      protocol: "test.ts.late/1",
      mediaType: "text/plain",
      text: "too late",
    });
    await engine.whenProcessorsIdle();

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
