/**
 * 02-wallet — Wallet UI operations (Phase 4).
 *
 * Tests wallet UI interactions against real blockchain data:
 * 1. Navigate to wallets and verify balance display
 * 2. Click wallet card to show detail view
 * 3. Verify Addresses tab shows real testnet addresses
 * 4. Open receive dialog, verify QR code + copyable address
 * 5. Conditional send-to-self (0.001 DASH if funded)
 * 6. Verify transactions tab (developer mode only)
 *
 * Requires: 00-setup (wallet imported, SPV synced), 01-faucet (wallet funded).
 */

import { navigateToSection, takeScreenshot } from "../helpers/tauri.js";
import {
  read as readContext,
  update as updateContext,
} from "../helpers/test-context.js";
import { ensureFunded } from "../helpers/ensure-funded.js";

describe("Wallet UI Operations", () => {
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

  // ─── UI: Navigate to wallets ──────────────────────────────────────

  it("should navigate to wallets and show the test wallet", async () => {
    await navigateToSection("wallets");

    await browser.waitUntil(
      async () => {
        const card = await browser.$(
          `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
        );
        return card.isExisting();
      },
      {
        timeout: 30_000,
        timeoutMsg: "Wallet card did not appear in wallet list",
      }
    );
  });

  // ─── UI: Verify wallet card balance ───────────────────────────────

  it("should display a non-zero balance on the wallet card", async () => {
    const card = await browser.$(
      `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
    );
    const text = await card.getText();
    const match = text.match(/([\d.]+)\s*DASH/);
    expect(match).toBeTruthy();

    const balanceDash = parseFloat(match![1]);
    expect(balanceDash).toBeGreaterThan(0);
    console.log(`  Wallet card balance: ${match![1]} DASH`);
  });

  // ─── UI: Click wallet card to open detail view ────────────────────

  it("should select the wallet and show detail view", async () => {
    const card = await browser.$(
      `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
    );
    await card.click();
    await browser.pause(500);

    // Detail view should show Send + Receive action buttons
    await browser.waitUntil(
      async () => {
        const sendBtn = await browser.$("button=Send");
        const receiveBtn = await browser.$("button=Receive");
        return (await sendBtn.isExisting()) && (await receiveBtn.isExisting());
      },
      {
        timeout: 10_000,
        timeoutMsg: "Wallet detail (Send/Receive buttons) did not appear",
      }
    );
  });

  // ─── UI: Verify Addresses tab ─────────────────────────────────────

  it("should show real testnet addresses in the Addresses tab", async () => {
    // Addresses tab is the default active tab in HdWalletDetail.
    // Each address row has a <span title="<full address>"> inside a <td>.
    await browser.waitUntil(
      async () => {
        const spans = await browser.$$("td span[title]");
        for (const span of spans) {
          const title = await span.getAttribute("title");
          // Dash testnet P2PKH addresses start with 'y'
          if (title && /^y[a-km-zA-HJ-NP-Z1-9]{25,34}$/.test(title)) {
            return true;
          }
        }
        return false;
      },
      {
        timeout: 15_000,
        timeoutMsg:
          "No testnet addresses (starting with 'y') found in Addresses tab",
      }
    );
    console.log("  Testnet addresses verified in Addresses tab");
  });

  // ─── UI: Receive dialog ───────────────────────────────────────────

  it("should open receive dialog with QR code and address", async () => {
    const receiveBtn = await browser.$("button=Receive");
    await receiveBtn.waitForClickable({ timeout: 10_000 });
    await receiveBtn.click();

    // Wait for dialog
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

    // Verify QR code (QRCodeSVG wrapped in a div with role="img")
    const qrWrapper = await browser.$('[role="dialog"] [role="img"]');
    expect(await qrWrapper.isExisting()).toBe(true);
    // Verify the actual SVG rendered inside
    const qrSvg = await browser.$('[role="dialog"] [role="img"] svg');
    expect(await qrSvg.isExisting()).toBe(true);
    console.log("  QR code SVG rendered");

    // Verify testnet address in <code> element
    const addressEl = await browser.$('[role="dialog"] code');
    await addressEl.waitForExist({ timeout: 5_000 });
    const address = await addressEl.getText();
    expect(address.charAt(0)).toBe("y");
    expect(address.length).toBeGreaterThanOrEqual(26);
    console.log(`  Receive address: ${address}`);

    // Verify copy button exists next to the address
    const copyBtn = await browser.$(
      '[role="dialog"] button*=Copy'
    );
    expect(await copyBtn.isExisting()).toBe(true);

    // Save address for send-to-self
    updateContext({ receiveAddress: address });
    ctx = readContext();
  });

  it("should dismiss the receive dialog", async () => {
    const closeBtn = await browser.$(
      '[role="dialog"] button[aria-label="Close"]'
    );
    if (await closeBtn.isExisting()) {
      await closeBtn.click();
    } else {
      await browser.keys("Escape");
    }

    await browser.waitUntil(
      async () => {
        const dialog = await browser.$('[role="dialog"]');
        return !(await dialog.isExisting());
      },
      { timeout: 5_000, timeoutMsg: "Receive dialog did not close" }
    );
  });

  // ─── Ensure funded before send ────────────────────────────────────

  it("should ensure wallet is funded for send-to-self", async function () {
    this.timeout(240_000);

    const result = await ensureFunded(10_000_000); // 0.1 DASH minimum
    ctx = readContext();

    if (!result.funded) {
      console.log("  Wallet not funded — send-to-self will be skipped");
      await takeScreenshot("wallet-unfunded");
    } else {
      console.log(`  Wallet funded: ${result.balanceDuffs} duffs`);
    }
  });

  // ─── UI: Send-to-self (conditional) ───────────────────────────────

  it("should send 0.001 DASH to self (conditional)", async function () {
    this.timeout(120_000);

    ctx = readContext();
    if (ctx.balanceDuffs < 10_000_000) {
      console.log("  Skipping send-to-self: insufficient funds");
      this.skip();
      return;
    }

    // Re-navigate and select wallet to ensure clean state
    await navigateToSection("wallets");
    await browser.pause(500);

    const card = await browser.$(
      `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
    );
    await card.waitForExist({ timeout: 10_000 });
    await card.click();
    await browser.pause(500);

    // Click "Send" button on wallet detail → navigates to SendScreen
    const detailSendBtn = await browser.$("button=Send");
    await detailSendBtn.waitForClickable({ timeout: 10_000 });
    await detailSendBtn.click();

    // Wait for SendScreen to load ("Send Dash" heading)
    await browser.waitUntil(
      async () => {
        const heading = await browser.$("h1=Send Dash");
        return heading.isExisting();
      },
      {
        timeout: 10_000,
        timeoutMsg: "Send screen ('Send Dash') did not appear",
      }
    );

    // Fill destination address (the receive address from TestContext)
    const addrInput = await browser.$('[aria-label="Destination address"]');
    await addrInput.waitForExist({ timeout: 5_000 });
    await addrInput.setValue(ctx.receiveAddress);
    // Wait for debounced address validation (300ms + network call)
    await browser.pause(1_000);

    // Fill amount
    const amountInput = await browser.$('input[placeholder="Enter amount"]');
    await amountInput.waitForExist({ timeout: 5_000 });
    await amountInput.setValue("0.001");
    await browser.pause(300);

    // Wait for "Core Transaction" button to be enabled
    // (core→core auto-detected from address format)
    await browser.waitUntil(
      async () => {
        const btn = await browser.$("button*=Core Transaction");
        if (!(await btn.isExisting())) return false;
        return btn.isEnabled();
      },
      {
        timeout: 15_000,
        timeoutMsg:
          "'Core Transaction' button not found or not enabled. " +
          "Address validation may have failed.",
      }
    );

    const coreTxBtn = await browser.$("button*=Core Transaction");
    await coreTxBtn.click();

    // Wait for confirmation dialog (ConfirmationDialog uses role="dialog")
    await browser.waitUntil(
      async () => {
        const dialog = await browser.$('[role="dialog"]');
        if (!(await dialog.isExisting())) return false;
        // Verify it's the confirmation dialog by checking for title
        const title = await browser.$('[role="dialog"] h2');
        if (!(await title.isExisting())) return false;
        const text = await title.getText();
        return text.includes("Confirm");
      },
      {
        timeout: 10_000,
        timeoutMsg: "Send confirmation dialog did not appear",
      }
    );

    // Click "Send" button inside confirmation dialog
    const dialogButtons = await browser.$$('[role="dialog"] button');
    let confirmed = false;
    for (const btn of dialogButtons) {
      const text = (await btn.getText()).trim();
      if (text === "Send") {
        await btn.click();
        confirmed = true;
        break;
      }
    }
    if (!confirmed) {
      await takeScreenshot("confirm-dialog-no-send-btn");
      throw new Error("Could not find 'Send' button in confirmation dialog");
    }

    // Wait for transaction to complete (handles fee dialog if it appears)
    let sendSucceeded = false;
    await browser.waitUntil(
      async () => {
        // Check for fee confirmation dialog (appears during "sending" state)
        const feeTitle = await browser.$("h2=Fee Confirmation Required");
        if (await feeTitle.isExisting()) {
          console.log("  Fee confirmation dialog detected — accepting...");
          const btns = await browser.$$('[role="dialog"] button');
          for (const b of btns) {
            const t = (await b.getText()).trim();
            if (t.includes("Confirm")) {
              await b.click();
              return false; // keep waiting for final result
            }
          }
        }

        // Check for success state ("Transaction Sent" heading)
        const success = await browser.$("h2=Transaction Sent");
        if (await success.isExisting()) {
          sendSucceeded = true;
          return true;
        }

        // Check for error state
        const errorBanner = await browser.$('[role="alert"]');
        if (await errorBanner.isExisting()) {
          return true; // stop waiting — will inspect error below
        }

        return false;
      },
      {
        timeout: 90_000,
        interval: 2_000,
        timeoutMsg: "Transaction did not complete within 90 seconds",
      }
    );

    if (sendSucceeded) {
      console.log("  Send-to-self transaction completed successfully!");

      // Click "Back to Wallet" to return to wallets screen
      const backBtn = await browser.$("button=Back to Wallet");
      if (await backBtn.isExisting()) {
        await backBtn.click();
        await browser.pause(1_000);
      }
    } else {
      // Error case — don't fail on insufficient funds
      const errorBanner = await browser.$('[role="alert"]');
      const errorText = await errorBanner.getText();
      const lowerError = errorText.toLowerCase();

      if (
        lowerError.includes("insufficient") ||
        lowerError.includes("not enough")
      ) {
        console.log(`  Send skipped (insufficient funds): ${errorText}`);
      } else {
        await takeScreenshot("send-to-self-error");
        throw new Error(`Send-to-self failed: ${errorText}`);
      }

      // Navigate back via the back arrow button
      const backArrow = await browser.$('[aria-label="Back to wallets"]');
      if (await backArrow.isExisting()) {
        await backArrow.click();
        await browser.pause(500);
      }
    }
  });

  // ─── UI: Verify transactions tab ──────────────────────────────────

  it("should show transactions in Transactions tab (if available)", async function () {
    this.timeout(30_000);

    // Navigate to wallets and select the test wallet
    await navigateToSection("wallets");
    await browser.pause(500);

    const card = await browser.$(
      `[data-wallet-type="hd"][data-seed-hash="${ctx.walletSeedHash}"]`
    );
    await card.waitForExist({ timeout: 10_000 });
    await card.click();
    await browser.pause(500);

    // Transactions tab is only visible in developer mode.
    // Look for a tab with text "Transactions".
    const tabs = await browser.$$('button[role="tab"]');
    let txTabFound = false;
    for (const tab of tabs) {
      const text = await tab.getText();
      if (text === "Transactions") {
        txTabFound = true;
        await tab.click();
        await browser.pause(500);
        break;
      }
    }

    if (!txTabFound) {
      console.log(
        "  Transactions tab not visible (developer mode off) — skipping"
      );
      this.skip();
      return;
    }

    // Verify content renders (transaction rows or "No transactions" message)
    await browser.waitUntil(
      async () => {
        const rows = await browser.$$("table tbody tr");
        if ((await rows.length) > 0) return true;
        const emptyEl = await browser.$("*=No transactions");
        return emptyEl.isExisting();
      },
      {
        timeout: 10_000,
        timeoutMsg: "Transactions tab did not render any content",
      }
    );

    const rows = await browser.$$("table tbody tr");
    const rowCount = await rows.length;
    console.log(
      rowCount > 0
        ? `  ${rowCount} transaction(s) found`
        : "  No transactions yet"
    );
  });
});
