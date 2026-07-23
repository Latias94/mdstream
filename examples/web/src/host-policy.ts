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

export type HostDeliveryOrigin =
  | "fresh-projection"
  | "forced-paced-prefix"
  | "pending-catch-up";

export interface HostDeliveryRecord {
  readonly sequence: number;
  readonly nodeKey: string;
  readonly nodeId: NodeId;
  readonly range: {
    readonly start: string;
    readonly end: string;
  };
  readonly text: string;
  readonly origin: HostDeliveryOrigin;
  readonly animationEligible: boolean;
}

export interface HostTextRun {
  readonly text: string;
  readonly range: {
    readonly start: string;
    readonly end: string;
  };
  readonly animationEligible: boolean;
  readonly animationSequence: number | null;
}

interface QueueEntry {
  readonly key: string;
  readonly nodeId: NodeId;
  readonly grapheme: string;
  readonly range: {
    readonly start: string;
    readonly end: string;
  };
  readonly ordinal: number;
  sealed: boolean;
}

interface TextPiece {
  readonly text: string;
  readonly fresh: boolean;
  readonly range: {
    readonly start: string;
    readonly end: string;
  };
}

interface DeliveredTail {
  readonly entry: QueueEntry;
  readonly record: HostDeliveryRecord;
}

interface TrailingEntry {
  readonly entry: QueueEntry;
  readonly delivered: DeliveredTail | undefined;
}

interface BatchNodePlan {
  lastInvalidationOrdinal: number;
  firstAppendStart: string | undefined;
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
  readonly #deliveredTailsByKey = new Map<string, DeliveredTail>();
  readonly #listeners = new Set<(changedNodes: readonly NodeId[] | null) => void>();
  readonly #dirtyNodes = new Set<NodeId>();
  readonly #events: HostPolicyEvent[] = [];
  readonly #deliveries: HostDeliveryRecord[] = [];
  readonly #pendingDeliveriesByKey = new Map<string, HostDeliveryRecord[]>();
  readonly #latestDeliveriesByKey = new Map<string, readonly HostDeliveryRecord[]>();
  readonly #animationEligibleKeys = new Set<string>();
  readonly #nonAnimatedDeliveryKeys = new Set<string>();
  readonly #deferredMutations: (() => void)[] = [];
  #queueHead = 0;
  #continuityGeneration = "0";
  #epoch = "1";
  #eventSequence = 0;
  #deliverySequence = 0;
  #enqueueOrdinal = 0;
  #pendingPresentedBytes = 0;
  #pendingCatchUpBytes = 0;
  #activeDeliveryStart: number | undefined;
  #deferredMutationHead = 0;
  #publishing = false;
  #notificationPending = false;
  #pendingFullRefresh = false;

  constructor(mode: PresentationMode, reducedMotion: boolean) {
    this.mode = mode;
    this.#reducedMotion = reducedMotion;
  }

  get reducedMotion(): boolean {
    return this.#reducedMotion;
  }

  get eventCount(): number {
    return this.#events.length;
  }

  eventsSince(index: number): readonly HostPolicyEvent[] {
    return Object.freeze(this.#events.slice(index));
  }

  get deliveryCount(): number {
    return this.#deliveries.length;
  }

  deliveriesSince(index: number): readonly HostDeliveryRecord[] {
    return Object.freeze(this.#deliveries.slice(index));
  }

  get queuedGraphemes(): number {
    return this.#queue.length - this.#queueHead;
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
    if (this.#publishing) {
      this.#deferMutation(() => this.#setReducedMotionNow(reducedMotion));
      return;
    }
    this.#setReducedMotionNow(reducedMotion);
  }

  #setReducedMotionNow(reducedMotion: boolean): void {
    if (this.#reducedMotion === reducedMotion) {
      return;
    }
    this.#reducedMotion = reducedMotion;
    if (reducedMotion) {
      this.#deliverQueued(
        this.queuedGraphemes,
        "fresh-projection",
        false,
        true,
      );
    }
  }

  consume(store: HostPolicyStore, batch: TransitionBatchView): void {
    if (this.#publishing) {
      this.#deferMutation(() => this.#consumeNow(store, batch));
      return;
    }
    this.#consumeNow(store, batch);
  }

