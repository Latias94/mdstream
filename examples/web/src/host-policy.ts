import type {
  Epoch,
  NodeId,
  NodeView,
  PendingSourceView,
  TransitionBatchView,
} from "@mdstream/core";

export type PresentationMode = "immediate" | "paced";

export type PresentationState =
  | "fresh"
  | "present"
  | "corrected"
  | "stabilized"
  | "structured"
  | "removed"
  | "replaced";

export type HostPolicyEventKind =
  | "append"
  | "pending"
  | "pending-catch-up"
  | "correction"
  | "stabilization"
  | "structure"
  | "removal"
  | "replacement"
  | "lifecycle"
  | "interrupted"
  | "no-change";

export interface HostPolicyEvent {
  readonly id: number;
  readonly kind: HostPolicyEventKind;
  readonly message: string;
  readonly nodeKey?: string;
}

export interface HostPolicyStore {
  getNodeSnapshot(id: NodeId): NodeView | undefined;
  getPendingSourceSnapshot(): PendingSourceView | undefined;
}

interface QueueEntry {
  readonly key: string;
  readonly nodeId: NodeId;
  readonly grapheme: string;
}

interface TextPiece {
  readonly text: string;
  readonly fresh: boolean;
}

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

export class HostPresentationPolicy {
  readonly mode: PresentationMode;
  #reducedMotion: boolean;
  readonly #displayedSource = new SourceIntervalSet();
  readonly #caughtUpSource = new SourceIntervalSet();
  readonly #textByKey = new Map<string, string>();
  readonly #nodeKeys = new Map<NodeId, string>();
  readonly #states = new Map<string, PresentationState>();
  readonly #queue: QueueEntry[] = [];
  readonly #listeners = new Set<(changedNodes: readonly NodeId[] | null) => void>();
  readonly #dirtyNodes = new Set<NodeId>();
  readonly #events: HostPolicyEvent[] = [];
  #continuityGeneration = "0";
  #epoch = "1";
  #eventSequence = 0;
  #pendingPresentedBytes = 0;
  #pendingCatchUpBytes = 0;

  constructor(mode: PresentationMode, reducedMotion: boolean) {
    this.mode = mode;
    this.#reducedMotion = reducedMotion;
  }

  get reducedMotion(): boolean {
    return this.#reducedMotion;
  }

