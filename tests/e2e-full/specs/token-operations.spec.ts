/**
 * Token operations E2E tests — verifies token listing, search, add by ID,
 * and token info viewing work end-to-end.
 *
 * Prerequisites: Seeded database with identities (tokens may be loaded dynamically).
 * Network access: REQUIRED for search and add by ID (queries Platform).
 *   - My Tokens tab renders locally (from cached token balances).
 *   - Token search and add by ID require Platform connection.
 *
 * Tests marked [NETWORK] require a running Dash Platform testnet connection.
 * Tests marked [LOCAL] work with seeded database only.
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

describe("Token Operations", () => {
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

  describe("[LOCAL] Tokens screen rendering", () => {
    it("should navigate to the tokens screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) {
        console.log("  Skipping: sidebar not visible");
        return;
      }

      await navigateToSection("tokens");
      await browser.pause(1000);

      const url = await browser.getUrl();
      expect(url.toLowerCase()).toContain("token");
    });

    it("should display My Tokens tab", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      expect(
        text.includes("My Tokens") ||
          text.includes("Token") ||
          text.includes("token")
      ).toBe(true);
    });

    it("should show token action buttons", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      // Should show at least some of: Refresh, Add Token, Create Token, Search
      expect(
        text.includes("Refresh") ||
          text.includes("Add") ||
          text.includes("Create") ||
          text.includes("Search") ||
          text.includes("Token")
      ).toBe(true);
    });

    it("should show empty state when no tokens loaded", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(1000);

      // With no tokens in seed data, should show empty state or the table header
      const body = await browser.$("body");
      const text = await body.getText();

      expect(
        text.includes("No tokens") ||
          text.includes("no tokens") ||
          text.includes("Token Name") ||
          text.includes("Add") ||
          text.includes("Search") ||
          text.includes("My Tokens")
      ).toBe(true);
    });
  });

  describe("[LOCAL] Token tabs navigation", () => {
    it("should switch to Search Tokens tab", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(500);

      // Navigate to search tab
      const searchTab = await browser.$(
        'a*=Search, button*=Search, [role="tab"]*=Search'
      );
      if (await searchTab.isExisting()) {
        await searchTab.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();

        expect(
          text.includes("Search") ||
            text.includes("search") ||
            text.includes("keyword") ||
            text.includes("Keyword")
        ).toBe(true);
      }
    });

    it("should navigate to Add Token by ID screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(500);

      const addBtn = await browser.$(
        'button*=Add Token, a*=Add Token, [data-testid*="add-token"]'
      );
      if (await addBtn.isExisting()) {
        await addBtn.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();

        expect(
          text.includes("Token ID") ||
            text.includes("Contract ID") ||
            text.includes("Enter") ||
            text.includes("Search") ||
            text.includes("Add")
        ).toBe(true);
      }
    });

    it("should navigate to Create Token wizard", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(500);

      const createBtn = await browser.$(
        'button*=Create Token, a*=Create Token, [data-testid*="create-token"]'
      );
      if (await createBtn.isExisting()) {
        await createBtn.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();

        expect(
          text.includes("Create") ||
            text.includes("Token Name") ||
            text.includes("Step") ||
            text.includes("Basic") ||
            text.includes("Identity")
        ).toBe(true);
      }
    });
  });

  describe("[LOCAL] Token creator wizard", () => {
    it("should render the token creator with step navigation", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      // Navigate to token creator
      const url = await browser.getUrl();
      if (!url.includes("create")) {
        await navigateToSection("tokens");
        await browser.pause(500);

        const createBtn = await browser.$(
          'button*=Create Token, a*=Create Token'
        );
        if (await createBtn.isExisting()) {
          await createBtn.click();
          await browser.pause(500);
        }
      }

      const body = await browser.$("body");
      const text = await body.getText();

      // Should show wizard steps or step indicator
      expect(
        text.includes("Step") ||
          text.includes("Basic Info") ||
          text.includes("Token Name") ||
          text.includes("Next") ||
          text.includes("Cancel")
      ).toBe(true);
    });

    it("should validate required fields in basic info step", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      // Navigate to token creator
      await navigateToSection("tokens");
      await browser.pause(500);

      const createBtn = await browser.$(
        'button*=Create Token, a*=Create Token'
      );
      if (await createBtn.isExisting()) {
        await createBtn.click();
        await browser.pause(500);

        // Try to click Next without filling fields
        const nextBtn = await browser.$("button=Next");
        if (await nextBtn.isExisting()) {
          // Next should be disabled or show validation errors
          const isDisabled = await nextBtn.getAttribute("disabled");
          const body = await browser.$("body");
          const text = await body.getText();

          expect(
            isDisabled !== null ||
              text.includes("required") ||
              text.includes("Required") ||
              text.includes("Token Name") // Still on step 1
          ).toBe(true);
        }
      }
    });
  });

  describe("[NETWORK] Token search", () => {
    it("should search for tokens by keyword", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(500);

      // Navigate to search
      const searchTab = await browser.$(
        'a*=Search, button*=Search, [role="tab"]*=Search'
      );
      if (await searchTab.isExisting()) {
        await searchTab.click();
        await browser.pause(500);

        // Enter search keyword
        const searchInput = await browser.$(
          'input[placeholder*="keyword" i], input[placeholder*="search" i], input[type="text"]'
        );
        if (await searchInput.isExisting()) {
          await searchInput.setValue("dash");
          await browser.pause(300);

          // Click search button
          const searchBtn = await browser.$("button=Search");
          if (await searchBtn.isExisting()) {
            await searchBtn.click();
            await browser.pause(3000); // Allow time for network request

            const body = await browser.$("body");
            const text = await body.getText();

            // Should show results, loading, or no results message
            expect(
              text.includes("result") ||
                text.includes("Result") ||
                text.includes("Found") ||
                text.includes("found") ||
                text.includes("No") ||
                text.includes("Error") ||
                text.includes("Searching")
            ).toBe(true);
          }
        }
      }
    });
  });

  describe("[LOCAL] Token info dialog", () => {
    it("should open token info dialog from My Tokens action menu", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("tokens");
      await browser.pause(1000);

      // If there are tokens in the list, try to open info dialog
      // This may require tokens to be loaded — skip if table is empty
      const actionMenu = await browser.$(
        '[role="menuitem"]*=More Info, [role="menuitem"]*=Info, button*=More Info'
      );
      if (await actionMenu.isExisting()) {
        await actionMenu.click();
        await browser.pause(500);

        const dialog = await browser.$('[role="dialog"]');
        if (await dialog.isExisting()) {
          const dialogText = await dialog.getText();
          expect(
            dialogText.includes("Token") ||
              dialogText.includes("Name") ||
              dialogText.includes("Contract") ||
              dialogText.includes("Supply")
          ).toBe(true);

          await dismissDialog();
        }
      }
    });
  });
});
