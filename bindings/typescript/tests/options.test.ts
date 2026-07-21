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

      processorMaxInFlightJobs(): never {
        throw new Error("not used");
      }

      processorMaxQueuedCandidates(): never {
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
      transitionSchema: () => "mdstream.transitions/1",
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
    runtime.createStore({
      compiler: {
        maxMarkdownEvents: "8",
        maxMarkdownOverlapWork: 16n,
        maxDefinitions: "32",
        maxDefinitionEdges: 64n,
        maxDefinitionMetadataBytes: "128",
      },
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
      {
        schema: "mdstream.bindings-options/0.4",
        compiler: {
          max_markdown_events: "8",
          max_markdown_overlap_work: "16",
          max_definitions: "32",
          max_definition_edges: "64",
          max_definition_metadata_bytes: "128",
        },
      },
    ]);

    const _removedOption: MdstreamSessionOptions = {
      wire: {
        // @ts-expect-error The impact-only budget name was removed in 0.4.
        maxImpactBytes: "32768",
      },
    };
    expect(_removedOption.wire).toBeDefined();

    const _movedProtocolOption: MdstreamSessionOptions = {
      protocol: {
        // @ts-expect-error Definition registry budgets belong to the compiler.
        maxDefinitions: "32",
      },
    };
    expect(_movedProtocolOption.protocol).toBeDefined();
  });

  it("snapshots getter-backed options exactly once before constructing sessions", async () => {
    let nativeJobReads = 0;
    let nativeSlotReads = 0;
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

      processorMaxInFlightJobs(): number {
        nativeJobReads += 1;
        return 2;
      }

      processorMaxQueuedCandidates(): number {
        nativeSlotReads += 1;
        return 4;
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
      transitionSchema: () => "mdstream.transitions/1",
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

    expect({ captureReads, jobReads, slotReads, nativeJobReads, nativeSlotReads }).toEqual({
      captureReads: 1,
      jobReads: 1,
      slotReads: 1,
      nativeJobReads: 1,
      nativeSlotReads: 1,
    });
  });
});
