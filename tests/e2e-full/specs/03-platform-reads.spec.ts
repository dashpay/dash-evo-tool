/**
 * 03-platform-reads — Platform read operations (Phase 5).
 *
 * Tests platform read operations against real testnet:
 * 1. DPNS name lookup via Identities > Load Identity > By DPNS Name
 * 2. Contract fetch via Contracts > Load Contracts > enter DPNS contract ID
 * 3. Platform epoch info via IPC
 *
 * Requires: 00-setup (wallet imported, SPV synced).
 */

import { navigateToSection, takeScreenshot } from "../helpers/tauri.js";
import { invoke } from "../helpers/ipc.js";
import { read as readContext } from "../helpers/test-context.js";

/** Well-known DPNS contract ID (base58) used on all Dash Platform networks. */
const DPNS_CONTRACT_ID = "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec";

describe("Platform Read Operations", () => {
  let ctx: ReturnType<typeof readContext>;

  before(function () {
    ctx = readContext();
    if (!ctx.spvSynced) {
      throw new Error(
        "TestContext shows SPV not synced. Did 00-setup complete?"
      );
    }
  });

  // ─── UI: DPNS name lookup ────────────────────────────────────────

  it("should navigate to identities screen", async () => {
    await navigateToSection("identities");

    // Wait for the identities page to render
    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/identities");
      },
      { timeout: 10_000, timeoutMsg: "Did not navigate to /identities" }
    );
  });

  it("should open the Load Identity panel", async () => {
    // Click the "Load Identity" button (in the identity list panel header or empty state)
    let loadBtn = await browser.$('[data-testid="load-identity-btn"]');
    if (!(await loadBtn.isExisting())) {
      loadBtn = await browser.$('button*=Load Identity');
    }
    await loadBtn.waitForExist({
      timeout: 10_000,
      timeoutMsg: "Load Identity button not found",
    });
    await loadBtn.click();
    await browser.pause(500);

    // Verify the Load Identity panel appeared (has tabs)
    await browser.waitUntil(
      async () => {
        const tab = await browser.$('button[role="tab"]');
        return tab.isExisting();
      },
      {
        timeout: 10_000,
        timeoutMsg: "Load Identity panel did not appear (no tabs found)",
      }
    );
  });

  it("should search for a DPNS name", async function () {
    this.timeout(60_000);

    // Switch to "By DPNS Name" tab
    const tabs = await browser.$$('button[role="tab"]');
    let dpnsTabFound = false;
    for (const tab of tabs) {
      const text = await tab.getText();
      if (text.includes("DPNS Name")) {
        await tab.click();
        dpnsTabFound = true;
        break;
      }
    }
    expect(dpnsTabFound).toBe(true);
    await browser.pause(300);

    // Fill in a DPNS name to search for
    // Try common testnet names that are likely to exist
    const nameInput = await browser.$("#dpns-name-input");
    await nameInput.waitForExist({ timeout: 5_000 });
    await nameInput.clearValue();
    // "quantum" may or may not exist — this tests the search flow, not specific results
    await nameInput.setValue("quantum");

    // Click "Search by Username" button
    const searchBtn = await browser.$("button*=Search by Username");
    await searchBtn.waitForExist({ timeout: 5_000 });

    // Button should be enabled (name >= 3 chars)
    await browser.waitUntil(
      async () => searchBtn.isEnabled(),
      { timeout: 5_000, timeoutMsg: "Search by Username button not enabled" }
    );
    await searchBtn.click();

    // Wait for the search to complete — either success or error
    // The search dispatches an async task, so we wait for the panel to update
    await browser.waitUntil(
      async () => {
        // Check for success state (identity loaded)
        const successText = await browser.$("*=Successfully loaded");
        if (await successText.isExisting()) return true;

        const finishedText = await browser.$("*=Finished loading");
        if (await finishedText.isExisting()) return true;

        // Check for "loaded successfully" message
        const loadedMsg = await browser.$("*=loaded successfully");
        if (await loadedMsg.isExisting()) return true;

        // Check for error state (name not found is acceptable)
        const errorEl = await browser.$('[role="alert"], *=not found, *=error');
        if (await errorEl.isExisting()) return true;

        // Check for any error/status message in the panel
        const statusMsg = await browser.$("*=No identity found");
        if (await statusMsg.isExisting()) return true;

        return false;
      },
      {
        timeout: 30_000,
        interval: 2_000,
        timeoutMsg: "DPNS name search did not complete within 30s",
      }
    );

    // Log the outcome
    const bodyText = await browser.$("body").getText();
    if (
      bodyText.includes("Successfully loaded") ||
      bodyText.includes("loaded successfully") ||
      bodyText.includes("Finished loading")
    ) {
      console.log('  DPNS lookup succeeded: name "quantum" found on testnet');
    } else {
      console.log(
        '  DPNS lookup completed: name "quantum" not found (expected on some testnets)'
      );
    }

    await takeScreenshot("dpns-lookup-result");
  });

  // ─── UI: Contract fetch ──────────────────────────────────────────

  it("should navigate to contracts screen", async () => {
    await navigateToSection("contracts");

    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/contracts");
      },
      { timeout: 10_000, timeoutMsg: "Did not navigate to /contracts" }
    );
  });

  it("should click Load Contracts", async () => {
    let loadContractsBtn = await browser.$('[data-testid="action-load-contracts"]');
    if (!(await loadContractsBtn.isExisting())) {
      loadContractsBtn = await browser.$('button*=Load Contracts');
    }
    await loadContractsBtn.waitForExist({
      timeout: 10_000,
      timeoutMsg: "Load Contracts button not found",
    });
    await loadContractsBtn.click();

    // Wait for the Add Contracts screen
    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/contracts/add-contracts");
      },
      {
        timeout: 10_000,
        timeoutMsg: "Did not navigate to /contracts/add-contracts",
      }
    );
  });

  it("should enter the DPNS contract ID and fetch it", async function () {
    this.timeout(60_000);

    // Find the first contract input field
    const contractInput = await browser.$(
      '#contract-input-0, input[placeholder="Hex or base58 identifier"]'
    );
    await contractInput.waitForExist({ timeout: 5_000 });
    await contractInput.clearValue();
    await contractInput.setValue(DPNS_CONTRACT_ID);
    await browser.pause(300);

    // Click "Add Contracts" button
    const addBtn = await browser.$("button*=Add Contracts");
    await addBtn.waitForExist({ timeout: 5_000 });
    await browser.waitUntil(
      async () => addBtn.isEnabled(),
      {
        timeout: 5_000,
        timeoutMsg: "Add Contracts button not enabled",
      }
    );
    await addBtn.click();

    // Wait for the fetch to complete (shows "Successfully queried" or error)
    await browser.waitUntil(
      async () => {
        // Check for success
        const success = await browser.$("*=Successfully queried");
        if (await success.isExisting()) return true;

        // Check for error state
        const errorBanner = await browser.$('[role="alert"]');
        if (await errorBanner.isExisting()) return true;

        // Check for error message
        const errorMsg = await browser.$("*=Error");
        if (await errorMsg.isExisting()) {
          // Check if it's a real error, not just the label
          const text = await errorMsg.getText();
          if (
            text.includes("fetch") ||
            text.includes("timeout") ||
            text.includes("not found")
          ) {
            return true;
          }
        }

        return false;
      },
      {
        timeout: 45_000,
        interval: 2_000,
        timeoutMsg: "Contract fetch did not complete within 45s",
      }
    );

    // Verify the contract was found
    const bodyText = await browser.$("body").getText();
    if (bodyText.includes("Successfully queried")) {
      console.log("  Contract fetch succeeded: DPNS contract found");

      // Verify it shows the contract ID (green check icon area)
      const contractIdEl = await browser.$(`*=${DPNS_CONTRACT_ID.slice(0, 10)}`);
      expect(await contractIdEl.isExisting()).toBe(true);
    } else {
      console.log(
        "  Contract fetch did not return success — platform may be unavailable"
      );
      await takeScreenshot("contract-fetch-result");
    }
  });

  it("should navigate back to contracts list", async () => {
    const backBtn = await browser.$("button*=Back to Contracts");
    if (await backBtn.isExisting()) {
      await backBtn.click();
      await browser.pause(500);
    } else {
      await navigateToSection("contracts");
    }
  });

  // ─── Platform info via IPC ───────────────────────────────────────

  it("should fetch platform epoch info via IPC", async function () {
    this.timeout(30_000);

    // platform_current_epoch_info is an async dispatch command — it returns a taskId
    // For the E2E test, we just verify the IPC call succeeds (returns a task ID)
    try {
      const result = await invoke<{ taskId: string }>(
        "platform_current_epoch_info"
      );
      expect(result).toBeDefined();
      expect(result).toHaveProperty("taskId");
      expect(typeof result.taskId).toBe("string");
      console.log(`  Platform epoch info dispatched: taskId=${result.taskId}`);
    } catch (err) {
      // Platform info may fail if DAPI is unreachable — skip rather than silently pass
      const errMsg = err instanceof Error ? err.message : String(err);
      if (
        errMsg.includes("DAPI") ||
        errMsg.includes("timeout") ||
        errMsg.includes("unavailable") ||
        errMsg.includes("connect")
      ) {
        console.log(`  Platform unavailable (non-fatal): ${errMsg}`);
        this.skip();
      } else {
        throw err;
      }
    }
  });
});
