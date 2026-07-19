import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  decodeBindingView,
  MdstreamError,
  type ContentKindView,
  type NodeView,
  type ReducerUpdateView,
  type SemanticResourceKindView,
  type TransitionFactsView,
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

describe("typed transition binding views", () => {
  it("decodes and deep-freezes a complete continuous transition", () => {
    const update = decodeUpdate(continuousTransition());
    const transition = update.transition;
    expect(transition?.schema).toBe("mdstream.transitions/1");
    expect(transition?.facts.scope).toBe("continuous");
    expect(Object.isFrozen(transition)).toBe(true);
    expect(Object.isFrozen(transition?.facts)).toBe(true);
    if (transition?.facts.scope !== "continuous") {
      throw new Error("fixture did not decode as continuous transition facts");
    }
    const facts: TransitionFactsView = transition.facts;
    expect(facts.nodes[0]?.text).toEqual({
      kind: "projection_append",
      range: { start: "1", end: "2" },
      text: "B",
    });
    expect(facts.structures[0]?.owner.kind).toBe("node");
    expect(facts.resources[0]?.affectedNodes[0]?.nodeId).toBe("7");
    expect(Object.isFrozen(facts.nodes)).toBe(true);
    expect(Object.isFrozen(facts.structures[0]?.inserted)).toBe(true);
    expect(() => {
      (facts.nodes as unknown[]).push({});
    }).toThrow();
  });

  it("keeps transition optional and decodes a coarse full replacement", () => {
    const withoutTransition = decodeUpdate();
    expect(withoutTransition.transition).toBeUndefined();
    expect(Object.hasOwn(withoutTransition, "transition")).toBe(false);

    const fullReplace = decodeUpdate({
      schema: "mdstream.transitions/1",
      facts: {
        scope: "full_replace",
        before: null,
        after: documentStamp("1"),
      },
    }).transition?.facts;
    expect(fullReplace).toMatchObject({
      scope: "full_replace",
      after: { continuityGeneration: "1" },
    });
    expect(fullReplace).not.toHaveProperty("nodes");
  });

  it("rejects the wrong transition schema and every unknown nested field", () => {
    const wrongSchema = continuousTransition();
    wrongSchema.schema = "mdstream.transitions/draft";
    expect(() => decodeUpdate(wrongSchema)).toThrowError(MdstreamError);

    const mutations: ((transition: MutableTransitionFixture) => void)[] = [
      (transition) => { transition.unexpected = true; },
      (transition) => { transition.facts.unexpected = true; },
      (transition) => { transition.facts.after.unexpected = true; },
      (transition) => { transition.facts.after.coordinate.unexpected = true; },
      (transition) => { transition.facts.nodes[0]!.key.unexpected = true; },
      (transition) => { transition.facts.nodes[0]!.after!.unexpected = true; },
      (transition) => {
        const parent = transition.facts.nodes[0]!.after!.parent!;
        parent.unexpected = true;
      },
      (transition) => { transition.facts.nodes[0]!.text!.unexpected = true; },
      (transition) => { transition.facts.structures[0]!.unexpected = true; },
      (transition) => { transition.facts.resources[0]!.unexpected = true; },
      (transition) => { transition.facts.resources[0]!.key.unexpected = true; },
    ];

    for (const mutate of mutations) {
      const transition = continuousTransition();
      mutate(transition);
      expect(() => decodeUpdate(transition)).toThrowError(
        expect.objectContaining({ detailCode: "bindings.invalid_payload" }),
      );
    }
  });

  it("rejects unknown transition variants and missing required nullable fields", () => {
    const unknownScope = continuousTransition();
    unknownScope.facts.scope = "incremental";
    expect(() => decodeUpdate(unknownScope)).toThrowError(MdstreamError);

    const unknownOwner = continuousTransition();
    unknownOwner.facts.structures[0]!.owner.kind = "root";
    expect(() => decodeUpdate(unknownOwner)).toThrowError(MdstreamError);

    const unknownText = continuousTransition();
    unknownText.facts.nodes[0]!.text!.kind = "append";
    expect(() => decodeUpdate(unknownText)).toThrowError(MdstreamError);

    const missingBefore = continuousTransition();
    delete missingBefore.facts.nodes[0]!.before;
    expect(() => decodeUpdate(missingBefore)).toThrowError(MdstreamError);

    const missingParent = continuousTransition();
    delete missingParent.facts.nodes[0]!.after!.parent;
    expect(() => decodeUpdate(missingParent)).toThrowError(MdstreamError);
  });

  it("enforces transition u64 and u128 decimal bounds", () => {
    const maximum = continuousTransition();
    maximum.facts.after.continuity_generation = "18446744073709551615";
    maximum.facts.after.coordinate.epoch = "18446744073709551615";
    maximum.facts.nodes[0]!.key.node_id =
      "340282366920938463463374607431768211455";
    maximum.facts.resources[0]!.key.resource_id =
      "340282366920938463463374607431768211455";
    expect(decodeUpdate(maximum).transition).toBeDefined();

    const overflowGeneration = continuousTransition();
    overflowGeneration.facts.after.continuity_generation = "18446744073709551616";
    expect(() => decodeUpdate(overflowGeneration)).toThrowError(MdstreamError);

    const overflowNode = continuousTransition();
    overflowNode.facts.nodes[0]!.key.node_id =
      "340282366920938463463374607431768211456";
    expect(() => decodeUpdate(overflowNode)).toThrowError(MdstreamError);

    const nonCanonical = continuousTransition();
    nonCanonical.facts.resources[0]!.key.resource_id = "07";
    expect(() => decodeUpdate(nonCanonical)).toThrowError(MdstreamError);
  });

  it("rejects invalid opaque identifiers in transition stamps", () => {
    for (const value of ["", "x".repeat(129), "版本", "invalid/value"]) {
      const invalidChange = continuousTransition();
      invalidChange.facts.after.coordinate.change_id = value;
      expect(() => decodeUpdate(invalidChange)).toThrowError(MdstreamError);

      const invalidNodeVersion = continuousTransition();
      invalidNodeVersion.facts.nodes[0]!.after!.version = value;
      expect(() => decodeUpdate(invalidNodeVersion)).toThrowError(MdstreamError);

      const invalidStructureVersion = continuousTransition();
      invalidStructureVersion.facts.structures[0]!.before_version = value;
      expect(() => decodeUpdate(invalidStructureVersion)).toThrowError(MdstreamError);

      const invalidResourceVersion = continuousTransition();
      invalidResourceVersion.facts.resources[0]!.after_version = value;
      expect(() => decodeUpdate(invalidResourceVersion)).toThrowError(MdstreamError);
    }
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

interface MutableRecord {
  [key: string]: unknown;
  unexpected?: unknown;
}

interface MutableCoordinate extends MutableRecord {
  epoch: string;
  sequence: string;
  change_id: string;
  source_cursor: string;
}

interface MutableDocumentStamp extends MutableRecord {
  continuity_generation: string;
  coordinate: MutableCoordinate;
  lifecycle: string;
  projection_cursor: string;
  roots_version: string;
}

interface MutableNodeKey extends MutableRecord {
  continuity_generation: string;
  epoch: string;
  node_id: string;
}

interface MutableResourceKey extends MutableRecord {
  continuity_generation: string;
  epoch: string;
  resource_id: string;
}

interface MutableOwner extends MutableRecord {
  kind: string;
  key?: MutableNodeKey;
}

interface MutableNodeStamp extends MutableRecord {
  version: string;
  stability: string;
  parent?: MutableOwner | null;
  children_version: string;
}

interface MutableTextTransition extends MutableRecord {
  kind: string;
  range?: { start: string; end: string };
  text?: string;
}

interface MutableNodeTransition extends MutableRecord {
  key: MutableNodeKey;
  before?: MutableNodeStamp | null;
  after?: MutableNodeStamp | null;
  text?: MutableTextTransition | null;
}

interface MutableStructureTransition extends MutableRecord {
  owner: MutableOwner;
  before_version: string;
  after_version: string;
  start: number;
  removed: MutableNodeKey[];
  inserted: MutableNodeKey[];
}

interface MutableResourceTransition extends MutableRecord {
  key: MutableResourceKey;
  before_version: string | null;
  after_version: string | null;
  affected_nodes: MutableNodeKey[];
}

interface MutableContinuousFacts extends MutableRecord {
  scope: string;
  before: MutableDocumentStamp | null;
  after: MutableDocumentStamp;
  nodes: MutableNodeTransition[];
  structures: MutableStructureTransition[];
  resources: MutableResourceTransition[];
}

interface MutableTransitionFixture extends MutableRecord {
  schema: string;
  facts: MutableContinuousFacts;
}

function decodeUpdate(transition?: unknown): ReducerUpdateView {
  const record: Record<string, unknown> = {
    schema,
    kind: "reducer_update",
    outcome: { kind: "idempotent" },
    status: { kind: "uninitialized" },
    impact: {
      changed_node_ids: [],
      removed_node_ids: [],
      changed_resource_ids: [],
      removed_resource_ids: [],
      source_changed: false,
      projection_changed: false,
      lifecycle_changed: false,
      roots_changed: false,
      full_replace: false,
    },
    document: null,
  };
  if (transition !== undefined) {
    record.transition = transition;
  }
  return decodeBindingView(
    BindingPayloadKind.ReducerUpdate,
    encoder.encode(JSON.stringify(record)),
    schema,
  ) as ReducerUpdateView;
}

function documentStamp(continuityGeneration: string): MutableDocumentStamp {
  return {
    continuity_generation: continuityGeneration,
    coordinate: {
      epoch: "1",
      sequence: "1",
      change_id: "transition:test",
      source_cursor: "2",
    },
    lifecycle: "open",
    projection_cursor: "2",
    roots_version: "sha256:roots-after",
  };
}

function nodeKey(): MutableNodeKey {
  return {
    continuity_generation: "0",
    epoch: "1",
    node_id: "7",
  };
}

function continuousTransition(): MutableTransitionFixture {
  return {
    schema: "mdstream.transitions/1",
    facts: {
      scope: "continuous",
      before: null,
      after: documentStamp("0"),
      nodes: [{
        key: nodeKey(),
        before: null,
        after: {
          version: "sha256:node-after",
          stability: "provisional",
          parent: { kind: "document" },
          children_version: "sha256:children-after",
        },
        text: {
          kind: "projection_append",
          range: { start: "1", end: "2" },
          text: "B",
        },
      }],
      structures: [{
        owner: { kind: "node", key: nodeKey() },
        before_version: "sha256:children-before",
        after_version: "sha256:children-after",
        start: 0,
        removed: [],
        inserted: [nodeKey()],
      }],
      resources: [{
        key: {
          continuity_generation: "0",
          epoch: "1",
          resource_id: "9",
        },
        before_version: null,
        after_version: "sha256:resource-after",
        affected_nodes: [nodeKey()],
      }],
    },
  };
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
