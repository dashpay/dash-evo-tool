import { test, expect } from "@playwright/test";

test.describe("App", () => {
  test("loads the app with sidebar and top bar", async ({ page }) => {
    await page.goto("/");
    // App should redirect to /identities and show the shell
    await expect(page.getByTestId("sidebar")).toBeVisible();
    await expect(page.getByTestId("top-bar")).toBeVisible();
    await expect(page).toHaveURL(/\/identities/);
  });
});
