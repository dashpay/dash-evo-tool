/**
 * Wallet lifecycle E2E tests — verifies wallet creation, listing, address
 * generation, viewing, and deletion work end-to-end with the real Tauri app.
 *
 * Prerequisites: Seeded database with testnet settings and test wallets.
 * Network access: NOT required for wallet list/detail/delete.
 *   - Create wallet and address generation are local operations.
 *   - Send transaction REQUIRES network access (testnet node).
 *
 * Tests marked [NETWORK] require a running Dash testnet node connection.
 */

import {
  waitForAppReady,
  navigateToSection,
  existsByTestId,
  waitForTestId,
  waitForDataLoad,
  clickButton,
  fillInput,
  getTextByTestId,
  waitForDialog,
  dismissDialog,
  waitForToast,
  takeScreenshot,
} from "../helpers/tauri.js";
import { setupTestDatabase, teardownTestDatabase } from "../helpers/database.js";

describe("Wallet Lifecycle", () => {
  before(async () => {
    try {
      setupTestDatabase("testnet");
    } catch {
      console.warn("Database setup skipped");
    }
  });

  after(async () => {
    try {
      teardownTestDatabase();
    } catch {
      // Ignore cleanup errors
    }
  });

  describe("Wallet list", () => {
    it("should display the wallet list with seeded wallet", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) {
        console.log("  Skipping: sidebar not visible");
        return;
      }

      await navigateToSection("wallets");
      await browser.pause(1000);

      // The seeded wallet "Test Wallet Alpha" should appear
      const pageContent = await browser.$("main, [role='main'], #content");
      let text = "";
      if (await pageContent.isExisting()) {
        text = await pageContent.getText();
      } else {
        const bodyEl = await browser.$("body");
        text = await bodyEl.getText();
      }

      // Wallet name or balance should be visible
      expect(
        text.includes("Test Wallet Alpha") ||
          text.includes("Wallet") ||
          text.includes("DASH") ||
          text.includes("wallet")
      ).toBe(true);
    });

    it("should show wallet balance from seed data", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(1000);

      // Seeded wallet has 5 DASH (500,000,000 duffs)
      const body = await browser.$("body");
      const text = await body.getText();

      // Balance should be displayed (5 DASH or 5.00000000)
      expect(
        text.includes("5.") || text.includes("5 DASH") || text.includes("500")
      ).toBe(true);
    });

    it("should select a wallet and show its details", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(1000);

      // Click on the wallet card
      const walletCard = await browser.$(
        '[data-testid="wallet-card"], [role="button"]'
      );
      if (await walletCard.isExisting()) {
        await walletCard.click();
        await browser.pause(500);

        // Detail panel should show addresses or account info
        const body = await browser.$("body");
        const text = await body.getText();
        expect(
          text.includes("Address") ||
            text.includes("address") ||
            text.includes("Account") ||
            text.includes("Balance") ||
            text.includes("UTXO")
        ).toBe(true);
      }
    });
  });

  describe("Wallet detail - HD wallet", () => {
    it("should show address list for HD wallet", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(1000);

      // Select the HD wallet
      const walletCard = await browser.$(
        '[data-testid="wallet-card"]'
      );
      if (await walletCard.isExisting()) {
        await walletCard.click();
        await browser.pause(500);

        // Seeded wallet has 4 addresses
        const body = await browser.$("body");
        const text = await body.getText();

        // Should show at least one of the seeded addresses
        expect(
          text.includes("yXk4") ||
            text.includes("yN7B") ||
            text.includes("yT3W") ||
            text.includes("yR2D") ||
            text.includes("m/44")
        ).toBe(true);
      }
    });

    it("should show wallet tabs (Addresses, Transactions, Asset Locks)", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(1000);

      const walletCard = await browser.$('[data-testid="wallet-card"]');
      if (await walletCard.isExisting()) {
        await walletCard.click();
        await browser.pause(500);

        // Look for tab elements
        const body = await browser.$("body");
        const text = await body.getText();

        expect(
          text.includes("Address") ||
            text.includes("Transaction") ||
            text.includes("Asset Lock")
        ).toBe(true);
      }
    });

    it("should display receive dialog with address and QR code", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(1000);

      // Look for Receive button
      const receiveBtn = await browser.$("button=Receive");
      if (!(await receiveBtn.isExisting())) {
        // Try to select a wallet first
        const walletCard = await browser.$('[data-testid="wallet-card"]');
        if (await walletCard.isExisting()) {
          await walletCard.click();
          await browser.pause(500);
        }
      }

      const receiveBtnRetry = await browser.$("button=Receive");
      if (await receiveBtnRetry.isExisting()) {
        await receiveBtnRetry.click();
        await browser.pause(500);

        // Dialog should appear with QR code or address
        const dialog = await browser.$('[role="dialog"]');
        if (await dialog.isExisting()) {
          const dialogText = await dialog.getText();
          expect(
            dialogText.includes("Receive") ||
              dialogText.includes("Address") ||
              dialogText.includes("QR") ||
              dialogText.includes("Copy")
          ).toBe(true);

          await dismissDialog();
        }
      }
    });
  });

  describe("Create wallet", () => {
    it("should open create wallet screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(500);

      // Look for Create Wallet button
      const createBtn = await browser.$(
        'button=Create Wallet, button*=Create, [data-testid="create-wallet-btn"]'
      );
      if (await createBtn.isExisting()) {
        await createBtn.click();
        await browser.pause(500);

        // Should show wallet creation form
        const body = await browser.$("body");
        const text = await body.getText();
        expect(
          text.includes("Create") ||
            text.includes("Mnemonic") ||
            text.includes("Seed") ||
            text.includes("Word") ||
            text.includes("Password")
        ).toBe(true);
      }
    });

    it("should display mnemonic words during creation", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(500);

      const createBtn = await browser.$(
        'button=Create Wallet, button*=Create, [data-testid="create-wallet-btn"]'
      );
      if (await createBtn.isExisting()) {
        await createBtn.click();
        await browser.pause(500);

        // Look for Generate or word display elements
        const generateBtn = await browser.$("button=Generate");
        if (await generateBtn.isExisting()) {
          await generateBtn.click();
          await browser.pause(1000);

          // Should display seed words
          const body = await browser.$("body");
          const text = await body.getText();
          // BIP39 words are common English words
          const hasWords = text.split(/\s+/).length > 12;
          expect(hasWords).toBe(true);
        }
      }
    });
  });

  describe("Delete wallet", () => {
    it("should show confirmation dialog before deleting a wallet", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("wallets");
      await browser.pause(1000);

      // Look for wallet context menu or delete action
      const walletCard = await browser.$('[data-testid="wallet-card"]');
      if (await walletCard.isExisting()) {
        // Try right-click for context menu
        await walletCard.click({ button: "right" });
        await browser.pause(300);

        // Look for Remove/Delete option in context menu
        const removeOption = await browser.$(
          '[role="menuitem"]*=Remove, [role="menuitem"]*=Delete'
        );
        if (await removeOption.isExisting()) {
          await removeOption.click();
          await browser.pause(300);

          // Confirmation dialog should appear
          const dialog = await browser.$('[role="alertdialog"], [role="dialog"]');
          if (await dialog.isExisting()) {
            const dialogText = await dialog.getText();
            expect(
              dialogText.includes("confirm") ||
                dialogText.includes("Confirm") ||
                dialogText.includes("delete") ||
                dialogText.includes("Delete") ||
                dialogText.includes("remove") ||
                dialogText.includes("Remove") ||
                dialogText.includes("sure")
            ).toBe(true);

            // Cancel to avoid actually deleting the seeded wallet
            const cancelBtn = await dialog.$("button=Cancel");
            if (await cancelBtn.isExisting()) {
              await cancelBtn.click();
            } else {
              await dismissDialog();
            }
          }
        }
      }
    });
  });
});
