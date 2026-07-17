import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it, vi } from "vitest";

import {
  initMdstream,
  type EngineResult,
  type MdstreamEngine,
  type MdstreamRuntime,
  type NodeId,
  type ProcessorRequestView,
} from "../src/index.js";
import {
  decodeJson,
  encodeChange,
  nodeWasmLoader,
  normalizeSnapshot,
  textDecoder,
} from "./helpers.js";

interface AdoptionFixture {
  readonly source: string;
  readonly traces: readonly AdoptionTrace[];
  readonly expected: { readonly normalized_snapshot: unknown };
}

interface AdoptionTrace {
  readonly id: string;
  readonly input_events: readonly (
    | { readonly kind: "append"; readonly chunk: string }
    | { readonly kind: "finish" }
  )[];
  readonly changes: readonly unknown[];
}

describe("framework-neutral TypeScript/WASM adoption", () => {
  it("streams stable keyed views, recovers a gap, and keeps artifacts derived", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const fixture = loadAdoptionFixture();
    const whole = await runEngineTrace(runtime, fixture, "whole");
    const adversarial = await runEngineTrace(runtime, fixture, "adversarial");

    expect(adversarial.normalized).toEqual(whole.normalized);
    expect(adversarial.normalized).toEqual(fixture.expected.normalized_snapshot);
    expect(adversarial.nodeIds).toEqual(whole.nodeIds);
    expect(adversarial.keyedNotifications).toBeGreaterThan(0);
    expect(adversarial.materializedNodeViews).toBe(adversarial.requestedNodeViews);

    const trace = fixture.traces.find(({ id }) => id === "adversarial")!;
    expect(trace.changes.length).toBeGreaterThan(4);
    const primary = runtime.createStore();
    for (const change of trace.changes.slice(0, 3)) {
      primary.applyChange(encodeChange(change));
    }
    const recovery = primary.createRecoverySnapshot()!;

    const replica = runtime.createStore();
    replica.applyChange(encodeChange(trace.changes[0]));
    const gap = replica.applyChange(encodeChange(trace.changes[2]));
    expect(gap.updates[0]?.outcome.kind).toBe("recovery_required");
    expect(replica.getSnapshot().status.kind).toBe("needs_snapshot");
    replica.recoverSnapshot(recovery);
    for (const change of trace.changes.slice(3)) {
      replica.applyChange(encodeChange(change));
    }
    expect(
      normalizeSnapshot(decodeJson(replica.createRecoverySnapshot()!)),
    ).toEqual(fixture.expected.normalized_snapshot);
    primary.close();
    replica.close();
  });
});

async function runEngineTrace(
  runtime: MdstreamRuntime,
  fixture: AdoptionFixture,
  traceId: string,
): Promise<{
  readonly normalized: unknown;
  readonly nodeIds: readonly NodeId[];
  readonly keyedNotifications: number;
  readonly requestedNodeViews: string;
  readonly materializedNodeViews: string;
}> {
  const trace = fixture.traces.find(({ id }) => id === traceId)!;
  const engine = runtime.createEngine();
  const rootListener = vi.fn();
  const keyedListener = vi.fn();
  const unsubscribeRoot = engine.store.subscribe(rootListener);
  const nodeSubscriptions = new Map<NodeId, () => void>();
  const nodeIds = new Set<NodeId>();
  let requestedNodeViews = 0n;
  let processorRequest: ProcessorRequestView | undefined;

  engine.registerProcessor({
    descriptor: { id: "adoption.ts.mermaid", version: "v1" },
    configurationVersion: "adoption.ts.mermaid.v1",
    matches: (node) =>
      node.content.kind === "code_block" && node.content.info === "mermaid",
    process(request) {
      processorRequest = request;
      return {
        kind: "text",
        protocol: "mdstream.adoption.mermaid-preview/1",
        mediaType: "text/plain",
        text: `preview:${request.input.body}`,
      };
    },
  });

  for (const event of trace.input_events) {
    const result = event.kind === "append"
      ? engine.append(event.chunk)
      : engine.finish();
    observeChangedNodes(
      engine,
      result,
      nodeIds,
      nodeSubscriptions,
      keyedListener,
      () => {
        requestedNodeViews += 1n;
      },
    );
  }
  await engine.whenProcessorsIdle();

  expect(rootListener).toHaveBeenCalled();
  expect(processorRequest).toBeDefined();
  const epoch = engine.store.getSnapshot().document!.coordinate.epoch;
  const artifact = engine.store.getArtifactSnapshot({
    epoch,
    nodeId: processorRequest!.key.nodeId,
    processorId: "adoption.ts.mermaid",
  });
  expect(artifact).toMatchObject({
    state: "ready",
    artifact: {
      protocol: "mdstream.adoption.mermaid-preview/1",
      payload: { kind: "text" },
    },
  });

  const snapshot = engine.createRecoverySnapshot()!;
  const canonical = textDecoder.decode(snapshot);
  expect(canonical).not.toContain("adoption.ts.mermaid");
  expect(canonical).not.toContain("preview:");
  const result = {
    normalized: normalizeSnapshot(decodeJson(snapshot)),
    nodeIds: [...nodeIds].sort(),
    keyedNotifications: keyedListener.mock.calls.length,
    requestedNodeViews: requestedNodeViews.toString(),
    materializedNodeViews: engine.store.metrics().materializedNodeViews,
  };
  unsubscribeRoot();
  for (const unsubscribe of nodeSubscriptions.values()) {
    unsubscribe();
  }
  engine.close();
  return result;
}

function observeChangedNodes(
  engine: MdstreamEngine,
  result: EngineResult,
  nodeIds: Set<NodeId>,
  subscriptions: Map<NodeId, () => void>,
  listener: () => void,
  recordMaterialization: () => void,
): void {
  for (const reducerResult of result.reducerResults) {
    for (const update of reducerResult.updates) {
      for (const id of update.impact.changedNodeIds) {
        const view = engine.store.getNodeSnapshot(id);
        if (view === undefined) {
          nodeIds.delete(id);
          subscriptions.get(id)?.();
          subscriptions.delete(id);
          continue;
        }
        recordMaterialization();
        nodeIds.add(id);
        if (!subscriptions.has(id)) {
          subscriptions.set(id, engine.store.subscribeNode(id, listener));
        }
      }
    }
  }
}

function loadAdoptionFixture(): AdoptionFixture {
  return JSON.parse(
    readFileSync(
      resolve(
        process.cwd(),
        "../../conformance/fixtures/adoption/headless-rich-content.json",
      ),
      "utf8",
    ),
  ) as AdoptionFixture;
}
