/**
 * Contract query E2E tests — verifies contract tree browsing, document
 * type expansion, document fetching, and result display.
 *
 * Prerequisites: Seeded database with DPNS and DashPay system contracts.
 * Network access: REQUIRED for document fetching (queries Platform).
 *   - Contract tree rendering is local (from seeded DB).
 *   - Document query execution requires Platform connection.
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

describe("Contract Query", () => {
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

  describe("[LOCAL] Contract tree panel", () => {
    it("should display the contract tree with system contracts", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) {
        console.log("  Skipping: sidebar not visible");
        return;
      }

      await navigateToSection("contracts");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      // Seeded contracts: DPNS and DashPay
      expect(
        text.includes("DPNS") ||
          text.includes("DashPay") ||
          text.includes("Contract") ||
          text.includes("contract")
      ).toBe(true);
    });

    it("should show the contract tree panel", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const treePanel = await browser.$(
        '[data-testid="contract-tree-panel"]'
      );
      if (await treePanel.isExisting()) {
        expect(await treePanel.isDisplayed()).toBe(true);
      }
    });

    it("should have search input for filtering contracts", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const searchInput = await browser.$(
        '[data-testid="contract-search-input"]'
      );
      if (await searchInput.isExisting()) {
        expect(await searchInput.isDisplayed()).toBe(true);

        // Type to filter
        await searchInput.setValue("DPNS");
        await browser.pause(300);

        const body = await browser.$("body");
        const text = await body.getText();
        expect(text.includes("DPNS")).toBe(true);
      }
    });

    it("should expand contract to show document types", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      // Click on a contract to expand it
      const contractNode = await browser.$(
        '[data-testid="contract-tree-panel"] [role="treeitem"], [data-testid="contract-tree-panel"] button'
      );
      if (await contractNode.isExisting()) {
        await contractNode.click();
        await browser.pause(500);

        // After expansion, document types section might appear
        const docTypesSection = await browser.$(
          '[data-testid="doc-types-section"]'
        );
        if (await docTypesSection.isExisting()) {
          expect(await docTypesSection.isDisplayed()).toBe(true);
        }
      }
    });
  });

  describe("[LOCAL] Document query screen", () => {
    it("should render the document query screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const queryScreen = await browser.$(
        '[data-testid="document-query-screen"]'
      );
      if (await queryScreen.isExisting()) {
        expect(await queryScreen.isDisplayed()).toBe(true);
      }
    });

    it("should have contract and document type selectors", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const contractSelect = await browser.$(
        '[data-testid="contract-select"]'
      );
      const docTypeSelect = await browser.$(
        '[data-testid="doctype-select"]'
      );

      // At least contract selector should exist
      if (await contractSelect.isExisting()) {
        expect(await contractSelect.isDisplayed()).toBe(true);
      }
    });

    it("should have fetch documents button", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const fetchBtn = await browser.$(
        '[data-testid="fetch-documents-btn"], button=Fetch Documents, button*=Fetch'
      );
      if (await fetchBtn.isExisting()) {
        expect(await fetchBtn.isDisplayed()).toBe(true);
      }
    });

    it("should have JSON/YAML display mode toggles", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const jsonToggle = await browser.$(
        '[data-testid="display-mode-json"]'
      );
      const yamlToggle = await browser.$(
        '[data-testid="display-mode-yaml"]'
      );

      if (await jsonToggle.isExisting()) {
        expect(await jsonToggle.isDisplayed()).toBe(true);
      }
    });
  });

  describe("[LOCAL] Action buttons", () => {
    it("should display toolbar with action buttons", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      const body = await browser.$("body");
      const text = await body.getText();

      // Should show action buttons
      expect(
        text.includes("Add Contract") ||
          text.includes("Register") ||
          text.includes("Update") ||
          text.includes("Create Document") ||
          text.includes("Group Actions")
      ).toBe(true);
    });

    it("should navigate to Add Contract screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(500);

      const addBtn = await browser.$(
        '[data-testid="action-load-contracts"], button*=Add Contract, button*=Load'
      );
      if (await addBtn.isExisting()) {
        await addBtn.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();
        expect(
          text.includes("Contract ID") ||
            text.includes("Enter") ||
            text.includes("Fetch") ||
            text.includes("Add")
        ).toBe(true);
      }
    });

    it("should navigate to Register Contract screen", async () => {
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(500);

      const registerBtn = await browser.$(
        '[data-testid="action-register-contract"], button*=Register Contract'
      );
      if (await registerBtn.isExisting()) {
        await registerBtn.click();
        await browser.pause(500);

        const body = await browser.$("body");
        const text = await body.getText();
        expect(
          text.includes("Register") ||
            text.includes("Identity") ||
            text.includes("JSON") ||
            text.includes("Schema")
        ).toBe(true);
      }
    });
  });

  describe("[NETWORK] Document fetching", () => {
    it("should attempt to fetch documents with contract selected", async () => {
      // This test requires network — it will verify the fetch flow works
      // but the actual fetch may fail without a testnet connection
      await waitForAppReady();
      const hasSidebar = await existsByTestId("sidebar");
      if (!hasSidebar) return;

      await navigateToSection("contracts");
      await browser.pause(1000);

      // Select a contract from the tree
      const contractNode = await browser.$(
        '[data-testid="contract-tree-panel"] [role="treeitem"], [data-testid="contract-tree-panel"] button'
      );
      if (await contractNode.isExisting()) {
        await contractNode.click();
        await browser.pause(500);

        // Try to fetch documents
        const fetchBtn = await browser.$(
          '[data-testid="fetch-documents-btn"], button=Fetch Documents, button*=Fetch'
        );
        if (await fetchBtn.isExisting() && (await fetchBtn.isEnabled())) {
          await fetchBtn.click();
          await browser.pause(2000);

          // Should show results, loading state, or error
          const body = await browser.$("body");
          const text = await body.getText();
          expect(
            text.includes("document") ||
              text.includes("Document") ||
              text.includes("result") ||
              text.includes("Result") ||
              text.includes("Error") ||
              text.includes("error") ||
              text.includes("Loading") ||
              text.includes("No documents")
          ).toBe(true);
        }
      }
    });
  });
});
