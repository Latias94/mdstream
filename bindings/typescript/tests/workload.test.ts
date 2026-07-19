import { describe, expect, it } from "vitest";

import { initMdstream } from "../src/index.js";
import { nodeWasmLoader } from "./helpers.js";

describe("large Rust-backed TypeScript workload", () => {
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
