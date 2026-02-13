import { test, expect } from "@playwright/test";

test.describe("DPNS Past Contests Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/dpns/past");
  });

  test("renders Past Contests heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Past Contests" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows description text", async ({ page }) => {
    await expect(
      page.getByText(
        "Historical DPNS name contests that have ended.",
      ),
    ).toBeVisible({ timeout: 5000 });
  });

  test("renders Refresh button", async ({ page }) => {
    const refreshBtn = page.getByRole("button", {
      name: "Refresh contests",
    });
    await expect(refreshBtn).toBeVisible({ timeout: 5000 });
  });

  test("shows empty state when no past contests are loaded", async ({
    page,
  }) => {
    await expect(
      page.getByText("No past contests yet."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("empty state shows help text", async ({ page }) => {
    await expect(
      page.getByText("Contests will appear here once they have ended."),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to past contests from another route", async ({
    page,
  }) => {
    await page.goto("/wallets");
    await expect(page).toHaveURL(/\/wallets/);

    await page.goto("/contracts/dpns/past");
    await expect(
      page.getByRole("heading", { name: "Past Contests" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("does not render vote or register buttons", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /cast or schedule/i }),
    ).not.toBeVisible();
    await expect(
      page.getByRole("button", { name: /register/i }),
    ).not.toBeVisible();
  });
});