  #consumeNow(store: HostPolicyStore, batch: TransitionBatchView): void {
    if (this.#activeDeliveryStart !== undefined) {
      throw new Error("host presentation consume cannot be reentered");
    }
    if (batch.facts.length === 0) {
      this.#record("no-change", "The operation preserved host continuity.");
      this.#notify(false);
      return;
    }

    const plan = batchPresentationPlan(batch);
    let fullRefresh = false;
    this.#activeDeliveryStart = this.#deliveries.length;
    try {
      let nodeOrdinal = 0;
      for (let factIndex = 0; factIndex < batch.facts.length; factIndex += 1) {
        const facts = batch.facts[factIndex]!;
        if (factIndex < plan.lastFullReplaceIndex) {
          continue;
        }
        this.#continuityGeneration = facts.after.continuityGeneration as string;
        this.#epoch = facts.after.coordinate.epoch as string;
        if (facts.scope === "full_replace") {
          this.#clearQueue();
          this.#deliveredTailsByKey.clear();
          this.#displayedSource.clear();
          this.#caughtUpSource.clear();
          this.#textByKey.clear();
          this.#nodeKeys.clear();
          this.#states.clear();
          this.#discardCurrentDeliveries();
          this.#latestDeliveriesByKey.clear();
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
          const nodePlan = plan.nodes.get(key)!;
          const currentOrdinal = nodeOrdinal;
          nodeOrdinal += 1;
          if (currentOrdinal < nodePlan.lastInvalidationOrdinal) {
            continue;
          }
          this.#nodeKeys.set(node.key.nodeId, key);
          this.#dirtyNodes.add(node.key.nodeId);
          if (node.after === null) {
            this.#dropNode(key);
            this.#states.set(key, "removed");
            this.#record("removal", "A canonical node was removed.", key);
            continue;
          }

          if (node.text?.kind === "projection_append") {
            if (!this.#textByKey.has(key)) {
              this.#textByKey.set(
                key,
                this.#bodyPrefix(store, node.key.nodeId, nodePlan.firstAppendStart!),
              );
            }
            this.#appendProjection(node.key.nodeId, key, node.text);
          } else if (node.text?.kind === "replacement") {
            this.#dropQueuedKey(key);
            this.#discardCurrentDeliveries(key);
            const body = store.getNodeSnapshot(node.key.nodeId)?.bodyText ?? "";
            this.#textByKey.set(
              key,
              nodePlan.firstAppendStart === undefined
                ? body
                : this.#bodyPrefix(store, node.key.nodeId, nodePlan.firstAppendStart),
            );
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
            this.#sealQueuedKey(key);
            this.#states.set(key, "stabilized");
            this.#record("stabilization", "A provisional node became stable.", key);
          }
        }

