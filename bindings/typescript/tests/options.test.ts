import { describe, expect, it } from "vitest";

import { initMdstream, type WasmModuleLoader } from "../src/index.js";

describe("binding option parity", () => {
  it("omits unspecified custom-block booleans and preserves explicit false", async () => {
    const encoded: unknown[] = [];
    class EngineSession {}
    class ReducerSession {
      constructor(options?: string) {
        encoded.push(options === undefined ? undefined : JSON.parse(options));
      }

      pendingSourceView(): never {
        throw new Error("not used");
      }

      beginProcessorIfCurrent(): never {
        throw new Error("not used");
      }

      free(): void {}
    }
    const loader: WasmModuleLoader = () => ({
      MdstreamEngineSession: EngineSession,
      MdstreamReducerSession: ReducerSession,
      abiVersion: () => 1,
      packageVersion: () => "0.4.0",
      bindingSchema: () => "mdstream.bindings/0.4",
      bindingOptionsSchema: () => "mdstream.bindings-options/0.4",
    });
    const runtime = await initMdstream({ loader });

    runtime.createStore({
      customBlocks: [{ namespace: "app", name: "defaulted" }],
    }).close();
    runtime.createStore({
      customBlocks: [{
        namespace: "app",
        name: "explicit",
        opaque: false,
        caseInsensitive: false,
      }],
    }).close();

    expect(encoded).toEqual([
      {
        schema: "mdstream.bindings-options/0.4",
        custom_blocks: [{ namespace: "app", name: "defaulted" }],
      },
      {
        schema: "mdstream.bindings-options/0.4",
        custom_blocks: [{
          namespace: "app",
          name: "explicit",
          opaque: false,
          case_insensitive: false,
        }],
      },
    ]);
  });
});
