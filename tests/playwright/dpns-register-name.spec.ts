import { test, expect } from "@playwright/test";

test.describe("DPNS Register Name Screen", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/contracts/dpns-register");
  });

  test("renders Register DPNS Name heading", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: "Register DPNS Name" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows DPNS breadcrumb", async ({ page }) => {
    await expect(page.getByText("DPNS")).toBeVisible({ timeout: 5000 });
    await expect(page.getByText("Register Name")).toBeVisible({ timeout: 5000 });
  });

  test("renders name input field", async ({ page }) => {
    await expect(page.getByTestId("name-input")).toBeVisible({ timeout: 5000 });
  });

  test("renders .dash suffix", async ({ page }) => {
    await expect(page.getByText(".dash")).toBeVisible({ timeout: 5000 });
  });

  test("renders register button (initially disabled)", async ({ page }) => {
    const btn = page.getByTestId("register-btn");
    await expect(btn).toBeVisible({ timeout: 5000 });
    await expect(btn).toBeDisabled();
  });

  test("shows back button that navigates to active contests", async ({
    page,
  }) => {
    const backBtn = page.getByTestId("back-btn");
    await expect(backBtn).toBeVisible({ timeout: 5000 });
    await backBtn.click();
    await expect(page).toHaveURL(/\/contracts\/dpns-active/);
  });

  test("shows validation feedback when typing a valid name", async ({
    page,
  }) => {
    const input = page.getByTestId("name-input");
    await input.fill("testname99");
    await expect(page.getByText("Valid name format")).toBeVisible();
  });

  test("shows contested name warning for short alpha name", async ({
    page,
  }) => {
    const input = page.getByTestId("name-input");
    await input.fill("alice");
    await expect(page.getByTestId("contested-warning")).toBeVisible();
  });

  test("shows not contested message for name with digits > 1", async ({
    page,
  }) => {
    const input = page.getByTestId("name-input");
    await input.fill("alice99");
    await expect(page.getByTestId("not-contested")).toBeVisible();
  });

  test("shows fee estimate for valid name", async ({ page }) => {
    const input = page.getByTestId("name-input");
    await input.fill("alice");
    await expect(page.getByTestId("fee-estimate")).toBeVisible();
  });

  test("shows error for too short name", async ({ page }) => {
    const input = page.getByTestId("name-input");
    await input.fill("ab");
    await expect(page.getByText(/at least 3 characters/)).toBeVisible();
  });

  test("shows advanced options toggle", async ({ page }) => {
    await expect(page.getByTestId("advanced-toggle")).toBeVisible({
      timeout: 5000,
    });
  });

  test("renders DPNS Name Constraints section", async ({ page }) => {
    await expect(page.getByText("DPNS Name Constraints")).toBeVisible({
      timeout: 5000,
    });
  });

  test("renders Contested Names Info section", async ({ page }) => {
    await expect(page.getByText("Contested Names Info")).toBeVisible({
      timeout: 5000,
    });
  });
});
