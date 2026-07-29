import { spawnSync } from "node:child_process";
import { cp, mkdir, readdir, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const source = resolve(packageRoot, "../../target/mdstream-wasm-pkg");
const destination = resolve(packageRoot, "wasm");
const extensions = new Set([".js", ".d.ts", ".wasm"]);
const wasmName = "mdstream_wasm_bg.wasm";

await rm(destination, { recursive: true, force: true });
await mkdir(destination, { recursive: true });

for (const entry of await readdir(source)) {
  if (entry === wasmName) {
    const stripped = spawnSync(
      "wasm-tools",
      ["strip", "--all", resolve(source, entry), "-o", resolve(destination, entry)],
      { encoding: "utf8" },
    );
    if (stripped.status !== 0) {
      throw new Error(stripped.stderr || stripped.stdout || "wasm-tools strip failed");
    }
  } else if ([...extensions].some((extension) => entry.endsWith(extension))) {
    await cp(resolve(source, entry), resolve(destination, entry));
  }
}
