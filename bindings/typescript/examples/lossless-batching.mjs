#!/usr/bin/env node

import assert from "node:assert/strict";

import {
  BatchOperationError,
  initMdstream,
} from "../dist/index.js";

const assertMode = process.argv.includes("--assert");
const unknownArguments = process.argv
  .slice(2)
  .filter((argument) => argument !== "--assert");
if (unknownArguments.length > 0) {
  throw new Error(`unknown argument(s): ${unknownArguments.join(", ")}`);
}

const decoder = new TextDecoder("utf-8", { fatal: true });
const runtime = await initMdstream();

const engine = runtime.createEngine();
const batcher = engine.createBatcher({
  maxBatchBytes: 64,
  maxPendingChunks: 32,
});
const results = [];
for (const chunk of ["# Batched\n\n", "one ", "ordered ", "stream"]) {
  results.push(...batcher.push(chunk));
}
results.push(...batcher.flush());
results.push(...batcher.finish());
batcher.release();
const snapshot = engine.createRecoverySnapshot();
assert(snapshot !== undefined);
const source = JSON.parse(decoder.decode(snapshot)).source;
assert.equal(source, "# Batched\n\none ordered stream");
engine.close();

const recoveryEngine = runtime.createEngine({
  protocol: { maxSourceBytes: 64n },
  wire: { maxCommandBytes: 3n },
});
const recoveryBatcher = recoveryEngine.createBatcher({
  maxBatchBytes: 64,
  maxPendingChunks: 8,
});
recoveryBatcher.push("a");
recoveryBatcher.push("1234");
recoveryBatcher.push("suffix");

let recovery;
try {
  recoveryBatcher.flush();
  assert.fail("the wire-limited constituent must fail");
} catch (error) {
  assert(error instanceof BatchOperationError);
  assert.equal(error.operation, "flush");
  assert.equal(error.completedResults.length, 1);
  assert.deepEqual(error.pending?.chunks, ["1234", "suffix"]);
  recovery = recoveryBatcher.takePending();
}
assert.deepEqual(recovery?.chunks, ["1234", "suffix"]);
recoveryBatcher.release();
recoveryEngine.close();

const report = {
  assertions: "passed",
  committedResults: results.length,
  source,
  transferredPendingChunks: recovery?.chunks.length ?? 0,
};
if (assertMode) {
  assert.equal(report.assertions, "passed");
}
process.stdout.write(`${JSON.stringify(report)}\n`);
