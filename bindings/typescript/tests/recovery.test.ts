import { describe, expect, it, vi } from "vitest";

import {
  asNodeId,
  initMdstream,
  MdstreamError,
} from "../src/index.js";
import {
  encodeChange,
  loadProtocolFixture,
  nodeWasmLoader,
} from "./helpers.js";

describe("Rust-backed external store recovery", () => {
  it("does not notify or replace snapshots for idempotent retries", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const trace = loadProtocolFixture().traces.find(({ id }) => id === "characters")!;
    const store = runtime.createStore();
    const listener = vi.fn();
    store.subscribe(() => {
      throw new Error("subscriber failure");
    });
    store.subscribe(listener);

    store.applyChange(encodeChange(trace.changes[0]));
    const first = store.getSnapshot();
    expect(listener).toHaveBeenCalledTimes(1);

    const retry = store.applyChange(encodeChange(trace.changes[0]));
    expect(retry.updates[0]?.outcome.kind).toBe("idempotent");
    expect(store.getSnapshot()).toBe(first);
    expect(listener).toHaveBeenCalledTimes(1);
    store.close();
  });

  it("retains last-good views through a gap and recovers only from an explicit snapshot", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const trace = loadProtocolFixture().traces.find(({ id }) => id === "characters")!;
    const source = runtime.createStore();
    for (const change of trace.changes.slice(0, 3)) {
      source.applyChange(encodeChange(change));
    }
    const recovery = source.createRecoverySnapshot()!;

    const target = runtime.createStore();
    const listener = vi.fn();
    target.subscribe(listener);
    target.applyChange(encodeChange(trace.changes[0]));
    const lastGoodDocument = target.getSnapshot().document;

    const gap = target.applyChange(encodeChange(trace.changes[2]));
    expect(gap.updates[0]?.outcome.kind).toBe("recovery_required");
    expect(target.getSnapshot().status.kind).toBe("needs_snapshot");
    expect(target.getSnapshot().document?.coordinate).toEqual(
      lastGoodDocument?.coordinate,
    );
    expect(listener).toHaveBeenCalledTimes(2);

    expect(() => target.applyChange(encodeChange(trace.changes[3]))).toThrowError(
      MdstreamError,
    );
    expect(listener).toHaveBeenCalledTimes(2);

    target.recoverSnapshot(recovery);
    expect(target.getSnapshot().status.kind).toBe("ready");
    target.applyChange(encodeChange(trace.changes[3]));
    expect(target.getSnapshot().document?.lifecycle).toBe("finalized");
    source.close();
    target.close();
  });

  it("invalidates only changed node caches and preserves unchanged references", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const first = engine.append("first paragraph\n\nsecond paragraph");
    engine.finish();
    const changed = first.reducerResults[0]?.updates[0]?.impact.changedNodeIds ?? [];
    expect(changed.length).toBeGreaterThan(1);

    const firstId = changed[0] ?? asNodeId("1");
    const secondId = changed[1] ?? asNodeId("2");
    const firstView = engine.store.getNodeSnapshot(firstId);
    const secondView = engine.store.getNodeSnapshot(secondId);
    expect(engine.store.getNodeSnapshot(firstId)).toBe(firstView);
    expect(engine.store.getNodeSnapshot(secondId)).toBe(secondView);

    const firstListener = vi.fn();
    const secondListener = vi.fn();
    engine.store.subscribeNode(firstId, firstListener);
    engine.store.subscribeNode(secondId, secondListener);

    const reset = engine.reset();
    expect(reset.reducerResults[0]?.updates[0]?.impact.fullReplace).toBe(true);
    expect(firstListener).toHaveBeenCalledTimes(1);
    expect(secondListener).toHaveBeenCalledTimes(1);
    expect(engine.store.getNodeSnapshot(firstId)).toBeUndefined();
    engine.close();
  });
});
