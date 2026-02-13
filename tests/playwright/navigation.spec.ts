import { test, expect } from "@playwright/test";

test.describe("App Shell Navigation", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    // Wait for the app to load (redirects to /identities by default)
    await page.waitForSelector("[data-testid='sidebar']");
  });

  test("sidebar shows all 7 navigation items", async ({ page }) => {
    const navIds = [
      "dashpay",
      "identities",
      "contracts",
      "tokens",
      "wallets",
      "tools",
      "settings",
    ];
    for (const id of navIds) {
      await expect(page.getByTestId(`nav-${id}`)).toBeVisible();
    }
  });

  test("clicking sidebar items navigates to correct routes", async ({
    page,
  }) => {
    // Click on Wallets
    await page.getByTestId("nav-wallets").click();
    await expect(page).toHaveURL(/\/wallets/);
    // Wallets should be highlighted (has aria-current="page")
    await expect(page.getByTestId("nav-wallets")).toHaveAttribute(
      "aria-current",
      "page",
    );

    // Click on Tokens
    await page.getByTestId("nav-tokens").click();
    await expect(page).toHaveURL(/\/tokens/);
    await expect(page.getByTestId("nav-tokens")).toHaveAttribute(
      "aria-current",
      "page",
    );
    // Wallets should no longer be active
    await expect(page.getByTestId("nav-wallets")).not.toHaveAttribute(
      "aria-current",
    );

    // Click on DashPay
    await page.getByTestId("nav-dashpay").click();
    await expect(page).toHaveURL(/\/dashpay/);

    // Click on Settings
    await page.getByTestId("nav-settings").click();
    await expect(page).toHaveURL(/\/settings/);
  });

  test("top bar is visible with connection indicator", async ({ page }) => {
    await expect(page.getByTestId("top-bar")).toBeVisible();
    await expect(page.getByTestId("connection-indicator")).toBeVisible();
  });

  test("top bar shows breadcrumbs for current section", async ({ page }) => {
    // Navigate to identities
    await page.getByTestId("nav-identities").click();
    // Should show "Identities" in breadcrumb area
    const breadcrumb = page.getByRole("navigation", { name: "Breadcrumb" });
    await expect(breadcrumb).toBeVisible();
    await expect(breadcrumb).toContainText("Identities");
  });

  test("default route redirects to /identities", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveURL(/\/identities/);
  });

  test("sidebar collapse toggle works", async ({ page }) => {
    const sidebar = page.getByTestId("sidebar");
    const toggle = page.getByTestId("sidebar-collapse-toggle");

    // Initially expanded (200px)
    await expect(sidebar).toHaveClass(/w-\[200px\]/);

    // Click collapse
    await toggle.click();
    await expect(sidebar).toHaveClass(/w-\[72px\]/);

    // Click expand
    await toggle.click();
    await expect(sidebar).toHaveClass(/w-\[200px\]/);
  });

  test("placeholder screens render for all sections", async ({ page }) => {
    const sections = [
      { nav: "dashpay", title: "DashPay" },
      { nav: "identities", title: "Identities" },
      { nav: "contracts", title: "Document Query" },
      { nav: "tokens", title: "My Tokens" },
      { nav: "wallets", title: "Wallets" },
      { nav: "tools", title: "Tools" },
      { nav: "settings", title: "Connection Settings" },
    ];

    for (const section of sections) {
      await page.getByTestId(`nav-${section.nav}`).click();
      await expect(
        page.getByRole("heading", { name: section.title }),
      ).toBeVisible();
    }
  });
});
