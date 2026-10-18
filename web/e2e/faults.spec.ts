import { expect, test, type Page } from "@playwright/test";
import { reset, runScene } from "./helpers";

/**
 * Core loop from fault injection to the verdict detail.
 *
 * Double-pay is injected from the dashboard. The other faults use the trace
 * screen's fault controls. Sealed spec names come from the replay scenes.
 * A violate is "needs review". This spec does not invent a Solana signature
 * or a Sentry incident.
 */

const SOLANA = /^[1-9A-HJ-NP-Za-km-z]{43,88}$/;

function recordedSignature(text: string): string {
  const found = text.match(/[1-9A-HJ-NP-Za-km-z]{43,88}/);
  const signature = found?.[0] ?? "";
  if (!SOLANA.test(signature)) {
    throw new Error(`signature does not match the settle span: ${text}`);
  }
  return signature;
}

test.describe.configure({ timeout: 180_000 });

test("clean scene raises Satisfy and keeps Violate at 0", async ({ page, request }) => {
  await reset(request);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  const satisfyBefore = await columnCount(page, "satisfied");
  const violateBefore = await columnCount(page, "needs-review");

  await runScene(request, "clean");

  await expect
    .poll(() => columnAfterReload(page, "satisfied"), {
      timeout: 60_000,
      intervals: [500, 1_000, 2_000],
    })
    .toBeGreaterThan(satisfyBefore);
  expect(await columnCount(page, "needs-review")).toBe(violateBefore);
  await expect(page.locator("body")).not.toContainText(/fraud/i);
});

test("double-pay from the dashboard opens the needs-review counterexample", async ({ page, request }) => {
  await reset(request);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  expect(await columnCount(page, "needs-review")).toBe(0);
  await expect(page.locator("[data-testid='chart-attention'] .bar-violate")).toHaveCount(0);

  await page.getByTestId("action-inject-fault").click();
  await expect(page.getByTestId("palette")).toBeVisible();
  await page.getByText("Inject a double-pay fault", { exact: true }).click();
  await expect(page.getByTestId("palette-confirm")).toContainText("needs review");
  const sent = page.waitForRequest(
    (req) => req.method() === "POST" && req.url().includes("/api/faults/double-pay"),
  );
  await page.getByTestId("palette-confirm-fault").click();
  await sent;

  await runScene(request, "double-pay");

  await expect
    .poll(() => columnAfterReload(page, "needs-review"), {
      timeout: 60_000,
      intervals: [500, 1_000, 2_000],
    })
    .toBeGreaterThan(0);
  // The attention chart buckets by wall-clock minute. Scenes stamp events on a
  // fixed 2020 timeline, so they never land in its window; the Needs-review
  // count above is the live signal this spec asserts.

  const id = await openSpec(page, "ledger.bill.no-second-payment", /needs review/i);
  const steps = page.locator("ol[aria-label='Counterexample'] [data-testid='timeline-event']");
  await expect(steps).toHaveCount(3);
  const labels = ["Payment", "Settlement", "Second payment"];
  const signatures: string[] = [];
  for (let index = 0; index < labels.length; index += 1) {
    const step = steps.nth(index);
    await expect(step.locator(".mono").first()).toHaveText(labels[index] ?? "");
    const signature = recordedSignature(await step.locator(".signature").innerText());
    signatures.push(signature);
  }
  expect(new Set(signatures).size).toBeGreaterThan(1);

  const sentry = page.getByTestId("verdict-sentry");
  await expect(sentry).toHaveText("Open Sentry trace");
  const href = await sentry.getAttribute("href");
  if (href) {
    expect(href.startsWith("https://")).toBe(true);
    expect(href.toLowerCase()).not.toContain("fraud");
    expect(href.includes(id) || signatures.some((signature) => href.includes(signature))).toBe(true);
  } else {
    await expect(sentry).toBeDisabled();
  }

  await page.goto("/traces");
  await expect(page.getByTestId("shell")).toBeVisible();
  const settle = page.locator(".trace-span", { hasText: "ledger.exchange.settled" }).first();
  await expect(settle).toBeVisible({ timeout: 30_000 });
  await settle.click();
  await page.getByTestId("tab-attributes").click();
  const signatureLine = page.locator(".attr-line", { hasText: "tx.signature" }).first();
  await expect(signatureLine).toBeVisible();
  const recorded = recordedSignature(await signatureLine.innerText());
  expect(signatures).toContain(recorded);
  await expect(page.locator(".attr-line", { hasText: "bill.id" }).first()).toContainText("BILL-4417");
  await expect(page.locator("body")).not.toContainText(/fraud/i);
});

