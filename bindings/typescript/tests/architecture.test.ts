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
      > & { readonly files?: readonly string[] }
    >;
    expect(packageJson.dependencies ?? {}).toEqual({});

    const forbidden = [
      "react",
      "react-dom",
      "@types/react",
      "@types/react-dom",
      "@react-spring/web",
      "animejs",
      "framer-motion",
      "gsap",
      "motion",
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

    expect(packageJson).toMatchObject({
      files: expect.not.arrayContaining(["examples"]),
    });
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

    const presentationImports = sources.flatMap(({ file, source }) =>
      /(?:^|\n)\s*import\s+[^;]*["'][^"']+\.(?:css|scss|sass|less)["']/u.test(source)
        ? [file]
        : [],
    );
    expect(presentationImports).toEqual([]);
  });

  it("keeps the private Web flagship outside the published package boundary", () => {
    const webRoot = resolve(process.cwd(), "../../examples/web");
    const webPackage = JSON.parse(
      readFileSync(resolve(webRoot, "package.json"), "utf8"),
    ) as {
      readonly private?: boolean;
      readonly dependencies?: Readonly<Record<string, string>>;
      readonly devDependencies?: Readonly<Record<string, string>>;
    };
    expect(webPackage.private).toBe(true);
    expect(webPackage.dependencies).toEqual({ "@mdstream/core": "workspace:*" });
    const forbiddenFrameworks = [
      "react",
      "react-dom",
      "vue",
      "svelte",
      "solid-js",
      "streamdown",
      "incremark",
      "marked",
      "merman",
    ];
    expect(Object.keys(webPackage.devDependencies ?? {})).not.toEqual(
      expect.arrayContaining(forbiddenFrameworks),
    );

    const webSource = sourceFiles(resolve(webRoot, "src"))
      .map((file) => readFileSync(file, "utf8"))
      .join("\n");
    expect(webSource).not.toMatch(/\.innerHTML\s*=/u);
    expect(webSource).not.toContain("bindings/typescript/src");
    expect(webSource).not.toMatch(/from\s+["'](?:react|vue|svelte|solid-js)["']/u);

    const workspace = readFileSync(
      resolve(process.cwd(), "../../pnpm-workspace.yaml"),
      "utf8",
    );
    expect(workspace).toContain("examples/web");
    const publishedSource = sourceFiles(resolve(process.cwd(), "src"))
      .map((file) => readFileSync(file, "utf8"))
      .join("\n");
    expect(publishedSource).not.toContain("examples/web");
  });

  it("takes effective processor scheduler limits only from the native session", () => {
    const sourceRoot = resolve(process.cwd(), "src");
    const engineSource = readFileSync(
      resolve(sourceRoot, "engine.ts"),
      "utf8",
    );
    const schedulerLimitTypeOwners = ["processors.ts", "wasm.ts"].flatMap(
      (file) => {
        const path = resolve(sourceRoot, file);
        const sourceFile = ts.createSourceFile(
          path,
          readFileSync(path, "utf8"),
          ts.ScriptTarget.Latest,
          true,
          ts.ScriptKind.TS,
        );
        return findNamedObjectTypesWithProperties(
          sourceFile,
          new Set(["maxInFlightJobs", "maxCandidates"]),
        ).map((name) => `${file}:${name}`);
      },
    );

    expect(engineSource).toContain("readProcessorSchedulerLimits(reducer)");
    expect(engineSource).not.toContain("schedulingLimit");
    expect(engineSource).not.toMatch(/maxInFlightJobs:\s*32/u);
    expect(engineSource).not.toMatch(/maxCandidates:\s*256/u);
    expect(schedulerLimitTypeOwners).toEqual([
      "processors.ts:ProcessorSchedulerLimits",
    ]);
  });

  it("keeps blocked processor retry promotion constant-time", () => {
    const processors = readFileSync(
      resolve(process.cwd(), "src/processors.ts"),
      "utf8",
    );

    expect(processors).not.toContain("this.#candidateQueue.splice");
    expect(processors).toContain("#retryCandidate");
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

function findNamedObjectTypesWithProperties(
  sourceFile: ts.SourceFile,
  requiredProperties: ReadonlySet<string>,
): string[] {
  const found: string[] = [];
  walk(sourceFile, (node) => {
    let name: string | undefined;
    let members: ts.NodeArray<ts.TypeElement> | undefined;
    if (ts.isInterfaceDeclaration(node)) {
      name = node.name.text;
      members = node.members;
    } else if (ts.isTypeAliasDeclaration(node) && ts.isTypeLiteralNode(node.type)) {
      name = node.name.text;
      members = node.type.members;
    }
    if (name === undefined || members === undefined) {
      return;
    }
    const properties = new Set(
      members.flatMap((member) =>
        ts.isPropertySignature(member) &&
        member.name !== undefined &&
        ts.isIdentifier(member.name)
          ? [member.name.text]
          : [],
      ),
    );
    if ([...requiredProperties].every((property) => properties.has(property))) {
      found.push(name);
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
