import { expect, test, type Page } from "@playwright/test";
import { reset, runScene } from "./helpers";

test.describe.configure({ mode: "serial", timeout: 180_000 });
test.use({ viewport: { width: 1440, height: 900 } });

test.beforeAll(async ({ request }) => {
  await reset(request);
  await runScene(request, "double-pay");
  await runScene(request, "hidden-memo");
  await runScene(request, "vendor-alias");
});

test("switching Board, List, and Split keeps the items and the selection", async ({ page }) => {
  await openInbox(page, "board");
  await page.keyboard.press("j");
  const selected = await selectedId(page);
  const items = await inboxIds(page);
  expect(selected).toBeTruthy();
  expect(items.length).toBeGreaterThan(1);

  await page.getByTestId("view-list").click();
  await expect(page).toHaveURL(/[?&]view=list/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "list");
  expect(await selectedId(page)).toBe(selected);
  expect(sorted(await inboxIds(page))).toEqual(sorted(items));

  await page.keyboard.press("3");
  await expect(page).toHaveURL(/[?&]view=split/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "split");
  expect(await selectedId(page)).toBe(selected);
  expect(sorted(await inboxIds(page))).toEqual(sorted(items));

  await page.keyboard.press("1");
  await expect(page).toHaveURL(/[?&]view=board/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "board");
  expect(await selectedId(page)).toBe(selected);
  expect(sorted(await inboxIds(page))).toEqual(sorted(items));

  await page.getByTestId("view-split").click();
  await page.keyboard.press("2");
  await expect(page).toHaveURL(/[?&]view=list/);
  expect(await selectedId(page)).toBe(selected);
  expect(sorted(await inboxIds(page))).toEqual(sorted(items));

  await shot(page, "board");
  await page.keyboard.press("2");
  await shot(page, "list");
  await page.keyboard.press("3");
  await shot(page, "split");
});

test("a filter chip narrows the queue in every mode", async ({ page }) => {
  for (const view of ["board", "list", "split"] as const) {
    await openInbox(page, view);
    const before = await inboxIds(page);
    await page.getByTestId("inbox-add-filter").click();
    await page.getByRole("menuitem", { name: "Verdict · Needs review" }).click();
    await expect(page.getByTestId("inbox-chip-verdict")).toContainText("Needs review");
    await expect(page).toHaveURL(/verdict=Violate/);
    await expect
      .poll(async () => (await inboxIds(page)).length, { timeout: 15_000 })
      .toBeLessThan(before.length);
    const after = await inboxIds(page);
    expect(sorted(before)).toEqual(expect.arrayContaining(sorted(after)));
    await expect(page.locator("[data-inbox-id]").getByText("inconclusive", { exact: true })).toHaveCount(0);
    await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", view);
    await page.getByTestId("inbox-chip-verdict").click();
    await expect(page.getByTestId("inbox-chip-verdict")).toHaveCount(0);
    await expect.poll(async () => (await inboxIds(page)).length).toBe(before.length);
  }
});

