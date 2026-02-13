import { test, expect } from "@playwright/test";

test.describe("Token My Tokens Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tokens");
  });

  test("renders My Tokens heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "My Tokens" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders description text", async ({ page }) => {
    await expect(
      page.getByText(/Manage your token balances/),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Refresh button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /refresh/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Add Token by ID button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /add token by id/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Search Tokens button", async ({ page }) => {
    // Two "Search Tokens" buttons exist: sidebar nav + action bar. Use .first() for assertion.
    await expect(
      page.getByRole("button", { name: /search tokens/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Create Token button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /create token/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no tokens", async ({ page }) => {
    await expect(
      page.getByText("No tokens yet"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to Search Tokens when button clicked", async ({ page }) => {
    // Two "Search Tokens" buttons exist: sidebar nav + action bar. Use .first() for click.
    await page.getByRole("button", { name: /search tokens/i }).first().click();
    await expect(page).toHaveURL(/\/tokens\/search/);
  });

  test("navigates to Create Token when button clicked", async ({ page }) => {
    await page.getByRole("button", { name: /create token/i }).click();
    await expect(page).toHaveURL(/\/tokens\/creator/);
  });

  test("navigates to Add Token by ID when button clicked", async ({ page }) => {
    await page.getByRole("button", { name: /add token by id/i }).click();
    await expect(page).toHaveURL(/\/tokens\/add-by-id/);
  });
});
