import { describe, expect, it } from "vitest";

import {
  HostPresentationPolicy,
  type HostPolicyStore,
} from "../src/host-policy.js";
import { classifyExternalUrl } from "../src/url-policy.js";
import type {
  NodeId,
  NodeView,
  PendingSourceView,
  TransitionBatchView,
} from "@mdstream/core";

const nodeId = "7" as NodeId;

describe("host-owned presentation policy", () => {
  it("does not reveal pending bytes again when projection catches up", () => {
    const store = fakeStore("Hello", pendingView("0", "3", "Hel"));
    const policy = new HostPresentationPolicy("paced", false);

    policy.observePending(store);
    policy.consume(store, appendBatch("Hello", "0", "5"));

    expect(policy.pendingPresentedBytes).toBe(3);
    expect(policy.pendingCatchUpBytes).toBe(3);
    expect(policy.queuedGraphemes).toBe(2);
    expect(policy.displayText(nodeId, "Hello")).toBe("Hel");
    policy.drain();
    expect(policy.displayText(nodeId, "Hello")).toBe("Hello");
  });

  it("settles immediate, paced, and reduced-motion policies to equal meaning", () => {
    const store = fakeStore("Stable stream");
    const batch = appendBatch("Stable stream", "0", "13");
    const immediate = new HostPresentationPolicy("immediate", false);
    const paced = new HostPresentationPolicy("paced", false);
    const reduced = new HostPresentationPolicy("paced", true);

    immediate.consume(store, batch);
    paced.consume(store, batch);
    reduced.consume(store, batch);

    expect(paced.queuedGraphemes).toBeGreaterThan(0);
    expect(reduced.queuedGraphemes).toBe(0);
    paced.drain();
    expect(paced.displayText(nodeId, "Stable stream")).toBe(
      immediate.displayText(nodeId, "Stable stream"),
    );
    expect(reduced.displayText(nodeId, "Stable stream")).toBe(
      immediate.displayText(nodeId, "Stable stream"),
    );
  });

  it("clears queued text and continuity-qualified keys on full replacement", () => {
    const store = fakeStore("queued");
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(store, appendBatch("queued", "0", "6"));
    const priorKey = policy.nodeKey(nodeId, "1");

    policy.consume(store, {
      facts: [{
        scope: "full_replace",
        before: documentStamp("0", "1", "1"),
        after: documentStamp("1", "2", "1"),
      }],
    } as unknown as TransitionBatchView);

    expect(policy.queuedGraphemes).toBe(0);
    expect(policy.nodeKey(nodeId, "2")).not.toBe(priorKey);
    expect(policy.events.at(-1)?.kind).toBe("replacement");
  });

  it("preserves eligible keys for an empty same-floor recovery batch", () => {
    const store = fakeStore("hello");
    const policy = new HostPresentationPolicy("immediate", false);
    policy.consume(store, appendBatch("hello", "0", "5"));
    const key = policy.nodeKey(nodeId, "1");

    policy.consume(store, { facts: [] });

    expect(policy.nodeKey(nodeId, "1")).toBe(key);
    expect(policy.events.at(-1)?.kind).toBe("no-change");
  });

  it("treats a pure resource change as a focused semantic correction", () => {
    const store = fakeStore("citation");
    const policy = new HostPresentationPolicy("immediate", false);
    policy.consume(store, appendBatch("citation", "0", "8"));

    policy.consume(store, {
      facts: [{
        scope: "continuous",
        before: documentStamp("0", "1", "1"),
        after: documentStamp("0", "1", "2"),
        nodes: [],
        structures: [],
        resources: [{
          key: { continuityGeneration: "0", epoch: "1", resourceId: "3" },
          beforeVersion: "1",
          afterVersion: "2",
          affectedNodes: [{ continuityGeneration: "0", epoch: "1", nodeId }],
        }],
      }],
    } as unknown as TransitionBatchView);

    expect(policy.stateForNode(nodeId)).toBe("corrected");
    expect(policy.events.at(-1)).toMatchObject({
      kind: "correction",
      message: expect.stringContaining("semantic resource"),
    });
  });
});

describe("example-local citation URL policy", () => {
  it.each([
    ["https://docs.rs/mdstream", "https://docs.rs/mdstream"],
    ["http://localhost:4173/reference", "http://localhost:4173/reference"],
  ])("allows HTTP(S) destinations", (input, expected) => {
    expect(classifyExternalUrl(input)).toEqual({ kind: "link", href: expected });
  });

  it.each([
    "javascript:alert(1)",
    "data:text/html,unsafe",
    "file:///tmp/private",
    "not a url",
  ])("renders %s as inert text", (input) => {
    expect(classifyExternalUrl(input)).toEqual({ kind: "inert", text: input });
  });
});

function fakeStore(
  bodyText: string,
  pending?: PendingSourceView,
): HostPolicyStore {
  const view = {
    schema: "mdstream.bindings/0.4",
    kind: "node_view",
    node: {
      id: nodeId,
      version: "1",
      stability: "provisional",
      source: { start: "0", end: String(new TextEncoder().encode(bodyText).length) },
      body: { start: "0", end: String(new TextEncoder().encode(bodyText).length) },
      children: { version: "1", children: [] },
      content: { kind: "text", text: { kind: "source" } },
    },
    bodyText,
  } as unknown as NodeView;
  return {
    getNodeSnapshot: (id: NodeId) => id === nodeId ? view : undefined,
    getPendingSourceSnapshot: () => pending,
  };
}

function appendBatch(
  text: string,
  start: string,
  end: string,
): TransitionBatchView {
  return {
    facts: [{
      scope: "continuous",
      before: null,
      after: documentStamp("0", "1", "1"),
      nodes: [{
        key: { continuityGeneration: "0", epoch: "1", nodeId },
        before: null,
        after: {
          version: "1",
          stability: "provisional",
          parent: { kind: "document" },
          childrenVersion: "1",
        },
        text: {
          kind: "projection_append",
          range: { start, end },
          text,
        },
      }],
      structures: [],
      resources: [],
    }],
  } as unknown as TransitionBatchView;
}

function documentStamp(
  continuityGeneration: string,
  epoch: string,
  sequence: string,
) {
  return {
    continuityGeneration,
    coordinate: {
      epoch,
      sequence,
      changeId: `test:${sequence}`,
      sourceCursor: "0",
    },
    lifecycle: "open" as const,
    projectionCursor: "0",
    rootsVersion: "1",
  };
}

function pendingView(start: string, end: string, text: string): PendingSourceView {
  return {
    schema: "mdstream.bindings/0.4",
    kind: "pending_source_view",
    range: { start, end },
    text,
  } as unknown as PendingSourceView;
}
