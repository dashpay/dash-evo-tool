import { test, expect } from "@playwright/test";

test.describe("App", () => {
  test("loads the homepage with heading and greet button", async ({
    page,
  }) => {
    await page.goto("/");

    await expect(
      page.getByRole("heading", { name: /dash evo tool/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /greet/i }),
    ).toBeVisible();
    await expect(
      page.getByPlaceholder(/enter your name/i),
    ).toBeVisible();
  });
});
