import authority from "../../fixtures/golden-ai-stream.json";

export interface AppendAction {
  readonly kind: "append";
  readonly id: string;
  readonly chunk: string;
}

export interface CheckpointAction {
  readonly kind: "checkpoint";
  readonly id: string;
  readonly scope: "schedule_local" | "boundary_invariant";
  readonly sourceCursor: number;
  readonly observations: readonly string[];
}

export interface FinishAction {
  readonly kind: "finish";
  readonly id: string;
  readonly observations: readonly string[];
}

export type MainlineAction = AppendAction | CheckpointAction | FinishAction;

export interface GoldenScenario {
  readonly schema: "mdstream.example-scenario/1";
  readonly id: string;
  readonly description: string;
  readonly actions: readonly MainlineAction[];
  readonly expected: {
    readonly finalSource: string;
    readonly lifecycle: "finalized";
  };
}

export class ScenarioError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ScenarioError";
  }
}

export function loadGoldenScenario(input: unknown = authority): GoldenScenario {
  const root = record(input, "scenario");
  requiredExact(root.schema, "mdstream.example-scenario/1", "scenario.schema");
  const id = nonEmptyString(root.id, "scenario.id");
  const description = nonEmptyString(root.description, "scenario.description");
  const episodes = record(root.episodes, "scenario.episodes");
  const mainline = record(episodes.mainline, "scenario.episodes.mainline");
  if (!Array.isArray(mainline.actions) || mainline.actions.length === 0) {
    throw new ScenarioError("scenario mainline must contain actions");
  }

  const actions: MainlineAction[] = [];
  const ids = new Set<string>();
  let cursor = 0;
  let finished = false;
  for (const [index, candidate] of mainline.actions.entries()) {
    const path = `scenario.episodes.mainline.actions[${index}]`;
    const action = record(candidate, path);
    const kind = nonEmptyString(action.kind, `${path}.kind`);
    const actionId = nonEmptyString(action.id, `${path}.id`);
    if (ids.has(actionId)) {
      throw new ScenarioError(`${path}.id duplicates ${actionId}`);
    }
    ids.add(actionId);
    if (finished) {
      throw new ScenarioError(`${path} follows the finish action`);
    }

    if (kind === "append") {
      const chunk = stringValue(action.chunk, `${path}.chunk`);
      cursor += new TextEncoder().encode(chunk).byteLength;
      actions.push(Object.freeze({ kind, id: actionId, chunk }));
      continue;
    }
    if (kind === "checkpoint") {
      const sourceCursor = safeInteger(action.source_cursor, `${path}.source_cursor`);
      if (sourceCursor !== cursor) {
        throw new ScenarioError(
          `${path}.source_cursor is ${sourceCursor}; current cursor is ${cursor}`,
        );
      }
      const scope = action.scope;
      if (scope !== "schedule_local" && scope !== "boundary_invariant") {
        throw new ScenarioError(`${path}.scope is unsupported`);
      }
      actions.push(Object.freeze({
        kind,
        id: actionId,
        scope,
        sourceCursor,
        observations: stringArray(action.observations, `${path}.observations`),
      }));
      continue;
    }
    if (kind === "finish") {
      finished = true;
      actions.push(Object.freeze({
        kind,
        id: actionId,
        observations: stringArray(action.observations, `${path}.observations`),
      }));
      continue;
    }
    throw new ScenarioError(`${path}.kind ${kind} is unsupported`);
  }
  if (!finished) {
    throw new ScenarioError("scenario mainline must end with finish");
  }

  const expected = record(root.expected, "scenario.expected");
  const finalSource = stringValue(expected.final_source, "scenario.expected.final_source");
  const replayedSource = actions
    .filter((action): action is AppendAction => action.kind === "append")
    .map(({ chunk }) => chunk)
    .join("");
  if (replayedSource !== finalSource) {
    throw new ScenarioError("scenario append actions do not equal expected.final_source");
  }
  requiredExact(expected.lifecycle, "finalized", "scenario.expected.lifecycle");

  return Object.freeze({
    schema: "mdstream.example-scenario/1",
    id,
    description,
    actions: Object.freeze(actions),
    expected: Object.freeze({ finalSource, lifecycle: "finalized" }),
  });
}

function record(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ScenarioError(`${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

function nonEmptyString(value: unknown, path: string): string {
  const result = stringValue(value, path);
  if (result.length === 0) {
    throw new ScenarioError(`${path} must not be empty`);
  }
  return result;
}

function stringValue(value: unknown, path: string): string {
  if (typeof value !== "string") {
    throw new ScenarioError(`${path} must be a string`);
  }
  return value;
}

function stringArray(value: unknown, path: string): readonly string[] {
  if (!Array.isArray(value) || value.length === 0) {
    throw new ScenarioError(`${path} must be a non-empty string array`);
  }
  return Object.freeze(value.map((entry, index) =>
    nonEmptyString(entry, `${path}[${index}]`)
  ));
}

function safeInteger(value: unknown, path: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new ScenarioError(`${path} must be a non-negative safe integer`);
  }
  return value;
}

function requiredExact(value: unknown, expected: string, path: string): void {
  if (value !== expected) {
    throw new ScenarioError(`${path} must be ${expected}`);
  }
}
