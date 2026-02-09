import { test, expect } from "@playwright/test";

test.describe("Welcome / Onboarding Screen", () => {
  test("navigating to /welcome shows the welcome screen", async ({ page }) => {
    await page.goto("/welcome");

    // Should show the welcome title
    await expect(
      page.getByRole("heading", { name: /welcome to dash evo tool/i }),
    ).toBeVisible();

    // Should show the subtitle
    await expect(
      page.getByText(/your gateway to decentralized data/i),
    ).toBeVisible();
  });

  test("welcome screen shows three action cards", async ({ page }) => {
    await page.goto("/welcome");

    await expect(
      page.getByRole("button", { name: /create wallet/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /import wallet/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /just explore/i }),
    ).toBeVisible();
  });

  test("welcome screen has no sidebar or top bar", async ({ page }) => {
    await page.goto("/welcome");
    await expect(
      page.getByRole("heading", { name: /welcome to dash evo tool/i }),
    ).toBeVisible();

    // Sidebar and top bar should not be visible
    await expect(page.getByTestId("sidebar")).not.toBeVisible();
    await expect(page.getByTestId("top-bar")).not.toBeVisible();
  });

  test("clicking Create Wallet navigates to wallets page", async ({
    page,
  }) => {
    await page.goto("/welcome");

    await page.getByRole("button", { name: /create wallet/i }).click();

    // Should navigate to wallets and show the app shell
    await expect(page).toHaveURL(/\/wallets/);
    await expect(page.getByTestId("sidebar")).toBeVisible();
  });

  test("clicking Import Wallet navigates to wallets page", async ({
    page,
  }) => {
    await page.goto("/welcome");

    await page.getByRole("button", { name: /import wallet/i }).click();

    await expect(page).toHaveURL(/\/wallets/);
    await expect(page.getByTestId("sidebar")).toBeVisible();
  });

  test("clicking Just Explore navigates to identities page", async ({
    page,
  }) => {
    await page.goto("/welcome");

    await page.getByRole("button", { name: /just explore/i }).click();

    await expect(page).toHaveURL(/\/identities/);
    await expect(page.getByTestId("sidebar")).toBeVisible();
  });

  test("index route redirects to /identities in browser mode (no backend)", async ({
    page,
  }) => {
    // In browser-only mode, settingsGet fails → falls through to /identities
    await page.goto("/");
    await expect(page).toHaveURL(/\/identities/);
    await expect(page.getByTestId("sidebar")).toBeVisible();
  });
});
