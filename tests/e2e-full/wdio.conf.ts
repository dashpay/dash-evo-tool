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
import { existsSync, readFileSync } from "fs";

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
    timeout: 660_000, // 11 min — SPV sync can take 10+ min on cold cache
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

    // Ensure we're on testnet. The setting is persisted from 00-setup,
    // but verify in case the app starts on a different default network.
    try {
      await browser.executeAsync(
        (done: (r: { ok: boolean }) => void) => {
          const t = (window as any).__TAURI_INTERNALS__;
          if (!t) return done({ ok: false });
          t.invoke("switch_network", { network: "testnet" })
            .then(() => done({ ok: true }))
            .catch(() => done({ ok: false }));
        }
      );
    } catch {
      // Best-effort — may already be on testnet
    }

    // Wait for wallets to be loaded from DB into memory.
    // Each spec file launches a fresh Tauri app instance. Wallet loading
    // from SQLite is async — wallet_list_all may return empty until done.
    // If TestContext has a walletSeedHash (from 00-setup), poll until
    // that wallet appears in wallet_list_all before running tests.
    const ctxPath = process.env.E2E_CONTEXT_PATH || "/tmp/e2e-test-context.json";
    let expectedSeedHash: string | null = null;
    try {
      if (existsSync(ctxPath)) {
        const ctx = JSON.parse(readFileSync(ctxPath, "utf-8"));
        expectedSeedHash = ctx.walletSeedHash || null;
      }
    } catch {
      // No context yet — 00-setup hasn't run
    }

    if (expectedSeedHash) {
      const seedHash = expectedSeedHash;
      await browser.waitUntil(
        async () => {
          try {
            const result = await browser.executeAsync(
              (
                hash: string,
                done: (r: { ok: boolean; found: boolean }) => void
              ) => {
                const t = (window as any).__TAURI_INTERNALS__;
                if (!t) return done({ ok: false, found: false });
                t.invoke("wallet_list_all")
                  .then((r: any) => {
                    const found =
                      r?.hdWallets?.some(
                        (w: any) => w.seedHash === hash
                      ) ?? false;
                    done({ ok: true, found });
                  })
                  .catch(() => done({ ok: false, found: false }));
              },
              seedHash
            );
            const res = result as { ok: boolean; found: boolean };
            return res.ok && res.found;
          } catch {
            return false;
          }
        },
        {
          timeout: 30_000,
          interval: 2_000,
          timeoutMsg:
            `Wallet ${seedHash.slice(0, 8)}... not loaded after 30s — ` +
            "wallet loading from DB may be slow or failed",
        }
      );
      console.log(`  Wallet ${seedHash.slice(0, 8)}... loaded in backend`);

      // Start SPV and wait for sync so the wallet can detect incoming
      // transactions (e.g. faucet funds). In 00-setup this block is skipped
      // because expectedSeedHash is null (no context file yet).
      try {
        // Stop any running SPV and clear cached data before starting.
        // Debug builds crash when a second app instance tries to sync
        // using stale cached SPV segment storage (known dash-spv bug).
        try {
          await browser.executeAsync(
            (done: (r: { ok: boolean; error?: string }) => void) => {
              const t = (window as any).__TAURI_INTERNALS__;
              if (!t) return done({ ok: false, error: "no bridge" });
              t.invoke("wallet_stop_spv")
                .then(() => done({ ok: true }))
                .catch((e: unknown) => done({ ok: false, error: String(e) }));
            }
          );
          await browser.pause(500);
          await browser.executeAsync(
            (done: (r: { ok: boolean; error?: string }) => void) => {
              const t = (window as any).__TAURI_INTERNALS__;
              if (!t) return done({ ok: false, error: "no bridge" });
              t.invoke("wallet_clear_spv_data")
                .then(() => done({ ok: true }))
                .catch((e: unknown) => done({ ok: false, error: String(e) }));
            }
          );
          console.log("  SPV stopped and cached data cleared");
        } catch (err) {
          console.warn(`  SPV stop/clear failed (non-fatal): ${err}`);
        }

        // Start SPV via IPC
        await browser.executeAsync(
          (done: (r: { ok: boolean; error?: string }) => void) => {
            const t = (window as any).__TAURI_INTERNALS__;
            if (!t) return done({ ok: false, error: "no bridge" });
            t.invoke("wallet_start_spv")
              .then(() => done({ ok: true }))
              .catch((e: unknown) => done({ ok: false, error: String(e) }));
          }
        );
        console.log("  SPV start requested");

        // Poll until SPV reaches "running" status for testnet
        await browser.waitUntil(
          async () => {
            try {
              const result = await browser.executeAsync(
                (done: (r: { ok: boolean; running: boolean }) => void) => {
                  const t = (window as any).__TAURI_INTERNALS__;
                  if (!t) return done({ ok: false, running: false });
                  t.invoke("get_spv_status")
                    .then((statuses: Array<{ network: string; status: string }>) => {
                      const entry = statuses.find(
                        (s) => s.network.toLowerCase() === "testnet"
                      );
                      done({ ok: true, running: entry?.status === "running" });
                    })
                    .catch(() => done({ ok: false, running: false }));
                }
              );
              const res = result as { ok: boolean; running: boolean };
              return res.ok && res.running;
            } catch {
              return false;
            }
          },
          {
            timeout: 120_000,
            interval: 3_000,
            timeoutMsg:
              "SPV did not reach 'running' status within 120s in before() hook",
          }
        );
        console.log("  SPV sync running");
      } catch (err) {
        console.warn(`  SPV start/sync in before() hook failed (non-fatal): ${err}`);
      }
    }

    // Allow background tasks (address scanning, SPV init) to settle.
    await browser.pause(5_000);
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
