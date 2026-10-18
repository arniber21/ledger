import { defineConfig, devices } from "@playwright/test";

declare const process: {
  env: Record<string, string | undefined>;
};

const ui = process.env.LEDGER_UI_URL ?? "http://127.0.0.1:5173";

export default defineConfig({
  testDir: ".",
  outputDir: "test-results",
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  forbidOnly: Boolean(process.env.CI),
  reporter: [["list"], ["html", { outputFolder: "report", open: "never" }]],
  use: {
    baseURL: ui,
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  // LEDGER_E2E_NO_STACK=1 skips `just e2e-stack` when the API and UI are
  // already running (for example a slot stack started by hand).
  ...(process.env.LEDGER_E2E_NO_STACK
    ? {}
    : {
        webServer: {
          command: "just e2e-stack",
          cwd: "../..",
          url: ui,
          reuseExistingServer: !process.env.CI,
          timeout: 1_200_000,
        },
      }),
});
