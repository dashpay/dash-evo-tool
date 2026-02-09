import { test, expect } from "@playwright/test";

test.describe("Network Chooser / Settings Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    // Wait for the app shell to load
    await page.waitForSelector("[data-testid='sidebar']");
    // Navigate to settings
    await page.getByTestId("nav-settings").click();
    await expect(page).toHaveURL(/\/settings/);
  });

  test("renders the network chooser screen", async ({ page }) => {
    await expect(
      page.getByTestId("network-chooser-screen"),
    ).toBeVisible();
  });

  test("shows Connection Settings heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Connection Settings" }),
    ).toBeVisible();
  });

  test("shows Connection Status heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Connection Status" }),
    ).toBeVisible();
  });

  test("shows network selector", async ({ page }) => {
    await expect(
      page.getByTestId("network-select-trigger"),
    ).toBeVisible();
  });

  test("shows Advanced Settings toggle", async ({ page }) => {
    const toggle = page.getByTestId("advanced-settings-toggle");
    await expect(toggle).toBeVisible();
    // Initially collapsed
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  test("clicking Advanced Settings toggle expands the section", async ({
    page,
  }) => {
    const toggle = page.getByTestId("advanced-settings-toggle");
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");

    // Theme selector should be visible
    await expect(
      page.getByTestId("theme-select-trigger"),
    ).toBeVisible();

    // Dash-Qt path input should be visible
    await expect(
      page.getByTestId("dashqt-path-input"),
    ).toBeVisible();

    // Configuration checkboxes visible
    await expect(page.getByTestId("overwrite-dash-conf")).toBeVisible();
    await expect(page.getByTestId("disable-zmq")).toBeVisible();
    await expect(page.getByTestId("developer-mode")).toBeVisible();
    await expect(page.getByTestId("close-dash-qt")).toBeVisible();
  });

  test("shows database maintenance button in Advanced Settings", async ({
    page,
  }) => {
    await page.getByTestId("advanced-settings-toggle").click();
    await expect(
      page.getByTestId("clear-database-button"),
    ).toBeVisible();
  });

  test("shows network badge at bottom", async ({ page }) => {
    await expect(page.getByTestId("network-badge")).toBeVisible();
  });
});
