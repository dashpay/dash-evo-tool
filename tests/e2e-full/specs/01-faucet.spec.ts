/**
 * 01-faucet — Faucet funding + balance verification (Phase 3).
 *
 * 1. Reads TestContext from 00-setup (walletSeedHash, network)
 * 2. Navigates to wallet detail by clicking the wallet card
 * 3. Opens receive dialog, extracts and saves a testnet receive address
 * 4. Calls ensureFunded() to fund the wallet if needed
 * 5. Verifies the UI shows a non-zero balance
 *
 * Requires: 00-setup to have completed (wallet imported, SPV synced).
 */

import { navigateToSection, takeScreenshot } from "../helpers/tauri.js";
import { read as readContext, update as updateContext } from "../helpers/test-context.js";
import { ensureFunded } from "../helpers/ensure-funded.js";

describe("Faucet Funding & Balance", () => {
  let ctx: ReturnType<typeof readContext>;

  before(function () {
    ctx = readContext();
    if (!ctx.walletSeedHash) {
      throw new Error(
        "TestContext missing walletSeedHash. Did 00-setup complete?"
      );
    }
    if (!ctx.spvSynced) {
      throw new Error(
        "TestContext shows SPV not synced. Did 00-setup complete?"
      );
    }
  });

  // ─── UI: Navigate to wallet detail ─────────────────────────────────

  it("should navigate to wallets and select the test wallet", async () => {
    await navigateToSection("wallets");

    // Wait for wallet list to render
    await browser.waitUntil(
      async () => {
        const card = await browser.$(
          `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
        );
        return card.isExisting();
      },
      {
        timeout: 15_000,
        timeoutMsg: "Wallet card did not appear in wallet list",
      }
    );

    // Click the wallet card to select it
    const card = await browser.$(
      `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
    );
    await card.click();
    await browser.pause(500);
  });

  // ─── UI: Open receive dialog and extract address ───────────────────

  it("should open the receive dialog", async () => {
    // Find and click the "Receive" button in the wallet detail
    const receiveBtn = await browser.$("button=Receive");
    await receiveBtn.waitForClickable({ timeout: 10_000 });
    await receiveBtn.click();

    // Wait for the receive dialog to open
    await browser.waitUntil(
      async () => {
        const dialog = await browser.$('[role="dialog"]');
        return dialog.isExisting();
      },
      {
        timeout: 10_000,
        timeoutMsg: "Receive dialog did not open",
      }
    );
  });

  it("should display a testnet receive address", async () => {
    // The address is shown in a <code> element inside the dialog
    const addressEl = await browser.$('[role="dialog"] code');
    await addressEl.waitForExist({
      timeout: 10_000,
      timeoutMsg: "Address element not found in receive dialog",
    });

    const address = await addressEl.getText();
    expect(address).toBeTruthy();
    // Testnet addresses start with 'y'
    expect(address.charAt(0)).toBe("y");

    // Save address to TestContext
    updateContext({ receiveAddress: address });
    ctx = readContext();
    console.log(`  Receive address: ${address}`);
  });

  it("should dismiss the receive dialog", async () => {
    // Close via the X button or Escape
    const closeBtn = await browser.$(
      '[role="dialog"] button[aria-label="Close"]'
    );
    if (await closeBtn.isExisting()) {
      await closeBtn.click();
    } else {
      await browser.keys("Escape");
    }

    // Wait for dialog to close
    await browser.waitUntil(
      async () => {
        const dialog = await browser.$('[role="dialog"]');
        return !(await dialog.isExisting());
      },
      { timeout: 5_000, timeoutMsg: "Receive dialog did not close" }
    );
  });

  // ─── Faucet funding ────────────────────────────────────────────────

  it("should ensure the wallet is funded", async function () {
    // Faucet + SPV polling can take up to 3 minutes
    this.timeout(240_000);

    const result = await ensureFunded(10_000_000); // 0.1 DASH

    if (result.faucetUsed) {
      console.log(
        `  Faucet was used. Final balance: ${result.balanceDuffs} duffs`
      );
    } else {
      console.log(
        `  Wallet already funded. Balance: ${result.balanceDuffs} duffs`
      );
    }

    if (!result.funded) {
      await takeScreenshot("faucet-failed");
    }
    expect(result.funded).toBe(true);
  });

  // ─── UI: Verify balance ────────────────────────────────────────────

  it("should show a non-zero balance in the wallet list", async () => {
    // Navigate back to wallets to see updated balance
    await navigateToSection("wallets");
    await browser.pause(1000);

    // The wallet card shows balance as "{amount} DASH"
    const card = await browser.$(
      `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
    );
    await card.waitForExist({ timeout: 10_000 });

    // Wait for a non-zero balance to display
    await browser.waitUntil(
      async () => {
        const text = await card.getText();
        // Match a balance like "0.10000000 DASH" — anything above "0.00000000 DASH"
        const match = text.match(/([\d.]+)\s*DASH/);
        if (!match) return false;
        const balanceDash = parseFloat(match[1]);
        return balanceDash > 0;
      },
      {
        timeout: 30_000,
        interval: 2_000,
        timeoutMsg:
          "Wallet card did not show a non-zero balance within 30s. " +
          "The faucet may have succeeded but SPV hasn't updated the UI yet.",
      }
    );

    // Log the displayed balance
    const text = await card.getText();
    const match = text.match(/([\d.]+)\s*DASH/);
    if (match) {
      console.log(`  UI balance: ${match[1]} DASH`);
    }
  });
});
