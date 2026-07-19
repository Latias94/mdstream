#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";

import { initMdstream } from "../dist/index.js";

const assertMode = process.argv.includes("--assert");
const unknownArguments = process.argv.slice(2).filter((argument) => argument !== "--assert");
if (unknownArguments.length > 0) {
  throw new Error(`unknown argument(s): ${unknownArguments.join(", ")}`);
}

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder("utf-8", { fatal: true });
const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

// Capture-on preflight accounts for the worst legal transition. These demo
// limits keep that proof bounded without weakening the package defaults.
const demoLimits = Object.freeze({
  maxSourceBytes: "65536",
  maxNodes: "4096",
  maxResources: "512",
  maxOperations: "4096",
  maxChangeStructuralItems: "4096",
  maxDocumentStructuralItems: "16384",
  maxChildrenPerList: "4096",
});

const projectionChunks = Object.freeze([
  "Hello ",
  "world",
  "\n\n",
  "Smooth ",
  "stream\n\n",
]);
const adoptionChunks = Object.freeze([
  "# Adoption\n\nSee [",
  "@E",
  "ngin",
  "e] while this diagram streams.\n\n`",
  "`",
  "`me",
  "rma",
  "id\nflowchart LR\n  To",
  "ken -",
  "->",
  " IR\n```\n\n[",
  "@en",
  "gine",
  "]: https://mdstream.dev/engine \"mdstream\"\n",
]);

function sessionOptions(captureTransitions) {
  return {
    captureTransitions,
    protocol: demoLimits,
    wire: { maxReducerUpdateBytes: "33554432" },
  };
}

function runHostSuite(mdstream, mode) {
  const policy = new SemanticHostPolicy(mode);
  const projection = runProjectionFlow(mdstream, policy);
  const adoption = runAdoptionFlow(mdstream, policy);
  const recovery = runAdvancedRecoveryFlow(mdstream, policy);

  return {
    mode,
    actions: policy.actions,
    actionCounts: countBy(policy.actions, (action) => action.kind),
    canonicalDigests: {
      projection: projection.snapshotDigest,
      adoptionBeforeReset: adoption.beforeResetDigest,
      adoptionAfterReset: adoption.afterResetDigest,
      advancedRecovery: recovery.snapshotDigest,
    },
    lazyMaterialization: adoption.lazyMaterialization,
    recoveryStatusBeforeSnapshot: recovery.statusBeforeSnapshot,
    presentation: policy.presentationSummary(),
    retention: policy.retentionSummary(),
  };
}

function runProjectionFlow(mdstream, policy) {
  const engine = mdstream.createEngine(sessionOptions(true));
  const operation = connectPolicy(engine.store, policy, "projection");

  for (const [index, chunk] of projectionChunks.entries()) {
    operation.run(`append-${index + 1}`, () => engine.append(chunk));
    policy.presentPending("projection", engine.store, `append-${index + 1}`);
    policy.advanceFrame("projection");
  }
  operation.run("finish", () => engine.finish());
  policy.drain("projection", "finish-tail");

  const snapshotDigest = digestBytes(requiredSnapshot(engine.createRecoverySnapshot()));
  operation.disconnect();
  engine.close();
  return { snapshotDigest };
}

