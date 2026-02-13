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
      // tauri-driver expects tauri:options with the app binary path.
      // Do NOT set browserName — WebdriverIO v9 injects webSocketUrl:true
      // when browserName is present, which tauri-driver rejects.
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
    timeout: 360_000, // 6 min — SPV sync can be slow on cold start
  },

  // Reporter
  reporters: ["spec" as const],

  // Log level (reduce noise in CI)
  logLevel: "warn" as const,

  // Wait for tauri-driver connection
  connectionRetryTimeout: 60_000,
  connectionRetryCount: 5,

  // Timeouts
  waitforTimeout: 30_000,
  waitforInterval: 500,

  // Hooks
  beforeSession() {
    console.log(
      `Connecting to tauri-driver at localhost:${driverPort} with app: ${appBinary}`
    );
    // Log mnemonic availability (not the value) for Phase 2+ wallet import
    const hasMnemonic = !!process.env.E2E_WALLET_MNEMONIC;
    console.log(`  E2E_WALLET_MNEMONIC: ${hasMnemonic ? "set" : "NOT set (wallet tests will be skipped)"}`);
  },

  async before() {
    // Wait for the Tauri app to be ready.
    // Debug builds load the frontend from devUrl (http://localhost:1420)
    // served by docker/e2e/static-server.cjs. We wait for:
    // 1. Page title to appear (confirms WebView loaded the HTML)
    // 2. React to render a known screen (sidebar, welcome, or network chooser)
    await browser.waitUntil(
      async () => {
        try {
          const title = await browser.getTitle();
          if (!title) return false;

          const sidebar = await browser.$('[data-testid="sidebar"]');
          const welcome = await browser.$('[data-testid="welcome-screen"]');
          const networkChooser = await browser.$(
            '[data-testid="network-chooser-screen"]'
          );
          const uiReady =
            (await sidebar.isExisting()) ||
            (await welcome.isExisting()) ||
            (await networkChooser.isExisting());
          if (!uiReady) return false;

          // Also verify the Tauri IPC bridge is injected
          const bridgeReady = await browser.execute(() => {
            const t = (window as any).__TAURI_INTERNALS__;
            return !!(t && typeof t.invoke === "function");
          });
          return bridgeReady;
        } catch {
          return false;
        }
      },
      {
        timeout: 60_000,
        timeoutMsg:
          "Tauri app did not become ready within 60 seconds. " +
          "Check that the frontend static server is running (port 1420) " +
          "and AppState initialized successfully.",
        interval: 1000,
      }
    );

    // Verify backend is fully responsive (not just bridge available)
    // by making an actual IPC call. Heavy startup work (wallet loading,
    // identity scanning) can make WebDriver unstable until settled.
    await browser.waitUntil(
      async () => {
        try {
          const result = await browser.executeAsync(
            (done: (r: { ok: boolean }) => void) => {
              const t = (window as any).__TAURI_INTERNALS__;
              if (!t) return done({ ok: false });
              t.invoke("get_app_version")
                .then(() => done({ ok: true }))
                .catch(() => done({ ok: false }));
            }
          );
          return (result as { ok: boolean })?.ok === true;
        } catch {
          return false;
        }
      },
      {
        timeout: 30_000,
        interval: 2_000,
        timeoutMsg:
          "Backend IPC not responsive after 30s — startup may have stalled",
      }
    );

    // If the welcome screen is visible, dismiss it so the sidebar
    // is accessible for subsequent navigation.
    try {
      const welcome = await browser.$('[data-testid="welcome-screen"]');
      if (await welcome.isExisting()) {
        const importBtn = await browser.$("button*=Import Wallet");
        if (await importBtn.isExisting()) {
          await importBtn.click();
          await browser.pause(500);
        }
        // Wait for sidebar to become available
        const sidebar = await browser.$('[data-testid="sidebar"]');
        await sidebar.waitForExist({ timeout: 10_000 });
      }
    } catch {
      // Welcome screen handling is best-effort
    }

    // Allow background tasks (wallet loading, address scanning) to settle
    // before running test commands via WebDriver.
    await browser.pause(3_000);
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

  // Context file cleanup is handled by 05-teardown.spec.ts.
  // Do NOT clean up here — `after()` runs per-worker (per-spec-file),
  // which would delete the shared context before later specs read it.
};
