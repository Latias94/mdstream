import {
  initMdstream,
  type MdstreamEngine,
  type MdstreamRuntime,
} from "@mdstream/core";

import {
  ContentIrView,
  MERMAID_PREVIEW_PROCESSOR_ID,
  type ContentIrDiagnostics,
} from "./content-ir-view.js";
import { HostPresentationPolicy, type PresentationMode } from "./host-policy.js";
import { HostState, type HostStateSnapshot } from "./host-state.js";
import {
  loadGoldenScenario,
  ScenarioError,
  type GoldenScenario,
} from "./scenario.js";
import "./styles.css";

const limits = Object.freeze({
  captureTransitions: true,
  protocol: {
    maxSourceBytes: "65536",
    maxNodes: "4096",
    maxResources: "512",
    maxOperations: "4096",
    maxChangeStructuralItems: "4096",
    maxDocumentStructuralItems: "16384",
    maxChildrenPerList: "4096",
  },
  wire: { maxReducerUpdateBytes: "33554432" },
});

interface HostSession {
  readonly engine: MdstreamEngine;
  readonly policy: HostPresentationPolicy;
  readonly view: ContentIrView;
  readonly unsubscribeTransitions: () => void;
  unsubscribePolicy: () => void;
  observedEvents: number;
}

class GoldenStreamApp {
  readonly #hostState = new HostState("immediate");
  readonly #motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  readonly #query = new URLSearchParams(window.location.search);
  #runtime: MdstreamRuntime | undefined;
  #scenario: GoldenScenario | undefined;
  #session: HostSession | undefined;
  #playback: AbortController | undefined;
  #faultConsumed = false;
  #frame: number | undefined;
  #diagnostics: ContentIrDiagnostics | undefined;

