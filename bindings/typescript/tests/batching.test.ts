import { describe, expect, it, vi } from "vitest";

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

const batchOptions = {
  maxBatchBytes: 128,
  maxPendingChunks: 32,
} as const;

describe("lossless UTF-8 input batching", () => {
  it("returns every committed result in wire order for replica replay", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const replica = runtime.createStore();
    const batcher = engine.createBatcher({
      maxBatchBytes: 4,
      maxPendingChunks: 16,
    });
    const results: EngineResult[] = [];

    expect(batcher.push("ab")).toEqual([]);
    const oversized = batcher.push("12345");
    expect(oversized).toHaveLength(2);
    results.push(...oversized);
    expect(batcher.push("cd")).toEqual([]);
    const finished = batcher.finish();
    expect(finished).toHaveLength(2);
    results.push(...finished);
    expect(batcher.flush()).toEqual([]);
    batcher.release();

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

  it("returns ordered results from reset and recovery snapshots", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(batchOptions);

    expect(batcher.push("before reset")).toEqual([]);
    expect(batcher.reset()).toHaveLength(2);
    expect(batcher.push("after reset")).toEqual([]);
    const recovery = batcher.createRecoverySnapshot();
    expect(recovery.flushed).toHaveLength(1);
    expect(recovery.snapshot).toBeDefined();
    expect(decodeJson(recovery.snapshot!).source).toBe("after reset");

    batcher.release();
    engine.close();
  });

  it("enforces one explicit engine batching lease for its full lifetime", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(batchOptions);

    expect(() => engine.createBatcher(batchOptions)).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_lease_active" }),
    );
    for (const operation of [
      () => engine.append("direct"),
      () => engine.finish(),
      () => engine.reset(),
      () => engine.createRecoverySnapshot(),
      () => engine.close(),
    ]) {
      expect(operation).toThrowError(
        expect.objectContaining({ detailCode: "bindings.batch_lease_active" }),
      );
    }

    batcher.push("pending");
    expect(() => batcher.release()).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_pending" }),
    );
    expect(batcher.flush()).toHaveLength(1);
    batcher.release();
    expect(() => batcher.push("released")).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_released" }),
    );

    const replacement = engine.createBatcher(batchOptions);
    replacement.release();
    engine.close();
  });

  it("validates both pending limits before acquiring the engine lease", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();

    for (const options of [
      { maxBatchBytes: 0, maxPendingChunks: 8 },
      { maxBatchBytes: 8, maxPendingChunks: 0 },
      { maxBatchBytes: 1.5, maxPendingChunks: 8 },
      { maxBatchBytes: 8, maxPendingChunks: Number.MAX_SAFE_INTEGER + 1 },
    ]) {
      expect(() => engine.createBatcher(options)).toThrow(RangeError);
    }

    const batcher = engine.createBatcher(batchOptions);
    batcher.release();
    engine.close();
  });

  it("allows retryPending only for unresolved accepted input", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(batchOptions);

    expect(() => batcher.retryPending()).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_pending" }),
    );
    batcher.push("ordinary pending");
    expect(() => batcher.retryPending()).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_pending" }),
    );
    expect(batcher.inspectPending()?.chunks).toEqual(["ordinary pending"]);

    expect(batcher.flush()).toHaveLength(1);
    batcher.release();
    engine.close();
  });

  it("bounds retained constituent metadata and ignores empty chunks", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher({
      maxBatchBytes: 128,
      maxPendingChunks: 2,
    });

    expect(batcher.push("a")).toEqual([]);
    expect(batcher.push("")).toEqual([]);
    expect(batcher.push("b")).toEqual([]);
    const preflushed = batcher.push("c");
    expect(preflushed).toHaveLength(2);
    expect(batcher.inspectPending()).toEqual({
      chunks: ["c"],
      bytes: "1",
      constituents: "1",
    });

    const metrics = batcher.metrics();
    expect(metrics.maxPendingChunks).toBe("2");
    expect(metrics.inputAttempts).toBe("4");
    expect(metrics.pendingConstituents).toBe("1");
    expect(metrics.boundaryMetadataBytes).toBe("8");
    expect(metrics.successfulAppends).toBe("2");
    expect(metrics.committedBytes).toBe("2");
    expect(metrics.scanBytes).toBe("3");
    expect(metrics.publishedResults).toBe("2");

    expect(batcher.flush()).toHaveLength(1);
    batcher.release();
    engine.close();
  });

  it("retains a failed constituent and untouched suffix for explicit recovery", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      protocol: { maxSourceBytes: 64n },
      wire: { maxCommandBytes: 3n },
    });
    const batcher = engine.createBatcher({
      maxBatchBytes: 1024,
      maxPendingChunks: 16,
    });
    const failing = "1234";

    batcher.push("a");
    batcher.push(failing);
    batcher.push("suffix");

    let failure: unknown;
    try {
      batcher.flush();
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(BatchOperationError);
    const batchError = failure as BatchOperationError;
    expect(batchError.completedResults).toHaveLength(1);
    expect(batchError.cause).toBeInstanceOf(MdstreamError);
    expect((batchError.cause as MdstreamError).splitSafety).toBe(
      "retry_at_original_boundaries",
    );
    expect(batchError.operation).toBe("flush");
    expect(batchError.newInputAccepted).toBeUndefined();
    expect(batchError.pending).toEqual({
      chunks: [failing, "suffix"],
      bytes: String(utf8ByteLength(failing) + utf8ByteLength("suffix")),
      constituents: "2",
    });
    expect(Object.isFrozen(batchError.pending)).toBe(true);
    expect(Object.isFrozen(batchError.pending?.chunks)).toBe(true);
    expect(batcher.inspectPending()).toEqual(batchError.pending);
    expect(() => batcher.release()).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_pending" }),
    );
    expect(() => engine.append("lease bypass")).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_lease_active" }),
    );

    for (const operation of [
      () => batcher.push("new input"),
      () => batcher.flush(),
      () => batcher.finish(),
      () => batcher.reset(),
      () => batcher.createRecoverySnapshot(),
    ]) {
      expect(operation).toThrowError(
        expect.objectContaining({ detailCode: "bindings.batch_unresolved" }),
      );
    }

    let retryFailure: unknown;
    try {
      batcher.retryPending();
    } catch (error) {
      retryFailure = error;
    }
    expect(retryFailure).toBeInstanceOf(BatchOperationError);
    expect(retryFailure).toMatchObject({
      completedResults: [],
      operation: "retry_pending",
      newInputAccepted: undefined,
      pending: batchError.pending,
    });
    expect(batcher.inspectPending()).toEqual(batchError.pending);
    expect(batcher.metrics().successfulAppends).toBe("1");
    expect(batcher.metrics().appendAttempts).toBe("3");

    const transferred = batcher.takePending();
    expect(transferred).toEqual(batchError.pending);
    expect(batcher.inspectPending()).toBeUndefined();
    batcher.release();
    expect(decodeJson(engine.createRecoverySnapshot()!).source).toBe("a");
    const replacement = engine.createBatcher(batchOptions);
    replacement.release();
    engine.close();
  });

  it("distinguishes failed preflush from an accepted auto-flush input", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });

    const preflushEngine = runtime.createEngine({
      protocol: { maxSourceBytes: 64n },
      wire: { maxCommandBytes: 3n },
    });
    const preflushBatcher = preflushEngine.createBatcher({
      maxBatchBytes: 5,
      maxPendingChunks: 8,
    });
    preflushBatcher.push("1234");
    let preflushFailure: unknown;
    try {
      preflushBatcher.push("xx");
    } catch (error) {
      preflushFailure = error;
    }
    expect(preflushFailure).toMatchObject({
      operation: "push",
      newInputAccepted: false,
      pending: { chunks: ["1234"] },
    });
    preflushBatcher.discardPending();
    preflushBatcher.release();
    preflushEngine.close();

    const acceptedEngine = runtime.createEngine({
      protocol: { maxSourceBytes: 64n },
      wire: { maxCommandBytes: 3n },
    });
    const acceptedBatcher = acceptedEngine.createBatcher({
      maxBatchBytes: 4,
      maxPendingChunks: 8,
    });
    let acceptedFailure: unknown;
    try {
      acceptedBatcher.push("1234");
    } catch (error) {
      acceptedFailure = error;
    }
    expect(acceptedFailure).toMatchObject({
      operation: "push",
      newInputAccepted: true,
      pending: { chunks: ["1234"] },
    });
    acceptedBatcher.discardPending();
    acceptedBatcher.release();
    acceptedEngine.close();
  });

  it("keeps preflush results when an accepted auto-flush fails", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      protocol: { maxSourceBytes: 64n },
      wire: { maxCommandBytes: 3n },
    });
    const batcher = engine.createBatcher({
      maxBatchBytes: 4,
      maxPendingChunks: 8,
    });

    expect(batcher.push("a")).toEqual([]);
    let failure: unknown;
    try {
      batcher.push("1234");
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(BatchOperationError);
    const batchError = failure as BatchOperationError;
    expect(batchError).toMatchObject({
      operation: "push",
      newInputAccepted: true,
      pending: { chunks: ["1234"] },
    });
    expect(batchError.completedResults).toHaveLength(1);
    expect(batchError.completedResults[0]).toEqual(
      expect.objectContaining({
        changes: expect.any(Array),
        reducerResults: expect.any(Array),
      }),
    );
    expect(batcher.metrics()).toMatchObject({
      successfulAppends: "1",
      publishedResults: "1",
    });

    batcher.discardPending();
    batcher.release();
    engine.close();
  });

  it("does not accept a new chunk when the constituent-budget preflush fails", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      protocol: { maxSourceBytes: 64n },
      wire: { maxCommandBytes: 3n },
    });
    const batcher = engine.createBatcher({
      maxBatchBytes: 1024,
      maxPendingChunks: 1,
    });

    batcher.push("1234");
    let failure: unknown;
    try {
      batcher.push("x");
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(BatchOperationError);
    expect(failure).toMatchObject({
      completedResults: [],
      operation: "push",
      newInputAccepted: false,
      pending: {
        chunks: ["1234"],
        bytes: "4",
        constituents: "1",
      },
    });
    expect(batcher.inspectPending()?.chunks).toEqual(["1234"]);
    expect(() => batcher.release()).toThrowError(
      expect.objectContaining({ detailCode: "bindings.batch_pending" }),
    );

    batcher.discardPending();
    batcher.release();
    expect(engine.createRecoverySnapshot()).toBeUndefined();
    engine.close();
  });

  it("reports unaccepted standalone append failures with the batch ownership envelope", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      protocol: { maxSourceBytes: 64n },
      wire: { maxCommandBytes: 3n },
    });
    const batcher = engine.createBatcher({
      maxBatchBytes: 2,
      maxPendingChunks: 8,
    });

    let failure: unknown;
    try {
      batcher.push("1234");
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(BatchOperationError);
    expect(failure).toMatchObject({
      completedResults: [],
      operation: "push",
      pending: undefined,
      newInputAccepted: false,
    });
    expect((failure as BatchOperationError).cause).toBeInstanceOf(MdstreamError);
    expect(batcher.inspectPending()).toBeUndefined();
    expect(batcher.metrics()).toMatchObject({
      appendAttempts: "1",
      successfulAppends: "0",
      committedBytes: "0",
    });

    batcher.release();
    expect(engine.createRecoverySnapshot()).toBeUndefined();
    engine.close();
  });

  it("wraps lifecycle failures with and without committed prefix results", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });

    const finishedEngine = runtime.createEngine();
    const finishedBatcher = finishedEngine.createBatcher(batchOptions);
    expect(finishedBatcher.finish()).toHaveLength(1);
    finishedBatcher.push("late input");
    let finishFailure: unknown;
    try {
      finishedBatcher.finish();
    } catch (error) {
      finishFailure = error;
    }
    expect(finishFailure).toBeInstanceOf(BatchOperationError);
    expect(finishFailure).toMatchObject({
      completedResults: [],
      operation: "finish",
      pending: { chunks: ["late input"] },
      newInputAccepted: undefined,
    });
    finishedBatcher.discardPending();
    finishedBatcher.release();
    finishedEngine.close();

    const snapshotEngine = runtime.createEngine({
      wire: { maxEncodedSnapshotBytes: 1n },
    });
    const snapshotBatcher = snapshotEngine.createBatcher(batchOptions);
    snapshotBatcher.push("already committed");
    expect(snapshotBatcher.flush()).toHaveLength(1);
    let emptySnapshotFailure: unknown;
    try {
      snapshotBatcher.createRecoverySnapshot();
    } catch (error) {
      emptySnapshotFailure = error;
    }
    expect(emptySnapshotFailure).toBeInstanceOf(BatchOperationError);
    expect(emptySnapshotFailure).toMatchObject({
      completedResults: [],
      operation: "recovery_snapshot",
      pending: undefined,
      newInputAccepted: undefined,
    });

    snapshotBatcher.push("committed before snapshot failure");
    let snapshotFailure: unknown;
    try {
      snapshotBatcher.createRecoverySnapshot();
    } catch (error) {
      snapshotFailure = error;
    }
    expect(snapshotFailure).toBeInstanceOf(BatchOperationError);
    expect(snapshotFailure).toMatchObject({
      completedResults: [expect.any(Object)],
      operation: "recovery_snapshot",
      pending: undefined,
      newInputAccepted: undefined,
    });
    snapshotBatcher.release();
    let directSnapshotFailure: unknown;
    try {
      snapshotEngine.createRecoverySnapshot();
    } catch (error) {
      directSnapshotFailure = error;
    }
    expect(directSnapshotFailure).toBeInstanceOf(MdstreamError);
    expect(directSnapshotFailure).not.toBeInstanceOf(BatchOperationError);
    snapshotEngine.close();
  });

  it("rejects obvious native overflow before accepting or attempting append", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({ protocol: { maxSourceBytes: 1n } });
    const batcher = engine.createBatcher(batchOptions);

    expect(() => batcher.push("xxx")).toThrowError(
      expect.objectContaining({ detailCode: "bindings.resource_limit" }),
    );
    expect(batcher.inspectPending()).toBeUndefined();
    expect(batcher.metrics()).toMatchObject({
      inputAttempts: "1",
      appendAttempts: "0",
      successfulAppends: "0",
      scanBytes: "0",
    });

    batcher.release();
    engine.close();
  });

  it("makes pending-data discard explicit before the lease can be released", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({ protocol: { maxSourceBytes: 1n } });
    const batcher = engine.createBatcher(batchOptions);

    batcher.push("ab");
    let failure: unknown;
    try {
      batcher.flush();
    } catch (error) {
      failure = error;
    }
    expect(failure).toBeInstanceOf(BatchOperationError);
    expect((failure as BatchOperationError).cause).toMatchObject({
      splitSafety: "not_safe",
    });
    expect(batcher.metrics().appendAttempts).toBe("1");
    expect(batcher.inspectPending()?.chunks).toEqual(["ab"]);
    expect(batcher.discardPending()?.chunks).toEqual(["ab"]);
    expect(batcher.inspectPending()).toBeUndefined();
    batcher.release();
    expect(engine.createRecoverySnapshot()).toBeUndefined();
    engine.close();
  });

  it("preserves committed results when an unaccepted oversized append fails", async () => {
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
    const batcher = engine.createBatcher({
      maxBatchBytes: 2,
      maxPendingChunks: 8,
    });
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
    expect(batchError.operation).toBe("push");
    expect(batchError.pending).toBeUndefined();
    expect(batchError.newInputAccepted).toBe(false);
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

    batcher.release();
    expect(decodeJson(engine.createRecoverySnapshot()!).source).toBe("a");
    replica.close();
    engine.close();
  });

  it("preserves state for 1/16/128/4096-byte batches", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const chunks = [
      "# Batching\r",
      "",
      "\n\n",
      "emoji 👩‍💻 and ",
      "accent é",
      "\n\n```mermaid\nflowchart LR\nA-->B\n```",
    ];
    let expected: unknown;

    for (const size of [1, 16, 128, 4096]) {
      const engine = runtime.createEngine();
      const batcher = engine.createBatcher({
        maxBatchBytes: size,
        maxPendingChunks: 64,
      });
      for (const chunk of chunks) {
        batcher.push(chunk);
      }
      batcher.finish();
      const recovery = batcher.createRecoverySnapshot();
      const snapshot = recovery.snapshot!;
      const normalized = normalizeSnapshot(decodeJson(snapshot));
      expected ??= normalized;
      expect(normalized).toEqual(expected);
      expect(decodeJson(snapshot).source).toContain("emoji 👩‍💻 and accent é");

      const metrics = batcher.metrics();
      expect(metrics.inputBytes).toBe(metrics.committedBytes);
      expect(metrics.pendingBytes).toBe("0");
      expect(BigInt(metrics.successfulAppends)).toBeGreaterThan(0n);
      expect(BigInt(metrics.outputPayloadBytes)).toBeGreaterThan(0n);
      expect(metrics.joinCopyBytes).toBe("0");
      expect(metrics.replayCount).toBe("0");
      batcher.release();
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
    const batcher = engine.createBatcher({
      maxBatchBytes: 16,
      maxPendingChunks: 8,
    });
    expect(() => batcher.push("\udc00")).toThrow(TypeError);
    batcher.release();
    engine.close();
  });

  it("does not rerun admission scanning before WASM string encoding", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(batchOptions);
    const charCodeAt = vi.spyOn(String.prototype, "charCodeAt");
    let scansAfterAdmission = 0;
    let scansAfterCommit = 0;
    let results: readonly EngineResult[] = [];
    try {
      batcher.push("scan-once");
      scansAfterAdmission = charCodeAt.mock.calls.length;
      results = batcher.flush();
      scansAfterCommit = charCodeAt.mock.calls.length;
    } finally {
      charCodeAt.mockRestore();
    }

    expect(scansAfterAdmission).toBe("scan-once".length);
    expect(scansAfterCommit - scansAfterAdmission).toBe("scan-once".length);
    expect(results).toHaveLength(1);
    expect(batcher.metrics().scanBytes).toBe(String("scan-once".length));
    batcher.release();
    engine.close();
  });

  it("does not retain an empty boundary or flush a pending carriage return", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(batchOptions);
    batcher.push("line\r");
    batcher.push("");
    expect(batcher.metrics().appendAttempts).toBe("0");
    expect(batcher.metrics().pendingConstituents).toBe("1");
    batcher.push("\nnext");
    batcher.finish();
    const recovery = batcher.createRecoverySnapshot();
    expect(decodeJson(recovery.snapshot!).source).toBe("line\nnext");
    batcher.release();
    engine.close();
  });
});
