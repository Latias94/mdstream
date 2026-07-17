import { describe, expect, it } from "vitest";

import { initMdstream, utf8ByteLength } from "../src/index.js";
import {
  decodeJson,
  nodeWasmLoader,
  normalizeSnapshot,
} from "./helpers.js";

describe("lossless UTF-8 input batching", () => {
  it("preserves state for 1/16/128/4096-byte batches", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const chunks = [
      "# 批处理\r",
      "",
      "\n\n",
      "emoji 👩‍💻 and ",
      "accent é",
      "\n\n```mermaid\nflowchart LR\nA-->B\n```",
    ];
    let expected: unknown;

    for (const size of [1, 16, 128, 4096]) {
      const engine = runtime.createEngine();
      const batcher = engine.createBatcher(size);
      for (const chunk of chunks) {
        batcher.push(chunk);
      }
      batcher.finish();
      const snapshot = batcher.createRecoverySnapshot()!;
      const normalized = normalizeSnapshot(decodeJson(snapshot));
      expected ??= normalized;
      expect(normalized).toEqual(expected);
      expect(decodeJson(snapshot).source).toContain("emoji 👩‍💻 and accent é");

      const metrics = batcher.metrics();
      expect(metrics.inputBytes).toBe(metrics.forwardedBytes);
      expect(metrics.pendingBytes).toBe("0");
      expect(BigInt(metrics.wasmAppendCalls)).toBeGreaterThan(0n);
      expect(BigInt(metrics.outputPayloadBytes)).toBeGreaterThan(0n);
      engine.close();
    }
  });

  it("counts UTF-8 without allocation and rejects ill-formed UTF-16", async () => {
    expect(utf8ByteLength("aé👩‍💻")).toBe(new TextEncoder().encode("aé👩‍💻").length);
    expect(() => utf8ByteLength("\ud800")).toThrow(/unpaired UTF-16 high/);
    expect(() => utf8ByteLength("\udc00")).toThrow(/unpaired UTF-16 low/);

    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    expect(() => engine.append("\ud800")).toThrow(TypeError);
    const batcher = engine.createBatcher(16);
    expect(() => batcher.push("\udc00")).toThrow(TypeError);
    engine.close();
  });

  it("does not let an empty chunk flush a pending carriage return", async () => {
    const runtime = await initMdstream({ loader: nodeWasmLoader });
    const engine = runtime.createEngine();
    const batcher = engine.createBatcher(128);
    batcher.push("line\r");
    batcher.push("");
    expect(batcher.metrics().wasmAppendCalls).toBe("0");
    batcher.push("\nnext");
    batcher.finish();
    expect(decodeJson(batcher.createRecoverySnapshot()!).source).toBe("line\nnext");
    engine.close();
  });
});
