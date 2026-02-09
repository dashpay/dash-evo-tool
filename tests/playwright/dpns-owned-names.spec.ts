import { test, expect } from "@playwright/test";

test.describe("DPNS Owned Names Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/dpns-owned");
  });

  test("renders My Usernames heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "My Usernames" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows description text", async ({ page }) => {
    await expect(
      page.getByText(
        "DPNS names owned by your loaded identities.",
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Refresh button", async ({ page }) => {
    const refreshBtn = page.getByRole("button", {
      name: "Refresh owned names",
    });
    await expect(refreshBtn).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no owned names are loaded", async ({
    page,
  }) => {
    await expect(
      page.getByText("No owned usernames."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("empty state shows help text", async ({ page }) => {
    await expect(
      page.getByText(
        "Register a DPNS name to see it here. Names owned by your loaded identities will appear in this list.",
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to owned names from another route", async ({
    page,
  }) => {
    await page.goto("/wallets");
    await expect(page).toHaveURL(/\/wallets/);

    await page.goto("/contracts/dpns-owned");
    await expect(
      page.getByRole("heading", { name: "My Usernames" }),
    ).toBeVisible({ timeout: 5000 });
  });
});