test("saved views apply their filter, group, and mode", async ({ page }) => {
  await openInbox(page, "board");
  await page.getByTestId("saved-violations").click();
  await expect(page).toHaveURL(/saved=violations/);
  await expect(page).toHaveURL(/view=board/);
  await expect(page).toHaveURL(/verdict=Violate/);
  await expect(page).toHaveURL(/group=status/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "board");
  await expect(page.getByTestId("inbox-group")).toHaveValue("status");
  await expect(page.getByTestId("inbox-chip-verdict")).toContainText("Needs review");
  await expect(page.locator("[data-inbox-id]").getByText("inconclusive", { exact: true })).toHaveCount(0);
  expect((await inboxIds(page)).length).toBeGreaterThan(0);

  await page.goto("/inbox?view=board");
  await expect(page.getByTestId("inbox-chip-verdict")).toHaveCount(0);
  await expect.poll(async () => (await inboxIds(page)).length).toBeGreaterThan(0);
  await expect.poll(async () => {
    if (await selectedId(page)) return true;
    await page.keyboard.press("j");
    return Boolean(await selectedId(page));
  }).toBe(true);
  await page.keyboard.press("a");
  await expect(page.getByTestId("assign-menu")).toBeVisible();
  await page.keyboard.press("1");
  await expect(page.locator(`[data-assignee="${await selectedId(page)}"]`)).toHaveText("ME");

  await page.getByTestId("saved-assigned").click();
  await expect(page).toHaveURL(/saved=assigned/);
  await expect(page).toHaveURL(/view=list/);
  await expect(page).toHaveURL(/assignee=me/);
  await expect(page).toHaveURL(/group=status/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "list");
  await expect(page.getByTestId("inbox-group")).toHaveValue("status");
  await expect(page.getByTestId("inbox-chip-assignee")).toHaveText(/Assigned to me/);
  const assigned = await inboxIds(page);
  expect(assigned.length).toBeGreaterThan(0);
  for (const id of assigned) {
    await expect(page.locator(`[data-assignee="${id}"]`)).toHaveText("ME");
  }

  await page.getByTestId("saved-unknowns").click();
  await expect(page).toHaveURL(/saved=unknowns/);
  await expect(page).toHaveURL(/view=list/);
  await expect(page).toHaveURL(/jev=unknown/);
  await expect(page).toHaveURL(/group=spec/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "list");
  await expect(page.getByTestId("inbox-group")).toHaveValue("spec");
  await expect(page.getByTestId("inbox-chip-jev")).toHaveText(/Jev unknowns/);

  await page.getByTestId("saved-over-1000").click();
  await expect(page).toHaveURL(/saved=over-1000/);
  await expect(page).toHaveURL(/view=list/);
  await expect(page).toHaveURL(/amount=over-1000/);
  await expect(page).toHaveURL(/group=agent/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "list");
  await expect(page.getByTestId("inbox-group")).toHaveValue("agent");
  await expect(page.getByTestId("inbox-chip-amount")).toHaveText(/Over \$1,000/);
});

test("priority weights reorder stored scores without calling Jev", async ({ page }) => {
  await openInbox(page, "split");
  await expect
    .poll(async () => {
      const text = (await page.getByTestId("inbox-split").innerText()).toLowerCase();
      const count = (await splitOrder(page)).length;
      return text.includes("inconclusive") && text.includes("needs review") && count >= 3;
    }, { timeout: 60_000 })
    .toBe(true);

  await page.getByTestId("inbox-sort").selectOption("amount");
  await expect(page).toHaveURL(/sort=amount/);
  await expect(page.getByTestId("inbox-split").locator("[data-inbox-id]").first()).toContainText(/inconclusive/i);
  const byAmount = await splitOrder(page);

  await page.getByTestId("rank-weights").click();
  await expect(page.getByTestId("rank-weights-popover")).toContainText("do not call Jev");
  const jev = armJev(page);
  await setSlider(page, "rank-weight-money", 100);
  await setSlider(page, "rank-weight-evidence", 0);
  await setSlider(page, "rank-weight-ambiguity", 0);
  await setSlider(page, "rank-weight-intent", 0);
  await expect(page).toHaveURL(/sort=priority/);
  await expect(page.getByTestId("inbox-split").locator("[data-inbox-id]").first()).toContainText(/needs review/i);
  const byPriority = await splitOrder(page);
  await page.waitForTimeout(500);
  jev.stop();
  expect(jev.hits).toEqual([]);
  expect(byPriority).not.toEqual(byAmount);
  expect(byPriority[0]).not.toBe(byAmount[0]);
});

