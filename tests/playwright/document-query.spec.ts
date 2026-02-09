import { test, expect } from "@playwright/test";

test.describe("Document Query Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts");
  });

  test("renders Document Query heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Document Query" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders contract tree panel", async ({ page }) => {
    await expect(
      page.getByTestId("contract-tree-panel"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders query input field", async ({ page }) => {
    await expect(
      page.getByTestId("query-input"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Fetch Documents button", async ({ page }) => {
    await expect(
      page.getByTestId("fetch-documents-btn"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders action buttons in toolbar", async ({ page }) => {
    await expect(
      page.getByTestId("action-load-contracts"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByTestId("action-register-contract"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByTestId("action-create-document"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no query has been run", async ({ page }) => {
    await expect(
      page.getByText("Query Documents", { exact: false }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("query input accepts text", async ({ page }) => {
    const input = page.getByTestId("query-input");
    await input.fill("SELECT * FROM domain");
    await expect(input).toHaveValue("SELECT * FROM domain");
  });

  test("navigates to document query from another route", async ({ page }) => {
    await page.goto("/wallets");
    await expect(page).toHaveURL(/\/wallets/);

    await page.goto("/contracts");
    await expect(
      page.getByRole("heading", { name: "Document Query" }),
    ).toBeVisible({ timeout: 5000 });
  });
});
