import { describe, expect, it } from "vitest";

import {
  BatchOperationError,
  initMdstream,
  MdstreamError,
  utf8ByteLength,
  type EngineResult,
  type TransitionBatchView,
} from "../src/index.js";
import {
  decodeJson,
  nodeWasmLoader,
  normalizeSnapshot,
} from "./helpers.js";

describe("lossless UTF-8 input batching", () => {
  it("returns every committed result in wire order for replica replay", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const replica = runtime.createStore();
    const batcher = engine.createBatcher(4);
    const results: EngineResult[] = [];

    expect(batcher.push("ab")).toEqual([]);
    const oversized = batcher.push("12345");
    expect(oversized).toHaveLength(2);
    results.push(...oversized);
    expect(batcher.push("cd")).toEqual([]);
    const finished = batcher.finish();
    expect(finished).toHaveLength(2);
    results.push(...finished);

    for (const result of results) {
      for (const change of result.changes) {
        replica.applyChange(change);
      }
    }
    expect(
      normalizeSnapshot(decodeJson(replica.createRecoverySnapshot()!)),
    ).toEqual(normalizeSnapshot(decodeJson(engine.createRecoverySnapshot()!)));

    replica.close();
    engine.close();
  });

  it("returns flushed results from reset and recovery snapshots", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(128);

    expect(batcher.push("before reset")).toEqual([]);
    expect(batcher.reset()).toHaveLength(2);
    expect(batcher.push("after reset")).toEqual([]);
    const recovery = batcher.createRecoverySnapshot();
    expect(recovery.flushed).toHaveLength(1);
    expect(recovery.snapshot).toBeDefined();
    expect(decodeJson(recovery.snapshot!).source).toBe("after reset");

    engine.close();
  });

  it("preserves committed results when an oversized forward fails", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      captureTransitions: true,
      protocol: {
        maxSourceBytes: 3n,
        maxNodes: 64n,
        maxResources: 16n,
        maxOperations: 64n,
        maxChangeStructuralItems: 64n,
        maxChildrenPerList: 64n,
      },
    });
    const replica = runtime.createStore();
    const batcher = engine.createBatcher(2);
    const batches: TransitionBatchView[] = [];
    const order: string[] = [];
    engine.store.subscribeTransitions((batch) => {
      batches.push(batch);
      order.push("transition");
    });
    engine.store.subscribe(() => order.push("invalidation"));

    expect(batcher.push("a")).toEqual([]);
    expect(batches).toEqual([{ facts: [] }]);
    order.length = 0;
    let failure: unknown;
    try {
      batcher.push("bbbb");
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(BatchOperationError);
    const batchError = failure as BatchOperationError;
    expect(batchError.completedResults).toHaveLength(1);
    expect(batchError.cause).toBeInstanceOf(MdstreamError);
    expect(batches).toHaveLength(2);
    expect(batches[1]!.facts.length).toBeGreaterThan(0);
    expect(order).toEqual(["transition", "invalidation"]);
    for (const result of batchError.completedResults) {
      for (const change of result.changes) {
        replica.applyChange(change);
      }
    }
    expect(decodeJson(replica.createRecoverySnapshot()!).source).toBe("a");
    expect(decodeJson(engine.createRecoverySnapshot()!).source).toBe("a");

    replica.close();
    engine.close();
  });

  it("preserves state for 1/16/128/4096-byte batches", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const chunks = [
      "# 批处理\r",
      "",
      "\n\n",
      "emoji 👩‍💻 and ",
      "accent é",
      "\n\n```mermaid\nflowchart LR\nA-->B\n```",
    ];
    let expected: unknown;

    for (const size of [1, 16, 128, 4096]) {
      const engine = runtime.createEngine();
      const batcher = engine.createBatcher(size);
      for (const chunk of chunks) {
        batcher.push(chunk);
      }
      batcher.finish();
      const snapshot = batcher.createRecoverySnapshot().snapshot!;
      const normalized = normalizeSnapshot(decodeJson(snapshot));
      expected ??= normalized;
      expect(normalized).toEqual(expected);
      expect(decodeJson(snapshot).source).toContain("emoji 👩‍💻 and accent é");

      const metrics = batcher.metrics();
      expect(metrics.inputBytes).toBe(metrics.forwardedBytes);
      expect(metrics.pendingBytes).toBe("0");
      expect(BigInt(metrics.wasmAppendCalls)).toBeGreaterThan(0n);
      expect(BigInt(metrics.outputPayloadBytes)).toBeGreaterThan(0n);
      engine.close();
    }
  });

  it("counts UTF-8 without allocation and rejects ill-formed UTF-16", async () => {
    expect(utf8ByteLength("aé👩‍💻")).toBe(new TextEncoder().encode("aé👩‍💻").length);
    expect(() => utf8ByteLength("\ud800")).toThrow(/unpaired UTF-16 high/);
    expect(() => utf8ByteLength("\udc00")).toThrow(/unpaired UTF-16 low/);

    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    expect(() => engine.append("\ud800")).toThrow(TypeError);
    const batcher = engine.createBatcher(16);
    expect(() => batcher.push("\udc00")).toThrow(TypeError);
    engine.close();
  });

  it("does not let an empty chunk flush a pending carriage return", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(128);
    batcher.push("line\r");
    batcher.push("");
    expect(batcher.metrics().wasmAppendCalls).toBe("0");
    batcher.push("\nnext");
    batcher.finish();
    expect(decodeJson(batcher.createRecoverySnapshot().snapshot!).source).toBe("line\nnext");
    engine.close();
  });
});
