import { expect, test, type Page } from "@playwright/test";

test("immediate and paced fresh sessions settle to equal meaning", async ({ page }) => {
  const errors = collectPageErrors(page);
  await page.goto("/?autoplay=false");
  await page.getByRole("button", { name: "Replay" }).click();
  const headingNode = page.locator('[data-node-kind="heading"]').first();
  await expect(headingNode).toBeVisible();
  await headingNode.evaluate((element) => {
    element.setAttribute("data-identity-probe", "preserved");
  });
  await settled(page);
  await expect(headingNode).toHaveAttribute("data-identity-probe", "preserved");
  const immediate = await settledMeaning(page);

  expect(Number(await page.locator("#answer").getAttribute("data-pending-presented-bytes")))
    .toBeGreaterThan(0);
  expect(await page.locator("#answer").getAttribute("data-pending-catch-up-bytes"))
    .toBe(await page.locator("#answer").getAttribute("data-pending-presented-bytes"));
  await expect(page.locator('[data-event-kind="correction"]')).not.toHaveCount(0);
  const safeLinks = page.locator('a[href="https://docs.rs/mdstream"]');
  await expect(safeLinks).toHaveCount(2);
  for (let index = 0; index < await safeLinks.count(); index += 1) {
    await expect(safeLinks.nth(index)).toHaveAttribute("rel", "noopener noreferrer");
    await expect(safeLinks.nth(index)).toHaveAttribute("target", "_blank");
  }
  await expect(page.locator('a[href^="javascript:"], a[href^="data:"]')).toHaveCount(0);
  const rejectedDestinations = await page.evaluate(async () => {
    const modulePath = "/src/content-ir-view.ts";
    const { renderExternalDestination } = await import(modulePath) as typeof import(
      "../src/content-ir-view.js"
    );
    return ["javascript:alert(1)", "data:text/html,unsafe", "not a url"].map(
      (destination) => {
        const element = renderExternalDestination(destination, destination);
        return {
          tag: element.tagName,
          href: element.getAttribute("href"),
          text: element.textContent,
        };
      },
    );
  });
  expect(rejectedDestinations).toEqual([
    { tag: "SPAN", href: null, text: "javascript:alert(1)" },
    { tag: "SPAN", href: null, text: "data:text/html,unsafe" },
    { tag: "SPAN", href: null, text: "not a url" },
  ]);

  await page.getByLabel("Paced").check();
  await expect(page.locator("html")).toHaveAttribute("data-mode", "paced");
  await expect(page.locator("html")).toHaveAttribute("data-lifecycle", /streaming|draining|settled/);
  await settled(page);
  expect(await settledMeaning(page)).toEqual(immediate);
  expect(errors).toEqual([]);
});

