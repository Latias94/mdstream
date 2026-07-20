import { readFile } from "node:fs/promises";

import { initMdstream } from "../dist/index.js";

const emittedWasmLoader = await readFile(
  new URL("../dist/wasm.js", import.meta.url),
  "utf8",
);
const viteIgnoreCount = emittedWasmLoader.match(/\/\* @vite-ignore \*\//gu)?.length ?? 0;
if (viteIgnoreCount !== 2) {
  throw new Error("packaged WASM loader lost its two intentional dynamic-import annotations");
}

const runtime = await initMdstream();
const engine = runtime.createEngine();
const appended = engine.append("# Package smoke\n\nbody");
if (appended.changes.length === 0) {
  throw new Error("packaged engine emitted no change");
}
engine.finish();
if (engine.createRecoverySnapshot() === undefined) {
  throw new Error("packaged engine emitted no explicit recovery snapshot");
}
engine.close();
