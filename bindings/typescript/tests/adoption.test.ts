import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it, vi } from "vitest";

import {
  initMdstream,
  type EngineResult,
  type MdstreamEngine,
  type MdstreamRuntime,
  type MdstreamSessionOptions,
  type NodeId,
  type ProcessorOutput,
  type ProcessorRequestView,
  type TransitionBatchView,
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

const capturedPartOptions = {
  captureTransitions: true,
  protocol: {
    maxSourceBytes: "65536",
    maxNodes: "256",
    maxResources: "64",
    maxOperations: "1024",
    maxChangeStructuralItems: "2048",
    maxChildrenPerList: "256",
  },
} satisfies MdstreamSessionOptions;

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

  it("keeps AI message-part sessions host-owned and generation-qualified", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const parts = new PartSessionRegistry(runtime);
    const first = parts.createMarkdown("answer:first");
    parts.append(first, "First answer");
    const tool = parts.createTool("tool:weather", { city: "Shanghai" });
    const second = parts.createMarkdown("answer:second");
    parts.append(second, "Second");
    parts.finish(first);

    expect(parts.order()).toEqual([first, tool, second]);
    expect(parts.markdownState(first)).toMatchObject({
      source: "First answer",
      lifecycle: "finalized",
    });
    expect(parts.markdownState(second)).toMatchObject({
      source: "Second",
      lifecycle: "open",
    });

    parts.reorder([second, tool, first]);
    expect(parts.order()).toEqual([second, tool, first]);
    expect(parts.toolValue(tool)).toEqual({ city: "Shanghai" });

    const siblingCommands = parts.engine(second).store.metrics().commands;
    parts.replaceHistory(first, ["Rewritten ", "answer"], true);
    expect(parts.markdownState(first)).toMatchObject({
      source: "Rewritten answer",
      lifecycle: "finalized",
    });
    expect(parts.markdownState(second).source).toBe("Second");
    expect(parts.engine(second).store.metrics().commands).toBe(siblingCommands);
    expect(parts.transitionBatches(first).some(({ facts }) =>
      facts.some(({ scope }) => scope === "full_replace")
    )).toBe(true);

    const pending: {
      signal: AbortSignal;
      resolve: (output: ProcessorOutput) => void;
    }[] = [];
    parts.engine(first).registerProcessor({
      descriptor: { id: "adoption.part.pending", version: "v1" },
      configurationVersion: "adoption.part.pending.default",
      matches: (node) => node.content.kind === "paragraph",
      process(_request, context) {
        return new Promise<ProcessorOutput>((resolve) => {
          pending.push({ signal: context.signal, resolve });
        });
      },
    });
    await vi.waitFor(() => expect(pending).toHaveLength(1));

    parts.remove(first);
    expect(pending[0]!.signal.aborted).toBe(true);
    parts.append(second, " answer");
    parts.finish(second);
    expect(parts.markdownState(second).source).toBe("Second answer");
    expect(parts.toolValue(tool)).toEqual({ city: "Shanghai" });

    const reused = parts.createMarkdown("answer:first");
    expect(reused.generation).toBeGreaterThan(first.generation);
    parts.append(reused, "New incarnation");
    let acceptedLateCallback = false;
    expect(parts.acceptCallback(first, () => {
      acceptedLateCallback = true;
    })).toBe(false);
    expect(acceptedLateCallback).toBe(false);
    expect(parts.acceptCallback(reused, () => undefined)).toBe(true);
    pending[0]!.resolve({
      kind: "text",
      protocol: "adoption.part.late/1",
      mediaType: "text/plain",
      text: "late old generation",
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(parts.markdownState(reused).source).toBe("New incarnation");

    parts.close();
  });

  it("keeps SVG artifacts outside transition facts and crosses an explicit trust boundary", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine(capturedPartOptions);
    const batches: TransitionBatchView[] = [];
    const requests: ProcessorRequestView[] = [];
    engine.store.subscribeTransitions((batch) => batches.push(batch));
    engine.registerProcessor({
      descriptor: {
        id: "adoption.svg.mermaid",
        version: "v1",
        acceptsProvisional: true,
      },
      configurationVersion: "adoption.svg.mermaid.default",
      allowProvisional: true,
      matches: (node) =>
        node.content.kind === "code_block" && node.content.info === "mermaid",
      process(request) {
        requests.push(request);
        return {
          kind: "text",
          protocol: "mdstream.merman.svg/1",
          mediaType: "image/svg+xml",
          text: "<svg role=\"img\"><text>graph</text></svg>",
        };
      },
    });

    engine.append("```mermaid\ngraph TD\nA-->B\n```");
    engine.finish();
    await engine.whenProcessorsIdle();

    expect(requests.length).toBeGreaterThan(0);
    const request = requests.at(-1)!;
    const artifact = engine.store.getArtifactSnapshot({
      epoch: request.key.epoch,
      nodeId: request.key.nodeId,
      processorId: "adoption.svg.mermaid",
    });
    expect(artifact?.artifact).toMatchObject({
      protocol: "mdstream.merman.svg/1",
      mediaType: "image/svg+xml",
      payload: { kind: "text" },
    });
    const transitionWire = JSON.stringify(batches.flatMap(({ facts }) => facts));
    expect(transitionWire).not.toContain("mdstream.merman.svg/1");
    expect(transitionWire).not.toContain("<svg");
    expect(batches.some(({ facts }) => facts.length === 0)).toBe(true);

    const payload = artifact?.artifact?.payload;
    if (payload?.kind !== "text") {
      throw new Error("SVG processor did not produce a text artifact");
    }
    const handoff = sanitizeOrIsolateSvg(payload.text);
    expect(handoff.kind).toBe("isolated_svg_handoff");
    expect(handoff.utf8.byteLength).toBeGreaterThan(0);
    const canonical = textDecoder.decode(engine.createRecoverySnapshot()!);
    expect(canonical).not.toContain("mdstream.merman.svg/1");
    expect(canonical).not.toContain("<svg");
    engine.close();
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

type PartKind = "markdown" | "tool";

interface PartReference<Kind extends PartKind = PartKind> {
  readonly partKey: string;
  readonly generation: number;
  readonly kind: Kind;
}

interface MarkdownPartSession {
  readonly kind: "markdown";
  readonly reference: PartReference<"markdown">;
  readonly engine: MdstreamEngine;
  readonly transitionBatches: TransitionBatchView[];
}

interface ToolPartSession {
  readonly kind: "tool";
  readonly reference: PartReference<"tool">;
  readonly value: unknown;
}

type PartSession = MarkdownPartSession | ToolPartSession;

class PartSessionRegistry {
  readonly #runtime: MdstreamRuntime;
  readonly #sessions = new Map<string, PartSession>();
  readonly #lastGeneration = new Map<string, number>();
  #ordered: PartReference[] = [];

  constructor(runtime: MdstreamRuntime) {
    this.#runtime = runtime;
  }

  createMarkdown(
    partKey: string,
  ): PartReference<"markdown"> {
    const reference = this.#newReference(partKey, "markdown");
    const engine = this.#runtime.createEngine(capturedPartOptions);
    const transitionBatches: TransitionBatchView[] = [];
    engine.store.subscribeTransitions((batch) => {
      transitionBatches.push(batch);
    });
    this.#sessions.set(partKey, {
      kind: "markdown",
      reference,
      engine,
      transitionBatches,
    });
    this.#ordered.push(reference);
    return reference;
  }

  createTool(
    partKey: string,
    value: unknown,
  ): PartReference<"tool"> {
    const reference = this.#newReference(partKey, "tool");
    this.#sessions.set(partKey, { kind: "tool", reference, value });
    this.#ordered.push(reference);
    return reference;
  }

  append(reference: PartReference, chunk: string): void {
    this.#markdown(reference).engine.append(chunk);
  }

  finish(reference: PartReference): void {
    this.#markdown(reference).engine.finish();
  }

  replaceHistory(
    reference: PartReference,
    chunks: readonly string[],
    finish: boolean,
  ): void {
    const engine = this.#markdown(reference).engine;
    engine.reset();
    for (const chunk of chunks) {
      engine.append(chunk);
    }
    if (finish) {
      engine.finish();
    }
  }

  reorder(references: readonly PartReference[]): void {
    if (references.length !== this.#sessions.size) {
      throw new TypeError("part reorder must include every live part exactly once");
    }
    const seen = new Set<string>();
    for (const reference of references) {
      this.#current(reference);
      if (seen.has(reference.partKey)) {
        throw new TypeError("part reorder contains a duplicate key");
      }
      seen.add(reference.partKey);
    }
    this.#ordered = [...references];
  }

  order(): readonly PartReference[] {
    return Object.freeze([...this.#ordered]);
  }

  engine(reference: PartReference): MdstreamEngine {
    return this.#markdown(reference).engine;
  }

  markdownState(reference: PartReference): {
    readonly source: string;
    readonly lifecycle: "open" | "finalized";
  } {
    return readMarkdownState(this.#markdown(reference).engine);
  }

  transitionBatches(reference: PartReference): readonly TransitionBatchView[] {
    return Object.freeze([...this.#markdown(reference).transitionBatches]);
  }

  toolValue(reference: PartReference): unknown {
    const session = this.#current(reference);
    if (session.kind !== "tool") {
      throw new TypeError("part is not a tool part");
    }
    return session.value;
  }

  remove(reference: PartReference): void {
    const session = this.#current(reference);
    if (session.kind === "markdown") {
      session.engine.close();
    }
    this.#sessions.delete(reference.partKey);
    this.#ordered = this.#ordered.filter(
      ({ partKey }) => partKey !== reference.partKey,
    );
  }

  acceptCallback(reference: PartReference, callback: () => void): boolean {
    const session = this.#sessions.get(reference.partKey);
    if (
      session === undefined ||
      session.reference.generation !== reference.generation ||
      session.reference.kind !== reference.kind
    ) {
      return false;
    }
    callback();
    return true;
  }

  close(): void {
    for (const session of this.#sessions.values()) {
      if (session.kind === "markdown") {
        session.engine.close();
      }
    }
    this.#sessions.clear();
    this.#ordered = [];
  }

  #newReference<Kind extends PartKind>(
    partKey: string,
    kind: Kind,
  ): PartReference<Kind> {
    if (partKey.length === 0 || this.#sessions.has(partKey)) {
      throw new TypeError("part key must be non-empty and unique among live parts");
    }
    const generation = (this.#lastGeneration.get(partKey) ?? 0) + 1;
    if (!Number.isSafeInteger(generation)) {
      throw new RangeError("part generation overflow");
    }
    this.#lastGeneration.set(partKey, generation);
    return Object.freeze({ partKey, generation, kind });
  }

  #current(reference: PartReference): PartSession {
    const session = this.#sessions.get(reference.partKey);
    if (
      session === undefined ||
      session.reference.generation !== reference.generation ||
      session.kind !== reference.kind
    ) {
      throw new TypeError("part reference is stale");
    }
    return session;
  }

  #markdown(reference: PartReference): MarkdownPartSession {
    const session = this.#current(reference);
    if (session.kind !== "markdown") {
      throw new TypeError("part is not a markdown part");
    }
    return session;
  }
}

function readMarkdownState(engine: MdstreamEngine): {
  readonly source: string;
  readonly lifecycle: "open" | "finalized";
} {
  const recovery = engine.createRecoverySnapshot();
  if (recovery === undefined) {
    throw new Error("markdown part has no canonical state");
  }
  const decoded = decodeJson(recovery);
  if (
    typeof decoded !== "object" ||
    decoded === null ||
    !("source" in decoded) ||
    typeof decoded.source !== "string"
  ) {
    throw new Error("markdown part snapshot has no source");
  }
  const lifecycle = engine.store.getSnapshot().document?.lifecycle;
  if (lifecycle === undefined) {
    throw new Error("markdown part has no document state");
  }
  return { source: decoded.source, lifecycle };
}

interface IsolatedSvgHandoff {
  readonly kind: "isolated_svg_handoff";
  readonly utf8: Uint8Array;
}

function sanitizeOrIsolateSvg(source: string): IsolatedSvgHandoff {
  if (!source.startsWith("<svg") || !source.endsWith("</svg>")) {
    throw new TypeError("processor artifact is not an SVG document");
  }
  return Object.freeze({
    kind: "isolated_svg_handoff",
    utf8: new TextEncoder().encode(source),
  });
}