  constructor() {
    this.#hostState.subscribe((snapshot) => this.#renderState(snapshot));
    elements.replay.addEventListener("click", () => void this.replay());
    elements.interrupt.addEventListener("click", () => this.interrupt());
    elements.replace.addEventListener("click", () => void this.replaceContinuity());
    elements.retry.addEventListener("click", () => void this.boot(true));
    for (const control of elements.modeControls) {
      control.addEventListener("change", () => {
        if (control.checked) {
          void this.changeMode(control.value as PresentationMode);
        }
      });
    }
    this.#motion.addEventListener("change", ({ matches }) => {
      this.#session?.policy.setReducedMotion(matches);
      document.documentElement.dataset.reducedMotion = String(matches);
    });
    document.documentElement.dataset.reducedMotion = String(this.#motion.matches);
  }

  async boot(fromRetry = false): Promise<void> {
    this.#closeSession();
    this.#hostState.transition("booting", "Loading the mdstream WebAssembly runtime.");
    try {
      if (!this.#faultConsumed && this.#query.get("init") === "fail") {
        this.#faultConsumed = true;
        throw new Error("The example requested a simulated initialization failure.");
      }
      this.#runtime = await initMdstream();
    } catch (error) {
      this.#hostState.fail("initialization-error", error);
      queueMicrotask(() => elements.retry.focus());
      return;
    }

    try {
      const input = !this.#faultConsumed && this.#query.get("scenario") === "invalid"
        ? { schema: "invalid" }
        : undefined;
      this.#faultConsumed = true;
      this.#scenario = loadGoldenScenario(input);
    } catch (error) {
      this.#hostState.fail("scenario-error", error);
      queueMicrotask(() => elements.retry.focus());
      return;
    }

    this.#createSession();
    this.#hostState.transition("ready-empty", "Ready to replay the Golden AI Stream.");
    if (fromRetry) {
      elements.replay.focus();
    }
    if (this.#query.get("autoplay") !== "false") {
      await this.replay();
    }
  }

  async replay(): Promise<void> {
    if (this.#runtime === undefined || this.#scenario === undefined) {
      return;
    }
    this.#closeSession();
    const session = this.#createSession();
    await this.#runScenario(session, false);
  }

  interrupt(): void {
    const lifecycle = this.#hostState.snapshot.lifecycle;
    if (lifecycle !== "streaming" && lifecycle !== "draining") {
      return;
    }
    this.#playback?.abort();
    this.#playback = undefined;
    this.#stopFrames();
    this.#session?.policy.interrupt();
    this.#hostState.transition(
      "interrupted",
      "Replay interrupted. Canonical state remains inspectable; replay starts fresh.",
    );
  }

  async replaceContinuity(): Promise<void> {
    const session = this.#session;
    if (session === undefined || this.#hostState.snapshot.lifecycle !== "settled") {
      return;
    }
    session.engine.reset();
    await this.#runScenario(session, true);
  }

  async changeMode(mode: PresentationMode): Promise<void> {
    if (mode === this.#hostState.snapshot.mode) {
      return;
    }
    this.#hostState.setMode(mode);
    this.#closeSession();
    if (this.#runtime === undefined || this.#scenario === undefined) {
      return;
    }
    const session = this.#createSession();
    await this.#runScenario(session, false);
  }

  #createSession(): HostSession {
    const runtime = required(this.#runtime, "runtime");
    const engine = runtime.createEngine(limits);
    const policy = new HostPresentationPolicy(
      this.#hostState.snapshot.mode,
      this.#motion.matches,
    );
    engine.registerProcessor({
      descriptor: {
        id: MERMAID_PREVIEW_PROCESSOR_ID,
        version: "v1",
        acceptsProvisional: true,
      },
      configurationVersion: "example.web.default",
      allowProvisional: true,
      matches: (node) =>
        node.content.kind === "code_block" && node.content.info === "mermaid",
      process: (request) => ({
        kind: "text",
        protocol: "mdstream.example.mermaid-summary/1",
        mediaType: "text/plain",
        text: `${request.input.body.split("\n").filter(Boolean).length} Mermaid source lines ready for a host renderer`,
      }),
    });

    let session: HostSession;
    const unsubscribeTransitions = engine.store.subscribeTransitions((batch) => {
      policy.consume(engine.store, batch);
    });
    const view = new ContentIrView({
      store: engine.store,
      policy,
      answerRoot: elements.answer,
      pendingRoot: elements.pending,
      onDiagnostics: (diagnostics) => {
        this.#diagnostics = diagnostics;
        this.#renderDiagnostics();
      },
    });
    session = {
      engine,
      policy,
      view,
      unsubscribeTransitions,
      unsubscribePolicy: () => undefined,
      observedEvents: 0,
    };
    session.unsubscribePolicy = policy.subscribe(() => {
      this.#renderPolicyEvents(session);
      this.#ensureFrames(session);
    });
    this.#session = session;
    elements.answer.dataset.finalDigest = "";
    elements.eventLog.replaceChildren();
    this.#renderDiagnostics();
    return session;
  }

  async #runScenario(session: HostSession, afterReplacement: boolean): Promise<void> {
    const scenario = required(this.#scenario, "scenario");
    this.#playback?.abort();
    const playback = new AbortController();
    this.#playback = playback;
    this.#hostState.transition(
      "streaming",
      afterReplacement
        ? "Continuity replaced. Streaming the canonical answer again."
        : "Streaming the canonical answer.",
    );
    try {
      for (const action of scenario.actions) {
        if (playback.signal.aborted || this.#session !== session) {
          return;
        }
        if (action.kind === "append") {
          session.engine.append(action.chunk);
          await delay(105, playback.signal);
        } else if (action.kind === "checkpoint") {
          this.#appendCheckpoint(action.id, action.observations);
          await delay(45, playback.signal);
        } else {
          session.engine.finish();
        }
      }
      await session.engine.whenProcessorsIdle();
      if (session.policy.queuedGraphemes > 0) {
        this.#hostState.transition(
          "draining",
          "Canonical content finalized. Finishing host presentation.",
        );
        await this.#waitForDrain(session, playback.signal);
      }
      if (playback.signal.aborted || this.#session !== session) {
        return;
      }
      this.#stopFrames();
      const snapshot = session.engine.createRecoverySnapshot();
      if (snapshot === undefined) {
        throw new Error("The finalized engine did not produce a canonical snapshot.");
      }
      const digest = await sha256(snapshot);
      elements.answer.dataset.finalDigest = digest;
      elements.canonicalDigest.textContent = digest.slice(0, 16);
      elements.answer.dataset.pendingPresentedBytes = String(
        session.policy.pendingPresentedBytes,
      );
      elements.answer.dataset.pendingCatchUpBytes = String(
        session.policy.pendingCatchUpBytes,
      );
      this.#hostState.transition(
        "settled",
        "Stream settled with finalized canonical content.",
      );
      this.#renderDiagnostics();
    } catch (error) {
      if (isAbort(error)) {
        return;
      }
      this.#stopFrames();
      this.#hostState.fail(
        error instanceof ScenarioError ? "scenario-error" : "initialization-error",
        error,
      );
    } finally {
      if (this.#playback === playback) {
        this.#playback = undefined;
      }
    }
  }

  #ensureFrames(session: HostSession): void {
    if (
      this.#frame !== undefined ||
      this.#session !== session ||
      this.#playback === undefined ||
      this.#playback.signal.aborted ||
      session.policy.queuedGraphemes === 0
    ) {
      return;
    }
    const tick = (): void => {
      this.#frame = undefined;
      if (this.#session !== session || this.#playback?.signal.aborted === true) {
        return;
      }
      session.policy.advance(session.policy.reducedMotion ? Number.MAX_SAFE_INTEGER : 2);
      this.#ensureFrames(session);
    };
    this.#frame = window.requestAnimationFrame(tick);
  }

  async #waitForDrain(session: HostSession, signal: AbortSignal): Promise<void> {
    while (session.policy.queuedGraphemes > 0) {
      await delay(16, signal);
    }
  }

  #stopFrames(): void {
    if (this.#frame !== undefined) {
      window.cancelAnimationFrame(this.#frame);
      this.#frame = undefined;
    }
  }

  #appendCheckpoint(id: string, observations: readonly string[]): void {
    const item = document.createElement("li");
    item.dataset.eventKind = "checkpoint";
    item.textContent = `${id}: ${observations.join(", ")}`;
    elements.eventLog.append(item);
  }

  #renderPolicyEvents(session: HostSession): void {
    for (const event of session.policy.eventsSince(session.observedEvents)) {
      const item = document.createElement("li");
      item.dataset.eventKind = event.kind;
      item.textContent = event.message;
      elements.eventLog.append(item);
      if (
        event.kind === "correction" ||
        event.kind === "replacement" ||
        event.kind === "stabilization"
      ) {
        this.#hostState.announce(event.message);
      }
    }
    session.observedEvents = session.policy.eventCount;
  }

  #renderState(snapshot: HostStateSnapshot): void {
    document.documentElement.dataset.lifecycle = snapshot.lifecycle;
    document.documentElement.dataset.mode = snapshot.mode;
    elements.app.setAttribute(
      "aria-busy",
      String(snapshot.lifecycle === "booting" || snapshot.lifecycle === "streaming"),
    );
    elements.lifecycle.textContent = lifecycleLabel(snapshot.lifecycle);
    elements.status.textContent = snapshot.message;
    elements.modeIndicator.textContent = `${capitalize(snapshot.mode)} host policy`;
    const active = snapshot.lifecycle === "streaming" || snapshot.lifecycle === "draining";
    const failed = snapshot.lifecycle === "initialization-error" || snapshot.lifecycle === "scenario-error";
    elements.replay.disabled = active || snapshot.lifecycle === "booting" || failed;
    elements.interrupt.disabled = !active;
    elements.replace.disabled = snapshot.lifecycle !== "settled";
    for (const control of elements.modeControls) {
      control.disabled = snapshot.lifecycle === "booting" || failed;
      control.checked = control.value === snapshot.mode;
    }
    elements.errorPanel.hidden = !failed;
    elements.errorMessage.textContent = snapshot.error ?? "";
  }

  #renderDiagnostics(): void {
    const session = this.#session;
    if (session === undefined) {
      return;
    }
    const diagnostics = this.#diagnostics;
    const snapshot = session.engine.store.getSnapshot();
    elements.canonicalLifecycle.textContent = snapshot.document?.lifecycle ?? "uninitialized";
    elements.nodeViews.textContent = diagnostics?.materializedNodeViews ?? "0";
    elements.resourceViews.textContent = diagnostics?.materializedResourceViews ?? "0";
    elements.pendingViews.textContent = diagnostics?.materializedPendingSourceViews ?? "0";
    elements.artifactState.textContent = diagnostics?.artifactState ?? "not requested";
    elements.queuedGraphemes.textContent = String(session.policy.queuedGraphemes);
    const keys = diagnostics?.nodeKeys ?? [];
    elements.stableKeys.replaceChildren(...keys.map((key) => {
      const item = document.createElement("li");
      item.textContent = key;
      return item;
    }));
    elements.answer.dataset.stableKeys = keys.join(",");
    elements.answer.dataset.canonicalLifecycle = snapshot.document?.lifecycle ?? "uninitialized";
  }

  #closeSession(): void {
    this.#playback?.abort();
    this.#playback = undefined;
    this.#stopFrames();
    const session = this.#session;
    this.#session = undefined;
    if (session !== undefined) {
      session.unsubscribeTransitions();
      session.unsubscribePolicy();
      session.view.close();
      session.engine.close();
    }
    this.#diagnostics = undefined;
    elements.answer.replaceChildren();
    elements.pending.replaceChildren();
    elements.pending.hidden = true;
  }
}

