/**
 * Phase 8 — DashPay Screens Smoke Tests
 *
 * Verifies DashPay route navigation renders correctly. Profile screen is now
 * fully implemented; other screens are still placeholders. Tests will be
 * expanded as screens are implemented (tasks 8.2d–8.4).
 */

import { test, expect } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const IDENTITY_ID = "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec";

/** Create a realistic identity matching QualifiedIdentityDto shape. */
function createProfileIdentity(overrides: Record<string, unknown> = {}) {
  return {
    id: IDENTITY_ID,
    alias: "Alice",
    identityType: "user",
    balance: 5000000000,
    dpnsNames: [{ name: "alice.dash", acquiredAt: 1700000000 }],
    keys: [
      {
        keyId: 0,
        purpose: "AUTHENTICATION",
        securityLevel: "MASTER",
        keyType: "ECDSA_SECP256K1",
        data: "02abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    associatedWalletHashes: ["a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

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

/** Handlers with identities loaded for profile screen tests. */
function dashpayWithIdentityHandlers(
  overrides: Record<string, unknown> = {},
) {
  const identity = createProfileIdentity();
  return dashpayHandlers({
    identity_list_local: [identity],
    identity_list_summaries: [identity],
    identity_load_order: [IDENTITY_ID],
    dashpay_db_load_profile: null,
    dashpay_db_load_contacts: [],
    dashpay_db_load_pending_requests: [],
    dashpay_db_load_payments: [],
    ...overrides,
  });
}

// ---------------------------------------------------------------------------
// DashPay Main Screen
// ---------------------------------------------------------------------------

test.describe("DashPay Main Screen", () => {
  test("renders no-identities state when no identities exist", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/dashpay", dashpayHandlers());

    await expect(
      page.getByText("No Identities Loaded"),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Load Identity button in no-identities state", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/dashpay", dashpayHandlers());

    await expect(
      page.getByRole("button", { name: "Load Identity" }),
    ).toBeVisible({ timeout: 10000 });
  });

  test("sidebar DashPay nav item is visible", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/dashpay", dashpayHandlers());

    await expect(page.getByTestId("nav-dashpay")).toBeVisible({
      timeout: 10000,
    });
  });
});

// ---------------------------------------------------------------------------
// DashPay Profile Screen (Implemented)
// ---------------------------------------------------------------------------

test.describe("DashPay Profile Screen", () => {
  test("shows no-profile empty state when identity has no profile", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({ dashpay_db_load_profile: null }),
    );

    await expect(
      page.getByText("No DashPay Profile"),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Create Profile button when no profile exists", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({ dashpay_db_load_profile: null }),
    );

    await expect(
      page.getByRole("button", { name: "Create Profile" }),
    ).toBeVisible({ timeout: 10000 });
  });

  test("renders profile when it exists", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({
        dashpay_db_load_profile: {
          identityId: IDENTITY_ID,
          displayName: "Alice",
          bio: "Dash developer",
          avatarUrl: null,
          publicMessage: null,
          createdAt: 1707500000,
          updatedAt: 1707500000,
        },
      }),
    );

    await expect(
      page.getByText("My DashPay Profile"),
    ).toBeVisible({ timeout: 10000 });
    await expect(page.getByText("Dash developer")).toBeVisible();
  });

  test("enters edit mode when Edit Profile is clicked", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({
        dashpay_db_load_profile: {
          identityId: IDENTITY_ID,
          displayName: "Alice",
          bio: "Dash developer",
          avatarUrl: null,
          publicMessage: null,
          createdAt: 1707500000,
          updatedAt: 1707500000,
        },
      }),
    );

    await expect(
      page.getByRole("button", { name: /Edit Profile/ }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByRole("button", { name: /Edit Profile/ }).click();

    // Should show edit form
    await expect(page.getByLabel(/Display Name/)).toBeVisible();
    await expect(page.getByLabel(/Bio/)).toBeVisible();
    await expect(page.getByLabel(/Avatar URL/)).toBeVisible();
  });

  test("enters create mode from empty state", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({ dashpay_db_load_profile: null }),
    );

    await expect(
      page.getByRole("button", { name: "Create Profile" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByRole("button", { name: "Create Profile" }).click();

    // Should show create form
    await expect(page.getByText("Create Profile")).toBeVisible();
    await expect(page.getByLabel(/Display Name/)).toBeVisible();
  });

  test("shows validation error for empty display name", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({ dashpay_db_load_profile: null }),
    );

    await page.getByRole("button", { name: "Create Profile" }).click();

    // Empty form should show validation error
    await expect(
      page.getByText("Display name is required").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows character counters in edit mode", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({ dashpay_db_load_profile: null }),
    );

    await page.getByRole("button", { name: "Create Profile" }).click();
    await expect(page.getByText("0/25")).toBeVisible({ timeout: 5000 });
    await expect(page.getByText("0/140")).toBeVisible();
  });

  test("cancel returns to view mode", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({
        dashpay_db_load_profile: {
          identityId: IDENTITY_ID,
          displayName: "Alice",
          bio: null,
          avatarUrl: null,
          publicMessage: null,
          createdAt: 1707500000,
          updatedAt: 1707500000,
        },
      }),
    );

    await page.getByRole("button", { name: /Edit Profile/ }).click();
    await expect(page.getByLabel(/Display Name/)).toBeVisible({ timeout: 5000 });

    await page.getByRole("button", { name: "Cancel" }).click();

    // Should be back in view mode
    await expect(page.getByText("My DashPay Profile")).toBeVisible();
  });

  test("shows fee estimation in edit mode", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/profile",
      dashpayWithIdentityHandlers({
        dashpay_db_load_profile: {
          identityId: IDENTITY_ID,
          displayName: "Alice",
          bio: null,
          avatarUrl: null,
          publicMessage: null,
          createdAt: 1707500000,
          updatedAt: 1707500000,
        },
      }),
    );

    await page.getByRole("button", { name: /Edit Profile/ }).click();
    await expect(page.getByText(/Estimated fee/)).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// DashPay Sub-Routes (Placeholders)
