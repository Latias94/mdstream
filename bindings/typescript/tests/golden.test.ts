import { describe, expect, it } from "vitest";

import {
  initMdstream,
  MdstreamError,
  type MdstreamSessionOptions,
  type WasmModuleLoader,
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

  it("rejects an incompatible binding schema before constructing sessions", async () => {
    let constructedSessions = 0;
    class UnexpectedSession {
      constructor() {
        constructedSessions += 1;
        throw new Error("session construction must not be reached");
      }
    }
    const loader: WasmModuleLoader = () => ({
      MdstreamEngineSession: UnexpectedSession,
      MdstreamReducerSession: UnexpectedSession,
      abiVersion: () => 1,
      packageVersion: () => "0.4.0",
      bindingSchema: () => "mdstream.bindings/0.3",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
    });

    let failure: unknown;
    try {
      const runtime = await initMdstream({ loader });
      runtime.createEngine();
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(MdstreamError);
    expect(failure).toMatchObject({
      status: 5,
      statusName: "MDSTREAM_UNSUPPORTED_SCHEMA",
      detailCode: "unsupported_schema",
      schema: "mdstream.bindings/0.3",
    });
    expect(constructedSessions).toBe(0);
  });

  it("rejects a same-schema WASM module missing required reducer capabilities", async () => {
    let constructedSessions = 0;
    class LegacySession {
      constructor() {
        constructedSessions += 1;
      }
    }
    const loader: WasmModuleLoader = () => ({
      MdstreamEngineSession: LegacySession,
      MdstreamReducerSession: LegacySession,
      abiVersion: () => 1,
      packageVersion: () => "0.4.0",
      bindingSchema: () => "mdstream.bindings/0.4",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
    });

    await expect(initMdstream({ loader })).rejects.toMatchObject({
      status: 5,
      statusName: "MDSTREAM_UNSUPPORTED_SCHEMA",
      detailCode: "unsupported_schema",
      schema: "mdstream.bindings/0.4",
    });
    expect(constructedSessions).toBe(0);
  });

  it("rejects a same-schema WASM module missing conditional processor begin", async () => {
    class PartialReducerSession {
      pendingSourceView(): never {
        throw new Error("session construction must not be reached");
      }
    }
    const loader: WasmModuleLoader = () => ({
      MdstreamEngineSession: PartialReducerSession,
      MdstreamReducerSession: PartialReducerSession,
      abiVersion: () => 1,
      packageVersion: () => "0.4.0",
      bindingSchema: () => "mdstream.bindings/0.4",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
    });

    await expect(initMdstream({ loader })).rejects.toThrow(
      "beginProcessorIfCurrent",
    );
  });
});
