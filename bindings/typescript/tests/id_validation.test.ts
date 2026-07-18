import { describe, expect, it } from "vitest";

import { asEpoch, asNodeId, asResourceId } from "../src/index.js";

describe("decimal identifier input validation", () => {
  it("rejects malformed and overflowing u64 caller inputs consistently", () => {
    expect(asEpoch("18446744073709551615")).toBe("18446744073709551615");
    for (const value of [
      "",
      "-1",
      "1.0",
      "01",
      "18446744073709551616",
      "9".repeat(4_096),
    ]) {
      expect(() => asEpoch(value)).toThrowError(
        expect.objectContaining({
          status: 1,
          statusName: "MDSTREAM_INVALID_ARGUMENT",
          detailCode: "bindings.decimal_id",
        }),
      );
    }
  });

  it("accepts the full u128 content-ID domain and rejects overflow", () => {
    expect(asNodeId("340282366920938463463374607431768211455")).toBe(
      "340282366920938463463374607431768211455",
    );
    expect(asResourceId("340282366920938463463374607431768211455")).toBe(
      "340282366920938463463374607431768211455",
    );
    for (const value of [
      "01",
      "340282366920938463463374607431768211456",
      "9".repeat(4_096),
    ]) {
      expect(() => asNodeId(value)).toThrowError(
        expect.objectContaining({ status: 1 }),
      );
    }
  });
});
