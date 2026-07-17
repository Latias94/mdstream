import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const packageRoot = resolve(scriptDirectory, "..");
const workspaceRoot = resolve(packageRoot, "../..");
const targetRoot = resolve(workspaceRoot, "target");
const rawPath = resolve(targetRoot, "mdstream-wasm-pkg/mdstream_wasm_bg.wasm");
const strippedPath = resolve(targetRoot, "mdstream-wasm-size/mdstream_wasm_bg.stripped.wasm");
const packageWasmPath = resolve(packageRoot, "wasm/mdstream_wasm_bg.wasm");
const gzipPath = `${strippedPath}.gz`;
const brotliPath = `${strippedPath}.br`;
const packPath = resolve(targetRoot, "npm-pack/mdstream-core.tgz");
const budgetsPath = resolve(workspaceRoot, "bindings/budgets.json");
const reportPath = resolve(targetRoot, "mdstream-binding-artifacts.json");

const check = process.argv.includes("--check");
if (!check) {
  throw new Error("measure-artifacts.mjs requires --check");
}

await mkdir(dirname(strippedPath), { recursive: true });
await mkdir(dirname(packPath), { recursive: true });
run("wasm-tools", ["strip", "--all", rawPath, "-o", strippedPath]);

const raw = await readFile(rawPath);
const stripped = await readFile(strippedPath);
const packageWasm = await readFile(packageWasmPath);
if (!packageWasm.equals(stripped)) {
  throw new Error("npm package WASM is not the stripped release artifact");
}

const gzip = gzipSync(stripped, { level: 9, mtime: 0 });
const brotli = brotliCompressSync(stripped, {
  params: {
    [constants.BROTLI_PARAM_MODE]: constants.BROTLI_MODE_GENERIC,
    [constants.BROTLI_PARAM_QUALITY]: 11,
    [constants.BROTLI_PARAM_LGWIN]: 22,
    [constants.BROTLI_PARAM_SIZE_HINT]: stripped.byteLength,
  },
});
await writeFile(gzipPath, gzip);
await writeFile(brotliPath, brotli);

run(
  "pnpm",
  ["pack", "--out", packPath],
  {
    cwd: packageRoot,
    env: {
      ...process.env,
      npm_config_ignore_scripts: "true",
      PNPM_CONFIG_IGNORE_SCRIPTS: "true",
    },
  },
);
const packed = await readFile(packPath);

const dependencyTree = run("cargo", ["tree", "-p", "mdstream-wasm", "--prefix", "none"], {
  cwd: workspaceRoot,
}).stdout.toLowerCase();
for (const forbidden of ["merman", "react", "streamdown", "incremark"]) {
  if (dependencyTree.includes(forbidden)) {
    throw new Error(`default WASM dependency tree contains forbidden dependency ${forbidden}`);
  }
}

const budgets = JSON.parse(await readFile(budgetsPath, "utf8"));
const measurements = new Map([
  ["wasm_raw", artifact(raw)],
  ["wasm_stripped", artifact(stripped)],
  ["wasm_gzip", artifact(gzip)],
  ["wasm_brotli", artifact(brotli)],
  ["npm_packed", artifact(packed)],
]);
const failures = [];
const warnings = [];
for (const budget of budgets.artifacts) {
  const measurement = measurements.get(budget.artifact);
  if (measurement === undefined) {
    continue;
  }
  if (measurement.bytes > budget.ceiling_bytes) {
    failures.push(
      `${budget.artifact}: ${measurement.bytes} exceeds ${budget.ceiling_bytes}`,
    );
  }
  const baseline = budget.measurement?.measured_bytes;
  const regression = budget.regression_percent;
  if (typeof baseline === "number" && typeof regression === "number") {
    const advisory = Math.floor((baseline * (100 + regression)) / 100);
    if (measurement.bytes > advisory) {
      warnings.push(
        `${budget.artifact}: ${measurement.bytes} exceeds advisory baseline ${advisory}`,
      );
    }
  }
}

const toolchain = {
  rust: run("rustup", ["run", "1.85.0", "rustc", "--version"]).stdout.trim(),
  wasmPack: run("wasm-pack", ["--version"]).stdout.trim(),
  wasmTools: run("wasm-tools", ["--version"]).stdout.trim(),
  node: process.version,
  pnpm: run("pnpm", ["--version"]).stdout.trim(),
};
const toolchainErrors = [
  [toolchain.rust.startsWith("rustc 1.85.0 "), `Rust 1.85.0, found ${toolchain.rust}`],
  [toolchain.wasmPack === "wasm-pack 0.15.0", `wasm-pack 0.15.0, found ${toolchain.wasmPack}`],
  [toolchain.wasmTools === "wasm-tools 1.253.0", `wasm-tools 1.253.0, found ${toolchain.wasmTools}`],
  [toolchain.node.startsWith("v24."), `Node 24.x, found ${toolchain.node}`],
  [toolchain.pnpm === "11.9.0", `pnpm 11.9.0, found ${toolchain.pnpm}`],
].flatMap(([matches, message]) => matches ? [] : [message]);
if (toolchainErrors.length > 0) {
  throw new Error(`artifact toolchain mismatch:\n${toolchainErrors.join("\n")}`);
}

const report = {
  schema: "mdstream.binding-artifacts/1",
  measurements: Object.fromEntries(measurements),
  toolchain,
  commands: {
    build: "pnpm wasm:build:web",
    strip: "wasm-tools strip --all",
    gzip: "node:zlib gzip level=9 mtime=0",
    brotli: "node:zlib brotli quality=11 lgwin=22",
    pack: "pnpm pack (lifecycle scripts disabled)",
  },
  warnings,
};
await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);

for (const [name, measurement] of measurements) {
  process.stdout.write(`${name.padEnd(16)} ${measurement.bytes.toString().padStart(9)}  ${measurement.sha256}\n`);
}
for (const warning of warnings) {
  process.stderr.write(`warning: ${warning}\n`);
}
if (failures.length > 0) {
  throw new Error(`artifact budgets failed:\n${failures.join("\n")}`);
}

function artifact(bytes) {
  return {
    bytes: bytes.byteLength,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? workspaceRoot,
    env: options.env ?? process.env,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed:\n${result.stderr || result.stdout}`,
    );
  }
  return result;
}
