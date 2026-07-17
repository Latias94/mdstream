import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

describe("framework-neutral package boundaries", () => {
  it("has no renderer, UI framework, Merman, or parser dependency", () => {
    const packageJson = JSON.parse(
      readFileSync(resolve(process.cwd(), "package.json"), "utf8"),
    ) as Readonly<
      Record<
        "dependencies" | "devDependencies" | "optionalDependencies" | "peerDependencies",
        Readonly<Record<string, string>> | undefined
      >
    >;
    expect(packageJson.dependencies ?? {}).toEqual({});

    const forbidden = [
      "react",
      "react-dom",
      "@types/react",
      "@types/react-dom",
      "streamdown",
      "incremark",
      "merman",
      "marked",
      "micromark",
      "remark",
      "unified",
    ];
    const dependencyGroups = [
      packageJson.dependencies,
      packageJson.devDependencies,
      packageJson.optionalDependencies,
      packageJson.peerDependencies,
    ];
    for (const dependencies of dependencyGroups) {
      for (const dependency of Object.keys(dependencies ?? {})) {
        expect(forbidden).not.toContain(dependency.toLowerCase());
      }
    }

    const workspace = readFileSync(
      resolve(process.cwd(), "../../pnpm-workspace.yaml"),
      "utf8",
    );
    expect(workspace).not.toContain("bindings/react");
    expect(existsSync(resolve(process.cwd(), "../react"))).toBe(false);
  });

  it("keeps JSON decoding and canonical reduction out of adapters", () => {
    const sourceRoot = resolve(process.cwd(), "src");
    const files = sourceFiles(sourceRoot);
    const sources = files.map((file) => ({
      file,
      source: readFileSync(file, "utf8"),
    }));
    const parseSites = sources.flatMap(({ file, source }) =>
      source.includes("JSON.parse") ? [file] : [],
    );
    expect(parseSites).toEqual([resolve(sourceRoot, "views.ts")]);

    const forbiddenReducerVocabulary = [
      "insert_node",
      "replace_node",
      "stabilize_node",
      "remove_node",
      "splice_children",
      "advance_projection",
      "finish_document",
      "interface ChangeSet",
      "interface ProjectionOp",
      "interface Snapshot",
    ];
    const combined = sources.map(({ source }) => source).join("\n");
    for (const token of forbiddenReducerVocabulary) {
      expect(combined).not.toContain(token);
    }
  });
});

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((entry) => {
    const path = resolve(directory, entry);
    return statSync(path).isDirectory() ? sourceFiles(path) : [path];
  });
}