test("no-approval names the precedence spec", async ({ page, request }) => {
  await reset(request);
  await armFault(page, "no-approval");
  await runScene(request, "no-approval");
  await openSpec(page, "ledger.bill.approval-before-payment", /needs review/i);
});

test("exceed-budget fault arms from the trace controls (no replay scene yet)", async ({ page, request }) => {
  await reset(request);
  await armFault(page, "exceed-budget");
  await expect(page.getByTestId("fault-exceed-budget")).toBeVisible();
  await expect(page.locator("body")).not.toContainText(/fraud/i);
});

test("swapped vendor names the vendor-alias spec", async ({ page, request }) => {
  await reset(request);
  await armFault(page, "swapped-vendor");
  await runScene(request, "vendor-alias");
  await openSpec(page, "ledger.binding.vendor-alias", /inconclusive/i);
  await expect(page.getByTestId("binding-exact")).toBeVisible();
  await expect(page.getByTestId("binding-fuzzy")).toBeVisible();
  await expect(page.getByTestId("binding-jev")).toContainText("Same");
});

test("hidden memo names the tainted-memo spec", async ({ page, request }) => {
  await reset(request);
  await armFault(page, "hidden-memo");
  await runScene(request, "hidden-memo");
  await openSpec(page, "ledger.bill.tainted-memo-needs-approval", /needs review/i);
});

async function armFault(page: Page, fault: string): Promise<void> {
  await page.goto("/traces");
  await expect(page.getByTestId("shell")).toBeVisible();
  const button = page.getByTestId(`fault-${fault}`);
  const sent = page.waitForRequest(
    (req) => req.method() === "POST" && req.url().includes(`/api/faults/${fault}`),
  );
  await button.click();
  await sent;
  await expect(button).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("body")).not.toContainText(/fraud/i);
}

async function openSpec(page: Page, spec: string, label: RegExp): Promise<string> {
  await page.goto(`/inbox?spec=${encodeURIComponent(spec)}&view=list`);
  const row = page.locator("[data-testid^='inbox-item-']", { hasText: spec });
  await expect
    .poll(
      async () => {
        await page.reload();
        await expect(page.getByTestId("shell")).toBeVisible();
        try {
          await expect(row.first()).toBeVisible({ timeout: 3_000 });
          return true;
        } catch {
          return false;
        }
      },
      { timeout: 60_000, intervals: [1_000, 2_000] },
    )
    .toBe(true);
  const testId = (await row.first().getAttribute("data-testid")) ?? "";
  const id = testId.replace(/^inbox-item-/, "");
  expect(id.length).toBeGreaterThan(0);

  await page.goto(`/verdicts?id=${encodeURIComponent(id)}`);
  await expect(page.getByTestId("verdict-detail")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("verdict-identity")).toContainText(id);
  await expect(page.getByTestId("verdict-detail")).toContainText(spec);
  await expect(page.getByTestId("verdict-detail")).toContainText(label);
  await expect(page.locator("body")).not.toContainText(/fraud/i);
  return id;
}

async function columnAfterReload(page: Page, id: string): Promise<number> {
  await page.reload();
  await expect(page.getByTestId("shell")).toBeVisible();
  try {
    await expect
      .poll(() => columnCount(page, id), { timeout: 8_000, intervals: [200, 500, 1_000] })
      .toBeGreaterThan(0);
  } catch {
    // The column can legitimately stay at 0. The caller compares the number.
  }
  return columnCount(page, id);
}

async function columnCount(page: Page, id: string): Promise<number> {
  const text = await page.locator(`#${id} .column-count`).innerText();
  return Number(text.replaceAll(",", ""));
}
