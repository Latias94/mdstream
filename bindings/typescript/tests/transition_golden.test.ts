import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { decodeBindingView } from "../src/views.js";
import {
  BindingPayloadKind,
  TRANSITION_SCHEMA,
} from "../src/wasm.js";

type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

interface TransitionGoldenCase {
  readonly id: string;
  readonly description: string;
  readonly covers: readonly string[];
  readonly wireJson: string;
  readonly normalized: Readonly<Record<string, JsonValue>>;
}

interface InvalidTransitionSchema {
  readonly id: string;
  readonly description: string;
  readonly baseCase: string;
  readonly schema: string;
}

interface TransitionGoldenFixture {
  readonly schema: string;
  readonly bindingSchema: string;
  readonly transitionSchema: string;
  readonly description: string;
  readonly cases: readonly TransitionGoldenCase[];
  readonly invalidTransitionSchemas: readonly InvalidTransitionSchema[];
}

const fixtureText = readFileSync(
  resolve(process.cwd(), "../../conformance/goldens/transition-v1.json"),
  "utf8",
);
const fixture = decodeFixture(JSON.parse(fixtureText) as unknown);
const encoder = new TextEncoder();

describe("shared transition /1 golden", () => {
  it("normalizes every exact Rust reducer-update wire through the typed decoder", () => {
    expect(fixture).toMatchObject({
      schema: "mdstream.transition-golden/1",
      bindingSchema: "mdstream.bindings/0.4",
      transitionSchema: TRANSITION_SCHEMA,
    });

    for (const goldenCase of fixture.cases) {
      const decoded = decodeBindingView(
        BindingPayloadKind.ReducerUpdate,
        encoder.encode(goldenCase.wireJson),
        fixture.bindingSchema,
      );
      if (decoded.kind !== "reducer_update") {
        throw new Error(`golden ${goldenCase.id} decoded as ${decoded.kind}`);
      }

      expect(decoded, goldenCase.id).toEqual(goldenCase.normalized);
      expect(Object.isFrozen(decoded), goldenCase.id).toBe(true);
      expect(Object.isFrozen(decoded.transition?.facts), goldenCase.id).toBe(
        true,
      );
    }
  });

  it("rejects old-draft and future transition schemas derived from valid wire", () => {
    for (const invalid of fixture.invalidTransitionSchemas) {
      const base = fixture.cases.find(
        (goldenCase) => goldenCase.id === invalid.baseCase,
      );
      if (base === undefined) {
        throw new Error(`unknown golden base case ${invalid.baseCase}`);
      }
      const wire = requiredRecord(JSON.parse(base.wireJson), "wire_json");
      const transition = requiredRecord(wire.transition, "wire_json.transition");
      transition.schema = invalid.schema;

      expect(
        () =>
          decodeBindingView(
            BindingPayloadKind.ReducerUpdate,
            encoder.encode(JSON.stringify(wire)),
            fixture.bindingSchema,
          ),
        invalid.id,
      ).toThrowError(
        expect.objectContaining({ detailCode: "bindings.invalid_payload" }),
      );
    }
  });

  it("denies unknown fields in the shared fixture schema", () => {
    const topLevel = requiredRecord(JSON.parse(fixtureText), "fixture");
    topLevel.unexpected = true;
    expect(() => decodeFixture(topLevel)).toThrow(/unknown field unexpected/);

    const caseField = requiredRecord(JSON.parse(fixtureText), "fixture");
    const cases = requiredArray(caseField.cases, "fixture.cases");
    requiredRecord(cases[0], "fixture.cases[0]").unexpected = true;
    expect(() => decodeFixture(caseField)).toThrow(/unknown field unexpected/);

    const invalidField = requiredRecord(JSON.parse(fixtureText), "fixture");
    const invalidSchemas = requiredArray(
      invalidField.invalid_transition_schemas,
      "fixture.invalid_transition_schemas",
    );
    requiredRecord(
      invalidSchemas[0],
      "fixture.invalid_transition_schemas[0]",
    ).unexpected = true;
    expect(() => decodeFixture(invalidField)).toThrow(/unknown field unexpected/);
  });
});

function decodeFixture(value: unknown): TransitionGoldenFixture {
  const fixture = requiredRecord(value, "fixture");
  exactKeys(
    fixture,
    [
      "schema",
      "binding_schema",
      "transition_schema",
      "description",
      "cases",
      "invalid_transition_schemas",
    ],
    "fixture",
  );
  return {
    schema: requiredString(fixture.schema, "fixture.schema"),
    bindingSchema: requiredString(
      fixture.binding_schema,
      "fixture.binding_schema",
    ),
    transitionSchema: requiredString(
      fixture.transition_schema,
      "fixture.transition_schema",
    ),
    description: requiredString(fixture.description, "fixture.description"),
    cases: requiredArray(fixture.cases, "fixture.cases").map((entry, index) =>
      decodeCase(entry, index),
    ),
    invalidTransitionSchemas: requiredArray(
      fixture.invalid_transition_schemas,
      "fixture.invalid_transition_schemas",
    ).map((entry, index) => decodeInvalidSchema(entry, index)),
  };
}

function decodeCase(value: unknown, index: number): TransitionGoldenCase {
  const field = `fixture.cases[${index}]`;
  const goldenCase = requiredRecord(value, field);
  exactKeys(
    goldenCase,
    ["id", "description", "covers", "wire_json", "normalized"],
    field,
  );
  return {
    id: requiredString(goldenCase.id, `${field}.id`),
    description: requiredString(goldenCase.description, `${field}.description`),
    covers: requiredStringArray(goldenCase.covers, `${field}.covers`),
    wireJson: requiredString(goldenCase.wire_json, `${field}.wire_json`),
    normalized: requiredJsonRecord(
      goldenCase.normalized,
      `${field}.normalized`,
    ),
  };
}

function decodeInvalidSchema(
  value: unknown,
  index: number,
): InvalidTransitionSchema {
  const field = `fixture.invalid_transition_schemas[${index}]`;
  const invalid = requiredRecord(value, field);
  exactKeys(
    invalid,
    ["id", "description", "base_case", "schema"],
    field,
  );
  return {
    id: requiredString(invalid.id, `${field}.id`),
    description: requiredString(invalid.description, `${field}.description`),
    baseCase: requiredString(invalid.base_case, `${field}.base_case`),
    schema: requiredString(invalid.schema, `${field}.schema`),
  };
}

function exactKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
  field: string,
): void {
  const expectedKeys = new Set(expected);
  for (const key of Object.keys(value)) {
    if (!expectedKeys.has(key)) {
      throw new Error(`${field} contains unknown field ${key}`);
    }
  }
  for (const key of expected) {
    if (!Object.hasOwn(value, key)) {
      throw new Error(`${field}.${key} is required`);
    }
  }
}

function requiredRecord(value: unknown, field: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requiredJsonRecord(
  value: unknown,
  field: string,
): Readonly<Record<string, JsonValue>> {
  return requiredRecord(value, field) as Readonly<Record<string, JsonValue>>;
}

function requiredArray(value: unknown, field: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new Error(`${field} must be an array`);
  }
  return value;
}

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new Error(`${field} must be a string`);
  }
  return value;
}

function requiredStringArray(value: unknown, field: string): readonly string[] {
  return requiredArray(value, field).map((entry, index) =>
    requiredString(entry, `${field}[${index}]`),
  );
}
