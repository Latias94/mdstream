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
  it("materializes pending source on demand with stable external-store snapshots", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    engine.append("a *b");

    const pending = engine.store.pendingSource();
    expect(pending.getSnapshot()).toBeUndefined();
    const listener = vi.fn();
    const unsubscribe = pending.subscribe(listener);

    engine.append("*");
    expect(listener).toHaveBeenCalledTimes(1);
    const view = pending.getSnapshot();
    expect(view).toMatchObject({
      kind: "pending_source_view",
      range: { start: "4", end: "5" },
      text: "*",
    });
    expect(pending.getSnapshot()).toBe(view);
    expect(engine.store.metrics().materializedPendingSourceViews).toBe("1");

    engine.append("é");
    expect(listener).toHaveBeenCalledTimes(2);
    const utf8View = pending.getSnapshot();
    expect(utf8View).not.toBe(view);
    expect(utf8View).toMatchObject({
      range: { start: "4", end: "7" },
      text: "*é",
    });
    expect(pending.getSnapshot()).toBe(utf8View);
    expect(engine.store.metrics().materializedPendingSourceViews).toBe("2");

    engine.append("");
    expect(listener).toHaveBeenCalledTimes(2);
    engine.finish();
    expect(listener).toHaveBeenCalledTimes(3);
    expect(pending.getSnapshot()).toBeUndefined();
    expect(engine.store.metrics().materializedPendingSourceViews).toBe("2");

    unsubscribe();
    engine.close();
  });

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
    const firstPending = store.getPendingSourceSnapshot();
    const pendingListener = vi.fn();
    store.subscribePendingSource(pendingListener);
    expect(listener).toHaveBeenCalledTimes(1);

    const retry = store.applyChange(encodeChange(trace.changes[0]));
    expect(retry.updates[0]?.outcome.kind).toBe("idempotent");
    expect(store.getSnapshot()).toBe(first);
    expect(store.getPendingSourceSnapshot()).toBe(firstPending);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(pendingListener).not.toHaveBeenCalled();
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
    const lastGoodPending = target.getPendingSourceSnapshot();
    const pendingListener = vi.fn();
    target.subscribePendingSource(pendingListener);

    const gap = target.applyChange(encodeChange(trace.changes[2]));
    expect(gap.updates[0]?.outcome.kind).toBe("recovery_required");
    expect(target.getSnapshot().status.kind).toBe("needs_snapshot");
    expect(target.getSnapshot().document?.coordinate).toEqual(
      lastGoodDocument?.coordinate,
    );
    expect(target.getPendingSourceSnapshot()).toBe(lastGoodPending);
    expect(pendingListener).not.toHaveBeenCalled();
    expect(listener).toHaveBeenCalledTimes(2);

    expect(() => target.applyChange(encodeChange(trace.changes[3]))).toThrowError(
      MdstreamError,
    );
    expect(listener).toHaveBeenCalledTimes(2);

    target.recoverSnapshot(recovery);
    expect(target.getSnapshot().status.kind).toBe("ready");
    expect(pendingListener).toHaveBeenCalledTimes(1);
    const recoveredPending = target.getPendingSourceSnapshot();
    expect(recoveredPending).not.toBe(lastGoodPending);
    expect(recoveredPending?.text).toBe("abc");
    target.applyChange(encodeChange(trace.changes[3]));
    expect(target.getSnapshot().document?.lifecycle).toBe("finalized");
    expect(pendingListener).toHaveBeenCalledTimes(2);
    expect(target.getPendingSourceSnapshot()).toBeUndefined();
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
