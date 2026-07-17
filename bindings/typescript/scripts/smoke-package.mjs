import { initMdstream } from "../dist/index.js";

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
