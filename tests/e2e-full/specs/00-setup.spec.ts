/**
 * 00-setup — Wallet import + SPV sync (Phase 2).
 *
 * Incorporates the Phase 1 IPC smoke tests, then:
 * 1. Switches to testnet via IPC
 * 2. Navigates to the wallets screen
 * 3. Imports a wallet using E2E_WALLET_MNEMONIC via UI
 * 4. Starts SPV sync and waits for completion
 * 5. Verifies connection state and persists TestContext
 *
 * Requires: E2E_WALLET_MNEMONIC env var (BIP39 testnet mnemonic, 12/15/18/21/24 words).
 */

import { waitForAppReady, navigateToSection, takeScreenshot } from "../helpers/tauri.js";
import { invoke, isBridgeAvailable, waitForSpvSync, getWalletBalance } from "../helpers/ipc.js";
import { update as updateContext } from "../helpers/test-context.js";

const MNEMONIC = process.env.E2E_WALLET_MNEMONIC;
const WALLET_ALIAS = "E2E Test Wallet";

// ─── IPC Smoke (from Phase 1) ──────────────────────────────────────

describe("IPC Smoke", () => {
  it("should launch the Tauri app and render the UI", async () => {
    await waitForAppReady();
    const title = await browser.getTitle();
    expect(title).toBe("Dash Evo Tool");
  });

  it("should have the Tauri IPC bridge available", async () => {
    expect(await isBridgeAvailable()).toBe(true);
  });

  it("should return the app version via IPC", async () => {
    const version = await invoke<string>("get_app_version");
    expect(typeof version).toBe("string");
    expect(version).toMatch(/^\d+\.\d+/);
  });

  it("should return network info via IPC", async () => {
    const info = await invoke<{
      activeNetwork: string;
      availableNetworks: string[];
    }>("get_network_info");
    expect(info).toBeDefined();
    expect(Array.isArray(info.availableNetworks)).toBe(true);
    expect(info.availableNetworks.length).toBeGreaterThan(0);
  });

  it("should return settings via IPC", async () => {
    const result = await invoke("settings_get");
    expect(result).toBeDefined();
  });

  it("should return SPV status array via IPC", async () => {
    const statuses = await invoke<Array<{ network: string; status: string }>>(
      "get_spv_status"
    );
    expect(Array.isArray(statuses)).toBe(true);
  });
});

// ─── Wallet Import + SPV Sync ──────────────────────────────────────

