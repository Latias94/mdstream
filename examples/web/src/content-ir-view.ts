import type {
  ArtifactSlot,
  ContentKindView,
  MdstreamStoreView,
  NodeId,
  NodeView,
  ResourceId,
  ResourceView,
} from "@mdstream/core";

import { HostPresentationPolicy } from "./host-policy.js";
import { classifyExternalUrl } from "./url-policy.js";

export const MERMAID_PREVIEW_PROCESSOR_ID = "example.web.mermaid-preview";

export interface ContentIrDiagnostics {
  readonly nodeKeys: readonly string[];
  readonly resourceKeys: readonly string[];
  readonly materializedNodeViews: string;
  readonly materializedResourceViews: string;
  readonly materializedPendingSourceViews: string;
  readonly artifactState: string;
}

export interface ContentIrViewOptions {
  readonly store: MdstreamStoreView;
  readonly policy: HostPresentationPolicy;
  readonly answerRoot: HTMLElement;
  readonly pendingRoot: HTMLElement;
  readonly onDiagnostics: (diagnostics: ContentIrDiagnostics) => void;
}

interface NodeBinding {
  readonly id: NodeId;
  readonly element: HTMLElement;
  readonly unsubscribe: () => void;
  resourceUnsubscribes: (() => void)[];
  artifactUnsubscribe: (() => void) | undefined;
  updating: boolean;
}

export class ContentIrView {
  readonly #store: MdstreamStoreView;
  readonly #policy: HostPresentationPolicy;
  readonly #answerRoot: HTMLElement;
  readonly #pendingRoot: HTMLElement;
  readonly #onDiagnostics: (diagnostics: ContentIrDiagnostics) => void;
  readonly #nodes = new Map<NodeId, NodeBinding>();
  readonly #resources = new Map<ResourceId, ResourceView>();
  readonly #unsubscribeRoot: () => void;
  readonly #unsubscribePending: () => void;
  readonly #unsubscribePolicy: () => void;
  #artifactState = "not requested";
  #closed = false;

