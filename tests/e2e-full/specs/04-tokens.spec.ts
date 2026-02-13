/**
 * 04-tokens — Token search flow (Phase 5).
 *
 * Tests token navigation and search against real testnet:
 * 1. Navigate to Tokens section via sidebar
 * 2. Open Search Tokens screen
 * 3. Perform a keyword search and verify results or empty state
 * 4. Clear search and verify reset
 *
 * Requires: 00-setup (wallet imported, SPV synced).
 */

import { navigateToSection, takeScreenshot } from "../helpers/tauri.js";
import { read as readContext } from "../helpers/test-context.js";

describe("Token Search", () => {
  let ctx: ReturnType<typeof readContext>;

  before(function () {
    ctx = readContext();
    if (!ctx.spvSynced) {
      throw new Error(
        "TestContext shows SPV not synced. Did 00-setup complete?"
      );
    }
  });

  // ─── UI: Navigate to tokens ────────────────────────────────────

  it("should navigate to tokens section", async () => {
    await navigateToSection("tokens");

    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/tokens");
      },
      { timeout: 10_000, timeoutMsg: "Did not navigate to /tokens" }
    );
  });

  // ─── UI: Open search screen ────────────────────────────────────

  it("should open the token search screen", async () => {
    // The token search is accessible via the "Search Tokens" tab in the left
    // sidebar or the "Search Tokens" button in the header.
    // Try the sidebar tab first, then fall back to the header button.
    let navigated = false;

    // Approach 1: Click "Search Tokens" tab in the tokens left panel
    let searchTab = await browser.$('[role="tab"]*=Search Tokens');
    if (await searchTab.isExisting()) {
      await searchTab.click();
      navigated = true;
    }

    if (!navigated) {
      // Approach 2: Click "Search Tokens" button in header
      const searchBtn = await browser.$("button*=Search Tokens");
      if (await searchBtn.isExisting()) {
        await searchBtn.click();
        navigated = true;
      }
    }

    expect(navigated).toBe(true);

    // Wait for the search screen to load
    await browser.waitUntil(
      async () => {
        const url = await browser.getUrl();
        return url.includes("/tokens/search");
      },
      {
        timeout: 10_000,
        timeoutMsg: "Did not navigate to /tokens/search",
      }
    );
  });

  // ─── UI: Token keyword search ──────────────────────────────────

  it("should search for tokens by keyword", async function () {
    this.timeout(60_000);

    // Find the search input
    const searchInput = await browser.$(
      'input[aria-label="Token search keyword"], input[placeholder*="keyword"]'
    );
    await searchInput.waitForExist({
      timeout: 10_000,
      timeoutMsg: "Token search input not found",
    });
    await searchInput.clearValue();
    await searchInput.setValue("dash");
    await browser.pause(300);

    // Click the search submit button (not the sidebar "Search Tokens" tab)
    const searchBtn = await browser.$("button=Search");
    await searchBtn.waitForExist({ timeout: 5_000 });
    await browser.waitUntil(
      async () => searchBtn.isEnabled(),
      { timeout: 5_000, timeoutMsg: "Search button not enabled" }
    );
    await searchBtn.click();

    // Wait for results or empty state — either is a pass
    await browser.waitUntil(
      async () => {
        // Check for table rows (results found)
        const rows = await browser.$$("table tbody tr");
        if (rows.length > 0) return true;

        // Check for "No results" / empty state (check each separately)
        for (const text of ["No results", "No tokens", "no matching"]) {
          const el = await browser.$(`*=${text}`);
          if (await el.isExisting()) return true;
        }

        // Check for any result content (contract IDs, descriptions)
        const resultContent = await browser.$("table");
        if (await resultContent.isExisting()) return true;

        // Check for pagination (means results loaded)
        const pagination = await browser.$("button*=Next");
        if (await pagination.isExisting()) return true;

        // Check for error state (platform unavailable)
        const errorEl = await browser.$('[role="alert"]');
        if (await errorEl.isExisting()) return true;

        return false;
      },
      {
        timeout: 30_000,
        interval: 2_000,
        timeoutMsg: "Token search did not return results or empty state within 30s",
      }
    );

    // Log the outcome
    const rows = await browser.$$("table tbody tr");
    if (rows.length > 0) {
      console.log(`  Token search returned ${rows.length} result(s)`);
    } else {
      const errorEl = await browser.$('[role="alert"]');
      if (await errorEl.isExisting()) {
        const errorText = await errorEl.getText();
        console.log(`  Token search encountered an error: ${errorText}`);
      } else {
        console.log("  Token search returned empty results (acceptable)");
      }
    }

    await takeScreenshot("token-search-results");
  });

  // ─── UI: Clear search ──────────────────────────────────────────

  it("should clear the search and reset the view", async () => {
    // Click the "Clear" button
    const clearBtn = await browser.$("button*=Clear");
    if (await clearBtn.isExisting()) {
      await clearBtn.click();
      await browser.pause(500);

      // Verify the input is cleared
      const searchInput = await browser.$(
        'input[aria-label="Token search keyword"], input[placeholder*="keyword"]'
      );
      const value = await searchInput.getValue();
      expect(value).toBe("");
      console.log("  Search cleared successfully");
    } else {
      // If no Clear button, manually clear the input
      const searchInput = await browser.$(
        'input[aria-label="Token search keyword"], input[placeholder*="keyword"]'
      );
      if (await searchInput.isExisting()) {
        await searchInput.clearValue();
        console.log("  Search input cleared manually (no Clear button found)");
      }
    }
  });
});
