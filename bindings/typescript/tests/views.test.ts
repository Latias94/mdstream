import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  decodeBindingView,
  MdstreamError,
  type ContentKindView,
  type NodeView,
  type SemanticResourceKindView,
} from "../src/views.js";
import { BindingPayloadKind } from "../src/wasm.js";

interface ContentIrFixture {
  readonly schema: string;
  readonly semantic_text: readonly Readonly<Record<string, unknown>>[];
  readonly code_block_syntax: readonly Readonly<Record<string, unknown>>[];
  readonly link_styles: readonly string[];
  readonly block_quote_kinds: readonly string[];
  readonly table_alignments: readonly string[];
  readonly content_kinds: readonly Readonly<Record<string, unknown>>[];
  readonly semantic_resource_kinds: readonly Readonly<Record<string, unknown>>[];
}

const schema = "mdstream.bindings/0.4";
const fixture = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "../../conformance/bindings/content-ir.json"),
    "utf8",
  ),
) as ContentIrFixture;
const encoder = new TextEncoder();

describe("typed Content IR binding views", () => {
  it("decodes every Rust ContentKind and SemanticResourceKind variant", () => {
    const contentKinds = fixture.content_kinds.map(decodeContent);
    const resourceKinds = fixture.semantic_resource_kinds.map(decodeResource);

    expect(new Set(contentKinds.map((content) => content.kind))).toEqual(
      new Set(fixture.content_kinds.map((content) => content.kind)),
    );
    expect(new Set(resourceKinds.map((content) => content.kind))).toEqual(
      new Set(fixture.semantic_resource_kinds.map((content) => content.kind)),
    );
    expect(contentKinds.map(describeContent)).toHaveLength(28);
    expect(resourceKinds.map(describeResource)).toHaveLength(3);
  });

  it("decodes every nested semantic-text, syntax, link, quote, and table variant", () => {
    for (const text of fixture.semantic_text) {
      expect(decodeContent({ kind: "text", text }).kind).toBe("text");
    }
    for (const syntax of fixture.code_block_syntax) {
      const content = decodeContent({
        kind: "code_block",
        syntax,
        info: null,
        text: { kind: "source" },
      });
      expect(content.kind).toBe("code_block");
    }
    for (const style of fixture.link_styles) {
      const content = decodeContent({
        kind: "link",
        target: null,
        reference_label: null,
        style,
      });
      expect(content.kind).toBe("link");
    }
    for (const style of fixture.block_quote_kinds) {
      expect(decodeContent({ kind: "block_quote", style }).kind).toBe(
        "block_quote",
      );
    }
    const table = decodeContent({
      kind: "table",
      alignments: fixture.table_alignments,
    });
    expect(table).toMatchObject({
      kind: "table",
      alignments: fixture.table_alignments,
    });
  });

  it("deep-freezes decoded arrays and extension attributes", () => {
    const table = decodeContent(
      fixture.content_kinds.find(({ kind }) => kind === "table")!,
    );
    const custom = decodeContent(
      fixture.content_kinds.find(({ kind }) => kind === "custom")!,
    );
    expect(table.kind).toBe("table");
    expect(custom.kind).toBe("custom");
    if (table.kind !== "table" || custom.kind !== "custom") {
      throw new Error("fixture variants did not decode");
    }

    expect(Object.isFrozen(table.alignments)).toBe(true);
    expect(Object.isFrozen(custom.attributes)).toBe(true);
    expect(() => (table.alignments as string[]).push("left")).toThrow();
    expect(() => {
      (custom.attributes as Record<string, string>).role = "changed";
    }).toThrow();
  });

  it("rejects unknown, malformed, and variant-incompatible content", () => {
    const malformed = [
      { kind: "unknown" },
      { kind: "heading" },
      { kind: "paragraph", level: 1 },
      {
        kind: "link",
        target: null,
        reference_label: null,
        style: "invalid",
      },
      {
        kind: "custom",
        namespace: "app",
        name: "panel",
        opaque: true,
        attributes: { invalid: 1 },
      },
    ];

    for (const content of malformed) {
      expect(() => decodeContent(content)).toThrowError(
        expect.objectContaining({
          status: 12,
          detailCode: "bindings.invalid_payload",
        }),
      );
    }
  });

  it("enforces u64 counters and u128 content IDs on native output", () => {
    const maxNode = decodeNode({ kind: "paragraph" }, {
      id: "340282366920938463463374607431768211455",
    });
    expect(maxNode.node.id).toBe("340282366920938463463374607431768211455");

    expect(() =>
      decodeNode({ kind: "paragraph" }, {
        source: {
          start: "0",
          end: "18446744073709551616",
        },
      }),
    ).toThrowError(MdstreamError);
    expect(() =>
      decodeNode({ kind: "paragraph" }, {
        id: "340282366920938463463374607431768211456",
      }),
    ).toThrowError(MdstreamError);
  });
});

