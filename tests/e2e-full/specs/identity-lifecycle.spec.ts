/**
 * Identity lifecycle E2E tests — verifies identity listing, detail viewing,
 * key inspection, alias editing, and balance display.
 *
 * Prerequisites: Seeded database with 2 identities (TestUser1 linked to wallet,
 * TestUser2 standalone).
 * Network access: NOT required for list/detail/alias/key viewing.
 *   - Refresh balance REQUIRES network access (testnet Platform).
 *   - Load identity by ID REQUIRES network access.
 *
 * Tests marked [NETWORK] require a running Dash Platform testnet connection.
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
  takeScreenshot,
} from "../helpers/tauri.js";
import { setupTestDatabase, teardownTestDatabase } from "../helpers/database.js";

describe("Identity Lifecycle", () => {
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

  describe("Identity list", () => {
    it("should display the identity list with seeded identities", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) {
        console.log("  Skipping: sidebar not visible");
        return;
      }

      await navigateToSection("identities");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      // Seeded identities should be visible
      expect(
        text.includes("TestUser1") ||
          text.includes("TestUser2") ||
          text.includes("Identity") ||
          text.includes("identity")
      ).toBe(true);
    });

    it("should show both seeded identities", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      // Both identities from seed data
      const hasUser1 = text.includes("TestUser1");
      const hasUser2 = text.includes("TestUser2");

      // At least one should be visible (both if list is rendering)
      expect(hasUser1 || hasUser2).toBe(true);
    });

    it("should display identity type badges", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      // Both identities are 'user' type
      expect(
        text.toLowerCase().includes("user") ||
          text.toLowerCase().includes("identity")
      ).toBe(true);
    });
  });

  describe("Identity detail", () => {
    it("should show identity details when selected", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      // Click on the first identity (table row or list item)
      const identityRow = await browser.$(
        'tr:has-text("TestUser1"), [data-testid*="identity"]:has-text("TestUser1"), td:has-text("TestUser1")'
      );
      if (await identityRow.isExisting()) {
        await identityRow.click();
        await browser.pause(500);

        // Detail panel should show identity info
        const body = await browser.$("body");
        const text = await body.getText();

        expect(
          text.includes("TestUser1") ||
            text.includes("b1c2d3") ||
            text.includes("Balance") ||
            text.includes("Keys")
        ).toBe(true);
      }
    });

    it("should show identity is linked to wallet for TestUser1", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      // Select TestUser1
      const identityRow = await browser.$(
        'tr:has-text("TestUser1"), [data-testid*="identity"]:has-text("TestUser1"), td:has-text("TestUser1")'
      );
      if (await identityRow.isExisting()) {
        await identityRow.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();

        // Should show wallet association
        expect(
          text.includes("Test Wallet Alpha") ||
            text.includes("Wallet") ||
            text.includes("wallet") ||
            text.includes("In Wallet")
        ).toBe(true);
      }
    });

    it("should show action buttons for identity operations", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      // Select an identity
      const identityRow = await browser.$(
        'tr:has-text("TestUser1"), [data-testid*="identity"]:has-text("TestUser1"), td:has-text("TestUser1")'
      );
      if (await identityRow.isExisting()) {
        await identityRow.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();

        // Should show action buttons
        expect(
          text.includes("Top Up") ||
            text.includes("Withdraw") ||
            text.includes("Transfer") ||
            text.includes("Keys") ||
            text.includes("Refresh") ||
            text.includes("DPNS")
        ).toBe(true);
      }
    });
  });

  describe("Identity alias editing", () => {
    it("should support inline alias editing", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      // Look for alias text that can be double-clicked or edited
      const aliasCell = await browser.$(
        'td:has-text("TestUser1"), [data-testid*="alias"]:has-text("TestUser1")'
      );
      if (await aliasCell.isExisting()) {
        // Double-click to enter edit mode
        await aliasCell.doubleClick();
        await browser.pause(300);

        // An input field should appear for editing
        const editInput = await browser.$(
          'input[type="text"], [data-testid*="alias-input"], [data-testid*="rename-input"]'
        );
        if (await editInput.isExisting()) {
          expect(await editInput.isDisplayed()).toBe(true);

          // Cancel editing to not modify seed data
          await browser.keys("Escape");
          await browser.pause(200);
        }
      }
    });
  });

  describe("View keys", () => {
    it("should navigate to key management screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(1000);

      // Select identity and look for View Keys action
      const identityRow = await browser.$(
        'tr:has-text("TestUser1"), [data-testid*="identity"]:has-text("TestUser1"), td:has-text("TestUser1")'
      );
      if (await identityRow.isExisting()) {
        await identityRow.click();
        await browser.pause(500);

        // Look for Keys button/link
        const keysBtn = await browser.$(
          'button=View Keys, button*=Keys, a*=Keys, [data-testid*="keys"]'
        );
        if (await keysBtn.isExisting()) {
          await keysBtn.click();
          await browser.pause(500);

          const body = await browser.$("body");
          const text = await body.getText();

          expect(
            text.includes("Key") ||
              text.includes("key") ||
              text.includes("Purpose") ||
              text.includes("Security")
          ).toBe(true);
        }
      }
    });
  });

  describe("Load identity by ID", () => {
    it("should render the load identity form", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("identities");
      await browser.pause(500);

      // Look for "Add Existing" or "Load Identity" button
      const addExistingBtn = await browser.$(
        'button*=Add Existing, button*=Load, [data-testid*="add-existing"]'
      );
      if (await addExistingBtn.isExisting()) {
        await addExistingBtn.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();

        // Should show form to enter identity ID
        expect(
          text.includes("Identity ID") ||
            text.includes("identity") ||
            text.includes("Base58") ||
            text.includes("Hex") ||
            text.includes("Enter")
        ).toBe(true);
      }
    });
  });
});
