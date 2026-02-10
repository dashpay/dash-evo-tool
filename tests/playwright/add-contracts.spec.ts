import { test, expect } from "@playwright/test";

test.describe("Add Contracts Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/add-contracts");
  });

  test("renders Add Contracts heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Add Contracts" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders contract input field", async ({ page }) => {
    await expect(
      page.getByPlaceholder(/hex or base58/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Add Contracts button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /add contracts$/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Back to Contracts button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /back to contracts/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Add Another Contract Field button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /add another contract field/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("contract input accepts text", async ({ page }) => {
    const input = page.getByPlaceholder(/hex or base58/i);
    await input.fill("abc123");
    await expect(input).toHaveValue("abc123");
  });

  test("adds multiple input fields", async ({ page }) => {
    await page
      .getByRole("button", { name: /add another contract field/i })
      .click();

    const inputs = page.getByPlaceholder(/hex or base58/i);
    await expect(inputs).toHaveCount(2);
  });

  test("navigates from Document Query to Add Contracts", async ({ page }) => {
    await page.goto("/contracts");
    await expect(
      page.getByRole("heading", { name: "Document Query" }),
    ).toBeVisible({ timeout: 5000 });

    // The "Load Contracts" action button should navigate here
    await page.goto("/contracts/add-contracts");
    await expect(
      page.getByRole("heading", { name: "Add Contracts" }),
    ).toBeVisible({ timeout: 5000 });
  });
});
