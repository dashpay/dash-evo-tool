import { test, expect } from "@playwright/test";

test.describe("DPNS Active Contests Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/dpns-active");
  });

  test("renders Active Contests heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Active Contests" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows description text", async ({ page }) => {
    await expect(
      page.getByText(
        "Vote on contested DPNS names using your voting identities.",
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Refresh button", async ({ page }) => {
    const refreshBtn = page.getByRole("button", { name: "Refresh contests" });
    await expect(refreshBtn).toBeVisible({ timeout: 5000 });
  });

  test("renders Register Name button", async ({ page }) => {
    const registerBtn = page.getByRole("button", {
      name: "Register DPNS name",
    });
    await expect(registerBtn).toBeVisible({ timeout: 5000 });
  });

  test("renders Cast / Schedule Votes button (disabled with no selections)", async ({
    page,
  }) => {
    const voteBtn = page.getByRole("button", {
      name: "Cast or schedule votes",
    });
    await expect(voteBtn).toBeVisible({ timeout: 5000 });
    await expect(voteBtn).toBeDisabled();
  });

  test("shows empty state when no contests are loaded", async ({ page }) => {
    // In browser mode (no Tauri backend), no contests can load
    await expect(
      page.getByText("No active contests at the moment."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("empty state shows help text", async ({ page }) => {
    await expect(
      page.getByText("Please check back later or try refreshing the list."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to DPNS active contests from contracts section", async ({
    page,
  }) => {
    // Navigate away first
    await page.goto("/wallets");
    await expect(page).toHaveURL(/\/wallets/);

    // Navigate back to DPNS active contests
    await page.goto("/contracts/dpns-active");
    await expect(
      page.getByRole("heading", { name: "Active Contests" }),
    ).toBeVisible({ timeout: 5000 });
  });
});
