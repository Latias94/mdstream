import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";

import {
  asCanonicalChangeBytes,
  type CanonicalChangeBytes,
} from "../src/index.js";
import type { WasmModuleLoader } from "../src/wasm.js";

const require = createRequire(import.meta.url);
const nodeWasm = require(resolve(
  process.cwd(),
  "../../target/mdstream-wasm-node/mdstream_wasm.js",
)) as unknown;

export const nodeWasmLoader: WasmModuleLoader = () => nodeWasm;
export const textEncoder = new TextEncoder();
export const textDecoder = new TextDecoder();

export interface ProtocolFixture {
  readonly traces: readonly {
    readonly id: string;
    readonly changes: readonly unknown[];
  }[];
  readonly expected: {
    readonly normalized_snapshot: unknown;
  };
}

export function loadProtocolFixture(): ProtocolFixture {
  return JSON.parse(
    readFileSync(
      resolve(process.cwd(), "../../conformance/fixtures/protocol-linear-source.json"),
      "utf8",
    ),
  ) as ProtocolFixture;
}

export function encodeChange(change: unknown): CanonicalChangeBytes {
  return asCanonicalChangeBytes(textEncoder.encode(JSON.stringify(change)));
}

export function decodeJson(bytes: Uint8Array): Record<string, unknown> {
  return JSON.parse(textDecoder.decode(bytes)) as Record<string, unknown>;
}

export function normalizeSnapshot(snapshot: Record<string, unknown>): unknown {
  const coordinate = snapshot.coordinate as Record<string, unknown>;
  return {
    schema: snapshot.schema,
    maturity: snapshot.maturity,
    epoch: coordinate.epoch,
    lifecycle: snapshot.lifecycle,
    source: snapshot.source,
    projection_cursor: snapshot.projection_cursor,
    roots: snapshot.roots,
    nodes: snapshot.nodes,
    resources: snapshot.resources,
  };
}