test("the command palette finds a verdict and turns a phrase into filter chips", async ({ page }) => {
  await openInbox(page, "board");
  const id = (await inboxIds(page))[0];
  expect(id).toBeTruthy();

  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.getByTestId("palette")).toBeVisible();
  await page.getByTestId("palette-input").fill(id!);
  await expect(page.getByTestId("palette-result").filter({ hasText: id! }).first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("palette")).toBeHidden();

  await page.keyboard.press("ControlOrMeta+k");
  await page.getByTestId("palette-input").fill("violations over $1,000 by agent-2");
  await expect(page.getByTestId("palette-route")).toBeVisible();
  await page.getByTestId("palette-input").press("Enter");
  await expect
    .poll(() => new URL(page.url()).search, { timeout: 45_000 })
    .toMatch(/verdict=|amount=|agent=|spec=/);
  const params = new URL(page.url()).searchParams;
  expect(params.get("q")).toBeNull();
  const chips = [
    params.get("verdict") ? "inbox-chip-verdict" : "",
    params.get("amount") ? "inbox-chip-amount" : "",
    params.get("agent") ? "inbox-chip-agent" : "",
    params.get("spec") ? "inbox-chip-spec" : "",
  ].filter(Boolean);
  expect(chips.length).toBeGreaterThan(0);
  for (const chip of chips) {
    await expect(page.getByTestId(chip)).toBeVisible();
  }
  await expect(page.getByText("fraud")).toHaveCount(0);
});

test("keyboard triage reviews, dismisses, and assigns without the pointer", async ({ page }) => {
  await openInbox(page, "board");
  const items = await inboxIds(page);
  expect(items.length).toBeGreaterThan(1);

  await page.keyboard.press("2");
  await expect(page).toHaveURL(/view=list/);
  await page.keyboard.press("3");
  await expect(page).toHaveURL(/view=split/);
  await page.keyboard.press("1");
  await expect(page).toHaveURL(/view=board/);
  expect(sorted(await inboxIds(page))).toEqual(sorted(items));

  await page.keyboard.press("j");
  expect(await selectedId(page)).toBe(items[0]);
  await page.keyboard.press("j");
  expect(await selectedId(page)).toBe(items[1]);
  await page.keyboard.press("k");
  expect(await selectedId(page)).toBe(items[0]);

  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(new RegExp(`/verdicts\\?id=${items[0]}`));
  await expect(page.getByTestId("verdict-detail")).toBeVisible();
  await expect(page.getByTestId("verdict-identity")).toContainText(items[0]!);
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/inbox/);
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", "board");
  expect(await selectedId(page)).toBe(items[0]);

  await page.keyboard.press("e");
  await expect(page.getByTestId("column-Confirmed").locator(`[data-inbox-id="${items[0]}"]`)).toBeVisible();

  await page.keyboard.press("j");
  const dismissed = await selectedId(page);
  expect(dismissed).toBeTruthy();
  expect(dismissed).not.toBe(items[0]);
  await page.keyboard.press("d");
  await expect(page.getByTestId("column-Dismissed").locator(`[data-inbox-id="${dismissed}"]`)).toBeVisible();

  await page.keyboard.press("j");
  const assigned = await selectedId(page);
  expect(assigned).toBeTruthy();
  await page.keyboard.press("a");
  const menu = page.getByTestId("assign-menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("button", { name: /Unassigned/ })).toBeVisible();
  await expect(menu.getByRole("button", { name: /^ME\s*Me/ })).toBeVisible();
  await expect(menu.locator("kbd", { hasText: "0" })).toBeVisible();
  await expect(menu.locator("kbd", { hasText: "1" })).toBeVisible();
  await page.keyboard.press("1");
  await expect(menu).toBeHidden();
  await expect(page.locator(`[data-assignee="${assigned}"]`)).toHaveText("ME");
  await expect(page.getByText("fraud")).toHaveCount(0);
});

