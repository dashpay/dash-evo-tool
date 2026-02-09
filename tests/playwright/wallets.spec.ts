import { test, expect } from "@playwright/test";

test.describe("Wallets Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/wallets");
  });

  test("renders wallet screen with empty state", async ({ page }) => {
    // In browser mode (no Tauri), wallet list shows empty state
    await expect(page.getByText("No Wallets Loaded")).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows Create Wallet button in empty state", async ({ page }) => {
    const createBtn = page.getByRole("button", { name: /Create Wallet/ });
    await expect(createBtn).toBeVisible({ timeout: 5000 });
  });

  test("shows Import Wallet button in empty state", async ({ page }) => {
    const importBtn = page.getByRole("button", { name: /Import Wallet/ });
    await expect(importBtn).toBeVisible({ timeout: 5000 });
  });

  test("shows 'Select a wallet' placeholder in detail panel", async ({
    page,
  }) => {
    // When no wallets and nothing selected, shows placeholder
    await expect(
      page.getByText("Select a wallet to view details"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("has split-pane layout with wallet list region", async ({ page }) => {
    const listRegion = page.getByRole("region", { name: "Wallet list" });
    await expect(listRegion).toBeVisible({ timeout: 5000 });
  });

  test("navigates to wallets from sidebar", async ({ page }) => {
    await page.goto("/identities");
    const walletsNav = page.getByRole("navigation").getByText("Wallets");
    await walletsNav.click();
    await expect(page).toHaveURL(/\/wallets/);
  });

  test("wallet list panel has proper accessibility role", async ({ page }) => {
    const region = page.getByRole("region", { name: "Wallet list" });
    await expect(region).toBeVisible({ timeout: 5000 });
    // The list region should contain the empty state within it
    await expect(region.getByText("No Wallets Loaded")).toBeVisible();
  });

  test("empty state has descriptive help text", async ({ page }) => {
    await expect(
      page.getByText("Create or import a wallet to get started."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("layout has two island panels side by side", async ({ page }) => {
    // The wallets screen renders two Island panels: left (list) and right (detail)
    await expect(page.getByRole("region", { name: "Wallet list" })).toBeVisible(
      { timeout: 5000 },
    );
    await expect(
      page.getByText("Select a wallet to view details"),
    ).toBeVisible();
  });
});
