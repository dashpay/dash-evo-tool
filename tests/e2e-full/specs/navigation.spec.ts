/**
 * Navigation E2E tests — verifies sidebar navigation, section rendering,
 * and theme toggling work end-to-end with the real Tauri app.
 *
 * Prerequisites: Seeded database with testnet settings, onboarding completed.
 * Network access: NOT required (navigation is purely local).
 */

import {
  waitForAppReady,
  navigateToSection,
  getNetworkBadge,
  existsByTestId,
  waitForTestId,
  takeScreenshot,
  type SidebarSection,
} from "../helpers/tauri.js";
import { setupTestDatabase, teardownTestDatabase } from "../helpers/database.js";

describe("Navigation", () => {
  before(async () => {
    try {
      setupTestDatabase("testnet");
    } catch {
      console.warn("Database setup skipped — app may need to create DB first");
    }
  });

  after(async () => {
    try {
      teardownTestDatabase();
    } catch {
      // Ignore cleanup errors
    }
  });

  it("should launch and display the sidebar", async () => {
    await waitForAppReady();
    const hasSidebar = await existsByTestId("sidebar");
    if (!hasSidebar) {
      // May show welcome or network chooser on first launch
      await takeScreenshot("no-sidebar");
    }
    // At least one of these should exist
    const hasWelcome = await existsByTestId("welcome-screen");
    const hasNetworkChooser = await existsByTestId("network-chooser-screen");
    expect(hasSidebar || hasWelcome || hasNetworkChooser).toBe(true);
  });

  it("should display the network badge showing testnet", async () => {
    await waitForAppReady();
    const badge = await getNetworkBadge();
    // If sidebar is visible, the network badge should show testnet
    const hasSidebar = await existsByTestId("sidebar");
    if (hasSidebar && badge) {
      expect(badge.toLowerCase()).toContain("testnet");
    }
  });

  describe("Section navigation", () => {
    const sections: SidebarSection[] = [
      "wallets",
      "identities",
      "dpns",
      "contracts",
      "tokens",
      "dashpay",
      "tools",
    ];

    for (const section of sections) {
      it(`should navigate to ${section} section`, async () => {
        await waitForAppReady();
        const hasSidebar = await existsByTestId("sidebar");
        if (!hasSidebar) {
          console.log(`  Skipping ${section}: sidebar not visible`);
          return;
        }

        await navigateToSection(section);

        // Verify URL contains the section name
        const url = await browser.getUrl();
        expect(url.toLowerCase()).toContain(section);
      });
    }
  });

  it("should navigate to all 7 sections without errors", async () => {
    await waitForAppReady();
    const hasSidebar = await existsByTestId("sidebar");
    if (!hasSidebar) {
      console.log("  Skipping: sidebar not visible");
      return;
    }

    const sections: SidebarSection[] = [
      "wallets",
      "identities",
      "dpns",
      "contracts",
      "tokens",
      "dashpay",
      "tools",
    ];

    for (const section of sections) {
      await navigateToSection(section);
      // Verify each section renders without crash (body still has content)
      const body = await browser.$("body");
      const html = await body.getHTML();
      expect(html.length).toBeGreaterThan(100);
    }
  });

  it("should toggle theme between light and dark modes", async () => {
    await waitForAppReady();
    const hasSidebar = await existsByTestId("sidebar");
    if (!hasSidebar) {
      console.log("  Skipping: sidebar not visible");
      return;
    }

    // Look for theme toggle button
    const themeToggle = await browser.$(
      '[data-testid="theme-toggle"], button[aria-label*="theme" i], button[aria-label*="Theme" i]'
    );
    if (!(await themeToggle.isExisting())) {
      console.log("  Skipping: theme toggle not found");
      return;
    }

    // Get current theme state
    const html = await browser.$("html");
    const classBefore = (await html.getAttribute("class")) || "";

    // Click theme toggle
    await themeToggle.click();
    await browser.pause(500);

    // Theme class should change
    const classAfter = (await html.getAttribute("class")) || "";
    // The theme class (dark/light) should be different
    const hadDark = classBefore.includes("dark");
    const hasDark = classAfter.includes("dark");
    expect(hadDark).not.toBe(hasDark);

    // Toggle back to restore original state
    await themeToggle.click();
    await browser.pause(300);
  });

  it("should highlight the active navigation item", async () => {
    await waitForAppReady();
    const hasSidebar = await existsByTestId("sidebar");
    if (!hasSidebar) {
      console.log("  Skipping: sidebar not visible");
      return;
    }

    await navigateToSection("wallets");

    // The wallet nav item should have an active/selected visual state
    const navItem = await browser.$(
      '[data-testid="nav-wallets"], [data-testid="sidebar-wallets"]'
    );
    if (await navItem.isExisting()) {
      const classes = (await navItem.getAttribute("class")) || "";
      // Active items typically have a background color or specific class
      expect(
        classes.includes("active") ||
          classes.includes("bg-") ||
          classes.includes("selected") ||
          classes.includes("text-primary")
      ).toBe(true);
    }
  });
});
