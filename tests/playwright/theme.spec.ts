import { test, expect } from "@playwright/test";

test.describe("Theme Switching", () => {
  test("starts with a theme class applied to html element", async ({
    page,
  }) => {
    await page.goto("/");

    // The html element should have either "light" or "dark" class
    const htmlClassList = await page.evaluate(() =>
      Array.from(document.documentElement.classList),
    );
    const hasThemeClass =
      htmlClassList.includes("light") || htmlClassList.includes("dark");
    expect(hasThemeClass).toBe(true);
  });

  test("can toggle between light and dark mode via theme dropdown", async ({
    page,
  }) => {
    await page.goto("/");

    // Open the theme toggle dropdown
    const themeButton = page.getByRole("button", { name: /toggle theme/i });
    await expect(themeButton).toBeVisible();
    await themeButton.click();

    // Select "Dark" from the dropdown
    const darkOption = page.getByRole("menuitem", { name: /dark/i });
    await expect(darkOption).toBeVisible();
    await darkOption.click();

    // Verify the html element has the "dark" class
    await expect(page.locator("html")).toHaveClass(/dark/);

    // Now switch to light mode
    await themeButton.click();
    const lightOption = page.getByRole("menuitem", { name: /light/i });
    await expect(lightOption).toBeVisible();
    await lightOption.click();

    // Verify the html element has the "light" class
    await expect(page.locator("html")).toHaveClass(/light/);
  });

  test("dark mode changes background color", async ({ page }) => {
    await page.goto("/");

    // Switch to light mode
    const themeButton = page.getByRole("button", { name: /toggle theme/i });
    await themeButton.click();
    await page.getByRole("menuitem", { name: /light/i }).click();

    const lightBg = await page.evaluate(() =>
      getComputedStyle(document.body).backgroundColor,
    );

    // Switch to dark mode
    await themeButton.click();
    await page.getByRole("menuitem", { name: /dark/i }).click();

    const darkBg = await page.evaluate(() =>
      getComputedStyle(document.body).backgroundColor,
    );

    // Background should be different between modes
    expect(lightBg).not.toBe(darkBg);
  });

  test("can navigate to design system showcase", async ({ page }) => {
    await page.goto("/");

    // Click the design system link
    const dsLink = page.getByTestId("design-system-link");
    await expect(dsLink).toBeVisible();
    await dsLink.click();

    // Verify design system page renders
    await expect(page.getByTestId("design-system")).toBeVisible();
    await expect(
      page.getByRole("heading", { name: /design system/i }),
    ).toBeVisible();

    // Verify sections render
    await expect(page.getByText("Buttons")).toBeVisible();
    await expect(page.getByText("Badges")).toBeVisible();
    await expect(page.getByText("Typography")).toBeVisible();
  });

  test("design system showcase works in dark mode", async ({ page }) => {
    await page.goto("/");

    // Switch to dark mode first
    const themeButton = page.getByRole("button", { name: /toggle theme/i });
    await themeButton.click();
    await page.getByRole("menuitem", { name: /dark/i }).click();
    await expect(page.locator("html")).toHaveClass(/dark/);

    // Navigate to design system
    await page.getByTestId("design-system-link").click();
    await expect(page.getByTestId("design-system")).toBeVisible();

    // Verify dark mode is still active
    await expect(page.locator("html")).toHaveClass(/dark/);

    // The theme indicator should show dark
    await expect(page.getByText(/current theme: dark/i)).toBeVisible();
  });
});
