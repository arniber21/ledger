import { expect, test, type Page } from "@playwright/test";
import { forceJev, reset, runScene } from "./helpers";

const POLICY =
  "After an agent pays a bill, no second payment may occur on that bill until a credit closes it.";
const TWO_CLAUSES = "Pay the bill. Keep a credit.";
const NOT_CATALOGUE = "Do not arm a pattern that is outside the catalogue.";
const MISSING = "This sentence has no saved draft.";

test.describe.configure({ timeout: 180_000 });

async function draft(page: Page, sentence: string): Promise<void> {
  await page.goto("/specs");
  await expect(page.getByTestId("shell")).toBeVisible();
  await page.getByTestId("new-spec").click();
  await page.getByTestId("spec-policy").fill(sentence);
  await page.getByTestId("draft-spec").click();
}

function kickers(page: Page) {
  return page.getByTestId("timeline-event").locator(".timeline-kicker");
}

async function openVerdict(page: Page, specId: string): Promise<void> {
  await page.goto("/verdicts");
  await expect(page.getByTestId("shell")).toBeVisible();
  const row = page.locator("[data-testid^='verdict-row-']", { hasText: specId });
  await expect(row.first()).toBeVisible();
  await row.first().click();
  await expect(page.getByTestId("verdict-detail")).toBeVisible();
}

function ys(points: string | null): number[] {
  if (!points) return [];
  return points
    .trim()
    .split(/\s+/)
    .map((pair) => Number(pair.split(",")[1]))
    .filter((value) => Number.isFinite(value));
}

test("an armed spec shows the authoring steps and starts a monitor", async ({ page, request }) => {
  await reset(request);
  await draft(page, POLICY);
  await expect(kickers(page)).toHaveText([
    "Policy entered",
    "Gemini drafted a pattern",
    "Catalogue validated",
    "Jev checked the draft",
    "Clause coverage",
    "Armed on the live stream",
  ]);
  await expect(page.getByTestId("timeline-event").nth(3)).toContainText("Same · threshold 0.80");
  await expect(page.getByTestId("spec-status")).toHaveText("Armed");
  await expect(page.getByTestId("armed-spec-ledger.authored.absence.after-until")).toBeVisible();
  await page.getByTestId("spec-tab-monitors").click();
  await expect(page.getByRole("cell", { name: "Running" })).toBeVisible();
  await expect(page.getByText(/fraud/i)).toHaveCount(0);
});

test("a catalogue miss is rejected with its reason and does not arm", async ({ page, request }) => {
  await reset(request);
  await draft(page, NOT_CATALOGUE);
  await expect(page.getByTestId("spec-status")).toHaveText("Rejected");
  await expect(page.getByTestId("timeline-event").last()).toContainText("Unknown pattern");
  await expect(page.locator("[data-testid^='armed-spec-']")).toHaveCount(0);
  await page.getByTestId("spec-tab-monitors").click();
  await expect(page.getByText("No monitor is running for this spec.")).toBeVisible();
});

test("a Jev Different check is rejected and does not arm", async ({ page, request }) => {
  await reset(request);
  await forceJev(request, "Different");
  await draft(page, POLICY);
  await expect(page.getByTestId("spec-status")).toHaveText("Rejected");
  await expect(page.getByTestId("timeline-event").last()).toContainText("Draft differs");
  await expect(page.getByTestId("timeline-event").filter({ hasText: "Jev checked the draft" })).toBeVisible();
  await expect(page.locator("[data-testid^='armed-spec-']")).toHaveCount(0);
  await page.getByTestId("spec-tab-monitors").click();
  await expect(page.getByText("No monitor is running for this spec.")).toBeVisible();
});

test("a missing draft fixture is rejected and does not arm", async ({ page, request }) => {
  await reset(request);
  await draft(page, MISSING);
  await expect(page.getByTestId("spec-status")).toHaveText("Rejected");
  await expect(page.getByTestId("timeline-event").last()).toContainText("Draft unavailable");
  await expect(page.locator("[data-testid^='armed-spec-']")).toHaveCount(0);
});

test("an uncovered clause blocks arming until override", async ({ page, request }) => {
  await reset(request);
  await draft(page, TWO_CLAUSES);
  await expect(page.getByTestId("clause-covered")).toContainText("Pay the bill");
  await expect(page.getByTestId("clause-uncovered")).toContainText("Keep a credit");
  await expect(page.getByTestId("clause-coverage")).toHaveText("1 uncovered");
  await expect(page.getByTestId("spec-status")).toHaveText("Needs review");
  await expect(page.locator("[data-testid^='armed-spec-']")).toHaveCount(0);
  await page.getByTestId("override-arm").click();
  await expect(page.getByTestId("spec-status")).toHaveText("Armed");
  await expect(page.getByTestId("armed-spec-ledger.authored.absence.after-until")).toBeVisible();
});

