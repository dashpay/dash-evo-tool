import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
  use: {
    baseURL: "http://localhost:1420",
    trace: "on-first-retry",
  },
  projects: [
    // Existing basic Playwright tests (no Tauri backend, basic rendering)
    {
      name: "chromium",
      testDir: "./tests/playwright",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "webkit",
      testDir: "./tests/playwright",
      use: { ...devices["Desktop Safari"] },
    },
    // Integration tests with mock IPC — runs against Vite dev server
    // with VITE_E2E_MOCK=true so all Tauri IPC calls are intercepted
    {
      name: "integration",
      testDir: "./tests/e2e-integration",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: process.env.VITE_E2E_MOCK === "true"
      ? "VITE_E2E_MOCK=true npm run dev"
      : "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    env: {
      ...process.env,
      VITE_E2E_MOCK: process.env.VITE_E2E_MOCK ?? "",
    },
  },
});