test("replacement, interruption, and retry discard stale host work", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "desktop-chromium", "state workflow runs once");
  const errors = collectPageErrors(page);
  await page.goto("/");
  await settled(page);
  const originalKeys = await page.locator("#answer").getAttribute("data-stable-keys");

  await page.getByRole("button", { name: "Replace continuity" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-lifecycle", /streaming|draining/);
  await settled(page);
  const replacementKeys = await page.locator("#answer").getAttribute("data-stable-keys");
  expect(replacementKeys).not.toBe(originalKeys);
  await expect(page.locator('[data-event-kind="replacement"]')).not.toHaveCount(0);

  await page.getByRole("button", { name: "Replay" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-lifecycle", "streaming");
  await page.getByRole("button", { name: "Interrupt" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-lifecycle", "interrupted");
  await expect(page.getByRole("status")).toContainText("interrupted");
  await expect(page.getByRole("button", { name: "Replay" })).toBeEnabled();
  await page.getByRole("button", { name: "Replay" }).click();
  await settled(page);
  expect(errors).toEqual([]);
});

test("a stale digest cannot settle a replacement session", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "desktop-chromium", "digest race runs once");
  await page.addInitScript(() => {
    const originalDigest = crypto.subtle.digest.bind(crypto.subtle);
    let releaseDigest = (): void => undefined;
    const digestGate = new Promise<void>((resolve) => {
      releaseDigest = resolve;
    });
    const state = {
      entered: false,
      release: (): void => releaseDigest(),
    };
    Object.defineProperty(globalThis, "__mdstreamDigestGate", { value: state });
    Object.defineProperty(crypto.subtle, "digest", {
      configurable: true,
      value: async (...args: Parameters<SubtleCrypto["digest"]>) => {
        const digest = await originalDigest(...args);
        state.entered = true;
        await digestGate;
        return digest;
      },
    });
  });
  await page.goto("/?autoplay=false");
  await page.getByRole("button", { name: "Replay" }).click();
  await expect.poll(() => page.evaluate(() =>
    (globalThis as typeof globalThis & {
      __mdstreamDigestGate: { readonly entered: boolean };
    }).__mdstreamDigestGate.entered
  )).toBe(true);

  await page.getByLabel("Paced").check();
  await expect(page.locator("html")).toHaveAttribute("data-lifecycle", "streaming");
  const staleCompletion = await page.evaluate(async () => {
    const gate = (globalThis as typeof globalThis & {
      __mdstreamDigestGate: { release(): void };
    }).__mdstreamDigestGate;
    gate.release();
    await new Promise((resolve) => window.setTimeout(resolve, 25));
    return {
      lifecycle: document.documentElement.dataset.lifecycle,
      digest: document.querySelector("#answer")?.getAttribute("data-final-digest"),
    };
  });

  expect(staleCompletion).toEqual({ lifecycle: "streaming", digest: "" });
  await settled(page);
});

test("initialization and scenario errors expose a focused retry path", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "desktop-chromium", "error workflow runs once");
  const errors = collectPageErrors(page);
  for (const fault of ["init=fail", "scenario=invalid"]) {
    await page.goto(`/?${fault}&autoplay=false`);
    await expect(page.locator("html")).toHaveAttribute(
      "data-lifecycle",
      /initialization-error|scenario-error/,
    );
    await expect(page.locator("#error-panel")).toBeVisible();
    await expect(page.getByRole("button", { name: "Retry" })).toBeFocused();
    await page.getByRole("button", { name: "Retry" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-lifecycle", "ready-empty");
    await expect(page.getByRole("button", { name: "Replay" })).toBeFocused();
    await page.getByRole("button", { name: "Replay" }).click();
    await settled(page);
  }
  expect(errors).toEqual([]);
});

test("keyboard and reduced motion retain the same accessible state meaning", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== "desktop-chromium", "keyboard workflow runs once");
  const errors = collectPageErrors(page);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/?autoplay=false");
  await expect(page.locator("html")).toHaveAttribute("data-reduced-motion", "true");
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Replay" })).toBeFocused();
  const outline = await page.getByRole("button", { name: "Replay" }).evaluate((element) =>
    getComputedStyle(element).outlineStyle
  );
  expect(outline).not.toBe("none");
  await page.keyboard.press("Enter");
  await settled(page);

  await page.getByLabel("Paced").check();
  await settled(page);
  await expect(page.locator("#queued-graphemes")).toHaveText("0");
  await expect(page.locator('[role="status"][aria-live="polite"][aria-atomic="true"]'))
    .toHaveCount(1);
  await expect(page.locator("#answer [aria-live]")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("answer-first layout stays readable without page overflow", async ({ page }, testInfo) => {
  const errors = collectPageErrors(page);
  await page.goto("/");
  await settled(page);
  await expect(page.locator("#answer")).not.toBeEmpty();
  const answer = await page.locator(".answer-region").boundingBox();
  const inspector = await page.locator("#inspector").boundingBox();
  expect(answer).not.toBeNull();
  expect(inspector).not.toBeNull();
  if (testInfo.project.name === "desktop-chromium") {
    expect(inspector!.x).toBeGreaterThan(answer!.x + answer!.width - 1);
  } else {
    expect(inspector!.y).toBeGreaterThan(answer!.y + answer!.height - 1);
  }
  expect(await page.evaluate(() =>
    document.documentElement.scrollWidth - document.documentElement.clientWidth
  )).toBe(0);
  expect(await controlsOverlap(page)).toBe(false);
  await page.screenshot({
    path: testInfo.outputPath("settled-answer.png"),
    fullPage: true,
    animations: "disabled",
  });
  expect(errors).toEqual([]);
});

async function settled(page: Page): Promise<void> {
  await expect(page.locator("html")).toHaveAttribute("data-lifecycle", "settled", {
    timeout: 15_000,
  });
  await expect(page.locator("#answer")).toHaveAttribute("data-canonical-lifecycle", "finalized");
  await expect(page.locator("#answer")).toHaveAttribute("data-final-digest", /^[a-f0-9]{64}$/);
}

async function settledMeaning(page: Page): Promise<Record<string, string | null>> {
  return page.locator("#answer").evaluate((element) => ({
    text: (element as HTMLElement).innerText,
    digest: element.getAttribute("data-final-digest"),
    lifecycle: element.getAttribute("data-canonical-lifecycle"),
    keys: element.getAttribute("data-stable-keys"),
    status: document.querySelector("#status")?.textContent ?? null,
    accessibleName: element.getAttribute("aria-label"),
  }));
}

function collectPageErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(message.text());
    }
  });
  page.on("pageerror", (error) => errors.push(error.message));
  return errors;
}

async function controlsOverlap(page: Page): Promise<boolean> {
  return page.locator(".control-band button, .mode-switch").evaluateAll((elements) => {
    const boxes = elements.map((element) => element.getBoundingClientRect());
    return boxes.some((left, leftIndex) => boxes.some((right, rightIndex) =>
      rightIndex > leftIndex &&
      left.left < right.right &&
      left.right > right.left &&
      left.top < right.bottom &&
      left.bottom > right.top
    ));
  });
}
