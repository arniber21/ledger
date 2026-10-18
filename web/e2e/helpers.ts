import { expect, type APIRequestContext, type Page } from "@playwright/test";

export async function reset(request: APIRequestContext): Promise<void> {
  const response = await request.post("/api/test/reset");
  const body = await response.text();
  expect(response.ok(), body).toBeTruthy();
}

export async function runScene(request: APIRequestContext, name: string): Promise<void> {
  const response = await request.post(`/api/test/scenes/${name}`);
  const body = await response.text();
  expect(response.ok(), body).toBeTruthy();
}

export async function injectFault(request: APIRequestContext, name: string): Promise<void> {
  const response = await request.post(`/api/faults/${name}`);
  const body = await response.text();
  expect(response.ok(), body).toBeTruthy();
}

export async function waitForVerdict(
  page: Page,
  idOrPredicate: string | ((verdict: { id: string; text: string }) => boolean),
): Promise<void> {
  if (typeof idOrPredicate === "string") {
    const located = page
      .getByTestId(`verdict-${idOrPredicate}`)
      .or(page.getByTestId(`inbox-item-${idOrPredicate}`))
      .or(page.getByTestId(`verdict-row-${idOrPredicate}`));
    await expect(located.first()).toBeVisible();
    return;
  }
  await expect
    .poll(async () => {
      const items = page.locator(
        "[data-testid^='verdict-'], [data-testid^='inbox-item-'], [data-testid^='verdict-row-']",
      );
      const count = await items.count();
      for (let index = 0; index < count; index += 1) {
        const item = items.nth(index);
        const testId = (await item.getAttribute("data-testid")) ?? "";
        const text = await item.innerText();
        const id = testId
          .replace(/^verdict-row-/, "")
          .replace(/^verdict-/, "")
          .replace(/^inbox-item-/, "");
        if (idOrPredicate({ id, text })) return true;
      }
      return false;
    })
    .toBe(true);
}

export async function eventCount(page: Page): Promise<number> {
  const text = await page.getByTestId("metric-events").locator(".metric-value").innerText();
  return Number(text.replaceAll(",", ""));
}
