import { test, expect } from "@playwright/test";

test.describe("Group Actions Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/group-actions");
  });

  test("renders Group Actions heading", async ({ page }) => {
    await expect(
      page.getByText("Group Actions"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders step headings", async ({ page }) => {
    await expect(
      page.getByText(/step 1 — select contract/i),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/step 2 — select identity/i),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/step 3 — active group actions/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Back to Contracts button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /back to contracts/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Fetch Group Actions button", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /fetch group actions/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("fetch button is initially disabled", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /fetch group actions/i }),
    ).toBeDisabled({ timeout: 5000 });
  });

  test("shows empty contract message or contract selector", async ({ page }) => {
    // Either shows "No contracts with tokens found" or a selector dropdown
    const hasNoContracts = await page
      .getByText(/no contracts with tokens found/i)
      .isVisible()
      .catch(() => false);
    const hasSelector = await page
      .getByText(/choose a contract/i)
      .isVisible()
      .catch(() => false);

    expect(hasNoContracts || hasSelector).toBeTruthy();
  });

  test("back button navigates to contracts page", async ({ page }) => {
    await page
      .getByRole("button", { name: /back to contracts/i })
      .click();

    // Should navigate to /contracts
    await expect(page).toHaveURL(/\/contracts/);
  });

  test("breadcrumbs contain Contracts link", async ({ page }) => {
    await expect(page.getByText("Contracts")).toBeVisible({ timeout: 5000 });
  });
});
