import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { resolve } from "node:path";

import ts from "typescript";
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
    const analyses = sources.map(({ file, source }) =>
      analyzeSource(file, source),
    );
    const parseSites = analyses.flatMap(({ file, jsonParseCalls }) =>
      jsonParseCalls > 0 ? [file] : [],
    );
    expect(parseSites).toEqual([resolve(sourceRoot, "views.ts")]);

    const forbiddenReducerOperations = new Set([
      "insert_node",
      "replace_node",
      "stabilize_node",
      "remove_node",
      "splice_children",
      "advance_projection",
      "finish_document",
    ]);
    const forbiddenReducerTypes = new Set([
      "ChangeSet",
      "ProjectionOp",
      "Snapshot",
    ]);
    const reducerConstructs = analyses.flatMap(({ file, sourceFile }) =>
      findReducerConstructs(
        file,
        sourceFile,
        forbiddenReducerOperations,
        forbiddenReducerTypes,
      ),
    );
    expect(reducerConstructs).toEqual([]);
  });
});

interface SourceAnalysis {
  readonly file: string;
  readonly sourceFile: ts.SourceFile;
  readonly jsonParseCalls: number;
}

function analyzeSource(file: string, source: string): SourceAnalysis {
  const sourceFile = ts.createSourceFile(
    file,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  let jsonParseCalls = 0;
  walk(sourceFile, (node) => {
    if (
      ts.isCallExpression(node) &&
      ts.isPropertyAccessExpression(node.expression) &&
      ts.isIdentifier(node.expression.expression) &&
      node.expression.expression.text === "JSON" &&
      node.expression.name.text === "parse"
    ) {
      jsonParseCalls += 1;
    }
  });
  return { file, sourceFile, jsonParseCalls };
}

function findReducerConstructs(
  file: string,
  sourceFile: ts.SourceFile,
  operations: ReadonlySet<string>,
  types: ReadonlySet<string>,
): string[] {
  const found: string[] = [];
  walk(sourceFile, (node) => {
    if (ts.isStringLiteral(node) && operations.has(node.text)) {
      found.push(`${file}: operation ${node.text}`);
    }
    if (
      (ts.isInterfaceDeclaration(node) ||
        ts.isTypeAliasDeclaration(node) ||
        ts.isClassDeclaration(node) ||
        ts.isEnumDeclaration(node)) &&
      node.name !== undefined &&
      types.has(node.name.text)
    ) {
      found.push(`${file}: type ${node.name.text}`);
    }
  });
  return found;
}

function walk(node: ts.Node, visit: (node: ts.Node) => void): void {
  visit(node);
  node.forEachChild((child) => walk(child, visit));
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory).flatMap((entry) => {
    const path = resolve(directory, entry);
    return statSync(path).isDirectory() ? sourceFiles(path) : [path];
  });
}