function runAdoptionFlow(mdstream, policy) {
  const engine = mdstream.createEngine(sessionOptions(true));
  const operation = connectPolicy(engine.store, policy, "adoption");

  for (const [index, chunk] of adoptionChunks.entries()) {
    const label = `append-${index + 1}`;
    operation.run(label, () => engine.append(chunk));
    // Rendering pending source is an explicit host choice. Recording the
    // painted source range prevents the same bytes from being revealed again
    // when the parser later projects them into semantic nodes.
    policy.presentPending("adoption", engine.store, label);
    policy.advanceFrame("adoption");
  }

  operation.run("finish", () => engine.finish());
  policy.drain("adoption", "finish-tail");
  const beforeResetDigest = digestBytes(requiredSnapshot(engine.createRecoverySnapshot()));

  const nodeViewsBeforeVisibleRead = BigInt(engine.store.metrics().materializedNodeViews);
  const firstRoot = engine.store.getSnapshot().document?.roots?.children[0];
  assert(firstRoot !== undefined, "adoption flow must expose a visible root");
  assert(engine.store.getNodeSnapshot(firstRoot) !== undefined, "visible root must be readable");
  const nodeViewsAfterVisibleRead = BigInt(engine.store.metrics().materializedNodeViews);

  operation.run("reset", () => engine.reset());
  const afterResetDigest = digestBytes(requiredSnapshot(engine.createRecoverySnapshot()));
  operation.disconnect();
  const metrics = engine.store.metrics();
  engine.close();

  return {
    beforeResetDigest,
    afterResetDigest,
    lazyMaterialization: {
      requestedVisibleNodes: 1,
      materializedBeforeVisibleRead: nodeViewsBeforeVisibleRead.toString(),
      materializedAfterVisibleRead: nodeViewsAfterVisibleRead.toString(),
      totalMaterializedNodeViews: metrics.materializedNodeViews,
      totalMaterializedResourceViews: metrics.materializedResourceViews,
      pendingSourceViews: metrics.materializedPendingSourceViews,
    },
  };
}

function runAdvancedRecoveryFlow(mdstream, policy) {
  const producer = mdstream.createEngine(sessionOptions(false));
  const first = producer.append("alpha ");
  producer.append("beta ");
  const third = producer.append("gamma\n");
  const advancedSnapshot = requiredSnapshot(producer.createRecoverySnapshot());

  const target = mdstream.createStore(sessionOptions(true));
  const operation = connectPolicy(target, policy, "advanced-recovery");
  for (const change of first.changes) {
    operation.run("apply-first", () => target.applyChange(change));
  }
  for (const change of third.changes) {
    operation.run("apply-gap", () => target.applyChange(change));
  }
  const statusBeforeSnapshot = target.getSnapshot().status.kind;
  operation.run("recover", () => target.recoverSnapshot(advancedSnapshot));
  policy.drain("advanced-recovery", "recovered-tail");

  const snapshotDigest = digestBytes(requiredSnapshot(target.createRecoverySnapshot()));
  operation.disconnect();
  target.close();
  producer.close();
  return { snapshotDigest, statusBeforeSnapshot };
}

function connectPolicy(store, policy, session) {
  let label = "unlabelled";
  const disconnect = store.subscribeTransitions((batch) => {
    policy.consume(session, label, batch);
  });
  return {
    run(nextLabel, operation) {
      label = nextLabel;
      return operation();
    },
    disconnect,
  };
}

class SemanticHostPolicy {
  constructor(mode) {
    this.mode = mode;
    this.actions = [];
    this.sessions = new Map();
    this.acceptedFreshText = "";
    this.presentedFreshText = "";
    this.pendingPresentedBytes = 0;
    this.pendingCatchUpBytes = 0;
    this.maxQueuedGraphemes = 0;
    this.presentationFrames = 0;
    this.transitionBatches = 0;
    this.maxDisplayedSourceIntervals = 0;
  }

  consume(sessionName, label, batch) {
    this.transitionBatches += 1;
    const session = this.#session(sessionName);
    if (batch.facts.length === 0) {
      this.#record(sessionName, label, "operation_no_change");
      return;
    }

    for (const facts of batch.facts) {
      if (facts.scope === "full_replace") {
        session.queue.length = 0;
        session.displayedSource.clear();
        this.#record(sessionName, label, "document_replace", {
          continuityGeneration: facts.after.continuityGeneration,
        });
        continue;
      }

