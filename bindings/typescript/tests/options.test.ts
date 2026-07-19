import { describe, expect, it } from "vitest";

import {
  initMdstream,
  type MdstreamSessionOptions,
  type WasmModuleLoader,
} from "../src/index.js";

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
      transitionSchema: () => "mdstream.transitions/draft",
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
    runtime.createStore({
      captureTransitions: true,
      wire: { maxReducerUpdateBytes: "32768" },
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
      {
        schema: "mdstream.bindings-options/0.4",
        capture_transitions: true,
        wire: { max_reducer_update_bytes: "32768" },
      },
    ]);

    const _removedOption: MdstreamSessionOptions = {
      wire: {
        // @ts-expect-error The impact-only budget name was removed in 0.4.
        maxImpactBytes: "32768",
      },
    };
    expect(_removedOption.wire).toBeDefined();
  });

  it("snapshots getter-backed options exactly once before constructing sessions", async () => {
    class EngineSession {
      free(): void {}
    }
    class ReducerSession {
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
      transitionSchema: () => "mdstream.transitions/draft",
    });
    const runtime = await initMdstream({ loader });
    let captureReads = 0;
    let jobReads = 0;
    let slotReads = 0;
    const processor = Object.defineProperties({}, {
      maxInFlightJobs: {
        enumerable: true,
        get: () => {
          jobReads += 1;
          return "2";
        },
      },
      maxSlots: {
        enumerable: true,
        get: () => {
          slotReads += 1;
          return "4";
        },
      },
    });
    const options = Object.defineProperties({ processor }, {
      captureTransitions: {
        enumerable: true,
        get: () => {
          captureReads += 1;
          return true;
        },
      },
    }) as MdstreamSessionOptions;

    runtime.createEngine(options).close();

    expect({ captureReads, jobReads, slotReads }).toEqual({
      captureReads: 1,
      jobReads: 1,
      slotReads: 1,
    });
  });
});
