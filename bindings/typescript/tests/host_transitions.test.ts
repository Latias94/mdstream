import { describe, expect, it, vi } from "vitest";

import {
  initMdstream,
  MdstreamError,
  type MdstreamSessionOptions,
  type TransitionBatchView,
  type WasmModuleLoader,
} from "../src/index.js";
import { BindingPayloadKind, type WasmOutput } from "../src/wasm.js";
import {
  encodeChange,
  loadProtocolFixture,
  nodeWasmLoader,
} from "./helpers.js";

const capturedOptions = {
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

describe("framework-neutral transition feed", () => {
  it("is inert unless transition capture is enabled", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const listener = vi.fn();
    engine.store.subscribeTransitions(listener);

    engine.append("plain");
    engine.append("");
    engine.finish();

    expect(listener).not.toHaveBeenCalled();
    engine.close();
  });

  it("publishes a coherent batch before invalidation listeners and guards reentry", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedOptions);
    const order: string[] = [];
    const observed: TransitionBatchView[] = [];
    let testedReentry = false;

    engine.store.subscribeTransitions((batch) => {
      order.push("transition");
      observed.push(batch);

      const tail = batch.facts.at(-1);
      expect(engine.store.getSnapshot().document?.coordinate).toEqual(
        tail?.after.coordinate,
      );

      if (tail?.scope === "continuous") {
        const nodeId = tail.nodes.find(({ after }) => after !== null)?.key.nodeId;
        if (nodeId !== undefined) {
          expect(engine.store.getNodeSnapshot(nodeId)?.node.id).toBe(nodeId);
        }
      }

      if (!testedReentry) {
        testedReentry = true;
        expect(() => engine.append("reentrant")).toThrowError(
          expect.objectContaining({ detailCode: "bindings.transition_reentry" }),
        );
        expect(() => engine.close()).toThrowError(
          expect.objectContaining({ detailCode: "bindings.transition_reentry" }),
        );
      }
    });
    engine.store.subscribe(() => order.push("invalidation"));

    engine.append("hello");

    expect(observed).toHaveLength(1);
    expect(observed[0]).toBeDefined();
    expect(observed[0]!.facts.length).toBeGreaterThan(0);
    expect(order).toEqual(["transition", "invalidation"]);
    engine.close();
  });

  it("publishes explicit empty batches for no-op and error operations", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedOptions);
    const batches: TransitionBatchView[] = [];
    const unsubscribe = engine.store.subscribeTransitions((batch) => {
      batches.push(batch);
      unsubscribe();
    });

    engine.append("");
    expect(batches).toEqual([{ facts: [] }]);

    const later = vi.fn();
    engine.store.subscribeTransitions(() => {
      throw new Error("isolated listener failure");
    });
    engine.store.subscribeTransitions(later);
    engine.finish();
    expect(later).toHaveBeenCalledTimes(1);

    expect(() => engine.append("late")).toThrowError(MdstreamError);
    expect(later).toHaveBeenCalledTimes(2);
    expect(later.mock.calls.at(-1)?.[0]).toEqual({ facts: [] });
    engine.close();
  });

  it("coalesces every reducer commit made by one batcher operation", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedOptions);
    const batcher = engine.createBatcher(4);
    const batches: TransitionBatchView[] = [];
    engine.store.subscribeTransitions((batch) => batches.push(batch));

    expect(batcher.push("ab")).toEqual([]);
    expect(batches).toEqual([{ facts: [] }]);

    const results = batcher.push("12345");
    expect(results).toHaveLength(2);
    expect(batches).toHaveLength(2);
    expect(batches[1]!.facts.length).toBeGreaterThanOrEqual(2);
    engine.close();
  });

  it("preserves A-to-B-to-A facts while exposing only the batch-tail view", async () => {
    const encoder = new TextEncoder();
    let reducerUpdates = 0;
    let materializedViews = 0;
    class EngineSession {
      append(): WasmOutput {
        return fakeOutput([
          [BindingPayloadKind.Change, new Uint8Array([1])],
          [BindingPayloadKind.Change, new Uint8Array([2])],
        ]);
      }

      free(): void {}
    }
    class ReducerSession {
      applyChange(): WasmOutput {
        reducerUpdates += 1;
        const before = reducerUpdates === 1 ? "A" : "B";
        const after = reducerUpdates === 1 ? "B" : "A";
        return fakeOutput([[
          BindingPayloadKind.ReducerUpdate,
          encoder.encode(JSON.stringify(fakeReducerUpdate(before, after, reducerUpdates))),
        ]]);
      }

      nodeView(): WasmOutput {
        materializedViews += 1;
        return fakeOutput([[
          BindingPayloadKind.NodeView,
          encoder.encode(JSON.stringify(fakeTailNodeView())),
        ]]);
      }

      pendingSourceView(): WasmOutput {
        return fakeOutput([]);
      }

      beginProcessorIfCurrent(): WasmOutput {
        return fakeOutput([]);
      }

      free(): void {}
    }
    const loader: WasmModuleLoader = () => ({
      MdstreamEngineSession: EngineSession,
      MdstreamReducerSession: ReducerSession,
      abiVersion: () => 1,
      packageVersion: () => "0.4.0",
      bindingSchema: () => "mdstream.bindings/0.4",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
      transitionSchema: () => "mdstream.transitions/draft",
    });
    const runtime = await initMdstream({ loader });
    const engine = runtime.createEngine(capturedOptions);
    const batches: TransitionBatchView[] = [];
    engine.store.subscribeTransitions((batch) => {
      batches.push(batch);
      const first = batch.facts[0];
      if (first?.scope !== "continuous") {
        throw new Error("fake ABA batch omitted continuous facts");
      }
      expect(
        engine.store.getNodeSnapshot(first.nodes[0]!.key.nodeId)?.node.version,
      ).toBe("A");
    });

    engine.append("ignored by fake transport");

    expect(batches).toHaveLength(1);
    const facts = batches[0]!.facts;
    expect(facts).toHaveLength(2);
    expect(facts.map((entry) =>
      entry.scope === "continuous" ? entry.nodes[0]?.after?.version : undefined
    )).toEqual(["B", "A"]);
    expect(materializedViews).toBe(1);
    engine.close();
  });

  it("guards batcher snapshot and processor disposal before host state mutates", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedOptions);
    const registration = engine.registerProcessor({
      descriptor: { id: "test.transition.guard", version: "v1" },
      configurationVersion: "default",
      matches: () => false,
      process: () => ({
        kind: "text",
        protocol: "test.transition.guard/1",
        mediaType: "text/plain",
        text: "unused",
      }),
    });
    const batcher = engine.createBatcher(128);
    batcher.push("a");
    batcher.push("b");
    let checked = false;
    const batches: TransitionBatchView[] = [];
    engine.store.subscribeTransitions((batch) => {
      batches.push(batch);
      if (checked) {
        return;
      }
      checked = true;
      const metrics = batcher.metrics();
      expect(() => batcher.createRecoverySnapshot()).toThrowError(
        expect.objectContaining({ detailCode: "bindings.transition_reentry" }),
      );
      expect(batcher.metrics()).toEqual(metrics);
      expect(() => registration.dispose()).toThrowError(
        expect.objectContaining({ detailCode: "bindings.transition_reentry" }),
      );
    });

    batcher.createRecoverySnapshot();
    expect(checked).toBe(true);

    registration.dispose();
    const afterDispose = batches.length;
    expect(batches.at(-1)).toEqual({ facts: [] });
    registration.dispose();
    expect(batches).toHaveLength(afterDispose);
    engine.close();
  });

  it("keeps same-floor recovery empty and advances continuity on replacement", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const trace = loadProtocolFixture().traces.find(({ id }) => id === "characters")!;
    const source = runtime.createStore(capturedOptions);
    const target = runtime.createStore(capturedOptions);
    const batches: TransitionBatchView[] = [];
    target.subscribeTransitions((batch) => batches.push(batch));

    source.applyChange(encodeChange(trace.changes[0]));
    target.applyChange(encodeChange(trace.changes[0]));
    const sameFloor = target.createRecoverySnapshot()!;

    target.applyChange(encodeChange(trace.changes[2]));
    target.recoverSnapshot(sameFloor);
    expect(batches.at(-1)).toEqual({ facts: [] });

    source.applyChange(encodeChange(trace.changes[1]));
    source.applyChange(encodeChange(trace.changes[2]));
    const advanced = source.createRecoverySnapshot()!;
    target.applyChange(encodeChange(trace.changes[2]));
    target.recoverSnapshot(advanced);

    const replacement = batches.at(-1);
    expect(replacement).toMatchObject({
      facts: [{
        scope: "full_replace",
        after: { continuityGeneration: "1" },
      }],
    });
    target.close();
    source.close();
  });
});