  constructor(options: ContentIrViewOptions) {
    this.#store = options.store;
    this.#policy = options.policy;
    this.#answerRoot = options.answerRoot;
    this.#pendingRoot = options.pendingRoot;
    this.#onDiagnostics = options.onDiagnostics;
    this.#unsubscribeRoot = this.#store.subscribe(() => this.#handleRootChange());
    this.#unsubscribePending = this.#store.subscribePendingSource(() => {
      this.#renderPending();
      this.#reportDiagnostics();
    });
    this.#unsubscribePolicy = this.#policy.subscribe((changedNodes) => {
      if (changedNodes === null) {
        this.#reconcileRoots(true);
      } else {
        for (const nodeId of changedNodes) {
          const binding = this.#nodes.get(nodeId);
          if (binding !== undefined) {
            this.#updateNode(binding);
          }
        }
      }
      this.#reportDiagnostics();
    });
    this.#reconcileRoots(true);
    this.#renderPending();
  }

  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#unsubscribeRoot();
    this.#unsubscribePending();
    this.#unsubscribePolicy();
    for (const binding of [...this.#nodes.values()]) {
      this.#disposeNode(binding);
    }
    this.#nodes.clear();
    this.#resources.clear();
  }

  visibleText(): string {
    return this.#answerRoot.textContent ?? "";
  }

  #handleRootChange(): void {
    const impact = this.#store.getSnapshot().impact;
    if (impact.fullReplace || impact.rootsChanged || this.#nodes.size === 0) {
      this.#reconcileRoots(impact.fullReplace);
    }
    this.#reportDiagnostics();
  }

  #reconcileRoots(fullReplace: boolean): void {
    if (this.#closed) {
      return;
    }
    if (fullReplace) {
      for (const binding of [...this.#nodes.values()]) {
        this.#disposeNode(binding);
      }
      this.#nodes.clear();
      this.#resources.clear();
      this.#answerRoot.replaceChildren();
    }

    const document = this.#store.getSnapshot().document;
    const roots = document?.roots?.children ?? [];
    const fragment = documentFragment();
    for (const rootId of roots) {
      fragment.append(this.#ensureNode(rootId).element);
    }
    this.#answerRoot.replaceChildren(fragment);
    this.#pruneDetachedNodes(new Set(roots));
  }

  #ensureNode(id: NodeId): NodeBinding {
    const existing = this.#nodes.get(id);
    if (existing !== undefined) {
      return existing;
    }
    const element = document.createElement("div");
    element.className = "content-node";
    const binding: NodeBinding = {
      id,
      element,
      unsubscribe: this.#store.subscribeNode(id, () => this.#updateNode(binding)),
      resourceUnsubscribes: [],
      artifactUnsubscribe: undefined,
      updating: false,
    };
    this.#nodes.set(id, binding);
    this.#updateNode(binding);
    return binding;
  }

  #updateNode(binding: NodeBinding): void {
    if (this.#closed || binding.updating) {
      return;
    }
    binding.updating = true;
    try {
      const view = this.#store.getNodeSnapshot(binding.id);
      if (view === undefined) {
        this.#disposeNode(binding);
        this.#nodes.delete(binding.id);
        binding.element.remove();
        return;
      }
      for (const unsubscribe of binding.resourceUnsubscribes) {
        unsubscribe();
      }
      binding.resourceUnsubscribes = [];
      binding.artifactUnsubscribe?.();
      binding.artifactUnsubscribe = undefined;

      const epoch = this.#store.getSnapshot().document?.coordinate.epoch ?? "0";
      binding.element.dataset.nodeId = binding.id as string;
      binding.element.dataset.hostKey = this.#policy.nodeKey(binding.id, epoch);
      binding.element.dataset.nodeKind = view.node.content.kind;
      binding.element.dataset.stability = view.node.stability;
      binding.element.dataset.presentation = this.#policy.stateForNode(binding.id);
      binding.element.replaceChildren(this.#renderContent(binding, view));
    } finally {
      binding.updating = false;
    }
  }

  #renderContent(binding: NodeBinding, view: NodeView): Node {
    const content = view.node.content;
    switch (content.kind) {
      case "heading": {
        const heading = document.createElement(`h${content.level}`);
        this.#appendChildren(heading, view);
        return heading;
      }
      case "paragraph":
        return this.#container("p", view);
      case "emphasis":
        return this.#container("em", view);
      case "strong":
        return this.#container("strong", view);
      case "strikethrough":
        return this.#container("s", view);
      case "text":
        return textElement("span", this.#policy.displayText(binding.id, view.bodyText));
      case "soft_break":
      case "hard_break":
        return document.createElement("br");
      case "thematic_break":
        return document.createElement("hr");
      case "inline_code":
        return textElement("code", this.#policy.displayText(binding.id, view.bodyText));
      case "code_block":
        return this.#renderCodeBlock(binding, view, content);
      case "link":
        return this.#renderLinkedChildren(binding, view, content.target?.id);
      case "image":
        return this.#renderInertImage(binding, view, content.target?.id);
      case "citation_reference":
        return this.#renderCitationReference(binding, content.key, content.target?.id);
      case "citation_definition":
        return this.#renderCitationDefinition(binding, content.key, content.target.id);
      case "footnote_reference":
        return textElement("sup", `[${content.label}]`);
      case "footnote_definition":
        return this.#container("aside", view, `Footnote ${content.label}: `);
      case "list": {
        const list = content.ordered
          ? document.createElement("ol")
          : document.createElement("ul");
        if (list instanceof HTMLOListElement && content.start !== null) {
          list.start = content.start;
        }
        this.#appendChildren(list, view);
        return list;
      }
      case "list_item": {
        const item = this.#container("li", view);
        if (content.checked !== null) {
          const marker = document.createElement("span");
          marker.className = "task-marker";
          marker.textContent = content.checked ? "[x] " : "[ ] ";
          marker.setAttribute("aria-label", content.checked ? "Completed" : "Not completed");
          item.prepend(marker);
        }
        return item;
      }
      case "block_quote": {
        const quote = this.#container("blockquote", view);
        quote.dataset.quoteKind = content.style;
        return quote;
      }
      case "table":
        return this.#container("table", view);
      case "table_head":
        return this.#container("thead", view);
      case "table_body":
        return this.#container("tbody", view);
      case "table_row":
        return this.#container("tr", view);
      case "table_cell":
        return this.#container("td", view);
      case "html": {
        const inert = textElement("pre", this.#policy.displayText(binding.id, view.bodyText));
        inert.className = "inert-html";
        inert.setAttribute("aria-label", "HTML source shown as inert text");
        return inert;
      }
      case "math": {
        const math = textElement("code", this.#policy.displayText(binding.id, view.bodyText));
        math.className = content.display ? "math math-display" : "math";
        return math;
      }
      case "custom": {
        const custom = this.#container("section", view);
        custom.dataset.customKind = `${content.namespace}:${content.name}`;
        return custom;
      }
    }
  }

  #renderCodeBlock(
    binding: NodeBinding,
    view: NodeView,
    content: Extract<ContentKindView, { readonly kind: "code_block" }>,
  ): HTMLElement {
    const figure = document.createElement("figure");
    figure.className = "code-figure";
    const caption = document.createElement("figcaption");
    caption.textContent = content.info ?? "code";
    const pre = document.createElement("pre");
    const code = textElement("code", this.#policy.displayText(binding.id, view.bodyText));
    if (content.info !== null) {
      code.dataset.language = content.info;
    }
    pre.append(code);
    figure.append(caption, pre);
    if (content.info === "mermaid") {
      figure.append(this.#renderArtifact(binding, view));
    }
    return figure;
  }

  #renderArtifact(binding: NodeBinding, view: NodeView): HTMLElement {
    const output = document.createElement("output");
    output.className = "artifact-preview";
    output.setAttribute("aria-label", "Derived Mermaid processor artifact");
    const epoch = this.#store.getSnapshot().document?.coordinate.epoch;
    if (epoch === undefined) {
      output.textContent = "Artifact unavailable";
      return output;
    }
    const slot: ArtifactSlot = {
      epoch,
      nodeId: view.node.id,
      processorId: MERMAID_PREVIEW_PROCESSOR_ID,
    };
    const artifact = this.#store.getArtifactSnapshot(slot);
    this.#artifactState = artifact?.state ?? "not ready";
    if (artifact?.state === "ready" && artifact.artifact?.payload.kind === "text") {
      output.textContent = artifact.artifact.payload.text;
    } else if (artifact?.state === "failed") {
      output.textContent = `Artifact failed: ${artifact.failure?.message ?? "unknown error"}`;
    } else {
      output.textContent = "Derived preview pending";
    }
    binding.artifactUnsubscribe = this.#store.subscribeArtifact(slot, () => {
      this.#updateNode(binding);
      this.#reportDiagnostics();
    });
    return output;
  }

  #renderLinkedChildren(
    binding: NodeBinding,
    view: NodeView,
    resourceId: ResourceId | undefined,
  ): HTMLElement {
    const destination = this.#resourceDestination(binding, resourceId);
    const classified = destination === undefined
      ? { kind: "inert" as const, text: "" }
      : classifyExternalUrl(destination);
    if (classified.kind === "link") {
      const anchor = safeAnchor(classified.href);
      this.#appendChildren(anchor, view);
      return anchor;
    }
    const inert = this.#container("span", view);
    inert.className = "inert-link";
    if (classified.text.length > 0) {
      inert.title = `Blocked destination: ${classified.text}`;
    }
    return inert;
  }

  #renderInertImage(
    binding: NodeBinding,
    view: NodeView,
    resourceId: ResourceId | undefined,
  ): HTMLElement {
    const destination = this.#resourceDestination(binding, resourceId);
    const figure = document.createElement("span");
    figure.className = "inert-image";
    figure.textContent = `[Image: ${view.bodyText || "untitled"}]`;
    if (destination !== undefined) {
      const classified = classifyExternalUrl(destination);
      figure.title = classified.kind === "link"
        ? `External image not fetched: ${classified.href}`
        : `Blocked image destination: ${classified.text}`;
    }
    return figure;
  }

  #renderCitationReference(
    binding: NodeBinding,
    key: string,
    resourceId: ResourceId | undefined,
  ): HTMLElement {
    const destination = this.#resourceDestination(binding, resourceId);
    if (destination !== undefined) {
      const classified = classifyExternalUrl(destination);
      if (classified.kind === "link") {
        const anchor = renderExternalDestination(destination, `[@${key}]`);
        if (!(anchor instanceof HTMLAnchorElement)) {
          throw new TypeError("allowed citation destination did not produce a link");
        }
        anchor.className = "citation-link";
        anchor.setAttribute("aria-label", `Citation ${key}`);
        return anchor;
      }
    }
    const inert = textElement("span", `[@${key}]`);
    inert.className = "citation-unresolved";
    inert.setAttribute("aria-label", `Unresolved citation ${key}`);
    return inert;
  }

  #renderCitationDefinition(
    binding: NodeBinding,
    key: string,
    resourceId: ResourceId,
  ): HTMLElement {
    const definition = document.createElement("aside");
    definition.className = "citation-definition";
    definition.setAttribute("aria-label", `Citation definition ${key}`);
    const destination = this.#resourceDestination(binding, resourceId);
    definition.append(textElement("span", `Source ${key}: `));
    if (destination === undefined) {
      definition.append(textElement("span", "unresolved"));
      return definition;
    }
    const classified = classifyExternalUrl(destination);
    if (classified.kind === "link") {
      definition.append(renderExternalDestination(destination, classified.href));
    } else {
      definition.append(textElement("span", classified.text));
    }
    return definition;
  }

  #resourceDestination(
    binding: NodeBinding,
    resourceId: ResourceId | undefined,
  ): string | undefined {
    if (resourceId === undefined) {
      return undefined;
    }
    const resource = this.#store.getResourceSnapshot(resourceId);
    if (resource !== undefined) {
      this.#resources.set(resourceId, resource);
    }
    binding.resourceUnsubscribes.push(
      this.#store.subscribeResource(resourceId, () => this.#updateNode(binding)),
    );
    const content = resource?.resource.content;
    return content?.kind === "link" || content?.kind === "citation"
      ? content.destination
      : undefined;
  }

  #container<Tag extends keyof HTMLElementTagNameMap>(
    tag: Tag,
    view: NodeView,
    prefix?: string,
  ): HTMLElementTagNameMap[Tag] {
    const element = document.createElement(tag);
    if (prefix !== undefined) {
      element.append(document.createTextNode(prefix));
    }
    this.#appendChildren(element, view);
    return element;
  }

  #appendChildren(parent: HTMLElement, view: NodeView): void {
    for (const childId of view.node.children.children) {
      parent.append(this.#ensureNode(childId).element);
    }
  }

  #renderPending(): void {
    if (this.#closed) {
      return;
    }
    const pending = this.#policy.observePending(this.#store);
    this.#pendingRoot.replaceChildren();
    if (pending === undefined) {
      this.#pendingRoot.hidden = true;
      return;
    }
    this.#pendingRoot.hidden = false;
    this.#pendingRoot.dataset.range = `${pending.range.start}:${pending.range.end}`;
    this.#pendingRoot.textContent = pending.text;
  }

  #pruneDetachedNodes(rootIds: ReadonlySet<NodeId>): void {
    const reachable = new Set<NodeId>();
    const queue = [...rootIds];
    for (let index = 0; index < queue.length; index += 1) {
      const id = queue[index]!;
      if (reachable.has(id)) {
        continue;
      }
      reachable.add(id);
      const view = this.#store.getNodeSnapshot(id);
      if (view !== undefined) {
        queue.push(...view.node.children.children);
      }
    }
    for (const [id, binding] of this.#nodes) {
      if (!reachable.has(id)) {
        this.#disposeNode(binding);
        this.#nodes.delete(id);
      }
    }
  }

  #disposeNode(binding: NodeBinding): void {
    binding.unsubscribe();
    for (const unsubscribe of binding.resourceUnsubscribes) {
      unsubscribe();
    }
    binding.artifactUnsubscribe?.();
  }

  #reportDiagnostics(): void {
    const metrics = this.#store.metrics();
    this.#onDiagnostics(Object.freeze({
      nodeKeys: Object.freeze([...this.#nodes.values()].map(({ element }) =>
        element.dataset.hostKey ?? "missing"
      ).sort()),
      resourceKeys: Object.freeze([...this.#resources.keys()].map((id) => id as string).sort()),
      materializedNodeViews: metrics.materializedNodeViews,
      materializedResourceViews: metrics.materializedResourceViews,
      materializedPendingSourceViews: metrics.materializedPendingSourceViews,
      artifactState: this.#artifactState,
    }));
  }
}

function documentFragment(): DocumentFragment {
  return document.createDocumentFragment();
}

function textElement<Tag extends keyof HTMLElementTagNameMap>(
  tag: Tag,
  text: string,
): HTMLElementTagNameMap[Tag] {
  const element = document.createElement(tag);
  element.textContent = text;
  return element;
}

function safeAnchor(href: string): HTMLAnchorElement {
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.target = "_blank";
  anchor.rel = "noopener noreferrer";
  return anchor;
}

export function renderExternalDestination(
  destination: string,
  label: string,
): HTMLAnchorElement | HTMLSpanElement {
  const classified = classifyExternalUrl(destination);
  if (classified.kind === "link") {
    const anchor = safeAnchor(classified.href);
    anchor.textContent = label;
    return anchor;
  }
  const inert = textElement("span", label);
  inert.className = "inert-link";
  inert.title = `Blocked destination: ${classified.text}`;
  return inert;
}
