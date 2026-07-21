import { describe, expect, it } from "vitest";

import { initMdstream, type WasmModuleLoader } from "../src/index.js";

describe("custom WASM loader contract", () => {
  it("snapshots validated metadata probes exactly once", async () => {
    const calls = {
      abi: 0,
      package: 0,
      binding: 0,
      options: 0,
      transition: 0,
    };
    class EngineSession {}
    class ReducerSession {
      pendingSourceView(): never {
        throw new Error("not used");
      }

      beginProcessorIfCurrent(): never {
        throw new Error("not used");
      }

      processorMaxInFlightJobs(): number {
        return 1;
      }

      processorMaxQueuedCandidates(): number {
        return 1;
      }
    }
    const once = <Value>(key: keyof typeof calls, value: Value, drift: Value) =>
      (): Value => {
        calls[key] += 1;
        return calls[key] === 1 ? value : drift;
      };
    const runtime = await initMdstream({
      loader: () => ({
        MdstreamEngineSession: EngineSession,
        MdstreamReducerSession: ReducerSession,
        abiVersion: once("abi", 1, 999),
        packageVersion: once("package", "0.4.0", "999.0.0"),
        bindingSchema: once(
          "binding",
          "mdstream.bindings/0.4",
          "mdstream.bindings/999",
        ),
        bindingOptionsSchema: once(
          "options",
          "mdstream.bindings-options/0.4",
          "mdstream.bindings-options/999",
        ),
        transitionSchema: once(
          "transition",
          "mdstream.transitions/1",
          "mdstream.transitions/999",
        ),
      }),
    });

    expect(runtime).toMatchObject({
      abiVersion: 1,
      packageVersion: "0.4.0",
      bindingSchema: "mdstream.bindings/0.4",
      bindingOptionsSchema: "mdstream.bindings-options/0.4",
      transitionSchema: "mdstream.transitions/1",
    });
    expect(calls).toEqual({
      abi: 1,
      package: 1,
      binding: 1,
      options: 1,
      transition: 1,
    });
  });

  it.each([
    ["missing", undefined],
    ["non-callable", "0.4.0"],
    ["non-string result", () => 400],
  ])("rejects a %s packageVersion probe before sessions", async (_, probe) => {
    let constructedSessions = 0;
    class ContractSession {
      constructor() {
        constructedSessions += 1;
      }

      pendingSourceView(): never {
        throw new Error("session construction must not be reached");
      }

      beginProcessorIfCurrent(): never {
        throw new Error("session construction must not be reached");
      }
    }
    const module: Record<string, unknown> = {
      MdstreamEngineSession: ContractSession,
      MdstreamReducerSession: ContractSession,
      abiVersion: () => 1,
      bindingSchema: () => "mdstream.bindings/0.4",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
      transitionSchema: () => "mdstream.transitions/1",
    };
    if (probe !== undefined) {
      module.packageVersion = probe;
    }
    const loader: WasmModuleLoader = () => module;

    await expect(initMdstream({ loader })).rejects.toMatchObject({
      status: 5,
      statusName: "MDSTREAM_UNSUPPORTED_SCHEMA",
      detailCode: "unsupported_schema",
    });
    expect(constructedSessions).toBe(0);
  });

  it.each([
    ["missing", undefined],
    ["non-callable", "mdstream.transitions/1"],
    ["non-string result", () => 1],
    ["old draft schema", () => "mdstream.transitions/draft"],
    ["future schema", () => "mdstream.transitions/2"],
  ])("rejects a %s transitionSchema probe before sessions", async (_, probe) => {
    let constructedSessions = 0;
    class ContractSession {
      constructor() {
        constructedSessions += 1;
      }

      pendingSourceView(): never {
        throw new Error("session construction must not be reached");
      }

      beginProcessorIfCurrent(): never {
        throw new Error("session construction must not be reached");
      }
    }
    const module: Record<string, unknown> = {
      MdstreamEngineSession: ContractSession,
      MdstreamReducerSession: ContractSession,
      abiVersion: () => 1,
      packageVersion: () => "0.4.0",
      bindingSchema: () => "mdstream.bindings/0.4",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
    };
    if (probe !== undefined) {
      module.transitionSchema = probe;
    }

    await expect(initMdstream({ loader: () => module })).rejects.toMatchObject({
      status: 5,
      statusName: "MDSTREAM_UNSUPPORTED_SCHEMA",
      detailCode: "unsupported_schema",
    });
    expect(constructedSessions).toBe(0);
  });

  it.each([
    "processorMaxInFlightJobs",
    "processorMaxQueuedCandidates",
  ])("rejects a reducer missing the %s capability before sessions", async (capability) => {
    let constructedSessions = 0;
    class EngineSession {}
    class ReducerSession {
      constructor() {
        constructedSessions += 1;
      }

      pendingSourceView(): never {
        throw new Error("not used");
      }

      beginProcessorIfCurrent(): never {
        throw new Error("not used");
      }

      processorMaxInFlightJobs(): number {
        return 1;
      }

      processorMaxQueuedCandidates(): number {
        return 1;
      }
    }
    Object.defineProperty(ReducerSession.prototype, capability, {
      configurable: true,
      value: undefined,
    });

    await expect(initMdstream({
      loader: () => ({
        MdstreamEngineSession: EngineSession,
        MdstreamReducerSession: ReducerSession,
        abiVersion: () => 1,
        packageVersion: () => "0.4.0",
        bindingSchema: () => "mdstream.bindings/0.4",
        bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
        transitionSchema: () => "mdstream.transitions/1",
      }),
    })).rejects.toThrow(capability);
    expect(constructedSessions).toBe(0);
  });

  it.each([
    ["zero in-flight", 0, 1],
    ["negative in-flight", -1, 1],
    ["fractional in-flight", 1.5, 1],
    ["unsafe in-flight", Number.MAX_SAFE_INTEGER + 1, 1],
    ["non-finite in-flight", Number.POSITIVE_INFINITY, 1],
    ["non-number in-flight", "1", 1],
    ["zero candidate", 1, 0],
    ["negative candidate", 1, -1],
  ])("rejects a %s effective scheduler limit and releases both sessions", async (_, jobs, candidates) => {
    let freedEngines = 0;
    let freedReducers = 0;
    class EngineSession {
      free(): void {
        freedEngines += 1;
      }
    }
    class ReducerSession {
      pendingSourceView(): never {
        throw new Error("not used");
      }

      beginProcessorIfCurrent(): never {
        throw new Error("not used");
      }

      processorMaxInFlightJobs(): number {
        return jobs as number;
      }

      processorMaxQueuedCandidates(): number {
        return candidates;
      }

      free(): void {
        freedReducers += 1;
      }
    }
    const runtime = await initMdstream({
      loader: () => ({
        MdstreamEngineSession: EngineSession,
        MdstreamReducerSession: ReducerSession,
        abiVersion: () => 1,
        packageVersion: () => "0.4.0",
        bindingSchema: () => "mdstream.bindings/0.4",
        bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
        transitionSchema: () => "mdstream.transitions/1",
      }),
    });

    expect(() => runtime.createEngine()).toThrow("positive safe integer");
    expect({ freedEngines, freedReducers }).toEqual({
      freedEngines: 1,
      freedReducers: 1,
    });
  });

});