function decodeContent(
  content: Readonly<Record<string, unknown>>,
): ContentKindView {
  return decodeNode(content).node.content;
}

function decodeNode(
  content: Readonly<Record<string, unknown>>,
  overrides: Readonly<Record<string, unknown>> = {},
): NodeView {
  const node = {
    id: "7",
    version: "sha256:node",
    stability: "stable",
    source: { start: "0", end: "4" },
    body: { start: "0", end: "4" },
    children: { version: "sha256:children", children: [] },
    content,
    ...overrides,
  };
  return decodeBindingView(
    BindingPayloadKind.NodeView,
    encoder.encode(JSON.stringify({
      schema,
      kind: "node_view",
      node,
      body_text: "body",
    })),
    schema,
  ) as NodeView;
}

function decodeResource(
  content: Readonly<Record<string, unknown>>,
): SemanticResourceKindView {
  const view = decodeBindingView(
    BindingPayloadKind.ResourceView,
    encoder.encode(JSON.stringify({
      schema,
      kind: "resource_view",
      resource: {
        id: "9",
        version: "sha256:resource",
        content,
      },
    })),
    schema,
  );
  if (view.kind !== "resource_view") {
    throw new Error("fixture did not decode as a resource view");
  }
  return view.resource.content;
}

function describeContent(content: ContentKindView): string {
  switch (content.kind) {
    case "paragraph":
    case "emphasis":
    case "strong":
    case "strikethrough":
    case "thematic_break":
    case "table_head":
    case "table_body":
    case "table_row":
    case "soft_break":
    case "hard_break":
      return content.kind;
    case "heading":
      return `heading:${content.level}`;
    case "text":
    case "inline_code":
      return `${content.kind}:${content.text.kind}`;
    case "link":
      return `link:${content.style}:${content.referenceLabel ?? content.target?.id ?? ""}`;
    case "image":
      return `image:${content.style}:${content.alt.kind}`;
    case "code_block":
      return `code:${content.syntax.kind}:${content.info ?? ""}:${content.text.kind}`;
    case "list":
      return `list:${content.ordered}:${content.start ?? ""}:${content.tight}`;
    case "list_item":
      return `item:${content.checked ?? ""}`;
    case "block_quote":
      return `quote:${content.style}`;
    case "table":
      return `table:${content.alignments.join(",")}`;
    case "table_cell":
      return `cell:${content.column}`;
    case "html":
      return `html:${content.block}:${content.text.kind}`;
    case "math":
      return `math:${content.display}:${content.text.kind}`;
    case "footnote_definition":
      return `footnote-definition:${content.label}:${content.target.id}`;
    case "footnote_reference":
      return `footnote-reference:${content.label}:${content.target?.id ?? ""}`;
    case "citation_definition":
      return `citation-definition:${content.key}:${content.target.id}`;
    case "citation_reference":
      return `citation-reference:${content.key}:${content.target?.id ?? ""}`;
    case "custom":
      return `custom:${content.namespace}:${content.name}:${content.opaque}:${content.attributes.role ?? ""}`;
  }
  const exhaustive: never = content;
  return exhaustive;
}

function describeResource(content: SemanticResourceKindView): string {
  switch (content.kind) {
    case "link":
      return `link:${content.destination}:${content.title ?? ""}`;
    case "footnote":
      return `footnote:${content.label}`;
    case "citation":
      return `citation:${content.protocol}:${content.key}:${content.destination}:${content.title ?? ""}`;
  }
  const exhaustive: never = content;
  return exhaustive;
}
