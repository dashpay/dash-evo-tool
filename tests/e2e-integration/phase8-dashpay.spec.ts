/**
 * Phase 8 — DashPay Screens Smoke Tests
 *
 * Verifies DashPay route navigation renders correctly. Currently all DashPay
 * screens are placeholders — tests verify navigation, placeholder rendering,
 * and sidebar nav integration. Tests will be expanded as screens are
 * implemented (tasks 8.2b–8.4).
 */

import { test, expect } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Standard IPC handlers for DashPay screens. */
function dashpayHandlers(overrides: Record<string, unknown> = {}) {
  return {
    settings_get: {
      theme: "Dark",
      developerMode: false,
      disableZmq: false,
      onboardingCompleted: true,
    },
    context_get_network: "Testnet",
    identity_list_local: [],
    identity_list_summaries: [],
    identity_load_order: [],
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// DashPay Main Screen (Placeholder)
// ---------------------------------------------------------------------------

test.describe("DashPay Main Screen", () => {
  test("renders DashPay heading from placeholder", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/dashpay", dashpayHandlers());

    await expect(
      page.getByRole("heading", { name: "DashPay" }),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows placeholder message", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/dashpay", dashpayHandlers());

    await expect(
      page.getByRole("heading", { name: "DashPay" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });

  test("sidebar DashPay nav item is visible", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/dashpay", dashpayHandlers());

    await expect(page.getByTestId("nav-dashpay")).toBeVisible({
      timeout: 10000,
    });
  });
});

// ---------------------------------------------------------------------------
// DashPay Sub-Routes (Placeholders)
// ---------------------------------------------------------------------------

test.describe("DashPay Profile Route", () => {
  test("renders DashPay Profile placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "DashPay Profile" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });
});

test.describe("DashPay Contacts Route", () => {
  test("renders DashPay Contacts placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/contacts",
      dashpayHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "DashPay Contacts" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });
});

test.describe("DashPay Payments Route", () => {
  test("renders DashPay Payments placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/payments",
      dashpayHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "DashPay Payments" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });
});

test.describe("DashPay Search Route", () => {
  test("renders Profile Search placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Profile Search" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Cross-section Navigation
// ---------------------------------------------------------------------------

test.describe("DashPay Navigation Integration", () => {
  test("can navigate from DashPay to other sections via sidebar", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay",
      dashpayHandlers({
        wallet_list_all: {
          hdWallets: [],
          singleKeyWallets: [],
          selected: null,
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "DashPay" }),
    ).toBeVisible({ timeout: 10000 });

    // Navigate to Tools via sidebar (has a clear heading)
    await page.getByTestId("nav-tools").click();

    // Should see Tools heading
    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("can navigate from another section to DashPay", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools",
      dashpayHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    // Navigate to DashPay via sidebar
    await page.getByTestId("nav-dashpay").click();

    await expect(
      page.getByRole("heading", { name: "DashPay" }),
    ).toBeVisible({ timeout: 5000 });
  });
});
