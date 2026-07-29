import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { initMdstream } from "../src/index.js";
import { evaluateBatchCandidateForTest } from "../src/engine.js";
import {
  decodeJson,
  nodeWasmLoader,
  normalizeSnapshot,
} from "./helpers.js";

describe("large Rust-backed TypeScript workload", () => {
  it("keeps constituent-first after the per-workload KTD3 value gate", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const workloads = [
      {
        name: "one-byte",
        chunks: Array.from("# Linear input\n\nOne byte at a time."),
      },
      {
        name: "bursty",
        chunks: [
          "# Bursty\n\n",
          "A ",
          "short burst",
          " followed by ",
          "another.\n",
        ],
      },
      {
        name: "unicode",
        chunks: ["多", "语言", " 🙂", " cafe\u0301", "\n"],
      },
      {
        name: "crlf",
        chunks: ["alpha\r", "\nbe", "ta\r", "gamma\r", "\n"],
      },
      { name: "golden-ai", chunks: goldenAiChunks() },
    ];
    const records: unknown[] = [];

    for (const { name, chunks } of workloads) {
      const joined = runCandidate(runtime, chunks, "joined-first");
      const constituent = runCandidate(runtime, chunks, "constituent-first");

      expect(joined.snapshot, `${name} final IR`).toEqual(constituent.snapshot);
      expect(joined.metrics.scanBytes, `${name} scan`).toBe(
        constituent.metrics.scanBytes,
      );
      expect(joined.metrics.replayCount, `${name} joined replay`).toBe("0");
      expect(constituent.metrics.replayCount, `${name} constituent replay`).toBe("0");

      const intendedBenefit =
        improvesByQuarter(
          joined.metrics.appendAttempts,
          constituent.metrics.appendAttempts,
        ) ||
        improvesByQuarter(
          joined.metrics.encodedResultBytes,
          constituent.metrics.encodedResultBytes,
        );
      expect(intendedBenefit, `${name} intended batching benefit`).toBe(true);
      expect(
        withinTwentyPercent(
          joined.metrics.appendAttempts,
          constituent.metrics.appendAttempts,
        ),
        `${name} append attempts`,
      ).toBe(true);
      expect(
        withinTwentyPercent(
          joined.metrics.encodedResultBytes,
          constituent.metrics.encodedResultBytes,
        ),
        `${name} encoded bytes`,
      ).toBe(true);
      expect(
        withinTwentyPercent(
          joined.metrics.scanBytes,
          constituent.metrics.scanBytes,
        ),
        `${name} scan work`,
      ).toBe(true);
      expect(
        withinTwentyPercent(
          joined.metrics.joinCopyBytes,
          constituent.metrics.joinCopyBytes,
        ),
        `${name} joined copy gate`,
      ).toBe(false);

      records.push({
        name,
        joined: joined.metrics,
        constituent: constituent.metrics,
        decision: "constituent-first",
      });
    }

    // Keep the pre-release architecture decision reproducible in CI.
    expect(records).toEqual([
      {
        name: "one-byte",
        joined: {
          appendAttempts: "1",
          encodedResultBytes: "5693",
          scanBytes: "35",
          joinCopyBytes: "35",
          replayCount: "0",
        },
        constituent: {
          appendAttempts: "35",
          encodedResultBytes: "55320",
          scanBytes: "35",
          joinCopyBytes: "0",
          replayCount: "0",
        },
        decision: "constituent-first",
      },
      {
        name: "bursty",
        joined: {
          appendAttempts: "1",
          encodedResultBytes: "5695",
          scanBytes: "45",
          joinCopyBytes: "45",
          replayCount: "0",
        },
        constituent: {
          appendAttempts: "5",
          encodedResultBytes: "10289",
          scanBytes: "45",
          joinCopyBytes: "0",
          replayCount: "0",
        },
        decision: "constituent-first",
      },
      {
        name: "unicode",
        joined: {
          appendAttempts: "1",
          encodedResultBytes: "4341",
          scanBytes: "22",
          joinCopyBytes: "22",
          replayCount: "0",
        },
        constituent: {
          appendAttempts: "5",
          encodedResultBytes: "11136",
          scanBytes: "22",
          joinCopyBytes: "0",
          replayCount: "0",
        },
        decision: "constituent-first",
      },
      {
        name: "crlf",
        joined: {
          appendAttempts: "1",
          encodedResultBytes: "7503",
          scanBytes: "19",
          joinCopyBytes: "19",
          replayCount: "0",
        },
        constituent: {
          appendAttempts: "5",
          encodedResultBytes: "10541",
          scanBytes: "19",
          joinCopyBytes: "0",
          replayCount: "0",
        },
        decision: "constituent-first",
      },
      {
        name: "golden-ai",
        joined: {
          appendAttempts: "1",
          encodedResultBytes: "10016",
          scanBytes: "372",
          joinCopyBytes: "372",
          replayCount: "0",
        },
        constituent: {
          appendAttempts: "9",
          encodedResultBytes: "22114",
          scanBytes: "372",
          joinCopyBytes: "0",
          replayCount: "0",
        },
        decision: "constituent-first",
      },
    ]);
  });

  it(
    "materializes only explicitly accessed nodes across 10k nodes and 100k reads",
    { timeout: 120_000 },
    async () => {
      const runtime = await initMdstream({ loader: nodeWasmLoader });
      const engine = runtime.createEngine({
        protocol: { maxOperations: "40000" },
      });
      const source = Array.from(
        { length: 10_000 },
        (_, index) => `paragraph ${index}\n\n`,
      ).join("");

      engine.append(source);
      engine.finish();
      const roots = engine.store.getSnapshot().document?.roots?.children ?? [];
      expect(roots).toHaveLength(10_000);
      expect(engine.store.metrics().materializedNodeViews).toBe("0");
      expect(engine.metrics().snapshotPayloads).toBe("0");

      const accessed = roots.slice(0, 16);
      const first = accessed.map((id) => engine.store.getNodeSnapshot(id));
      let referencesStable = true;
      for (let index = 0; index < 100_000; index += 1) {
        const slot = index % accessed.length;
        const id = accessed[slot]!;
        referencesStable &&= engine.store.getNodeSnapshot(id) === first[slot];
      }

      expect(referencesStable).toBe(true);
      expect(engine.store.metrics().materializedNodeViews).toBe("16");
      expect(engine.store.metrics().snapshotPayloads).toBe("0");
      engine.close();

      const pendingEngine = runtime.createEngine();
      pendingEngine.append("a *b");
      pendingEngine.append("*");
      expect(pendingEngine.store.metrics().materializedPendingSourceViews).toBe("0");
      const pending = pendingEngine.store.getPendingSourceSnapshot();
      let pendingReferenceStable = true;
      for (let index = 0; index < 100_000; index += 1) {
        pendingReferenceStable &&=
          pendingEngine.store.getPendingSourceSnapshot() === pending;
      }
      expect(pendingReferenceStable).toBe(true);
      expect(pendingEngine.store.metrics().materializedPendingSourceViews).toBe("1");
      pendingEngine.close();
    },
  );

  it(
    "keeps 10k-node transition classification lazy when capture is enabled",
    { timeout: 120_000 },
    async () => {
      const runtime = await initMdstream({ loader: nodeWasmLoader });
      const engine = runtime.createEngine({
        captureTransitions: true,
        protocol: {
          maxSourceBytes: "1048576",
          maxNodes: "25000",
          maxResources: "100",
          maxOperations: "40000",
          maxChangeStructuralItems: "25000",
          maxChildrenPerList: "10000",
        },
        wire: { maxReducerUpdateBytes: "268435456" },
      });
      let transitionedNodes = 0;
      engine.store.subscribeTransitions((batch) => {
        transitionedNodes += batch.facts.reduce(
          (count, facts) => count + (facts.scope === "continuous" ? facts.nodes.length : 0),
          0,
        );
      });
      const source = Array.from(
        { length: 10_000 },
        (_, index) => `paragraph ${index}\n\n`,
      ).join("");

      engine.append(source);
      engine.finish();
      const roots = engine.store.getSnapshot().document?.roots?.children ?? [];
      expect(roots).toHaveLength(10_000);
      expect(transitionedNodes).toBeGreaterThanOrEqual(10_000);
      expect(engine.store.metrics().materializedNodeViews).toBe("0");

      const accessed = roots.slice(0, 16);
      accessed.forEach((id) => engine.store.getNodeSnapshot(id));
      expect(engine.store.metrics().materializedNodeViews).toBe("16");
      engine.close();
    },
  );
});

