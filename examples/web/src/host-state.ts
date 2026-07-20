import type { PresentationMode } from "./host-policy.js";

export type HostLifecycle =
  | "booting"
  | "ready-empty"
  | "streaming"
  | "draining"
  | "settled"
  | "interrupted"
  | "initialization-error"
  | "scenario-error";

export interface HostStateSnapshot {
  readonly lifecycle: HostLifecycle;
  readonly mode: PresentationMode;
  readonly message: string;
  readonly error: string | null;
}

export class HostState {
  readonly #listeners = new Set<(snapshot: HostStateSnapshot) => void>();
  #snapshot: HostStateSnapshot;

  constructor(mode: PresentationMode) {
    this.#snapshot = freeze({
      lifecycle: "booting",
      mode,
      message: "Loading the mdstream WebAssembly runtime.",
      error: null,
    });
  }

  get snapshot(): HostStateSnapshot {
    return this.#snapshot;
  }

  subscribe(listener: (snapshot: HostStateSnapshot) => void): () => void {
    this.#listeners.add(listener);
    listener(this.#snapshot);
    return () => this.#listeners.delete(listener);
  }

  setMode(mode: PresentationMode): void {
    this.#publish({ ...this.#snapshot, mode });
  }

  transition(lifecycle: HostLifecycle, message: string): void {
    this.#publish({
      ...this.#snapshot,
      lifecycle,
      message,
      error: null,
    });
  }

  announce(message: string): void {
    this.#publish({ ...this.#snapshot, message });
  }

  fail(kind: "initialization-error" | "scenario-error", error: unknown): void {
    const message = error instanceof Error ? error.message : String(error);
    this.#publish({
      ...this.#snapshot,
      lifecycle: kind,
      message: kind === "initialization-error"
        ? "The runtime could not be initialized."
        : "The Golden AI Stream is invalid.",
      error: message,
    });
  }

  #publish(snapshot: HostStateSnapshot): void {
    this.#snapshot = freeze(snapshot);
    for (const listener of [...this.#listeners]) {
      listener(this.#snapshot);
    }
  }
}

function freeze(snapshot: HostStateSnapshot): HostStateSnapshot {
  return Object.freeze(snapshot);
}
