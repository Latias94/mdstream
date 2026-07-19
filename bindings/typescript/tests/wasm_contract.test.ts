import { describe, expect, it } from "vitest";

import { initMdstream, type WasmModuleLoader } from "../src/index.js";

describe("custom WASM loader contract", () => {
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
});
