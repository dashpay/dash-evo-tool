import { test, expect } from "@playwright/test";

test.describe("Document Action Screens", () => {
  test.describe("Create Document", () => {
    test.beforeEach(async ({ page }) => {
      await page.goto("/contracts/create-document");
    });

    test("renders Create Document heading", async ({ page }) => {
      await expect(
        page.getByRole("heading", { name: "Create Document" }),
      ).toBeVisible({ timeout: 5000 });
    });

    test("renders contract and document type selectors", async ({ page }) => {
      await expect(page.getByTestId("contract-select")).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId("doctype-select")).toBeVisible({ timeout: 5000 });
    });

    test("renders step headings", async ({ page }) => {
      await expect(page.getByText("1. Select a contract and document type")).toBeVisible({ timeout: 5000 });
      await expect(page.getByText("2. Select an identity")).toBeVisible({ timeout: 5000 });
      await expect(page.getByText(/3\. Fill out the document type fields/)).toBeVisible({ timeout: 5000 });
    });

    test("renders back button that navigates to contracts", async ({ page }) => {
      const backButton = page.getByTestId("back-button");
      await expect(backButton).toBeVisible({ timeout: 5000 });
      await backButton.click();
      await expect(page).toHaveURL(/\/contracts\/?$/, { timeout: 5000 });
    });

    test("renders advanced options toggle", async ({ page }) => {
      await expect(page.getByTestId("advanced-toggle")).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("Delete Document", () => {
    test.beforeEach(async ({ page }) => {
      await page.goto("/contracts/delete-document");
    });

    test("renders Delete Document heading", async ({ page }) => {
      await expect(
        page.getByRole("heading", { name: "Delete Document" }),
      ).toBeVisible({ timeout: 5000 });
    });

    test("renders document ID input", async ({ page }) => {
      await expect(page.getByTestId("document-id-input")).toBeVisible({ timeout: 5000 });
    });

    test("accepts text in document ID input", async ({ page }) => {
      const input = page.getByTestId("document-id-input");
      await input.fill("test-document-id-123");
      await expect(input).toHaveValue("test-document-id-123");
    });

    test("shows correct step 3 label", async ({ page }) => {
      await expect(page.getByText(/3\. Enter document ID to delete/)).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("Replace Document", () => {
    test.beforeEach(async ({ page }) => {
      await page.goto("/contracts/replace-document");
    });

    test("renders Replace Document heading", async ({ page }) => {
      await expect(
        page.getByRole("heading", { name: "Replace Document" }),
      ).toBeVisible({ timeout: 5000 });
    });

    test("renders document ID input and fetch button", async ({ page }) => {
      await expect(page.getByTestId("document-id-input")).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId("fetch-document-button")).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("Transfer Document", () => {
    test.beforeEach(async ({ page }) => {
      await page.goto("/contracts/transfer-document");
    });

    test("renders Transfer Document heading", async ({ page }) => {
      await expect(
        page.getByRole("heading", { name: "Transfer Document" }),
      ).toBeVisible({ timeout: 5000 });
    });

    test("renders document ID and recipient ID inputs", async ({ page }) => {
      await expect(page.getByTestId("document-id-input")).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId("recipient-id-input")).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("Purchase Document", () => {
    test.beforeEach(async ({ page }) => {
      await page.goto("/contracts/purchase-document");
    });

    test("renders Purchase Document heading", async ({ page }) => {
      await expect(
        page.getByRole("heading", { name: "Purchase Document" }),
      ).toBeVisible({ timeout: 5000 });
    });

    test("renders fetch price button", async ({ page }) => {
      await expect(page.getByTestId("fetch-price-button")).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("Set Document Price", () => {
    test.beforeEach(async ({ page }) => {
      await page.goto("/contracts/set-document-price");
    });

    test("renders Set Document Price heading", async ({ page }) => {
      await expect(
        page.getByRole("heading", { name: "Set Document Price" }),
      ).toBeVisible({ timeout: 5000 });
    });

    test("renders document ID and price inputs", async ({ page }) => {
      await expect(page.getByTestId("document-id-input")).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId("price-input")).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe("Cross-route navigation", () => {
    test("navigates between document action routes", async ({ page }) => {
      await page.goto("/contracts/create-document");
      await expect(
        page.getByRole("heading", { name: "Create Document" }),
      ).toBeVisible({ timeout: 5000 });

      await page.goto("/contracts/delete-document");
      await expect(
        page.getByRole("heading", { name: "Delete Document" }),
      ).toBeVisible({ timeout: 5000 });

      await page.goto("/contracts/purchase-document");
      await expect(
        page.getByRole("heading", { name: "Purchase Document" }),
      ).toBeVisible({ timeout: 5000 });
    });
  });
});
