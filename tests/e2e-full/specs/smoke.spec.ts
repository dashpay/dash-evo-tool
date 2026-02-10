/**
 * Smoke test — verifies WebdriverIO connects to tauri-driver,
 * the app launches, and basic DOM interaction works.
 */

import { waitForAppReady, navigateToSection } from "../helpers/tauri.js";

describe("Smoke Test", () => {
  it("should launch the Tauri app and render the UI", async () => {
    await waitForAppReady();

    // The app should have a title
    const title = await browser.getTitle();
    expect(title).toBe("Dash Evo Tool");
  });

  it("should have a visible document body", async () => {
    const body = await browser.$("body");
    expect(await body.isExisting()).toBe(true);

    // The body should have content (not empty)
    const html = await body.getHTML();
    expect(html.length).toBeGreaterThan(0);
  });

  it("should render the sidebar or welcome screen", async () => {
    const sidebar = await browser.$('[data-testid="sidebar"]');
    const welcome = await browser.$('[data-testid="welcome-screen"]');
    const networkChooser = await browser.$(
      '[data-testid="network-chooser-screen"]'
    );

    const hasUI =
      (await sidebar.isExisting()) ||
      (await welcome.isExisting()) ||
      (await networkChooser.isExisting());

    expect(hasUI).toBe(true);
  });

  it("should navigate between sections when sidebar is present", async () => {
    const sidebar = await browser.$('[data-testid="sidebar"]');
    if (!(await sidebar.isExisting())) {
      // Skip if app shows welcome/network chooser instead of sidebar
      console.log("  Skipping: sidebar not visible (app may need setup)");
      return;
    }

    // Navigate to wallets section
    await navigateToSection("wallets");

    // Verify we navigated (URL or content change)
    const url = await browser.getUrl();
    expect(url).toContain("wallet");
  });
});