      if (facts.before === null) {
        this.#record(sessionName, label, "document_begin", {
          lifecycle: facts.after.lifecycle,
        });
      } else if (facts.before.lifecycle !== facts.after.lifecycle) {
        this.#record(sessionName, label, "document_finish", {
          lifecycle: facts.after.lifecycle,
        });
      }

      for (const node of facts.nodes) {
        const key = transitionNodeKey(node.key);
        if (node.before === null && node.after !== null) {
          this.#record(sessionName, label, "node_insert", {
            key,
            version: node.after.version,
            stability: node.after.stability,
          });
        } else if (node.before !== null && node.after === null) {
          this.#record(sessionName, label, "node_remove", { key });
        } else if (node.before !== null && node.after !== null) {
          if (node.before.version !== node.after.version) {
            this.#record(sessionName, label, "node_update", {
              key,
              beforeVersion: node.before.version,
              afterVersion: node.after.version,
            });
          }
          if (node.before.stability !== node.after.stability) {
            this.#record(sessionName, label, "node_stabilize", {
              key,
              stability: node.after.stability,
            });
          }
          const beforeParent = transitionOwnerKey(node.before.parent);
          const afterParent = transitionOwnerKey(node.after.parent);
          if (beforeParent !== afterParent) {
            this.#record(sessionName, label, "node_move", {
              key,
              beforeParent,
              afterParent,
            });
          }
        }

        if (node.text?.kind === "projection_append") {
          const partition = session.displayedSource.partition(
            node.text.range.start,
            node.text.range.end,
            node.text.text,
          );
          this.#record(sessionName, label, "text_append", {
            key,
            range: node.text.range,
            freshText: partition.freshText,
            alreadyDisplayedBytes: partition.alreadyDisplayedBytes,
          });
          if (partition.alreadyDisplayedBytes > 0) {
            this.pendingCatchUpBytes += partition.alreadyDisplayedBytes;
            this.#record(sessionName, label, "pending_catch_up", {
              key,
              range: node.text.range,
              alreadyDisplayedBytes: partition.alreadyDisplayedBytes,
            });
          }
          session.displayedSource.add(node.text.range.start, node.text.range.end);
          this.#deliver(session, sessionName, label, key, partition.freshText);
        } else if (node.text?.kind === "replacement") {
          // The fact deliberately carries no old/new body. A mounted renderer
          // can crossfade its existing view against a lazy batch-tail read.
          this.#record(sessionName, label, "text_replace", { key });
        }
      }

      for (const structure of facts.structures) {
        this.#record(sessionName, label, "children_splice", {
          owner: transitionOwnerKey(structure.owner),
          start: structure.start,
          removed: structure.removed.map(transitionNodeKey),
          inserted: structure.inserted.map(transitionNodeKey),
        });
      }

      for (const resource of facts.resources) {
        this.#record(sessionName, label, "resource_change", {
          key: transitionResourceKey(resource.key),
          beforeVersion: resource.beforeVersion,
          afterVersion: resource.afterVersion,
          affectedNodes: resource.affectedNodes.map(transitionNodeKey),
        });
      }
    }
  }

  presentPending(sessionName, store, label) {
    const pending = store.getPendingSourceSnapshot();
    if (pending === undefined || pending.text.length === 0) {
      return;
    }
    const session = this.#session(sessionName);
    const newlyDisplayedBytes = session.displayedSource.add(
      pending.range.start,
      pending.range.end,
    );
    if (newlyDisplayedBytes === 0) {
      return;
    }
    this.pendingPresentedBytes += newlyDisplayedBytes;
    this.maxDisplayedSourceIntervals = Math.max(
      this.maxDisplayedSourceIntervals,
      session.displayedSource.size,
    );
    this.#record(sessionName, label, "pending_present", {
      range: pending.range,
      newlyDisplayedBytes,
    });
  }

  advanceFrame(sessionName) {
    if (this.mode !== "paced") {
      return;
    }
    const session = this.#session(sessionName);
    const next = session.queue.shift();
    if (next === undefined) {
      return;
    }
    this.presentedFreshText += next.text;
    this.presentationFrames += 1;
  }

  drain(sessionName, label) {
    const session = this.#session(sessionName);
    let graphemes = 0;
    while (session.queue.length > 0) {
      const next = session.queue.shift();
      this.presentedFreshText += next.text;
      graphemes += 1;
    }
    if (graphemes > 0) {
      this.presentationFrames += 1;
      this.#record(sessionName, label, "presentation_drain", { graphemes });
    }
  }

  presentationSummary() {
    return {
      acceptedFreshTextDigest: digestString(this.acceptedFreshText),
      presentedFreshTextDigest: digestString(this.presentedFreshText),
      caughtUp: this.acceptedFreshText === this.presentedFreshText,
      acceptedFreshTextBytes: textEncoder.encode(this.acceptedFreshText).byteLength,
      pendingPresentedBytes: this.pendingPresentedBytes,
      pendingCatchUpBytes: this.pendingCatchUpBytes,
      frames: this.presentationFrames,
      maxQueuedGraphemes: this.maxQueuedGraphemes,
    };
  }

  retentionSummary() {
    return {
      transitionDerivation: {
        oldCanonicalNodeViews: 0,
        completeParentIndexEntries: 0,
        oldCanonicalResourceViews: 0,
        completeStructureItems: 0,
      },
      presentationPolicy: {
        maxQueuedGraphemes: this.maxQueuedGraphemes,
        maxDisplayedSourceIntervals: this.maxDisplayedSourceIntervals,
      },
      diagnosticActionLogEntries: this.actions.length,
      diagnosticTraceExcludedFromProductionRetention: true,
      transitionBatches: this.transitionBatches,
    };
  }

  #deliver(session, sessionName, label, key, text) {
    if (text.length === 0) {
      return;
    }
    this.acceptedFreshText += text;
    const graphemes = Array.from(segmenter.segment(text), ({ segment }) => segment);
    if (this.mode === "immediate") {
      this.presentedFreshText += text;
      return;
    }
    for (const grapheme of graphemes) {
      session.queue.push({ key, label, text: grapheme });
    }
    this.maxQueuedGraphemes = Math.max(this.maxQueuedGraphemes, session.queue.length);
    this.#record(sessionName, label, "presentation_queue", {
      key,
      graphemes: graphemes.length,
    });
  }

  #record(session, operation, kind, details = {}) {
    this.actions.push({ session, operation, kind, ...details });
  }

  #session(name) {
    let session = this.sessions.get(name);
    if (session === undefined) {
      session = { displayedSource: new SourceIntervalSet(), queue: [] };
      this.sessions.set(name, session);
    }
    return session;
  }
}