// ---------------------------------------------------------------------------

test.describe("DashPay Contacts Route", () => {
  test("renders DashPay Contacts screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/contacts",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contacts" }),
    ).toBeVisible({ timeout: 10000 });
  });
});

test.describe("DashPay Payments Route", () => {
  test("renders Payment History screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/payments",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByText("Payment History"),
    ).toBeVisible({ timeout: 10000 });
  });
});

test.describe("DashPay Search Route", () => {
  test("renders Profile Search heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByText("Search Public Profiles"),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows search input and button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByPlaceholder("Enter DPNS username..."),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByRole("button", { name: /^search$/i }),
    ).toBeVisible();
  });

  test("shows tip text about prefix search", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByText(/tip: search by dpns username prefix/i),
    ).toBeVisible({ timeout: 10000 });
  });

  test("search button is disabled when input is empty", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByRole("button", { name: /^search$/i }),
    ).toBeDisabled({ timeout: 10000 });
  });

  test("search button enables when text is entered", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayWithIdentityHandlers(),
    );

    await page.getByPlaceholder("Enter DPNS username...").fill("alice");
    await expect(
      page.getByRole("button", { name: /^search$/i }),
    ).toBeEnabled({ timeout: 5000 });
  });

  test("shows no-identities state when no identities loaded", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayHandlers(),
    );

    // DashPay layout blocks child routes when no identities exist
    await expect(
      page.getByText("No Identities Loaded"),
    ).toBeVisible({ timeout: 10000 });
  });

  test("opens info dialog on info button click", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/search",
      dashpayWithIdentityHandlers(),
    );

    await page
      .getByRole("button", { name: /about profile search/i })
      .click();
    await expect(
      page.getByText("About Profile Search"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText("Add contacts directly from search results."),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// DashPay Add Contact Route
// ---------------------------------------------------------------------------

test.describe("DashPay Add Contact Screen", () => {
  test("renders Add Contact form with identity selected", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByRole("heading", { level: 2, name: "Add Contact" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(page.getByTestId("username-input")).toBeVisible();
    await expect(page.getByTestId("label-input")).toBeVisible();
    await expect(page.getByTestId("send-button")).toBeVisible();
  });

  test("shows sender identity card", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByText("Alice"),
    ).toBeVisible({ timeout: 10000 });
    await expect(page.getByText("user")).toBeVisible();
  });

  test("validates username format inline", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await page.getByTestId("username-input").fill("bob.eth");
    await expect(
      page.getByText(/usernames must end with/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("enables send button with valid username", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await expect(page.getByTestId("send-button")).toBeDisabled({ timeout: 10000 });
    await page.getByTestId("username-input").fill("bob.dash");
    await expect(page.getByTestId("send-button")).toBeEnabled();
  });

  test("shows request summary when username is entered", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await page.getByTestId("username-input").fill("bob.dash");
    await expect(page.getByTestId("request-summary")).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByTestId("request-summary")).toContainText("bob.dash");
  });

  test("opens info dialog", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await page
      .getByRole("button", { name: /contact request info/i })
      .click();
    await expect(
      page.getByText("About Contact Requests"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows advanced options with key selector on toggle", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByTestId("key-selector"),
    ).not.toBeVisible({ timeout: 5000 });
    await page.getByTestId("advanced-options-toggle").click();
    await expect(page.getByTestId("key-selector")).toBeVisible();
  });

  test("navigates back to contacts on Cancel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/add-contact",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByRole("heading", { level: 2, name: "Add Contact" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByRole("button", { name: /cancel/i }).click();
    await expect(
      page.getByRole("heading", { name: "Contacts" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to add-contact from Contacts screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/dashpay/contacts",
      dashpayWithIdentityHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contacts" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByRole("button", { name: /add contact/i }).click();
    await expect(
      page.getByRole("heading", { level: 2, name: "Add Contact" }),
    ).toBeVisible({ timeout: 5000 });
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
      page.getByText("No Identities Loaded"),
    ).toBeVisible({ timeout: 10000 });

    // Navigate to Tools via sidebar
    await page.getByTestId("nav-tools").click();

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
      page.getByText("No Identities Loaded"),
    ).toBeVisible({ timeout: 5000 });
  });
});