const elements = {
  app: requiredElement<HTMLElement>("app"),
  replay: requiredElement<HTMLButtonElement>("replay"),
  interrupt: requiredElement<HTMLButtonElement>("interrupt"),
  replace: requiredElement<HTMLButtonElement>("replace"),
  retry: requiredElement<HTMLButtonElement>("retry"),
  lifecycle: requiredElement<HTMLElement>("lifecycle"),
  status: requiredElement<HTMLElement>("status"),
  modeIndicator: requiredElement<HTMLElement>("mode-indicator"),
  modeControls: [...document.querySelectorAll<HTMLInputElement>('input[name="mode"]')],
  errorPanel: requiredElement<HTMLElement>("error-panel"),
  errorMessage: requiredElement<HTMLElement>("error-message"),
  answer: requiredElement<HTMLElement>("answer"),
  pending: requiredElement<HTMLElement>("pending"),
  canonicalLifecycle: requiredElement<HTMLElement>("canonical-lifecycle"),
  canonicalDigest: requiredElement<HTMLElement>("canonical-digest"),
  nodeViews: requiredElement<HTMLElement>("node-views"),
  resourceViews: requiredElement<HTMLElement>("resource-views"),
  pendingViews: requiredElement<HTMLElement>("pending-views"),
  artifactState: requiredElement<HTMLElement>("artifact-state"),
  queuedGraphemes: requiredElement<HTMLElement>("queued-graphemes"),
  stableKeys: requiredElement<HTMLOListElement>("stable-keys"),
  eventLog: requiredElement<HTMLOListElement>("event-log"),
};

function requiredElement<ElementType extends HTMLElement>(id: string): ElementType {
  const element = document.getElementById(id);
  if (element === null) {
    throw new Error(`Example shell is missing #${id}`);
  }
  return element as ElementType;
}

function required<Value>(value: Value | undefined, name: string): Value {
  if (value === undefined) {
    throw new Error(`${name} is unavailable`);
  }
  return value;
}

function delay(milliseconds: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) {
    return Promise.reject(new DOMException("Playback aborted", "AbortError"));
  }
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      window.clearTimeout(timer);
      signal.removeEventListener("abort", onAbort);
      reject(new DOMException("Playback aborted", "AbortError"));
    };
    const timer = window.setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, milliseconds);
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

async function sha256(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", Uint8Array.from(bytes).buffer);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function lifecycleLabel(lifecycle: HostStateSnapshot["lifecycle"]): string {
  return lifecycle.split("-").map(capitalize).join(" ");
}

function capitalize(value: string): string {
  return `${value.charAt(0).toUpperCase()}${value.slice(1)}`;
}

void new GoldenStreamApp().boot();