function fakeOutput(
  payloads: readonly (readonly [BindingPayloadKind, Uint8Array])[],
): WasmOutput {
  const taken = new Set<number>();
  return {
    len: payloads.length,
    remaining: () => payloads.length - taken.size,
    kind: (index) => payloads[index]?.[0] ?? 0,
    count: (kind) => payloads.filter(([candidate]) => candidate === kind).length,
    take(index) {
      const payload = payloads[index]?.[1];
      if (payload === undefined || taken.has(index)) {
        throw new Error("fake output payload was already taken");
      }
      taken.add(index);
      return payload;
    },
    free(): void {},
  };
}

function fakeReducerUpdate(before: string, after: string, sequence: number): unknown {
  const coordinate = {
    epoch: "1",
    sequence: String(sequence),
    change_id: `aba:${sequence}`,
    source_cursor: "1",
  };
  const stamp = (version: string) => ({
    version,
    stability: "stable",
    parent: { kind: "document" },
    children_version: "children.A",
  });
  const documentStamp = (version: string) => ({
    continuity_generation: "0",
    coordinate,
    lifecycle: "open",
    projection_cursor: "1",
    roots_version: `roots.${version}`,
  });
  return {
    schema: "mdstream.bindings/0.4",
    kind: "reducer_update",
    outcome: { kind: "applied", coordinate },
    status: { kind: "ready" },
    impact: {
      changed_node_ids: ["7"],
      removed_node_ids: [],
      changed_resource_ids: [],
      removed_resource_ids: [],
      source_changed: false,
      projection_changed: true,
      lifecycle_changed: false,
      roots_changed: false,
      full_replace: false,
    },
    document: {
      coordinate,
      lifecycle: "open",
      projection_cursor: "1",
      roots: { version: `roots.${after}`, children: ["7"] },
    },
    transition: {
      schema: "mdstream.transitions/draft",
      facts: {
        scope: "continuous",
        before: documentStamp(before),
        after: documentStamp(after),
        nodes: [{
          key: { continuity_generation: "0", epoch: "1", node_id: "7" },
          before: stamp(before),
          after: stamp(after),
          text: { kind: "replacement" },
        }],
        structures: [],
        resources: [],
      },
    },
  };
}

function fakeTailNodeView(): unknown {
  return {
    schema: "mdstream.bindings/0.4",
    kind: "node_view",
    node: {
      id: "7",
      version: "A",
      stability: "stable",
      source: { start: "0", end: "1" },
      body: { start: "0", end: "1" },
      children: { version: "children.A", children: [] },
      content: { kind: "text", text: { kind: "source" } },
    },
    body_text: "A",
  };
}