class SourceIntervalSet {
  constructor() {
    this.intervals = [];
  }

  get size() {
    return this.intervals.length;
  }

  clear() {
    this.intervals.length = 0;
  }

  add(startValue, endValue) {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    if (end <= start) {
      return 0;
    }
    const previouslyCovered = this.#coveredBytes(start, end);
    const next = [];
    let mergedStart = start;
    let mergedEnd = end;
    let inserted = false;
    for (const interval of this.intervals) {
      if (interval.end < mergedStart) {
        next.push(interval);
      } else if (mergedEnd < interval.start) {
        if (!inserted) {
          next.push({ start: mergedStart, end: mergedEnd });
          inserted = true;
        }
        next.push(interval);
      } else {
        mergedStart = minBigInt(mergedStart, interval.start);
        mergedEnd = maxBigInt(mergedEnd, interval.end);
      }
    }
    if (!inserted) {
      next.push({ start: mergedStart, end: mergedEnd });
    }
    this.intervals = next;
    return Number(end - start - previouslyCovered);
  }

  partition(startValue, endValue, text) {
    const start = BigInt(startValue);
    const end = BigInt(endValue);
    const bytes = textEncoder.encode(text);
    assert.equal(
      BigInt(bytes.byteLength),
      end - start,
      "projection append text must cover its declared UTF-8 source range",
    );
    const freshBytes = [];
    let alreadyDisplayedBytes = 0;
    for (let index = 0; index < bytes.byteLength; index += 1) {
      if (this.#contains(start + BigInt(index))) {
        alreadyDisplayedBytes += 1;
      } else {
        freshBytes.push(bytes[index]);
      }
    }
    return {
      freshText: textDecoder.decode(Uint8Array.from(freshBytes)),
      alreadyDisplayedBytes,
    };
  }

  #contains(cursor) {
    return this.intervals.some(({ start, end }) => start <= cursor && cursor < end);
  }

  #coveredBytes(start, end) {
    let covered = 0n;
    for (const interval of this.intervals) {
      const overlapStart = maxBigInt(start, interval.start);
      const overlapEnd = minBigInt(end, interval.end);
      if (overlapEnd > overlapStart) {
        covered += overlapEnd - overlapStart;
      }
    }
    return covered;
  }
}