test("vendor-alias shows exact Different, fuzzy Different, and Jev Same", async ({ page, request }) => {
  await reset(request);
  await runScene(request, "vendor-alias");
  await openVerdict(page, "ledger.binding.vendor-alias");
  await expect(page.getByTestId("binding-exact")).toContainText("Different");
  await expect(page.getByTestId("binding-fuzzy")).toContainText("Different");
  await expect(page.getByTestId("binding-jev")).toContainText("Same");
  await expect(page.getByTestId("verdict-detail")).toContainText("Inconclusive");
});

test("recurring has no violation and the comparison explains why", async ({ page, request }) => {
  await reset(request);
  await runScene(request, "recurring");
  await page.goto("/verdicts");
  await expect(page.getByTestId("shell")).toBeVisible();
  await expect(page.locator("[data-outcome='Violate']")).toHaveCount(0);
  await openVerdict(page, "ledger.binding.recurring");
  await expect(page.getByTestId("binding-exact")).toContainText("Different");
  await expect(page.getByTestId("binding-fuzzy")).toContainText("Same");
  await expect(page.getByTestId("binding-jev")).toContainText("Different");
  await expect(page.getByTestId("verdict-detail")).toContainText("not a second payment");
});

test("Jev Unknown and timeout stay inconclusive", async ({ page, request }) => {
  await reset(request);
  await forceJev(request, "Unknown");
  await runScene(request, "vendor-alias");
  await openVerdict(page, "ledger.binding.vendor-alias");
  await expect(page.getByTestId("binding-jev")).toContainText("Unknown");
  await expect(page.getByTestId("verdict-detail")).toContainText("Inconclusive");
  await expect(page.locator("[data-outcome='Violate']")).toHaveCount(0);
  await expect(page.locator("[data-outcome='Satisfy']")).toHaveCount(0);

  await reset(request);
  await forceJev(request, "timeout");
  await runScene(request, "vendor-alias");
  await openVerdict(page, "ledger.binding.vendor-alias");
  await expect(page.getByTestId("binding-jev")).toContainText("Unknown");
  await expect(page.getByTestId("binding-jev")).toContainText("timeout");
  await expect(page.getByTestId("verdict-detail")).toContainText("Inconclusive");
  await expect(page.locator("[data-outcome='Violate']")).toHaveCount(0);
  await expect(page.locator("[data-outcome='Satisfy']")).toHaveCount(0);
});

test("Jev Different suppresses the double-pay", async ({ page, request }) => {
  await reset(request);
  await forceJev(request, "Different");
  await runScene(request, "double-pay");
  await page.goto("/verdicts");
  await expect(page.getByTestId("shell")).toBeVisible();
  await expect(page.locator("[data-outcome='Violate']")).toHaveCount(0);
  await expect(page.locator("[data-outcome='Satisfy']")).toHaveCount(0);
  await openVerdict(page, "ledger.binding.double-pay");
  await expect(page.getByTestId("binding-jev")).toContainText("Different");
  await expect(page.getByTestId("verdict-detail")).toContainText("keeps the second payment off this bill");
  await expect(page.getByTestId("verdict-detail")).toContainText("Inconclusive");
});

test("the soak chart stays flat and a later duplicate is needs review", async ({ page, request }) => {
  await reset(request);
  await runScene(request, "gc-soak");
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  const chart = page.getByTestId("chart-monitors");
  await expect(chart).toContainText("With GC 15");
  await expect(chart).toContainText("Without GC 615");
  const accent = ys(await chart.locator("polyline.line-accent").getAttribute("points"));
  const grey = ys(await chart.locator("polyline.line-dashed").getAttribute("points"));
  expect(accent.length).toBeGreaterThan(2);
  expect(grey.length).toBeGreaterThan(2);
  const tail = accent.slice(-3);
  expect(new Set(tail).size).toBe(1);
  expect(grey[grey.length - 1]).toBeLessThan(grey[0]);
  expect(accent[accent.length - 1]).toBeGreaterThan(grey[grey.length - 1]);

  await openVerdict(page, "ledger.bill.no-second-payment");
  await expect(page.locator("[data-outcome='Violate']")).toHaveCount(1);
  await expect(page.getByTestId("verdict-detail")).toContainText("Needs review");
  await expect(page.getByTestId("verdict-detail")).toContainText("monitor stayed live");
  await expect(page.getByText(/fraud/i)).toHaveCount(0);
});
