import { describe, expect, it } from "vitest";

import {
  initMdstream,
  MdstreamError,
  type MdstreamSessionOptions,
} from "../src/index.js";
import {
  decodeJson,
  encodeChange,
  loadProtocolFixture,
  nodeWasmLoader,
  normalizeSnapshot,
} from "./helpers.js";

describe("Rust/WASM/TypeScript structural goldens", () => {
  it("replays every shared trace to the native normalized snapshot", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const fixture = loadProtocolFixture();

    for (const trace of fixture.traces) {
      const store = runtime.createStore();
      for (const change of trace.changes) {
        const result = store.applyChange(encodeChange(change));
        expect(result.updates).toHaveLength(1);
        expect(result.outputPayloadBytes).toMatch(/^[1-9][0-9]*$/);
      }
      expect(store.metrics().snapshotPayloads).toBe("0");
      const snapshot = store.createRecoverySnapshot();
      expect(snapshot).toBeDefined();
      expect(normalizeSnapshot(decodeJson(snapshot!))).toEqual(
        fixture.expected.normalized_snapshot,
      );
      expect(store.metrics().snapshotPayloads).toBe("1");
      store.close();
    }
  });

  it("keeps normal engine operations delta-first and preserves structured errors", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();

    expect(engine.append("# TypeScript\n\nbody").changes).toHaveLength(1);
    expect(engine.finish().changes).toHaveLength(1);
    expect(engine.metrics().snapshotPayloads).toBe("0");
    expect(engine.store.metrics().snapshotPayloads).toBe("0");

    const snapshot = engine.createRecoverySnapshot();
    expect(snapshot).toBeDefined();
    expect(decodeJson(snapshot!).source).toBe("# TypeScript\n\nbody");
    expect(engine.metrics().snapshotPayloads).toBe("1");

    expect(() => engine.append("late")).toThrowError(MdstreamError);
    try {
      engine.append("late");
    } catch (error) {
      expect(error).toMatchObject({
        status: 6,
        statusName: "MDSTREAM_TERMINAL",
      });
    }
    engine.close();
  });

  it("exposes an engine-owned read-only store facade", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const store = engine.store as unknown as Record<string, unknown>;

    expect("applyChange" in store).toBe(false);
    expect("recoverSnapshot" in store).toBe(false);
    expect("createRecoverySnapshot" in store).toBe(false);
    expect("close" in store).toBe(false);

    engine.append("# Engine-owned state");
    expect(engine.store.getSnapshot().status.kind).toBe("ready");
    engine.close();
  });

  it("accepts bigint limits, rejects JavaScript numbers, and retries failed loaders", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine({
      processor: { maxInFlightJobs: 32n },
    });
    engine.close();

    expect(() =>
      runtime.createEngine({
        processor: { maxInFlightJobs: 32 as never },
      }),
    ).toThrow(/bigint or decimal strings/);

    let attempts = 0;
    const loader = async () => {
      attempts += 1;
      if (attempts === 1) {
        throw new Error("transient load failure");
      }
      return nodeWasmLoader();
    };
    await expect(initMdstream({ loader })).rejects.toThrow("transient load failure");
    const retried = await initMdstream({ loader });
    expect(retried.abiVersion).toBe(1);
    expect(attempts).toBe(2);

    const opaqueChange = (change: import("../src/index.js").CanonicalChangeBytes) => {
      // @ts-expect-error Canonical bytes intentionally expose no reducer operations.
      return change.operations;
    };
    expect(opaqueChange).toBeTypeOf("function");

    const _typedOptions: MdstreamSessionOptions = {
      wire: { maxCommandBytes: "536870912" },
    };
    expect(_typedOptions.wire?.maxCommandBytes).toBe("536870912");
  });
});