describe("Wallet Import & SPV Sync", () => {
  before(function () {
    if (!MNEMONIC) {
      throw new Error(
        "E2E_WALLET_MNEMONIC env var is required. " +
          "Set it to a BIP39 testnet mnemonic (12/15/18/21/24 words)."
      );
    }
    const wordCount = MNEMONIC.trim().split(/\s+/).length;
    if (![12, 15, 18, 21, 24].includes(wordCount)) {
      throw new Error(
        `E2E_WALLET_MNEMONIC has ${wordCount} words, expected 12/15/18/21/24.`
      );
    }
  });

  it("should switch to testnet via IPC", async () => {
    await invoke("switch_network", { network: "testnet" });

    // Verify network switched
    const info = await invoke<{ activeNetwork: string }>("get_network_info");
    expect(info.activeNetwork).toBe("testnet");
  });

  it("should navigate to wallets screen", async () => {
    // Wait for the app to settle after network switch
    await browser.pause(1000);

    // Dismiss the welcome screen if visible
    const welcome = await browser.$('[data-testid="welcome-screen"]');
    if (await welcome.isExisting()) {
      // Click "Import Wallet" on the welcome screen to mark onboarding complete
      const importCard = await browser.$("button*=Import Wallet");
      if (await importCard.isExisting()) {
        await importCard.click();
        await browser.pause(500);
      }
    }

    await navigateToSection("wallets");

    // Verify we're on the wallets page
    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/wallets");
      },
      { timeout: 10_000, timeoutMsg: "Did not navigate to /wallets" }
    );
  });

  it("should navigate to the import wallet screen", async () => {
    // Click "Import Wallet" button (either in empty state or action buttons)
    const importBtn = await browser.$("button=Import Wallet");
    await importBtn.waitForClickable({ timeout: 10_000 });
    await importBtn.click();

    // Wait for the import wallet screen to load
    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/wallets/import");
      },
      { timeout: 10_000, timeoutMsg: "Did not navigate to /wallets/import" }
    );
  });

  it("should fill in the mnemonic seed phrase via UI", async () => {
    // Wait for the seed phrase grid to appear
    const grid = await browser.$('[data-testid="seed-phrase-grid"]');
    await grid.waitForExist({ timeout: 10_000 });

    const words = MNEMONIC!.trim().split(/\s+/);

    // The default word count is 24 — adjust if our mnemonic has a different length.
    // The word count must match for BIP39 validation to pass.
    if (words.length !== 24) {
      // Open the Radix Select dropdown for word count
      const wordCountTrigger = await browser.$("#import-word-count");
      await wordCountTrigger.click();
      await browser.pause(300);

      // Radix Select items render with role="option"
      const option = await browser.$(`[role="option"]=${words.length} words`);
      await option.waitForExist({ timeout: 5_000 });
      await option.click();
      await browser.pause(300);
    }

    // Paste the entire mnemonic into the first word field.
    // The component's handleWordChange detects multi-word paste and distributes
    // words across all input fields automatically.
    const firstWordInput = await browser.$('[data-testid="seed-word-1"]');
    await firstWordInput.waitForExist({ timeout: 5_000 });
    await firstWordInput.click();
    await firstWordInput.setValue(MNEMONIC!.trim());

    // Wait for validation to process
    await browser.pause(500);

    // Verify the mnemonic is valid — the name/password section should appear
    const aliasInput = await browser.$('[data-testid="import-alias"]');
    await aliasInput.waitForExist({
      timeout: 10_000,
      timeoutMsg: "Name field did not appear — mnemonic may be invalid",
    });
  });

  it("should set wallet alias and submit import", async () => {
    // Fill in the wallet alias
    const aliasInput = await browser.$('[data-testid="import-alias"]');
    await aliasInput.clearValue();
    await aliasInput.setValue(WALLET_ALIAS);

    // Click the import button
    const submitBtn = await browser.$('[data-testid="import-wallet-submit"]');
    await submitBtn.waitForClickable({ timeout: 5_000 });
    await submitBtn.click();

    // Wait for either success screen or error
    // The import can take a few seconds for identity scanning
    const success = await browser.$('[data-testid="import-success"]');
    try {
      await success.waitForExist({
        timeout: 60_000,
        timeoutMsg: "Import did not complete within 60s",
      });
    } catch (err) {
      // Check if it failed because wallet already exists
      const toast = await browser.$('[data-sonner-toast]');
      if (await toast.isExisting()) {
        const toastText = await toast.getText();
        if (toastText.toLowerCase().includes("already")) {
          // Wallet already imported — navigate back to wallets list
          console.log("Wallet already imported, continuing with existing wallet");
          const backBtn = await browser.$('button[aria-label="Back to wallets"]');
          if (await backBtn.isExisting()) {
            await backBtn.click();
            await browser.pause(500);
          }
          await navigateToSection("wallets");
          return;
        }
      }
      await takeScreenshot("import-failed");
      throw err;
    }

    // Click "Go to Wallet" on the success screen
    const goToWalletBtn = await browser.$("button=Go to Wallet");
    if (await goToWalletBtn.isExisting()) {
      await goToWalletBtn.click();
      await browser.pause(500);
    }
  });

  it("should show the imported wallet in the wallet list", async () => {
    // Navigate to wallets if not already there
    const url = await browser.getUrl();
    if (!url.includes("/wallets")) {
      await navigateToSection("wallets");
    }

    // Wait for wallet list to display our wallet
    await browser.waitUntil(
      async () => {
        const pageText = await browser.$("body").getText();
        return pageText.includes(WALLET_ALIAS);
      },
      {
        timeout: 15_000,
        timeoutMsg: `Wallet "${WALLET_ALIAS}" did not appear in wallet list`,
      }
    );
  });

  it("should start SPV sync via IPC", async () => {
    await invoke("wallet_start_spv");
    // Give SPV a moment to begin
    await browser.pause(2000);
  });

  it("should complete SPV sync", async function () {
    // SPV sync can take 1-5 minutes depending on cache state
    this.timeout(360_000);

    try {
      await waitForSpvSync("testnet", 300_000);
    } catch (err) {
      await takeScreenshot("spv-sync-timeout");

      // Log the last SPV status for debugging
      try {
        const statuses = await invoke<Array<{ network: string; status: string }>>(
          "get_spv_status"
        );
        console.log("SPV status at timeout:", JSON.stringify(statuses));
      } catch {
        // ignore
      }
      throw err;
    }
  });

  it("should save wallet info to test context", async () => {
    // Get wallet list to find our wallet's seedHash and balance
    const result = await invoke<{
      hdWallets: Array<{
        seedHash: string;
        alias: string | null;
        totalBalance: number;
      }>;
    }>("wallet_list_all");

    const wallet = result.hdWallets.find(
      (w) => w.alias === WALLET_ALIAS
    );

    // If not found by alias, use the first HD wallet
    const targetWallet = wallet || result.hdWallets[0];
    expect(targetWallet).toBeDefined();

    const seedHash = targetWallet!.seedHash;
    const balance = await getWalletBalance(seedHash);

    if (balance === 0) {
      console.warn(
        "WARNING: Imported wallet has zero balance. " +
          "Subsequent tests requiring funds will fail until the faucet runs (Phase 3)."
      );
    }

    // Persist to test context for later specs
    const ctx = updateContext({
      walletSeedHash: seedHash,
      balanceDuffs: balance,
      spvSynced: true,
      network: "testnet",
    });

    console.log(
      `Test context saved: seedHash=${seedHash.slice(0, 8)}..., balance=${balance} duffs`
    );

    expect(ctx.walletSeedHash).toBeTruthy();
    expect(ctx.spvSynced).toBe(true);
  });
});