function runOldViewBaseline(mdstream) {
  const projection = runBaselineFlow(mdstream, projectionChunks, false);
  const adoption = runBaselineFlow(mdstream, adoptionChunks, true);
  return {
    maxRetained: componentMaximum(projection.maxRetained, adoption.maxRetained),
    work: {
      materializedNodeViews: (
        BigInt(projection.work.materializedNodeViews) +
        BigInt(adoption.work.materializedNodeViews)
      ).toString(),
      materializedResourceViews: (
        BigInt(projection.work.materializedResourceViews) +
        BigInt(adoption.work.materializedResourceViews)
      ).toString(),
    },
  };
}

function runBaselineFlow(mdstream, chunks, resetAfterFinish) {
  const engine = mdstream.createEngine(sessionOptions(false));
  const baseline = new OldViewParentIndexBaseline();
  const unsubscribe = engine.store.subscribe(() => baseline.rebuild(engine.store));
  for (const chunk of chunks) {
    engine.append(chunk);
  }
  engine.finish();
  if (resetAfterFinish) {
    engine.reset();
  }
  unsubscribe();
  const metrics = engine.store.metrics();
  engine.close();
  return {
    maxRetained: baseline.maxRetained,
    work: {
      materializedNodeViews: metrics.materializedNodeViews,
      materializedResourceViews: metrics.materializedResourceViews,
    },
  };
}

class OldViewParentIndexBaseline {
  constructor() {
    this.nodeViews = new Map();
    this.parentIndex = new Map();
    this.resourceViews = new Map();
    this.maxRetained = emptyRetainedState();
  }

  rebuild(store) {
    const snapshot = store.getSnapshot();
    const nextNodes = new Map();
    const nextParents = new Map();
    let structureItems = 0;

    const visit = (nodeId, parent) => {
      if (nextNodes.has(nodeId)) {
        return;
      }
      const view = store.getNodeSnapshot(nodeId);
      if (view === undefined) {
        return;
      }
      nextNodes.set(nodeId, view);
      nextParents.set(nodeId, parent);
      structureItems += view.node.children.children.length;
      for (const child of view.node.children.children) {
        visit(child, `node:${nodeId}`);
      }
    };

    const roots = snapshot.document?.roots?.children ?? [];
    structureItems += roots.length;
    for (const root of roots) {
      visit(root, "document");
    }

    if (snapshot.impact.fullReplace) {
      this.resourceViews.clear();
    }
    for (const resourceId of snapshot.impact.changedResourceIds) {
      const view = store.getResourceSnapshot(resourceId);
      if (view === undefined) {
        this.resourceViews.delete(resourceId);
      } else {
        this.resourceViews.set(resourceId, view);
      }
    }

    this.nodeViews = nextNodes;
    this.parentIndex = nextParents;
    const retained = {
      oldCanonicalNodeViews: this.nodeViews.size,
      completeParentIndexEntries: this.parentIndex.size,
      oldCanonicalResourceViews: this.resourceViews.size,
      completeStructureItems: structureItems,
      semanticTextBytes: Array.from(this.nodeViews.values()).reduce(
        (total, view) => total + textEncoder.encode(view.bodyText).byteLength,
        0,
      ),
    };
    this.maxRetained = componentMaximum(this.maxRetained, retained);
  }
}

