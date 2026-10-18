import { expect, test } from "@playwright/test";
import { eventCount, reset, runScene } from "./helpers";

test("clean scene increases the event counter", async ({ page, request }) => {
  await reset(request);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  await expect(page.getByTestId("nav-dashboard")).toBeVisible();
  await expect(page.getByTestId("metric-events")).toBeVisible();
  const before = await eventCount(page);

  await runScene(request, "clean");

  await expect
    .poll(
      async () => {
        await page.reload();
        await expect(page.getByTestId("shell")).toBeVisible();
        return eventCount(page);
      },
      { timeout: 60_000, intervals: [500, 1_000, 2_000] },
    )
    .toBeGreaterThan(before);
});
