import { test, expect } from "@playwright/test";

test.describe("DPNS Scheduled Votes Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/dpns-scheduled");
  });

  test("renders Scheduled Votes heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Scheduled Votes" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows description text with running warning", async ({ page }) => {
    await expect(
      page.getByText(
        "Votes scheduled for future execution. Dash Evo Tool must remain running for votes to execute on time.",
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Clear Casted button (disabled when no completed votes)", async ({
    page,
  }) => {
    const clearCastedBtn = page.getByRole("button", {
      name: "Clear casted votes",
    });
    await expect(clearCastedBtn).toBeVisible({ timeout: 5000 });
    await expect(clearCastedBtn).toBeDisabled();
  });

  test("renders Clear All button (disabled when no votes)", async ({
    page,
  }) => {
    const clearAllBtn = page.getByRole("button", {
      name: "Clear all scheduled votes",
    });
    await expect(clearAllBtn).toBeVisible({ timeout: 5000 });
    await expect(clearAllBtn).toBeDisabled();
  });

  test("renders Refresh button", async ({ page }) => {
    const refreshBtn = page.getByRole("button", {
      name: "Refresh scheduled votes",
    });
    await expect(refreshBtn).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no scheduled votes exist", async ({
    page,
  }) => {
    await expect(
      page.getByText("No scheduled votes."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("empty state shows scheduling instructions", async ({ page }) => {
    await expect(
      page.getByText(/Schedule votes from the Active Contests tab/),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to scheduled votes from another route", async ({
    page,
  }) => {
    await page.goto("/wallets");
    await expect(page).toHaveURL(/\/wallets/);

    await page.goto("/contracts/dpns-scheduled");
    await expect(
      page.getByRole("heading", { name: "Scheduled Votes" }),
    ).toBeVisible({ timeout: 5000 });
  });
});