function buildReport(mdstream, immediateRun, pacedRun, oldBaseline) {
  const actionCounts = immediateRun.actionCounts;
  const representatives = firstBy(immediateRun.actions, (action) => action.kind);
  const factDerivation = immediateRun.retention.transitionDerivation;
  const factDerivationItems = retainedItems(factDerivation);
  const baselineItems = retainedItems(oldBaseline.maxRetained);

  return {
    schema: "mdstream.transition-host-example/1",
    package: {
      name: "@mdstream/core",
      version: mdstream.packageVersion,
      bindingSchema: mdstream.bindingSchema,
      transitionSchema: mdstream.transitionSchema,
    },
    hostBoundary: {
      input: "typed TransitionBatchView callbacks plus lazy store views",
      rawWireDecoded: false,
      markdownReparsedByHost: false,
      animationApiExportedByMdstream: false,
      operationBatchTailIsCanonicalAuthority: true,
    },
    coverage: {
      actionCounts,
      structureSplicesWithRemoval: immediateRun.actions.filter(
        (action) => action.kind === "children_splice" && action.removed.length > 0,
      ).length,
      resetFullReplacements: immediateRun.actions.filter(
        (action) => action.kind === "document_replace" && action.session === "adoption",
      ).length,
      advancedRecoveryFullReplacements: immediateRun.actions.filter(
        (action) => action.kind === "document_replace" && action.session === "advanced-recovery",
      ).length,
      representativeActions: Object.fromEntries(representatives),
    },
    semanticEquivalence: {
      semanticActionsEqual:
        stableJson(immediateRun.actions.filter(isSemanticAction)) ===
        stableJson(pacedRun.actions.filter(isSemanticAction)),
      canonicalDigestsEqual:
        stableJson(immediateRun.canonicalDigests) === stableJson(pacedRun.canonicalDigests),
      immediateCaughtUp: immediateRun.presentation.caughtUp,
      pacedCaughtUp: pacedRun.presentation.caughtUp,
      acceptedTextEqual:
        immediateRun.presentation.acceptedFreshTextDigest ===
        pacedRun.presentation.acceptedFreshTextDigest,
      immediateFrames: immediateRun.presentation.frames,
      pacedFrames: pacedRun.presentation.frames,
      pacedMaxQueuedGraphemes: pacedRun.presentation.maxQueuedGraphemes,
    },
    pendingSourcePolicy: {
      presentedBytes: immediateRun.presentation.pendingPresentedBytes,
      catchUpBytesNotRevealedAgain: immediateRun.presentation.pendingCatchUpBytes,
    },
    lazyMaterialization: immediateRun.lazyMaterialization,
    retainedState: {
      factHostTransitionDerivation: factDerivation,
      factHostPresentationPolicy: {
        immediate: immediateRun.retention.presentationPolicy,
        paced: pacedRun.retention.presentationPolicy,
      },
      oldViewParentIndexBaseline: oldBaseline.maxRetained,
      factDerivationItems,
      baselineItems,
      strictlySmallerTransitionDerivation: factDerivationItems < baselineItems,
      baselineWork: oldBaseline.work,
      accountingNote:
        "Presentation queues and painted-source intervals are reported separately from transition derivation state.",
    },
    recovery: {
      statusBeforeAdvancedSnapshot: immediateRun.recoveryStatusBeforeSnapshot,
      advancedSnapshotProducedFullReplace:
        immediateRun.actions.some(
          (action) =>
            action.session === "advanced-recovery" &&
            action.operation === "recover" &&
            action.kind === "document_replace",
        ),
    },
    hostOwnedStrategies: {
      immediate: "apply semantic actions synchronously",
      paced: "segment projection-append deltas into graphemes and drain on host frames",
      layout:
        "children_splice and node_move are measurement triggers; geometry, FLIP, resize, and scroll ownership remain host inputs",
    },
    u6ExtensionPoints: {
      artifacts:
        "Consume processor artifact events beside, never inside, canonical transition facts; sanitize or isolate SVG before display.",
      messageParts:
        "Create one host session per stable message part key plus host generation; part order and tool state remain outside mdstream.",
    },
    assertions: assertMode ? "pending" : "not_requested",
  };
}

