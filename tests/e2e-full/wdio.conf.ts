/**
 * WebdriverIO configuration for full E2E tests against real Tauri app.
 *
 * Connects to tauri-driver (WebDriver server) which manages the Tauri application.
 * Intended to run inside the Docker E2E environment (docker/e2e/).
 *
 * Usage:
 *   npx wdio tests/e2e-full/wdio.conf.ts
 *   npm run test:e2e-full  (via Docker Compose)
 */

import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Path to the built Tauri binary (set in Docker, overridable for local dev)
const appBinary =
  process.env.TAURI_APP_BINARY ||
  path.resolve(__dirname, "../../src-tauri/target/debug/dash-evo-tool-tauri");

// tauri-driver port (default matches docker/e2e/entrypoint.sh)
const driverPort = parseInt(process.env.TAURI_DRIVER_PORT || "4444", 10);

// WebdriverIO v9 config — exported as a plain object because the
// Options.Testrunner type does not include `capabilities` (it's merged at runtime by the CLI).
export const config = {
  runner: "local" as const,

  // Test specs
  specs: [path.resolve(__dirname, "specs/**/*.spec.ts")],
  exclude: [],

  // Max parallel instances (1 for Tauri — single app window)
  maxInstances: 1,

  capabilities: [
    {
      // tauri-driver uses the WebKitGTK WebDriver protocol
      browserName: "wry",
      "tauri:options": {
        application: appBinary,
      },
    },
  ],

  // Connection to tauri-driver
  port: driverPort,
  hostname: "localhost",
  path: "/",

  // Test framework
  framework: "mocha" as const,
  mochaOpts: {
    ui: "bdd" as const,
    timeout: 60_000, // 60s — Tauri app startup can be slow
  },

  // Reporter
  reporters: ["spec" as const],

  // Log level (reduce noise in CI)
  logLevel: "warn" as const,

  // Wait for tauri-driver connection
  connectionRetryTimeout: 30_000,
  connectionRetryCount: 5,

  // Timeouts
  waitforTimeout: 10_000,
  waitforInterval: 500,

  // Hooks
  beforeSession() {
    console.log(
      `Connecting to tauri-driver at localhost:${driverPort} with app: ${appBinary}`
    );
  },

  async before() {
    // Wait for the app to be ready (web content loaded)
    await browser.waitUntil(
      async () => {
        try {
          const title = await browser.getTitle();
          return title.length > 0;
        } catch {
          return false;
        }
      },
      {
        timeout: 30_000,
        timeoutMsg: "Tauri app did not become ready within 30 seconds",
        interval: 1000,
      }
    );
  },

  async afterTest(
    _test: unknown,
    _context: unknown,
    result: { passed: boolean }
  ) {
    // Take screenshot on failure
    if (!result.passed) {
      const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
      const screenshotPath = path.resolve(
        __dirname,
        `../../test-results/screenshot-${timestamp}.png`
      );
      try {
        await browser.saveScreenshot(screenshotPath);
        console.log(`  Screenshot saved: ${screenshotPath}`);
      } catch (err) {
        console.warn(`  Failed to save screenshot: ${err}`);
      }
    }
  },
};