        for (const structure of facts.structures) {
          const affected = [...structure.inserted, ...structure.removed];
          for (const key of affected) {
            const encoded = transitionNodeKey(key);
            this.#nodeKeys.set(key.nodeId, encoded);
            this.#discardCurrentDeliveries(encoded);
            this.#states.set(encoded, "structured");
            this.#dirtyNodes.add(key.nodeId);
          }
          this.#record("structure", "The canonical child structure changed.");
        }
        for (const resource of facts.resources) {
          for (const affected of resource.affectedNodes) {
            const key = transitionNodeKey(affected);
            this.#nodeKeys.set(affected.nodeId, key);
            this.#discardCurrentDeliveries(key);
            this.#states.set(key, "corrected");
            this.#dirtyNodes.add(affected.nodeId);
          }
          this.#record(
            "correction",
            "A semantic resource changed the meaning of affected content.",
          );
        }
        if (facts.after.lifecycle === "finalized") {
          this.#sealQueue();
        }
      }
    } finally {
      this.#activeDeliveryStart = undefined;
    }
    this.#notify(fullRefresh);
  }

  observePending(store: HostPolicyStore): PendingSourceView | undefined {
    const pending = store.getPendingSourceSnapshot();
    if (pending === undefined || pending.text.length === 0) {
      return undefined;
    }
    if (this.#publishing) {
      this.#deferMutation(() => this.#observePendingNow(pending));
    } else {
      this.#observePendingNow(pending);
    }
    return pending;
  }

  #observePendingNow(pending: PendingSourceView): void {
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
  }

  displayText(nodeId: NodeId, canonicalBodyText: string): string {
    const key = this.#nodeKeys.get(nodeId) ?? this.nodeKey(nodeId, this.#epoch);
    return this.#textByKey.get(key) ?? canonicalBodyText;
  }

  textRuns(
    nodeId: NodeId,
    bodyStart: string,
    canonicalBodyText: string,
  ): readonly HostTextRun[] {
    const key = this.#nodeKeys.get(nodeId) ?? this.nodeKey(nodeId, this.#epoch);
    const text = this.#textByKey.get(key) ?? canonicalBodyText;
    if (text.length === 0) {
      return Object.freeze([]);
    }
    const animated = new SourceIntervalSet();
    let animationSequence: number | null = null;
    for (const record of this.#latestDeliveriesByKey.get(key) ?? []) {
      if (record.animationEligible) {
        animated.add(record.range.start, record.range.end);
        animationSequence = Math.max(animationSequence ?? 0, record.sequence);
      }
    }
    const end = BigInt(bodyStart) + BigInt(encoder.encode(text).byteLength);
    return Object.freeze(animated.partition(bodyStart, end.toString(), text).map((piece) =>
      Object.freeze({
        text: piece.text,
        range: piece.range,
        animationEligible: !piece.fresh,
        animationSequence: piece.fresh ? null : animationSequence,
      })
    ));
  }

  nodeKey(nodeId: NodeId, epoch: Epoch | string): string {
    return `${this.#continuityGeneration}:${epoch as string}:${nodeId as string}`;
  }

  stateForNode(nodeId: NodeId): PresentationState {
    const key = this.#nodeKeys.get(nodeId) ?? this.nodeKey(nodeId, this.#epoch);
    return this.#states.get(key) ?? "present";
  }

  advance(graphemes = 1): number {
    if (this.#publishing) {
      const requested = normalizedDeliveryCount(graphemes);
      this.#deferMutation(() => {
        this.#deliverQueued(
          requested,
          "fresh-projection",
          !this.#reducedMotion,
        );
      });
      return 0;
    }
    return this.#deliverQueued(
      graphemes,
      "fresh-projection",
      !this.#reducedMotion,
    );
  }

  drain(): void {
    if (this.#publishing) {
      this.#deferMutation(() => this.#drainNow());
      return;
    }
    this.#drainNow();
  }

  #drainNow(): void {
    if (this.queuedGraphemes === 0) {
      return;
    }
    this.#deliverQueued(
      this.queuedGraphemes,
      "fresh-projection",
      !this.#reducedMotion,
      true,
    );
  }

  interrupt(): void {
    if (this.#publishing) {
      this.#deferMutation(() => this.#interruptNow());
      return;
    }
    this.#interruptNow();
  }

  #interruptNow(): void {
    this.#clearQueue();
    this.#record("interrupted", "Replay interrupted; queued presentation was discarded.");
    this.#notify(false);
  }

  #appendProjection(
    nodeId: NodeId,
    key: string,
    transition: {
      readonly range: { readonly start: string; readonly end: string };
      readonly text: string;
    },
  ): void {
    const transitionStart = BigInt(transition.range.start);
    const transitionEnd = BigInt(transition.range.end);
    if (
      BigInt(encoder.encode(transition.text).byteLength) !==
        transitionEnd - transitionStart
    ) {
      throw new RangeError("projection text must cover its declared UTF-8 source range");
    }
    const trailing = this.#takeTrailingEntry(key, transition.range.start);
    const combined: TextPiece = {
      text: (trailing?.entry.grapheme ?? "") + transition.text,
      fresh: true,
      range: Object.freeze({
        start: trailing?.entry.range.start ?? transition.range.start,
        end: transition.range.end,
      }),
    };
    const entries = this.#graphemeEntries(
      key,
      nodeId,
      combined,
      trailing?.entry.ordinal,
    );
    this.#sealBefore(BigInt(combined.range.start));

    this.#recordCatchUpRange(transition.range.start, transition.range.end, key);
    for (let index = 0; index < entries.length; index += 1) {
      const entry = entries[index]!;
      const entryEnd = BigInt(entry.range.end);
      const currentStart = max(BigInt(entry.range.start), transitionStart);
      const catchesUp = currentStart < entryEnd && this.#displayedSource.overlaps(
        currentStart.toString(),
        entry.range.end,
      );
      entry.sealed = index + 1 < entries.length;
      if (index === 0 && trailing?.delivered !== undefined) {
        if (entry.range.end === trailing.entry.range.end) {
          this.#deliveredTailsByKey.delete(key);
        } else {
          this.#extendDeliveredTail(trailing.delivered, entry);
        }
        continue;
      }
      if (catchesUp) {
        this.#deliverCausallyEarlier(entry);
        this.#deliver(entry, "pending-catch-up", false);
      } else if (this.mode === "paced" && !this.#reducedMotion) {
        this.#enqueue(entry);
      } else {
        this.#deliverCausallyEarlier(entry);
        this.#deliver(entry, "fresh-projection", !this.#reducedMotion);
      }
    }
    this.#displayedSource.add(transition.range.start, transition.range.end);
    if (!this.#states.has(key)) {
      this.#states.set(key, "present");
    }
    this.#record("append", "Fresh canonical text appended.", key);
  }

  #bodyPrefix(store: HostPolicyStore, nodeId: NodeId, start: string): string {
    const view = store.getNodeSnapshot(nodeId);
    if (view === undefined) {
      return "";
    }
    const offset = BigInt(start) - BigInt(view.node.body.start as string);
    const bytes = encoder.encode(view.bodyText);
    if (offset < 0n || offset > BigInt(bytes.byteLength)) {
      throw new RangeError("projection append starts outside the node body range");
    }
    try {
      return decoder.decode(bytes.slice(0, Number(offset)));
    } catch (error) {
      throw new RangeError(
        "projection append starts inside a UTF-8 code point",
        { cause: error },
      );
    }
  }

  #graphemeEntries(
    key: string,
    nodeId: NodeId,
    piece: TextPiece,
    firstOrdinal?: number,
  ): readonly QueueEntry[] {
    const segments = [...segmenter.segment(piece.text)].map(({ segment }) => segment);
    const entries: QueueEntry[] = [];
    let cursor = BigInt(piece.range.start);
    for (let index = 0; index < segments.length; index += 1) {
      const segment = segments[index]!;
      const end = cursor + BigInt(encoder.encode(segment).byteLength);
      entries.push({
        key,
        nodeId,
        grapheme: segment,
        range: Object.freeze({ start: cursor.toString(), end: end.toString() }),
        ordinal: index === 0 && firstOrdinal !== undefined
          ? firstOrdinal
          : ++this.#enqueueOrdinal,
        sealed: index + 1 < segments.length,
      });
      cursor = end;
    }
    if (cursor !== BigInt(piece.range.end)) {
      throw new RangeError("segmented text must cover its declared UTF-8 source range");
    }
    return entries;
  }

  #takeTrailingEntry(key: string, start: string): TrailingEntry | undefined {
    for (let index = this.#queue.length - 1; index >= this.#queueHead; index -= 1) {
      const candidate = this.#queue[index];
      if (
        candidate !== undefined &&
        !candidate.sealed &&
        candidate.key === key &&
        candidate.range.end === start
      ) {
        this.#queue.splice(index, 1);
        return { entry: candidate, delivered: undefined };
      }
    }
    const delivered = this.#deliveredTailsByKey.get(key);
    if (delivered !== undefined && delivered.entry.range.end === start) {
      this.#deliveredTailsByKey.delete(key);
      return { entry: delivered.entry, delivered };
    }
    return undefined;
  }

  #sealBefore(start: bigint): void {
    for (let index = this.#queueHead; index < this.#queue.length; index += 1) {
      const candidate = this.#queue[index];
      if (candidate !== undefined && BigInt(candidate.range.end) <= start) {
        candidate.sealed = true;
      }
    }
    for (const [key, tail] of this.#deliveredTailsByKey) {
      if (BigInt(tail.entry.range.end) <= start) {
        this.#deliveredTailsByKey.delete(key);
      }
    }
  }

  #enqueue(entry: QueueEntry): void {
    let low = this.#queueHead;
    let high = this.#queue.length;
    while (low < high) {
      const middle = low + Math.floor((high - low) / 2);
      const candidate = this.#queue[middle];
      if (candidate !== undefined && compareQueueOrder(candidate, entry) <= 0) {
        low = middle + 1;
      } else {
        high = middle;
      }
    }
    this.#queue.splice(low, 0, entry);
  }

  #deliverCausallyEarlier(boundary: QueueEntry): void {
    let delivered = 0;
    while (true) {
      const next = this.#queue[this.#queueHead];
      if (next === undefined || compareQueueOrder(next, boundary) >= 0) {
        break;
      }
      this.#queueHead += 1;
      this.#deliver(next, "forced-paced-prefix", !this.#reducedMotion);
      delivered += 1;
    }
    if (delivered > 0) {
      this.#compactQueue();
    }
  }

  #deliverQueued(
    limit: number,
    origin: HostDeliveryOrigin,
    animationEligible: boolean,
    forceTrailing = false,
  ): number {
    const requested = normalizedDeliveryCount(limit);
    if (requested === 0) {
      return 0;
    }
    const delivered = this.#deliverQueuedNow(
      requested,
      origin,
      animationEligible,
      forceTrailing,
    );
    if (delivered > 0) {
      this.#notify(false);
    }
    return delivered;
  }

  #deliverableQueuedCount(forceTrailing: boolean): number {
    if (forceTrailing) {
      return this.queuedGraphemes;
    }
    let count = 0;
    for (let index = this.#queueHead; index < this.#queue.length; index += 1) {
      const entry = this.#queue[index];
      if (entry === undefined || !entry.sealed) {
        break;
      }
      count += 1;
    }
    return count;
  }

  #deliverQueuedNow(
    limit: number,
    origin: HostDeliveryOrigin,
    animationEligible: boolean,
    forceTrailing: boolean,
  ): number {
    let delivered = 0;
    while (delivered < limit) {
      const next = this.#queue[this.#queueHead];
      if (next === undefined || (!forceTrailing && !next.sealed)) {
        break;
      }
      this.#queueHead += 1;
      this.#deliver(next, origin, animationEligible);
      delivered += 1;
    }
    this.#compactQueue();
    return delivered;
  }

  #deliver(
    entry: QueueEntry,
    origin: HostDeliveryOrigin,
    animationEligible: boolean,
  ): void {
    this.#textByKey.set(
      entry.key,
      (this.#textByKey.get(entry.key) ?? "") + entry.grapheme,
    );
    this.#dirtyNodes.add(entry.nodeId);
    if (animationEligible) {
      this.#animationEligibleKeys.add(entry.key);
    } else {
      this.#nonAnimatedDeliveryKeys.add(entry.key);
    }
    const record = Object.freeze({
      sequence: ++this.#deliverySequence,
      nodeKey: entry.key,
      nodeId: entry.nodeId,
      range: Object.freeze({ ...entry.range }),
      text: entry.grapheme,
      origin,
      animationEligible,
    });
    this.#deliveries.push(record);
    const pending = this.#pendingDeliveriesByKey.get(entry.key) ?? [];
    pending.push(record);
    this.#pendingDeliveriesByKey.set(entry.key, pending);
    if (!entry.sealed) {
      this.#deliveredTailsByKey.set(entry.key, { entry, record });
    }
  }

  #extendDeliveredTail(tail: DeliveredTail, entry: QueueEntry): void {
    if (
      entry.key !== tail.entry.key ||
      !entry.grapheme.startsWith(tail.entry.grapheme)
    ) {
      throw new RangeError("delivered grapheme continuation must retain its prior tail");
    }
    const current = this.#textByKey.get(entry.key) ?? "";
    if (!current.endsWith(tail.entry.grapheme)) {
      throw new RangeError("delivered grapheme continuation must remain at the text tail");
    }
    this.#textByKey.set(
      entry.key,
      current.slice(0, -tail.entry.grapheme.length) + entry.grapheme,
    );
    this.#dirtyNodes.add(entry.nodeId);

    const replacement = Object.freeze({
      ...tail.record,
      range: Object.freeze({ ...entry.range }),
      text: entry.grapheme,
    });
    const deliveryIndex = this.#deliveries.indexOf(tail.record);
    if (deliveryIndex < 0) {
      throw new Error("delivered grapheme continuation lost its delivery record");
    }
    this.#deliveries[deliveryIndex] = replacement;

    const pending = this.#pendingDeliveriesByKey.get(entry.key);
    if (pending === undefined) {
      this.#pendingDeliveriesByKey.set(entry.key, [replacement]);
    } else {
      const pendingIndex = pending.indexOf(tail.record);
      if (pendingIndex < 0) {
        pending.push(replacement);
      } else {
        pending[pendingIndex] = replacement;
      }
    }
    const latest = this.#latestDeliveriesByKey.get(entry.key);
    if (latest !== undefined) {
      this.#latestDeliveriesByKey.set(
        entry.key,
        Object.freeze(latest.map((record) => record === tail.record ? replacement : record)),
      );
    }
    if (replacement.animationEligible) {
      this.#animationEligibleKeys.add(entry.key);
    } else {
      this.#nonAnimatedDeliveryKeys.add(entry.key);
    }
    if (!entry.sealed) {
      this.#deliveredTailsByKey.set(entry.key, { entry, record: replacement });
    }
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
    this.#discardCurrentDeliveries(key);
    this.#textByKey.delete(key);
    for (const [nodeId, candidate] of this.#nodeKeys) {
      if (candidate === key) {
        this.#nodeKeys.delete(nodeId);
      }
    }
  }

  #dropQueuedKey(key: string): void {
    let retained = 0;
    for (let index = this.#queueHead; index < this.#queue.length; index += 1) {
      const entry = this.#queue[index];
      if (entry !== undefined && entry.key !== key) {
        this.#queue[retained] = entry;
        retained += 1;
      }
    }
    this.#queue.length = retained;
    this.#queueHead = 0;
    this.#deliveredTailsByKey.delete(key);
  }

  #clearQueue(): void {
    this.#queue.length = 0;
    this.#queueHead = 0;
  }

  #sealQueuedKey(key: string): void {
    for (let index = this.#queueHead; index < this.#queue.length; index += 1) {
      const entry = this.#queue[index];
      if (entry?.key === key) {
        entry.sealed = true;
      }
    }
  }

  #sealQueue(): void {
    for (let index = this.#queueHead; index < this.#queue.length; index += 1) {
      const entry = this.#queue[index];
      if (entry !== undefined) {
        entry.sealed = true;
      }
    }
    this.#deliveredTailsByKey.clear();
  }

  #discardCurrentDeliveries(key?: string): void {
    const start = this.#activeDeliveryStart;
    if (start !== undefined) {
      const retained = this.#deliveries.slice(start).filter(
        (record) => key !== undefined && record.nodeKey !== key,
      );
      this.#deliveries.splice(start, this.#deliveries.length - start, ...retained);
    }
    if (key === undefined) {
      this.#pendingDeliveriesByKey.clear();
      this.#animationEligibleKeys.clear();
      this.#nonAnimatedDeliveryKeys.clear();
      return;
    }
    this.#clearDeliveryMarkers(key);
  }

  #clearDeliveryMarkers(key: string): void {
    this.#pendingDeliveriesByKey.delete(key);
    this.#latestDeliveriesByKey.delete(key);
    this.#animationEligibleKeys.delete(key);
    this.#nonAnimatedDeliveryKeys.delete(key);
    this.#deliveredTailsByKey.delete(key);
  }

  #compactQueue(): void {
    if (this.#queueHead === 0) {
      return;
    }
    if (this.#queueHead === this.#queue.length) {
      this.#clearQueue();
      return;
    }
    if (this.#queueHead < 1_024 || this.#queueHead * 2 < this.#queue.length) {
      return;
    }
    this.#queue.copyWithin(0, this.#queueHead);
    this.#queue.length -= this.#queueHead;
    this.#queueHead = 0;
  }

  #record(kind: HostPolicyEventKind, message: string, nodeKey?: string): void {
    const event = nodeKey === undefined
      ? { id: ++this.#eventSequence, kind, message }
      : { id: ++this.#eventSequence, kind, message, nodeKey };
    this.#events.push(Object.freeze(event));
  }

  #deferMutation(mutation: () => void): void {
    this.#deferredMutations.push(mutation);
  }

  #notify(fullRefresh: boolean): void {
    this.#notificationPending = true;
    this.#pendingFullRefresh ||= fullRefresh;
    if (this.#publishing) {
      return;
    }

    this.#publishing = true;
    let publicationError: unknown;
    try {
      while (
        this.#notificationPending ||
        this.#deferredMutationHead < this.#deferredMutations.length
      ) {
        if (!this.#notificationPending) {
          const mutation = this.#deferredMutations[this.#deferredMutationHead];
          this.#deferredMutationHead += 1;
          try {
            mutation?.();
          } catch (error) {
            publicationError ??= error;
          }
          continue;
        }

        const refresh = this.#pendingFullRefresh;
        this.#notificationPending = false;
        this.#pendingFullRefresh = false;

        for (const key of this.#animationEligibleKeys) {
          this.#states.set(key, "fresh");
        }
        for (const key of this.#nonAnimatedDeliveryKeys) {
          this.#states.set(key, "present");
        }

        const changedNodes = [...this.#dirtyNodes];
        if (refresh) {
          this.#latestDeliveriesByKey.clear();
        }
        for (const nodeId of changedNodes) {
          const key = this.#nodeKeys.get(nodeId);
          if (key !== undefined) {
            this.#latestDeliveriesByKey.set(
              key,
              Object.freeze([...(this.#pendingDeliveriesByKey.get(key) ?? [])]),
            );
          }
        }
        this.#pendingDeliveriesByKey.clear();
        this.#animationEligibleKeys.clear();
        this.#nonAnimatedDeliveryKeys.clear();
        this.#dirtyNodes.clear();

        const changed = refresh ? null : Object.freeze(changedNodes);
        for (const listener of [...this.#listeners]) {
          try {
            listener(changed);
          } catch (error) {
            publicationError ??= error;
          }
        }
      }
    } finally {
      this.#deferredMutations.length = 0;
      this.#deferredMutationHead = 0;
      this.#publishing = false;
    }
    if (publicationError !== undefined) {
      throw publicationError;
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

  partition(startValue: string, endValue: string, text: string): readonly TextPiece[] {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    const bytes = encoder.encode(text);
    if (BigInt(bytes.byteLength) !== end - start) {
      throw new RangeError("projection text must cover its declared UTF-8 source range");
    }

    const pieces: TextPiece[] = [];
    let offset = 0;
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
      pieces.push({
        text: value,
        fresh,
        range: Object.freeze({
          start: (start + BigInt(offset)).toString(),
          end: (start + BigInt(boundary)).toString(),
        }),
      });
      offset = boundary;
    }
    return pieces;
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

  overlaps(startValue: string, endValue: string): boolean {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    return this.#intervals.some((interval) =>
      max(start, interval.start) < min(end, interval.end)
    );
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

function batchPresentationPlan(batch: TransitionBatchView): {
  readonly lastFullReplaceIndex: number;
  readonly nodes: ReadonlyMap<string, BatchNodePlan>;
} {
  let lastFullReplaceIndex = -1;
  for (let index = 0; index < batch.facts.length; index += 1) {
    if (batch.facts[index]?.scope === "full_replace") {
      lastFullReplaceIndex = index;
    }
  }

  const nodes = new Map<string, BatchNodePlan>();
  let ordinal = 0;
  for (
    let factIndex = lastFullReplaceIndex + 1;
    factIndex < batch.facts.length;
    factIndex += 1
  ) {
    const facts = batch.facts[factIndex];
    if (facts === undefined || facts.scope === "full_replace") {
      continue;
    }
    for (const node of facts.nodes) {
      const key = transitionNodeKey(node.key);
      const plan = nodes.get(key) ?? {
        lastInvalidationOrdinal: -1,
        firstAppendStart: undefined,
      };
      if (node.after === null || node.text?.kind === "replacement") {
        plan.lastInvalidationOrdinal = ordinal;
        plan.firstAppendStart = undefined;
      } else if (
        node.text?.kind === "projection_append" &&
        plan.firstAppendStart === undefined
      ) {
        plan.firstAppendStart = node.text.range.start as string;
      }
      nodes.set(key, plan);
      ordinal += 1;
    }
  }
  return { lastFullReplaceIndex, nodes };
}

function normalizedDeliveryCount(value: number): number {
  if (value === Number.POSITIVE_INFINITY) {
    return Number.MAX_SAFE_INTEGER;
  }
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function compareQueueOrder(left: QueueEntry, right: QueueEntry): number {
  const start = compareBigInt(BigInt(left.range.start), BigInt(right.range.start));
  if (start !== 0) {
    return start;
  }
  const end = compareBigInt(BigInt(left.range.end), BigInt(right.range.end));
  return end !== 0 ? end : left.ordinal - right.ordinal;
}

function compareBigInt(left: bigint, right: bigint): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function min(left: bigint, right: bigint): bigint {
  return left < right ? left : right;
}

function max(left: bigint, right: bigint): bigint {
  return left > right ? left : right;
}