function runCandidate(
  runtime: Awaited<ReturnType<typeof initMdstream>>,
  chunks: readonly string[],
  candidate: "joined-first" | "constituent-first",
) {
  const engine = runtime.createEngine();
  try {
    const metrics = evaluateBatchCandidateForTest(chunks, candidate, {
      append: (chunk) => engine.append(chunk),
      finish: () => engine.finish(),
    });
    const snapshot = normalizeSnapshot(
      decodeJson(engine.createRecoverySnapshot()!),
    );
    return { metrics, snapshot };
  } finally {
    engine.close();
  }
}

function improvesByQuarter(candidate: string, baseline: string): boolean {
  return BigInt(candidate) * 4n <= BigInt(baseline) * 3n;
}

function withinTwentyPercent(candidate: string, baseline: string): boolean {
  return BigInt(candidate) * 5n <= BigInt(baseline) * 6n;
}

function goldenAiChunks(): readonly string[] {
  const scenario = JSON.parse(
    readFileSync(
      resolve(process.cwd(), "../../examples/fixtures/golden-ai-stream.json"),
      "utf8",
    ),
  ) as {
    readonly episodes: {
      readonly mainline: {
        readonly actions: readonly {
          readonly kind: string;
          readonly chunk?: string;
        }[];
      };
    };
  };
  return scenario.episodes.mainline.actions.flatMap((action) =>
    action.kind === "append" && action.chunk !== undefined ? [action.chunk] : []
  );
}