test("the inbox is empty when nothing still needs review", async ({ page }) => {
  await openInbox(page, "board");
  for (let step = 0; step < 40; step += 1) {
    if (await page.getByTestId("inbox-empty").isVisible()) break;
    const selected = await selectedId(page);
    if (!selected) await page.keyboard.press("j");
    const current = (await selectedId(page)) ?? "";
    const inNew = await page.getByTestId("column-New").locator(`[data-inbox-id="${current}"]`).count();
    const inReviewing = await page
      .getByTestId("column-Reviewing")
      .locator(`[data-inbox-id="${current}"]`)
      .count();
    if (inNew + inReviewing === 0) {
      await page.keyboard.press("j");
      continue;
    }
    await page.keyboard.press("e");
  }
  await expect(page.getByTestId("inbox-empty")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Nothing needs review" })).toBeVisible();
  await expect(page.getByText("needs review").first()).toBeVisible();
  await expect(page.getByText("fraud")).toHaveCount(0);
});

async function openInbox(page: Page, view: "board" | "list" | "split"): Promise<void> {
  await page.goto(`/inbox?view=${view}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", view, { timeout: 20_000 });
  await expect.poll(async () => (await inboxIds(page)).length, { timeout: 60_000 }).toBeGreaterThan(1);
  await page.evaluate(() => document.fonts.ready);
}

async function inboxIds(page: Page): Promise<string[]> {
  const view = await page.getByTestId("inbox-mode").getAttribute("data-view");
  if (view !== "list") {
    return page.locator("[data-inbox-id]").evaluateAll((nodes) =>
      nodes.map((node) => node.getAttribute("data-inbox-id") ?? "").filter((id) => id.length > 0),
    );
  }
  const seen = new Set<string>();
  const scroller = page.locator(".list-scroll");
  await scroller.evaluate((node) => {
    node.scrollTop = 0;
  });
  let previous = -1;
  for (let step = 0; step < 30; step += 1) {
    const batch = await page.locator("[data-inbox-id]").evaluateAll((nodes) =>
      nodes.map((node) => node.getAttribute("data-inbox-id") ?? ""),
    );
    for (const id of batch) {
      if (id) seen.add(id);
    }
    const top = await scroller.evaluate((node) => {
      const next = node.scrollTop + Math.max(node.clientHeight, 1);
      node.scrollTop = next;
      return node.scrollTop;
    });
    if (top === previous) break;
    previous = top;
  }
  return [...seen];
}

async function splitOrder(page: Page): Promise<string[]> {
  return page.getByTestId("inbox-split").locator("[data-inbox-id]").evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute("data-inbox-id") ?? "").filter((id) => id.length > 0),
  );
}

async function selectedId(page: Page): Promise<string | null> {
  const selected = page.locator("[data-inbox-id].is-selected");
  if ((await selected.count()) === 0) return null;
  return selected.first().getAttribute("data-inbox-id");
}

async function shot(page: Page, mode: "board" | "list" | "split"): Promise<void> {
  await page.keyboard.press(mode === "board" ? "1" : mode === "list" ? "2" : "3");
  await expect(page.getByTestId("inbox-mode")).toHaveAttribute("data-view", mode);
  await expect(page.locator("[data-inbox-id]").first()).toBeVisible();
  await expect(page).toHaveScreenshot(`inbox-${mode}.png`, {
    maxDiffPixelRatio: 0.02,
    animations: "disabled",
    caret: "hide",
  });
}

async function setSlider(page: Page, testId: string, value: number): Promise<void> {
  const slider = page.getByTestId(testId);
  await slider.evaluate((element, next) => {
    const input = element as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, String(next));
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  }, value);
  await expect(slider).toHaveValue(String(value));
}

function armJev(page: Page): { hits: string[]; stop: () => void } {
  const hits: string[] = [];
  const onRequest = (request: { url: () => string }) => {
    const url = request.url();
    if (/typesafe\.ai|systemone|quick-actions|\/api\/rank/i.test(url)) hits.push(url);
  };
  page.on("request", onRequest);
  return {
    hits,
    stop: () => page.off("request", onRequest),
  };
}

function sorted(ids: string[]): string[] {
  return [...ids].sort();
}
