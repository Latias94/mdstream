import { describe, expect, it, vi } from "vitest";

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
  it("commits causally earlier paced text before pending catch-up", () => {
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(fakeStore("abc"), appendBatch("abc", "0", "3"));
    policy.observePending(fakeStore("abcdef", pendingView("3", "6", "def")));

    policy.consume(fakeStore("abcdef"), appendBatch("def", "3", "6"));

    expect(policy.displayText(nodeId, "abcdef")).toBe("abcdef");
    expect(policy.queuedGraphemes).toBe(0);
    expect(policy.stateForNode(nodeId)).toBe("present");
    expect(policy.deliveriesSince(0).map(({ origin, text, animationEligible }) => ({
      origin,
      text,
      animationEligible,
    }))).toEqual([
      { origin: "forced-paced-prefix", text: "a", animationEligible: true },
      { origin: "forced-paced-prefix", text: "b", animationEligible: true },
      { origin: "forced-paced-prefix", text: "c", animationEligible: true },
      { origin: "pending-catch-up", text: "d", animationEligible: false },
      { origin: "pending-catch-up", text: "e", animationEligible: false },
      { origin: "pending-catch-up", text: "f", animationEligible: false },
    ]);
    expect(policy.textRuns(nodeId, "0", "abcdef")).toEqual([
      {
        text: "abc",
        range: { start: "0", end: "3" },
        animationEligible: true,
        animationSequence: 3,
      },
      {
        text: "def",
        range: { start: "3", end: "6" },
        animationEligible: false,
        animationSequence: null,
      },
    ]);
  });

  it("derives the initial prefix from every append in one operation", () => {
    const policy = new HostPresentationPolicy("paced", false);

    policy.consume(fakeStore("xaa"), appendFactsBatch([
      { text: "a", start: "1", end: "2" },
      { text: "a", start: "2", end: "3" },
    ]));
    policy.drain();

    expect(policy.displayText(nodeId, "xaa")).toBe("xaa");
  });

  it("leaves causally later text paced after pending catch-up", () => {
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(fakeStore("abcdefghi"), appendBatch("abc", "0", "3"));
    policy.consume(fakeStore("abcdefghi"), appendBatch("ghi", "6", "9"));
    policy.observePending(fakeStore("abcdefghi", pendingView("3", "6", "def")));

    policy.consume(fakeStore("abcdefghi"), appendBatch("def", "3", "6"));

    expect(policy.displayText(nodeId, "abcdefghi")).toBe("abcdef");
    expect(policy.queuedGraphemes).toBe(3);
    policy.drain();
    expect(policy.displayText(nodeId, "abcdefghi")).toBe("abcdefghi");
  });

  it("orders multiple nodes by source position rather than node id", () => {
    const earlyNode = "90" as NodeId;
    const catchUpNode = "50" as NodeId;
    const lateNode = "2" as NodeId;
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(
      fakeStoreFor(new Map([[earlyNode, "A"]]), new Map([[earlyNode, "0"]])),
      appendBatchFor(earlyNode, "A", "0", "1"),
    );
    policy.consume(
      fakeStoreFor(new Map([[lateNode, "C"]]), new Map([[lateNode, "2"]])),
      appendBatchFor(lateNode, "C", "2", "3"),
    );
    policy.observePending(fakeStore("", pendingView("1", "2", "B")));

    policy.consume(
      fakeStoreFor(
        new Map([[catchUpNode, "B"]]),
        new Map([[catchUpNode, "1"]]),
      ),
      appendBatchFor(catchUpNode, "B", "1", "2"),
    );

    expect(policy.displayText(earlyNode, "A")).toBe("A");
    expect(policy.displayText(catchUpNode, "B")).toBe("B");
    expect(policy.displayText(lateNode, "C")).toBe("");
    expect(policy.deliveriesSince(0).map((record) => record.nodeId)).toEqual([
      earlyNode,
      catchUpNode,
    ]);
    policy.drain();
    expect(policy.displayText(lateNode, "C")).toBe("C");
  });

  it("records exact UTF-8 intervals for intact grapheme delivery", () => {
    const text = "🙂e\u0301";
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(fakeStore(text), appendBatch(text, "0", "7"));

    expect(policy.advance()).toBe(1);
    expect(policy.displayText(nodeId, text)).toBe("🙂");
    expect(policy.deliveriesSince(0)).toMatchObject([{
      sequence: 1,
      nodeKey: "0:1:7",
      range: { start: "0", end: "4" },
      text: "🙂",
      origin: "fresh-projection",
      animationEligible: true,
    }]);

    policy.drain();
    expect(policy.displayText(nodeId, text)).toBe(text);
    expect(policy.deliveriesSince(1)[0]).toMatchObject({
      sequence: 2,
      range: { start: "4", end: "7" },
      text: "e\u0301",
    });
  });

  it("retains a trailing grapheme across contiguous appends", () => {
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(fakeStore("e"), appendBatch("e", "0", "1"));
    expect(policy.advance()).toBe(0);

    policy.consume(fakeStore("e\u0301"), appendBatch("\u0301", "1", "3"));
    expect(policy.advance()).toBe(0);
    policy.consume(fakeStore("e\u0301x"), appendBatch("x", "3", "4"));

    expect(policy.advance()).toBe(1);
    expect(policy.displayText(nodeId, "e\u0301x")).toBe("e\u0301");
    expect(policy.deliveriesSince(0)[0]).toMatchObject({
      text: "e\u0301",
      range: { start: "0", end: "3" },
    });
    policy.drain();
    expect(policy.displayText(nodeId, "e\u0301x")).toBe("e\u0301x");
  });

  it("assigns one non-animated delivery to a grapheme spanning pending coverage", () => {
    const text = "e\u0301x";
    const policy = new HostPresentationPolicy("paced", false);
    policy.observePending(fakeStore(text, pendingView("0", "1", "e")));

    policy.consume(fakeStore(text), appendBatch(text, "0", "4"));

    expect(policy.deliveriesSince(0)[0]).toMatchObject({
      text: "e\u0301",
      range: { start: "0", end: "3" },
      origin: "pending-catch-up",
      animationEligible: false,
    });
    expect(policy.displayText(nodeId, text)).toBe("e\u0301");
    expect(policy.textRuns(nodeId, "0", text)).toEqual([{
      text: "e\u0301",
      range: { start: "0", end: "3" },
      animationEligible: false,
      animationSequence: null,
    }]);
  });

  it("keeps a ZWJ sequence intact when pending coverage ends inside it", () => {
    const emoji = "👩‍💻";
    const text = `${emoji}x`;
    const emojiBytes = String(new TextEncoder().encode(emoji).byteLength);
    const textBytes = String(new TextEncoder().encode(text).byteLength);
    const policy = new HostPresentationPolicy("paced", false);
    policy.observePending(fakeStore(text, pendingView("0", "4", "👩")));

    policy.consume(fakeStore(text), appendBatch(text, "0", textBytes));

    expect(policy.deliveriesSince(0)[0]).toMatchObject({
      text: emoji,
      range: { start: "0", end: emojiBytes },
      origin: "pending-catch-up",
      animationEligible: false,
    });
    expect(policy.displayText(nodeId, text)).toBe(emoji);
  });

  it.each([
    {
      label: "combining-mark",
      base: "e",
      continuation: "\u0301",
      combined: "e\u0301",
    },
    {
      label: "ZWJ",
      base: "👩",
      continuation: "\u200d💻",
      combined: "👩‍💻",
    },
  ])(
    "extends a pending catch-up $label without a fresh split delivery",
    ({ base, continuation, combined }) => {
      const encoder = new TextEncoder();
      const baseEnd = String(encoder.encode(base).byteLength);
      const combinedEnd = String(encoder.encode(combined).byteLength);
      const policy = new HostPresentationPolicy("paced", false);

      policy.observePending(fakeStore(base, pendingView("0", baseEnd, base)));
      policy.consume(fakeStore(base), appendBatch(base, "0", baseEnd));
      policy.consume(
        fakeStore(combined),
        appendBatch(continuation, baseEnd, combinedEnd),
      );

      expect(policy.deliveriesSince(0)).toEqual([expect.objectContaining({
        range: { start: "0", end: combinedEnd },
        text: combined,
        origin: "pending-catch-up",
        animationEligible: false,
      })]);
      expect(policy.displayText(nodeId, combined)).toBe(combined);
      expect(policy.textRuns(nodeId, "0", combined)).toEqual([{
        text: combined,
        range: { start: "0", end: combinedEnd },
        animationEligible: false,
        animationSequence: null,
      }]);
    },
  );

  it("uses enqueue order when source ranges are identical", () => {
    const firstNode = "90" as NodeId;
    const secondNode = "2" as NodeId;
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(
      fakeStoreFor(new Map([[firstNode, "A"]])),
      appendBatchFor(firstNode, "A", "0", "1"),
    );
    policy.consume(
      fakeStoreFor(new Map([[secondNode, "B"]])),
      appendBatchFor(secondNode, "B", "0", "1"),
    );

    policy.drain();

    expect(policy.deliveriesSince(0).map((record) => record.nodeId)).toEqual([
      firstNode,
      secondNode,
    ]);
  });

  it("seals the final trailing grapheme when the document finalizes", () => {
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(fakeStore("e"), appendBatch("e", "0", "1"));
    expect(policy.advance()).toBe(0);

    policy.consume(fakeStore("e"), finalizeBatch("1"));

    expect(policy.advance()).toBe(1);
    expect(policy.displayText(nodeId, "e")).toBe("e");
  });

  it("drains once without animation when reduced motion is enabled mid-queue", () => {
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(fakeStore("abc"), appendBatch("abc", "0", "3"));
    expect(policy.advance()).toBe(1);

    policy.setReducedMotion(true);

    expect(policy.displayText(nodeId, "abc")).toBe("abc");
    expect(policy.queuedGraphemes).toBe(0);
    expect(policy.deliveriesSince(1).map((record) => record.animationEligible)).toEqual([
      false,
      false,
    ]);
    const settledDeliveryCount = policy.deliveryCount;
    policy.setReducedMotion(true);
    expect(policy.deliveryCount).toBe(settledDeliveryCount);

    policy.setReducedMotion(false);
    policy.consume(fakeStore("abcd"), appendBatch("d", "3", "4"));
    expect(policy.queuedGraphemes).toBe(1);
    expect(policy.displayText(nodeId, "abcd")).toBe("abc");
  });

  it("drops invalidated paced entries on correction and removal", () => {
    const corrected = new HostPresentationPolicy("paced", false);
    corrected.consume(fakeStore("queued"), appendBatch("queued", "0", "6"));
    corrected.consume(fakeStore("fixed"), replacementBatch());

    expect(corrected.queuedGraphemes).toBe(0);
    expect(corrected.displayText(nodeId, "fixed")).toBe("fixed");
    expect(corrected.stateForNode(nodeId)).toBe("corrected");

    const removed = new HostPresentationPolicy("paced", false);
    removed.consume(fakeStore("queued"), appendBatch("queued", "0", "6"));
    removed.consume(fakeStore(""), removalBatch());
    expect(removed.queuedGraphemes).toBe(0);
    expect(removed.stateForNode(nodeId)).toBe("removed");
  });

  it("does not publish stale delivery or animation state invalidated in one batch", () => {
    const policy = new HostPresentationPolicy("immediate", false);

    policy.consume(fakeStore(""), appendThenRemoveBatch("stale"));

    expect(policy.deliveryCount).toBe(0);
    expect(policy.stateForNode(nodeId)).toBe("removed");
  });

  it("discards a delivery invalidated by a later structure fact in one batch", () => {
    const policy = new HostPresentationPolicy("immediate", false);

    policy.consume(fakeStore("stale"), appendThenStructureBatch("stale"));

    expect(policy.deliveryCount).toBe(0);
    expect(policy.stateForNode(nodeId)).toBe("structured");
  });

  it("amortizes queue compaction during one-grapheme advances", () => {
    const policy = new HostPresentationPolicy("paced", false);
    const text = "x".repeat(4_096);
    const copyWithin = vi.spyOn(Array.prototype, "copyWithin");
    try {
      policy.consume(fakeStore(text), appendBatch(text, "0", "4096"));
      for (let index = 0; index < 2_048; index += 1) {
        policy.advance();
      }
      expect(copyWithin.mock.calls.length).toBeLessThanOrEqual(2);
    } finally {
      copyWithin.mockRestore();
    }
  });

  it("defers reentrant advances until every listener observes the current state", () => {
    const policy = new HostPresentationPolicy("paced", false);
    const observed: string[] = [];
    let requested = false;
    policy.subscribe(() => {
      if (!requested) {
        requested = true;
        policy.drain();
      }
    });
    policy.subscribe(() => {
      observed.push(policy.displayText(nodeId, "abc"));
    });

    policy.consume(fakeStore("abc"), appendBatch("abc", "0", "3"));

    expect(observed).toEqual(["", "abc"]);
  });

  it("serializes a reentrant consume after the current listener snapshot", () => {
    const policy = new HostPresentationPolicy("immediate", false);
    const observed: string[] = [];
    let requested = false;
    policy.subscribe(() => {
      if (!requested) {
        requested = true;
        policy.consume(fakeStore("ab"), appendBatch("b", "1", "2"));
      }
    });
    policy.subscribe(() => observed.push(policy.displayText(nodeId, "ab")));

    policy.consume(fakeStore("a"), appendBatch("a", "0", "1"));

    expect(observed).toEqual(["a", "ab"]);
  });

  it("serializes a reentrant interrupt after the current listener snapshot", () => {
    const policy = new HostPresentationPolicy("paced", false);
    const observed: number[] = [];
    let requested = false;
    policy.subscribe(() => {
      if (!requested) {
        requested = true;
        policy.interrupt();
      }
    });
    policy.subscribe(() => observed.push(policy.queuedGraphemes));

    policy.consume(fakeStore("ab"), appendBatch("ab", "0", "2"));

    expect(observed).toEqual([2, 0]);
  });

  it("serializes reentrant pending observation after the current listener snapshot", () => {
    const policy = new HostPresentationPolicy("immediate", false);
    const observed: number[] = [];
    let requested = false;
    policy.subscribe(() => {
      if (!requested) {
        requested = true;
        policy.observePending(fakeStore("", pendingView("0", "1", "x")));
      }
    });
    policy.subscribe(() => observed.push(policy.pendingPresentedBytes));

    policy.consume(fakeStore(""), { facts: [] });

    expect(observed).toEqual([0, 1]);
  });

  it("serializes a reduced-motion change and its drain after the current snapshot", () => {
    const policy = new HostPresentationPolicy("paced", false);
    const observed: { reduced: boolean; text: string }[] = [];
    let requested = false;
    policy.subscribe(() => {
      if (!requested) {
        requested = true;
        policy.setReducedMotion(true);
      }
    });
    policy.subscribe(() => observed.push({
      reduced: policy.reducedMotion,
      text: policy.displayText(nodeId, "ab"),
    }));

    policy.consume(fakeStore("ab"), appendBatch("ab", "0", "2"));

    expect(observed).toEqual([
      { reduced: false, text: "" },
      { reduced: true, text: "ab" },
    ]);
  });

  it("notifies every listener before propagating a listener failure", () => {
    const policy = new HostPresentationPolicy("paced", false);
    let observed = 0;
    policy.subscribe(() => {
      throw new Error("listener failed");
    });
    policy.subscribe(() => {
      observed += 1;
    });

    expect(() => {
      policy.consume(fakeStore("abc"), appendBatch("abc", "0", "3"));
    }).toThrow("listener failed");
    expect(observed).toBe(1);
  });

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
    expect(policy.eventsSince(policy.eventCount - 1)[0]?.kind).toBe("replacement");
  });

  it("preserves eligible keys for an empty same-floor recovery batch", () => {
    const store = fakeStore("hello");
    const policy = new HostPresentationPolicy("immediate", false);
    policy.consume(store, appendBatch("hello", "0", "5"));
    const key = policy.nodeKey(nodeId, "1");

    policy.consume(store, { facts: [] });

    expect(policy.nodeKey(nodeId, "1")).toBe(key);
    expect(policy.eventsSince(policy.eventCount - 1)[0]?.kind).toBe("no-change");
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
    expect(policy.eventsSince(policy.eventCount - 1)[0]).toMatchObject({
      kind: "correction",
      message: expect.stringContaining("semantic resource"),
    });
  });

  it("reports queued graphemes and policy events incrementally", () => {
    const store = fakeStore("abcdef");
    const policy = new HostPresentationPolicy("paced", false);
    policy.consume(store, appendBatch("abcdef", "0", "6"));
    const firstEventCount = policy.eventCount;

    expect(policy.advance(2)).toBe(2);
    expect(policy.queuedGraphemes).toBe(4);
    expect(policy.eventsSince(firstEventCount)).toEqual([]);

    policy.consume(store, { facts: [] });
    expect(policy.eventsSince(firstEventCount)).toHaveLength(1);
    expect(policy.eventsSince(firstEventCount)[0]?.kind).toBe("no-change");

    policy.interrupt();
    expect(policy.queuedGraphemes).toBe(0);
    expect(policy.displayText(nodeId, "abcdef")).toBe("ab");
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

function fakeStoreFor(
  bodyByNode: ReadonlyMap<NodeId, string>,
  bodyStartByNode: ReadonlyMap<NodeId, string> = new Map(),
): HostPolicyStore {
  return {
    getNodeSnapshot: (id: NodeId) => {
      const body = bodyByNode.get(id);
      return body === undefined
        ? undefined
        : nodeView(id, body, bodyStartByNode.get(id) ?? "0");
    },
    getPendingSourceSnapshot: () => undefined,
  };
}

function nodeView(id: NodeId, bodyText: string, bodyStart = "0"): NodeView {
  const byteLength = new TextEncoder().encode(bodyText).length;
  const bodyEnd = (BigInt(bodyStart) + BigInt(byteLength)).toString();
  return {
    schema: "mdstream.bindings/0.4",
    kind: "node_view",
    node: {
      id,
      version: "1",
      stability: "provisional",
      source: { start: bodyStart, end: bodyEnd },
      body: { start: bodyStart, end: bodyEnd },
      children: { version: "1", children: [] },
      content: { kind: "text", text: { kind: "source" } },
    },
    bodyText,
  } as unknown as NodeView;
}

function appendBatch(
  text: string,
  start: string,
  end: string,
): TransitionBatchView {
  return appendBatchFor(nodeId, text, start, end);
}

function appendBatchFor(
  id: NodeId,
  text: string,
  start: string,
  end: string,
): TransitionBatchView {
  return {
    facts: [{
      scope: "continuous",
      before: documentStamp("0", "1", "0", start),
      after: documentStamp("0", "1", "1", end),
      nodes: [{
        key: { continuityGeneration: "0", epoch: "1", nodeId: id },
        before: {
          version: "0",
          stability: "provisional",
          parent: { kind: "document" },
          childrenVersion: "1",
        },
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

function appendFactsBatch(
  appends: readonly {
    readonly text: string;
    readonly start: string;
    readonly end: string;
  }[],
): TransitionBatchView {
  return {
    facts: appends.map(({ text, start, end }, index) => ({
      scope: "continuous",
      before: documentStamp("0", "1", String(index), start),
      after: documentStamp("0", "1", String(index + 1), end),
      nodes: [{
        key: { continuityGeneration: "0", epoch: "1", nodeId },
        before: {
          version: String(index + 1),
          stability: "provisional",
          parent: { kind: "document" },
          childrenVersion: "1",
        },
        after: {
          version: String(index + 2),
          stability: "provisional",
          parent: { kind: "document" },
          childrenVersion: "1",
        },
        text: { kind: "projection_append", range: { start, end }, text },
      }],
      structures: [],
      resources: [],
    })),
  } as unknown as TransitionBatchView;
}

function appendThenRemoveBatch(text: string): TransitionBatchView {
  const append = (appendBatch(text, "0", String(text.length)) as TransitionBatchView)
    .facts[0]!;
  const removal = (removalBatch() as TransitionBatchView).facts[0]!;
  return { facts: [append, removal] } as TransitionBatchView;
}

function appendThenStructureBatch(text: string): TransitionBatchView {
  const append = (appendBatch(text, "0", String(text.length)) as TransitionBatchView)
    .facts[0]!;
  return {
    facts: [append, {
      scope: "continuous",
      before: documentStamp("0", "1", "1"),
      after: documentStamp("0", "1", "2"),
      nodes: [],
      structures: [{
        owner: { kind: "document" },
        beforeVersion: "1",
        afterVersion: "2",
        start: 0,
        removed: [{ continuityGeneration: "0", epoch: "1", nodeId }],
        inserted: [],
      }],
      resources: [],
    }],
  } as unknown as TransitionBatchView;
}

function replacementBatch(): TransitionBatchView {
  return {
    facts: [{
      scope: "continuous",
      before: documentStamp("0", "1", "1"),
      after: documentStamp("0", "1", "2"),
      nodes: [{
        key: { continuityGeneration: "0", epoch: "1", nodeId },
        before: { version: "1", stability: "provisional" },
        after: { version: "2", stability: "provisional" },
        text: { kind: "replacement" },
      }],
      structures: [],
      resources: [],
    }],
  } as unknown as TransitionBatchView;
}

function removalBatch(): TransitionBatchView {
  return {
    facts: [{
      scope: "continuous",
      before: documentStamp("0", "1", "1"),
      after: documentStamp("0", "1", "2"),
      nodes: [{
        key: { continuityGeneration: "0", epoch: "1", nodeId },
        before: { version: "1", stability: "provisional" },
        after: null,
      }],
      structures: [],
      resources: [],
    }],
  } as unknown as TransitionBatchView;
}

function finalizeBatch(cursor: string): TransitionBatchView {
  return {
    facts: [{
      scope: "continuous",
      before: documentStamp("0", "1", "1", cursor),
      after: {
        ...documentStamp("0", "1", "2", cursor),
        lifecycle: "finalized",
      },
      nodes: [],
      structures: [],
      resources: [],
    }],
  } as unknown as TransitionBatchView;
}

function documentStamp(
  continuityGeneration: string,
  epoch: string,
  sequence: string,
  projectionCursor = "0",
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
    projectionCursor,
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
