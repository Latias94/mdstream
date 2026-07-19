import { spawnSync } from "node:child_process";
import { access, readdir, rename, rm, stat } from "node:fs/promises";
import { constants } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";

const expectedVersion = "wasm-opt version 117 (version_117)";
const workspaceRoot = resolve(import.meta.dirname, "..");
const artifact = resolve(
  workspaceRoot,
  "target/mdstream-wasm-pkg/mdstream_wasm_bg.wasm",
);
const output = `${artifact}.optimized`;

const wasmOpt = await findWasmOpt();
const before = (await stat(artifact)).size;
await rm(output, { force: true });
run(wasmOpt, [
  artifact,
  "--all-features",
  "--duplicate-function-elimination",
  "--merge-similar-functions",
  "--code-folding",
  "-o",
  output,
]);
await rm(artifact, { force: true });
await rename(output, artifact);
const after = (await stat(artifact)).size;
process.stdout.write(`post-link wasm optimization: ${before} -> ${after} bytes\n`);

async function findWasmOpt() {
  const candidates = [];
  if (process.env.WASM_OPT !== undefined) {
    candidates.push(process.env.WASM_OPT);
  }
  for (const directory of (process.env.PATH ?? "").split(delimiter)) {
    if (directory.length > 0) {
      candidates.push(join(directory, executableName()));
    }
  }

  const home = homedir();
  const cacheRoots = new Set([
    process.env.XDG_CACHE_HOME === undefined
      ? undefined
      : join(process.env.XDG_CACHE_HOME, ".wasm-pack"),
    process.env.LOCALAPPDATA === undefined
      ? undefined
      : join(process.env.LOCALAPPDATA, ".wasm-pack"),
    join(home, ".cache", ".wasm-pack"),
    join(home, "Library", "Caches", ".wasm-pack"),
  ]);
  for (const root of cacheRoots) {
    if (root === undefined) {
      continue;
    }
    let entries;
    try {
      entries = await readdir(root, { withFileTypes: true });
    } catch (error) {
      if (error?.code === "ENOENT") {
        continue;
      }
      throw error;
    }
    for (const entry of entries) {
      if (entry.isDirectory() && entry.name.startsWith("wasm-opt-")) {
        candidates.push(join(root, entry.name, "bin", executableName()));
      }
    }
  }

  for (const candidate of [...new Set(candidates)].sort()) {
    try {
      await access(candidate, constants.X_OK);
    } catch {
      continue;
    }
    const probe = spawnSync(candidate, ["--version"], { encoding: "utf8" });
    if (probe.status === 0 && probe.stdout.trim() === expectedVersion) {
      return candidate;
    }
  }
  throw new Error(
    `wasm-opt 117 was not found; set WASM_OPT to the executable downloaded by wasm-pack`,
  );
}

function executableName() {
  return process.platform === "win32" ? "wasm-opt.exe" : "wasm-opt";
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: workspaceRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed:\n${result.stderr || result.stdout}`,
    );
  }
}