  get events(): readonly HostPolicyEvent[] {
    return Object.freeze([...this.#events]);
  }

  get queuedGraphemes(): number {
    return this.#queue.length;
  }

  get pendingPresentedBytes(): number {
    return this.#pendingPresentedBytes;
  }

  get pendingCatchUpBytes(): number {
    return this.#pendingCatchUpBytes;
  }

  subscribe(listener: (changedNodes: readonly NodeId[] | null) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  setReducedMotion(reducedMotion: boolean): void {
    if (this.#reducedMotion === reducedMotion) {
      return;
    }
    this.#reducedMotion = reducedMotion;
    if (reducedMotion) {
      this.drain();
    }
  }

  consume(store: HostPolicyStore, batch: TransitionBatchView): void {
    if (batch.facts.length === 0) {
      this.#record("no-change", "The operation preserved host continuity.");
      this.#notify(false);
      return;
    }

    let fullRefresh = false;
    for (const facts of batch.facts) {
      this.#continuityGeneration = facts.after.continuityGeneration as string;
      this.#epoch = facts.after.coordinate.epoch as string;
      if (facts.scope === "full_replace") {
        this.#queue.length = 0;
        this.#displayedSource.clear();
        this.#caughtUpSource.clear();
        this.#textByKey.clear();
        this.#nodeKeys.clear();
        this.#states.clear();
        fullRefresh = true;
        this.#record(
          "replacement",
          `Continuity replaced at generation ${this.#continuityGeneration}.`,
        );
        continue;
      }

      if (facts.before?.lifecycle !== facts.after.lifecycle) {
        this.#record(
          "lifecycle",
          facts.after.lifecycle === "finalized"
            ? "Canonical content finalized."
            : "Canonical content opened.",
        );
      }

      this.#recordCatchUpRange(
        facts.before?.projectionCursor as string | undefined ?? "0",
        facts.after.projectionCursor as string,
      );

      for (const node of facts.nodes) {
        const key = transitionNodeKey(node.key);
        this.#nodeKeys.set(node.key.nodeId, key);
        this.#dirtyNodes.add(node.key.nodeId);
        if (node.after === null) {
          this.#dropNode(key);
          this.#states.set(key, "removed");
          this.#record("removal", "A canonical node was removed.", key);
          continue;
        }

        if (node.text?.kind === "projection_append") {
          this.#appendProjection(store, node.key.nodeId, key, node.text);
        } else if (node.text?.kind === "replacement") {
          this.#dropQueuedKey(key);
          const body = store.getNodeSnapshot(node.key.nodeId)?.bodyText;
          if (body !== undefined) {
            this.#textByKey.set(key, body);
          }
          this.#states.set(key, "corrected");
          this.#record("correction", "Previously presented content was corrected.", key);
        } else if (node.before === null) {
          this.#states.set(key, "fresh");
        }

        if (
          node.before !== null &&
          node.before.stability !== node.after.stability &&
          node.after.stability === "stable"
        ) {
          this.#states.set(key, "stabilized");
          this.#record("stabilization", "A provisional node became stable.", key);
        }
      }

      for (const structure of facts.structures) {
        const affected = [...structure.inserted, ...structure.removed];
        for (const key of affected) {
          const encoded = transitionNodeKey(key);
          this.#nodeKeys.set(key.nodeId, encoded);
          this.#states.set(encoded, "structured");
          this.#dirtyNodes.add(key.nodeId);
        }
        this.#record("structure", "The canonical child structure changed.");
      }
      for (const resource of facts.resources) {
        for (const affected of resource.affectedNodes) {
          const key = transitionNodeKey(affected);
          this.#nodeKeys.set(affected.nodeId, key);
          this.#states.set(key, "corrected");
          this.#dirtyNodes.add(affected.nodeId);
        }
        this.#record(
          "correction",
          "A semantic resource changed the meaning of affected content.",
        );
      }
    }
    this.#notify(fullRefresh);
  }

  observePending(store: HostPolicyStore): PendingSourceView | undefined {
    const pending = store.getPendingSourceSnapshot();
    if (pending === undefined || pending.text.length === 0) {
      return undefined;
    }
    const freshBytes = this.#displayedSource.add(
      pending.range.start as string,
      pending.range.end as string,
    );
    if (freshBytes > 0) {
      this.#pendingPresentedBytes += freshBytes;
      this.#record(
        "pending",
        `${freshBytes} pending source byte${freshBytes === 1 ? "" : "s"} presented once.`,
      );
      this.#notify(false);
    }
    return pending;
  }

  displayText(nodeId: NodeId, canonicalBodyText: string): string {
    const key = this.#nodeKeys.get(nodeId) ?? this.nodeKey(nodeId, this.#epoch);
    return this.#textByKey.get(key) ?? canonicalBodyText;
  }

  nodeKey(nodeId: NodeId, epoch: Epoch | string): string {
    return `${this.#continuityGeneration}:${epoch as string}:${nodeId as string}`;
  }

  stateForNode(nodeId: NodeId): PresentationState {
    const key = this.#nodeKeys.get(nodeId) ?? this.nodeKey(nodeId, this.#epoch);
    return this.#states.get(key) ?? "present";
  }

  advance(graphemes = 1): number {
    let delivered = 0;
    while (delivered < graphemes) {
      const next = this.#queue.shift();
      if (next === undefined) {
        break;
      }
      this.#textByKey.set(next.key, (this.#textByKey.get(next.key) ?? "") + next.grapheme);
      this.#states.set(next.key, "fresh");
      this.#dirtyNodes.add(next.nodeId);
      delivered += 1;
    }
    if (delivered > 0) {
      this.#notify(false);
    }
    return delivered;
  }

  drain(): void {
    if (this.#queue.length === 0) {
      return;
    }
    this.advance(this.#queue.length);
  }

  interrupt(): void {
    this.#queue.length = 0;
    this.#record("interrupted", "Replay interrupted; queued presentation was discarded.");
    this.#notify(false);
  }

  #appendProjection(
    store: HostPolicyStore,
    nodeId: NodeId,
    key: string,
    transition: {
      readonly range: { readonly start: string; readonly end: string };
      readonly text: string;
    },
  ): void {
    const canonical = store.getNodeSnapshot(nodeId)?.bodyText ?? transition.text;
    let current = this.#textByKey.get(key);
    if (current === undefined) {
      current = canonical.endsWith(transition.text)
        ? canonical.slice(0, canonical.length - transition.text.length)
        : "";
    }
    this.#textByKey.set(key, current);

    const partition = this.#displayedSource.partition(
      transition.range.start,
      transition.range.end,
      transition.text,
    );
    this.#recordCatchUpRange(transition.range.start, transition.range.end, key);
    this.#displayedSource.add(transition.range.start, transition.range.end);

    for (const piece of partition.pieces) {
      if (!piece.fresh || this.mode === "immediate" || this.#reducedMotion) {
        this.#textByKey.set(key, (this.#textByKey.get(key) ?? "") + piece.text);
        continue;
      }
      for (const { segment } of segmenter.segment(piece.text)) {
        this.#queue.push({ key, nodeId, grapheme: segment });
      }
    }
    this.#states.set(key, "fresh");
    this.#record("append", "Fresh canonical text appended.", key);
  }

  #recordCatchUpRange(start: string, end: string, key?: string): void {
    let newlyCaughtUp = 0;
    for (const intersection of this.#displayedSource.intersections(start, end)) {
      newlyCaughtUp += this.#caughtUpSource.add(intersection.start, intersection.end);
    }
    if (newlyCaughtUp === 0) {
      return;
    }
    this.#pendingCatchUpBytes += newlyCaughtUp;
    this.#record(
      "pending-catch-up",
      `${newlyCaughtUp} already-painted byte${
        newlyCaughtUp === 1 ? "" : "s"
      } moved into canonical projection without a second reveal.`,
      key,
    );
  }

  #dropNode(key: string): void {
    this.#dropQueuedKey(key);
    this.#textByKey.delete(key);
    for (const [nodeId, candidate] of this.#nodeKeys) {
      if (candidate === key) {
        this.#nodeKeys.delete(nodeId);
      }
    }
  }

  #dropQueuedKey(key: string): void {
    for (let index = this.#queue.length - 1; index >= 0; index -= 1) {
      if (this.#queue[index]?.key === key) {
        this.#queue.splice(index, 1);
      }
    }
  }

  #record(kind: HostPolicyEventKind, message: string, nodeKey?: string): void {
    const event = nodeKey === undefined
      ? { id: ++this.#eventSequence, kind, message }
      : { id: ++this.#eventSequence, kind, message, nodeKey };
    this.#events.push(Object.freeze(event));
  }

  #notify(fullRefresh: boolean): void {
    const changed = fullRefresh ? null : Object.freeze([...this.#dirtyNodes]);
    this.#dirtyNodes.clear();
    for (const listener of [...this.#listeners]) {
      listener(changed);
    }
  }
}

class SourceIntervalSet {
  readonly #intervals: { start: bigint; end: bigint }[] = [];

  clear(): void {
    this.#intervals.length = 0;
  }

  add(startValue: string, endValue: string): number {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    if (end <= start) {
      return 0;
    }
    const previouslyCovered = this.#coveredBytes(start, end);
    const next: { start: bigint; end: bigint }[] = [];
    let mergedStart = start;
    let mergedEnd = end;
    let inserted = false;
    for (const interval of this.#intervals) {
      if (interval.end < mergedStart) {
        next.push(interval);
      } else if (mergedEnd < interval.start) {
        if (!inserted) {
          next.push({ start: mergedStart, end: mergedEnd });
          inserted = true;
        }
        next.push(interval);
      } else {
        mergedStart = min(mergedStart, interval.start);
        mergedEnd = max(mergedEnd, interval.end);
      }
    }
    if (!inserted) {
      next.push({ start: mergedStart, end: mergedEnd });
    }
    this.#intervals.splice(0, this.#intervals.length, ...next);
    return Number(end - start - previouslyCovered);
  }

  partition(startValue: string, endValue: string, text: string): {
    readonly pieces: readonly TextPiece[];
    readonly alreadyDisplayedBytes: number;
  } {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    const bytes = encoder.encode(text);
    if (BigInt(bytes.byteLength) !== end - start) {
      throw new RangeError("projection text must cover its declared UTF-8 source range");
    }

    const pieces: TextPiece[] = [];
    let offset = 0;
    let alreadyDisplayedBytes = 0;
    while (offset < bytes.byteLength) {
      const fresh = !this.#contains(start + BigInt(offset));
      let boundary = offset + 1;
      while (
        boundary < bytes.byteLength &&
        !this.#contains(start + BigInt(boundary)) === fresh
      ) {
        boundary += 1;
      }
      const value = decoder.decode(bytes.slice(offset, boundary));
      pieces.push({ text: value, fresh });
      if (!fresh) {
        alreadyDisplayedBytes += boundary - offset;
      }
      offset = boundary;
    }
    return { pieces, alreadyDisplayedBytes };
  }

  intersections(startValue: string, endValue: string): readonly {
    readonly start: string;
    readonly end: string;
  }[] {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    return this.#intervals.flatMap((interval) => {
      const overlapStart = max(start, interval.start);
      const overlapEnd = min(end, interval.end);
      return overlapEnd > overlapStart
        ? [{ start: overlapStart.toString(), end: overlapEnd.toString() }]
        : [];
    });
  }

  #contains(cursor: bigint): boolean {
    return this.#intervals.some(({ start, end }) => start <= cursor && cursor < end);
  }

  #coveredBytes(start: bigint, end: bigint): bigint {
    let covered = 0n;
    for (const interval of this.#intervals) {
      const overlapStart = max(start, interval.start);
      const overlapEnd = min(end, interval.end);
      if (overlapEnd > overlapStart) {
        covered += overlapEnd - overlapStart;
      }
    }
    return covered;
  }
}

function transitionNodeKey(key: {
  readonly continuityGeneration: string;
  readonly epoch: string;
  readonly nodeId: NodeId;
}): string {
  return `${key.continuityGeneration}:${key.epoch}:${key.nodeId as string}`;
}

function min(left: bigint, right: bigint): bigint {
  return left < right ? left : right;
}

function max(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
}
