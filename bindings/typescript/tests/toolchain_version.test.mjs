import { describe, expect, it } from "vitest";

import { matchesToolVersion } from "../scripts/toolchain-version.mjs";

describe("artifact toolchain version matching", () => {
  it("accepts exact versions and diagnostic suffixes", () => {
    expect(
      matchesToolVersion("wasm-tools 1.253.0", "wasm-tools", "1.253.0"),
    ).toBe(true);
    expect(
      matchesToolVersion(
        "wasm-tools 1.253.0 (c799bb87b 2026-07-07)",
        "wasm-tools",
        "1.253.0",
      ),
    ).toBe(true);
  });

  it("rejects other tools and version-prefix collisions", () => {
    expect(
      matchesToolVersion("wasm-tools 1.253.1", "wasm-tools", "1.253.0"),
    ).toBe(false);
    expect(
      matchesToolVersion("wasm-tools 1.253.01", "wasm-tools", "1.253.0"),
    ).toBe(false);
    expect(
      matchesToolVersion("other 1.253.0", "wasm-tools", "1.253.0"),
    ).toBe(false);
  });
});
