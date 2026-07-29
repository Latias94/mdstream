import { expect, test, type Page } from "@playwright/test";

test("immediate and paced fresh sessions settle to equal meaning", async ({ page }) => {
  const errors = collectPageErrors(page);
  await installDeliveryProbe(page);
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
  const animatedRuns = page.locator('#answer [data-delivery-animation="eligible"]');
  const settledRuns = page.locator('#answer [data-delivery-animation="ineligible"]');
  await expect(animatedRuns).toHaveCount(0);
  await expect(settledRuns).not.toHaveCount(0);
  for (let index = 0; index < await settledRuns.count(); index += 1) {
    await expect(settledRuns.nth(index)).toHaveAttribute("data-source-range", /^\d+:\d+$/);
  }
  await expect(page.locator('[data-event-kind="correction"]')).not.toHaveCount(0);
  const deliveryProbe = await page.evaluate(() =>
    (globalThis as typeof globalThis & {
      __mdstreamDeliveryProbe: {
        readonly started: readonly string[];
        readonly duplicates: readonly string[];
      };
    }).__mdstreamDeliveryProbe
  );
  expect(deliveryProbe.started).toEqual([]);
  expect(deliveryProbe.duplicates).toEqual([]);
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

test("a fresh delivery animates once before a late node update settles it", async ({ page }) => {
  const errors = collectPageErrors(page);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await installDeliveryProbe(page);
  await page.goto("/?autoplay=false");

  const rendered = await page.evaluate(async () => {
    const hostPolicyPath = "/src/host-policy.ts";
    const contentIrViewPath = "/src/content-ir-view.ts";
    const [{ HostPresentationPolicy }, { ContentIrView }] = await Promise.all([
      import(hostPolicyPath) as Promise<typeof import("../src/host-policy.js")>,
      import(contentIrViewPath) as Promise<typeof import("../src/content-ir-view.js")>,
    ]);
    const nodeId = "browser-animation";
    const bodyText = "fresh";
    const byteLength = (text: string) => new TextEncoder().encode(text).byteLength;
    const makeNodeView = (version: string) => ({
      schema: "mdstream.bindings/0.4",
      kind: "node_view",
      node: {
        id: nodeId,
        version,
        stability: "provisional",
        source: { start: "0", end: String(byteLength(bodyText)) },
        body: { start: "0", end: String(byteLength(bodyText)) },
        children: { version: "1", children: [] },
        content: { kind: "text", text: { kind: "source" } },
      },
      bodyText,
    });
    const documentStamp = (sequence: string, projectionCursor: string) => ({
      continuityGeneration: "0",
      coordinate: {
        epoch: "1",
        sequence,
        changeId: `browser:${sequence}`,
        sourceCursor: "0",
      },
      lifecycle: "open",
      projectionCursor,
      rootsVersion: "1",
    });
    const appendBatch = {
      facts: [{
        scope: "continuous",
        before: documentStamp("0", "0"),
        after: documentStamp("1", String(byteLength(bodyText))),
        nodes: [{
          key: { continuityGeneration: "0", epoch: "1", nodeId },
          before: {
            version: "0",
            stability: "provisional",
            parent: { kind: "document" },
            childrenVersion: "1",
          },
          after: {
            version: "1",
            stability: "provisional",
            parent: { kind: "document" },
            childrenVersion: "1",
          },
          text: {
            kind: "projection_append",
            range: { start: "0", end: String(byteLength(bodyText)) },
            text: bodyText,
          },
        }],
        structures: [],
        resources: [],
      }],
    };
    const nextFrame = () => new Promise<void>((resolve) => {
      requestAnimationFrame(() => resolve());
    });
    const nodeListeners = new Set<() => void>();
    let nodeView = makeNodeView("1");
    const store = {
      subscribe: () => () => undefined,
      subscribePendingSource: () => () => undefined,
      subscribeNode: (_id: string, listener: () => void) => {
        nodeListeners.add(listener);
        return () => nodeListeners.delete(listener);
      },
      getSnapshot: () => ({
        document: {
          coordinate: { epoch: "1" },
          roots: { children: [nodeId] },
        },
        impact: { fullReplace: false, rootsChanged: false },
      }),
      getNodeSnapshot: (id: string) => id === nodeId ? nodeView : undefined,
      getPendingSourceSnapshot: () => undefined,
      metrics: () => ({
        materializedNodeViews: "1",
        materializedResourceViews: "0",
        materializedPendingSourceViews: "0",
      }),
    };
    const policy = new HostPresentationPolicy("immediate", false);
    policy.consume(store as never, appendBatch as never);

    const fixture = document.createElement("section");
    const answer = document.createElement("article");
    const pending = document.createElement("aside");
    fixture.append(answer, pending);
    document.body.append(fixture);
    const view = new ContentIrView({
      store: store as never,
      policy,
      answerRoot: answer,
      pendingRoot: pending,
      onDiagnostics: () => undefined,
    });
    await nextFrame();
    await nextFrame();
    const initial = answer.querySelector<HTMLElement>("[data-delivery-animation]");
    const initialState = {
      animation: initial?.dataset.deliveryAnimation ?? null,
      range: initial?.dataset.sourceRange ?? null,
      cssAnimation: initial === null ? null : getComputedStyle(initial).animationName,
    };

    nodeView = makeNodeView("2");
    for (const listener of [...nodeListeners]) {
      listener();
    }
    await nextFrame();
    await nextFrame();
    const settled = answer.querySelector<HTMLElement>("[data-delivery-animation]");
    const settledState = {
      animation: settled?.dataset.deliveryAnimation ?? null,
      range: settled?.dataset.sourceRange ?? null,
      cssAnimation: settled === null ? null : getComputedStyle(settled).animationName,
    };
    view.close();
    fixture.remove();
    return { initialState, settledState };
  });

  expect(rendered.initialState).toEqual({
    animation: "eligible",
    range: "0:5",
    cssAnimation: "fresh-ink",
  });
  expect(rendered.settledState).toEqual({
    animation: "ineligible",
    range: "0:5",
    cssAnimation: "none",
  });
  const deliveryProbe = await page.evaluate(() =>
    (globalThis as typeof globalThis & {
      __mdstreamDeliveryProbe: {
        readonly started: readonly string[];
        readonly duplicates: readonly string[];
      };
    }).__mdstreamDeliveryProbe
  );
  expect(deliveryProbe.started).toHaveLength(1);
  expect(deliveryProbe.duplicates).toEqual([]);
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

async function installDeliveryProbe(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const seen = new Set<string>();
    const started: string[] = [];
    const duplicates: string[] = [];
    document.addEventListener("DOMContentLoaded", () => {
      document.addEventListener("animationstart", (event) => {
        const candidate = event.target;
        if (
          event.animationName !== "fresh-ink" ||
          !(candidate instanceof HTMLElement) ||
          candidate.dataset.deliveryAnimation !== "eligible"
        ) {
          return;
        }
        const hostKey = candidate.closest<HTMLElement>("[data-host-key]")?.dataset.hostKey;
        const range = candidate.dataset.sourceRange;
        const sequence = candidate.dataset.deliverySequence;
        const key = `${hostKey ?? "missing"}:${range ?? "missing"}:${sequence ?? "missing"}`;
        started.push(key);
        if (seen.has(key)) {
          duplicates.push(key);
        } else {
          seen.add(key);
        }
      }, { capture: true });
    }, { once: true });
    Object.defineProperty(globalThis, "__mdstreamDeliveryProbe", {
      value: { started, duplicates },
    });
  });
}