function assertReport(result, immediateRun, pacedRun, oldBaseline) {
  const counts = result.coverage.actionCounts;
  assert((counts.pending_present ?? 0) > 0, "pending source must be presented by host policy");
  assert((counts.pending_catch_up ?? 0) > 0, "pending catch-up must avoid duplicate reveal");
  assert((counts.text_append ?? 0) > 0, "projection append must produce a semantic action");
  assert((counts.text_replace ?? 0) > 0, "correction must produce replacement action");
  assert((counts.resource_change ?? 0) > 0, "resource correction must be targeted");
  assert((counts.node_remove ?? 0) > 0, "removed node must be classified");
  assert(result.coverage.structureSplicesWithRemoval > 0, "structural removal must be classified");
  assert((counts.document_finish ?? 0) > 0, "finish must be classified");
  assert(result.coverage.resetFullReplacements > 0, "reset must be a full replacement");
  assert(
    result.coverage.advancedRecoveryFullReplacements > 0,
    "advanced recovery must be a full replacement",
  );
  assert.equal(result.recovery.statusBeforeAdvancedSnapshot, "needs_snapshot");
  assert.equal(result.recovery.advancedSnapshotProducedFullReplace, true);

  assert.equal(result.semanticEquivalence.semanticActionsEqual, true);
  assert.equal(result.semanticEquivalence.canonicalDigestsEqual, true);
  assert.equal(result.semanticEquivalence.immediateCaughtUp, true);
  assert.equal(result.semanticEquivalence.pacedCaughtUp, true);
  assert.equal(result.semanticEquivalence.acceptedTextEqual, true);
  assert.equal(immediateRun.presentation.maxQueuedGraphemes, 0);
  assert(pacedRun.presentation.maxQueuedGraphemes > 1, "paced host must queue graphemes");

  assert.equal(result.lazyMaterialization.materializedBeforeVisibleRead, "0");
  assert.equal(result.lazyMaterialization.materializedAfterVisibleRead, "1");
  assert.equal(result.lazyMaterialization.requestedVisibleNodes, 1);
  assert.equal(result.hostBoundary.rawWireDecoded, false);
  assert.equal(result.hostBoundary.markdownReparsedByHost, false);

  assert.equal(result.retainedState.factHostTransitionDerivation.oldCanonicalNodeViews, 0);
  assert.equal(result.retainedState.factHostTransitionDerivation.completeParentIndexEntries, 0);
  assert(oldBaseline.maxRetained.oldCanonicalNodeViews > 0);
  assert(oldBaseline.maxRetained.completeParentIndexEntries > 0);
  assert.equal(result.retainedState.strictlySmallerTransitionDerivation, true);
  assert(
    BigInt(oldBaseline.work.materializedNodeViews) >
      BigInt(result.lazyMaterialization.materializedAfterVisibleRead),
    "old-view baseline must materialize more node views than the lazy fact host",
  );
}

function transitionNodeKey(key) {
  return `${key.continuityGeneration}:${key.epoch}:${key.nodeId}`;
}

function transitionResourceKey(key) {
  return `${key.continuityGeneration}:${key.epoch}:${key.resourceId}`;
}

function transitionOwnerKey(owner) {
  if (owner === null) {
    return null;
  }
  return owner.kind === "document" ? "document" : `node:${transitionNodeKey(owner.key)}`;
}

function requiredSnapshot(snapshot) {
  assert(snapshot !== undefined, "scenario must produce a recovery snapshot");
  return snapshot;
}

function digestBytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function digestString(value) {
  return createHash("sha256").update(value).digest("hex");
}

function stableJson(value) {
  return JSON.stringify(value);
}

function isSemanticAction(action) {
  return action.kind !== "presentation_queue" && action.kind !== "presentation_drain";
}

function countBy(values, keyOf) {
  const counts = {};
  for (const value of values) {
    const key = keyOf(value);
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return counts;
}

function firstBy(values, keyOf) {
  const entries = new Map();
  for (const value of values) {
    const key = keyOf(value);
    if (!entries.has(key)) {
      entries.set(key, value);
    }
  }
  return entries;
}

function emptyRetainedState() {
  return {
    oldCanonicalNodeViews: 0,
    completeParentIndexEntries: 0,
    oldCanonicalResourceViews: 0,
    completeStructureItems: 0,
    semanticTextBytes: 0,
  };
}

function componentMaximum(left, right) {
  const result = {};
  for (const key of new Set([...Object.keys(left), ...Object.keys(right)])) {
    result[key] = Math.max(left[key] ?? 0, right[key] ?? 0);
  }
  return result;
}

function retainedItems(retained) {
  return (
    retained.oldCanonicalNodeViews +
    retained.completeParentIndexEntries +
    retained.oldCanonicalResourceViews +
    retained.completeStructureItems
  );
}

function minBigInt(left, right) {
  return left < right ? left : right;
}

function maxBigInt(left, right) {
  return left > right ? left : right;
}

const runtime = await initMdstream();
const immediate = runHostSuite(runtime, "immediate");
const paced = runHostSuite(runtime, "paced");
const baseline = runOldViewBaseline(runtime);
const report = buildReport(runtime, immediate, paced, baseline);

if (assertMode) {
  assertReport(report, immediate, paced, baseline);
  report.assertions = "passed";
}

console.log(JSON.stringify(report, null, 2));
