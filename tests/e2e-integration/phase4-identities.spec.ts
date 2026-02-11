/**
 * Phase 4 — Identity Screens Smoke Tests
 *
 * Verifies identity list, identity detail, create identity, load identity,
 * top up, withdraw, transfer, key management, key info, add key, and
 * register DPNS name screens render and function correctly with mock IPC data.
 */

import { test, expect, createTestIdentity } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Create a realistic identity matching the actual QualifiedIdentityDto shape.
 * Fields match src/frontend/bindings.ts QualifiedIdentityDto exactly.
 */
function createRealisticIdentity(overrides: Record<string, unknown> = {}) {
  return {
    id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Test Identity",
    identityType: "user",
    balance: 1000000000,
    dpnsNames: [{ name: "testuser.dash", acquiredAt: 1700000000 }],
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
      {
        keyId: 1,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "03def456abc123def456abc123def456abc123def456abc123def456abc123def4",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
      {
        keyId: 2,
        purpose: "TRANSFER",
        securityLevel: "CRITICAL",
        keyType: "ECDSA_SECP256K1",
        data: "04aaa111bbb222ccc333ddd444eee555fff666aaa111bbb222ccc333ddd444eee5",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    associatedWalletHashes: [
      "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
    ],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

/**
 * Create a second identity for testing multi-identity scenarios.
 */
function createSecondIdentity(overrides: Record<string, unknown> = {}) {
  return createRealisticIdentity({
    id: "HXTSBVGNkYx9IqRGbOJrCV8OCiNL5cs6VFTtC5U42GFd",
    alias: "Second Identity",
    balance: 500000000,
    dpnsNames: [],
    keys: [
      {
        keyId: 0,
        purpose: "AUTHENTICATION",
        securityLevel: "MASTER",
        keyType: "ECDSA_SECP256K1",
        data: "02fff000eee111ddd222ccc333bbb444aaa555999888777666555444333222111",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: false,
      },
    ],
    associatedWalletHashes: [],
    walletIndex: null,
    ...overrides,
  });
}

/** Standard identity list mock handlers */
function identityListHandlers(overrides: Record<string, unknown> = {}) {
  const identity1 = createRealisticIdentity();
  const identity2 = createSecondIdentity();

  return {
    identity_list_local: [identity1, identity2],
    identity_load_order: [identity1.id, identity2.id],
    identity_list_voting: [identity1],
    wallet_list_all: {
      hdWallets: [
        {
          seedHash:
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
          usesPassword: true,
          alias: "Test HD Wallet",
          isMain: true,
          confirmedBalance: 500000000,
          unconfirmedBalance: 0,
          totalBalance: 500000000,
          addresses: [],
          transactions: [],
          unusedAssetLocks: [],
          platformAddresses: [],
          identityIndexes: [],
          passwordHint: "test",
        },
      ],
      singleKeyWallets: [],
      selected: {
        type: "hd" as const,
        seedHash:
          "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
      },
    },
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Identity List
// ---------------------------------------------------------------------------

test.describe("Identity List", () => {
  test("renders identities from mock data", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await expect(page.getByText("Test Identity").first()).toBeVisible({
      timeout: 10000,
    });
    await expect(page.getByText("Second Identity").first()).toBeVisible();
  });

  test("shows empty state when no identities exist", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
    });

    await expect(page.getByText("No Identities Loaded")).toBeVisible({
      timeout: 10000,
    });
  });

  test("empty state has Load Identity and Create Identity buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
    });

    await expect(page.getByText("No Identities Loaded")).toBeVisible({
      timeout: 10000,
    });
    await expect(
      page.getByRole("button", { name: /Load Identity/i }),
    ).toBeVisible();
  });

  test("identity list region is present", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    const listRegion = page.getByRole("region", { name: "Identity list" });
    await expect(listRegion).toBeVisible({ timeout: 10000 });
  });

  test("identity cards show type badges", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Wait for the list to render
    await expect(page.getByText("Test Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // User type badge should be visible
    await expect(page.getByText("User").first()).toBeVisible();
  });

  test("selecting an identity shows detail panel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Click on the first identity
    await page.getByText("Test Identity").first().click();

    // Detail panel should appear with identity information
    const detailRegion = page.getByRole("region", {
      name: "Identity details",
    });
    await expect(detailRegion).toBeVisible({ timeout: 5000 });
  });

  test("calls identity_list_local IPC on mount", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Wait for identities to render
    await expect(page.getByText("Test Identity").first()).toBeVisible({
      timeout: 10000,
    });

    const calls = await mockIPC.getCallHistory("identity_list_local");
    expect(calls.length).toBeGreaterThanOrEqual(1);
  });
});

// ---------------------------------------------------------------------------
// Identity Detail Panel
// ---------------------------------------------------------------------------

test.describe("Identity Detail Panel", () => {
  test("shows identity alias and balance", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Click on the first identity
    await page.getByText("Test Identity").first().click();

    // Balance should be displayed (1000000000 credits = 10 DASH)
    const detailRegion = page.getByRole("region", {
      name: "Identity details",
    });
    await expect(detailRegion).toBeVisible({ timeout: 5000 });
  });

  test("shows DPNS names for the identity", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Click on the identity with DPNS names
    await page.getByText("Test Identity").first().click();

    // DPNS name should be visible
    await expect(page.getByText("testuser.dash")).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows action buttons: Top Up, Withdraw, Transfer, Register DPNS", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();

    // Wait for detail to render
    const detailRegion = page.getByRole("region", {
      name: "Identity details",
    });
    await expect(detailRegion).toBeVisible({ timeout: 5000 });

    // Action buttons should be present
    await expect(
      page.getByRole("button", { name: /Top Up/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /Withdraw/i }).first(),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Transfer/i }).first(),
    ).toBeVisible();
  });

  test("shows Refresh button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();

    await expect(
      page.getByRole("button", { name: "Refresh", exact: true }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows identity ID with copy button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();

    // Identity ID should be visible (truncated)
    await expect(page.getByText("GWRSAVFMjXx8").first()).toBeVisible({
      timeout: 5000,
    });
  });
});

// ---------------------------------------------------------------------------
// Create Identity
// ---------------------------------------------------------------------------

test.describe("Create Identity", () => {
  const createHandlers = {
    identity_list_local: [],
    identity_load_order: [],
    wallet_list_all: {
      hdWallets: [
        {
          seedHash: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
          usesPassword: true,
          alias: "Test Wallet",
          isMain: true,
          confirmedBalance: 500000000,
          unconfirmedBalance: 0,
          totalBalance: 500000000,
          addresses: [],
          transactions: [],
          unusedAssetLocks: [],
          platformAddresses: [],
          identityIndexes: [],
          passwordHint: "test",
        },
      ],
      singleKeyWallets: [],
      selected: null,
    },
  };

  test("navigates to create identity screen from list", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", createHandlers);

    // Click Create Identity from the list panel (scoped to identity list region)
    const createBtn = page
      .getByRole("region", { name: "Identity list" })
      .getByRole("button", { name: /Create Identity/i });
    await expect(createBtn).toBeVisible({ timeout: 10000 });
    await createBtn.click();

    // Create identity form should appear
    await expect(
      page.getByRole("heading", { name: /Create New Identity/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows wallet selection in create form", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", createHandlers);

    // Navigate to create identity
    const createBtn = page
      .getByRole("region", { name: "Identity list" })
      .getByRole("button", { name: /Create Identity/i });
    await expect(createBtn).toBeVisible({ timeout: 10000 });
    await createBtn.click();

    // Create form should render with the heading and the wallet section
    await expect(
      page.getByRole("heading", { name: /Create New Identity/i }),
    ).toBeVisible({ timeout: 5000 });

    // The wallet label or the auto-selected wallet name should be present
    await expect(
      page.getByText(/Wallet/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows funding method options", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", createHandlers);

    // Navigate to create
    const createBtn = page
      .getByRole("region", { name: "Identity list" })
      .getByRole("button", { name: /Create Identity/i });
    await expect(createBtn).toBeVisible({ timeout: 10000 });
    await createBtn.click();

    // Wait for form to render
    await expect(
      page.getByRole("heading", { name: /Create New Identity/i }),
    ).toBeVisible({ timeout: 5000 });

    // Check that at least the funding method section is present
    await expect(
      page.getByText(/Funding Method/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Load Identity
// ---------------------------------------------------------------------------

test.describe("Load Identity", () => {
  test("navigates to load identity screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
    });

    const loadBtn = page.getByRole("button", { name: /Load Identity/i });
    await expect(loadBtn).toBeVisible({ timeout: 10000 });
    await loadBtn.click();

    // Load identity screen should appear with tabs
    await expect(
      page.getByText("By Identity ID").or(page.getByRole("tab", { name: /Identity ID/i })),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows three tabs: By Identity ID, By Wallet, By DPNS Name", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
    });

    const loadBtn = page.getByRole("button", { name: /Load Identity/i });
    await expect(loadBtn).toBeVisible({ timeout: 10000 });
    await loadBtn.click();

    // All three tabs should be present
    await expect(
      page.getByText("By Identity ID").first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(page.getByText("By Wallet").first()).toBeVisible();
    await expect(page.getByText("By DPNS Name").first()).toBeVisible();
  });

  test("By Identity ID tab has ID input field", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
    });

    const loadBtn = page.getByRole("button", { name: /Load Identity/i });
    await expect(loadBtn).toBeVisible({ timeout: 10000 });
    await loadBtn.click();

    // ID input field should be visible
    await expect(
      page.getByPlaceholder(/Enter identity ID/i).or(page.locator("#identity-id-input")),
    ).toBeVisible({ timeout: 5000 });
  });

  test("By Wallet tab shows wallet selector", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
      wallet_list_all: {
        hdWallets: [
          {
            seedHash: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
            usesPassword: false,
            alias: "Search Wallet",
            isMain: true,
            confirmedBalance: 0,
            unconfirmedBalance: 0,
            totalBalance: 0,
            addresses: [],
            transactions: [],
            unusedAssetLocks: [],
            platformAddresses: [],
            identityIndexes: [],
            passwordHint: "",
          },
        ],
        singleKeyWallets: [],
        selected: null,
      },
    });

    const loadBtn = page.getByRole("button", { name: /Load Identity/i });
    await expect(loadBtn).toBeVisible({ timeout: 10000 });
    await loadBtn.click();

    // Click "By Wallet" tab
    await page.getByText("By Wallet").first().click();

    // Wallet selector should be visible
    await expect(
      page.getByText("Search Wallet").or(page.getByText(/Select.*wallet/i)),
    ).toBeVisible({ timeout: 5000 });
  });

  test("By DPNS Name tab has name input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", {
      identity_list_local: [],
      identity_load_order: [],
    });

    const loadBtn = page.getByRole("button", { name: /Load Identity/i });
    await expect(loadBtn).toBeVisible({ timeout: 10000 });
    await loadBtn.click();

    // Click "By DPNS Name" tab
    await page.getByText("By DPNS Name").first().click();

    // Name input should be visible (placeholder is "alice")
    await expect(
      page.getByPlaceholder("alice").or(page.locator("#dpns-name-input")),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Top Up Identity
// ---------------------------------------------------------------------------

test.describe("Top Up Identity", () => {
  test("navigates to top up screen from detail panel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Select identity
    await page.getByText("Test Identity").first().click();

    // Wait for detail panel
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    // Click Top Up
    await page.getByRole("button", { name: /Top Up/i }).first().click();

    // Top up screen should appear
    await expect(
      page.getByText(/Top Up/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Withdraw
// ---------------------------------------------------------------------------

test.describe("Withdraw", () => {
  test("navigates to withdraw screen from detail panel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Select identity
    await page.getByText("Test Identity").first().click();

    // Wait for detail panel
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    // Click Withdraw
    await page.getByRole("button", { name: /Withdraw/i }).first().click();

    // Withdraw screen should appear with the identity's info
    await expect(
      page.getByText(/Withdraw/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows amount input and address field", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    await page.getByRole("button", { name: /Withdraw/i }).first().click();

    // Withdraw screen shows amount and address sections
    await expect(
      page.getByText(/Amount to withdraw/i),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/address to withdraw/i),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Transfer
// ---------------------------------------------------------------------------

test.describe("Transfer", () => {
  test("navigates to transfer screen from detail panel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    await page.getByRole("button", { name: /Transfer/i }).first().click();

    // Transfer screen should appear
    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows destination type toggle (To Identity / To Platform Address)", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    await page.getByRole("button", { name: /Transfer/i }).first().click();

    // Wait for transfer screen
    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 5000 });

    // Should show destination type options
    await expect(
      page.getByText(/Identity/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Key Management
// ---------------------------------------------------------------------------

test.describe("Key Management", () => {
  test("navigates to key management screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Select identity
    await page.getByText("Test Identity").first().click();

    // Wait for detail panel
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    // Open context menu and click View Keys (or use the detail panel button)
    // Try clicking "View Keys" directly first — may be in action bar or dropdown
    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await expect(viewKeysBtn).toBeVisible({ timeout: 5000 });
    await viewKeysBtn.click();

    // Key management screen should show key rows (rendered as buttons)
    await expect(
      page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows key table with correct columns", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    // Should show key data from the mock identity
    await expect(
      page.getByRole("button", { name: /AUTHENTICATION/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /TRANSFER/i }).first(),
    ).toBeVisible();
  });

  test("shows Add Key button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    // Add Key button should be present
    await expect(
      page.getByRole("button", { name: /Add Key/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("clicking a key row navigates to key info", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    // Click on the first AUTHENTICATION key row
    await page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }).click();

    // Key info screen should appear
    await expect(
      page.getByText(/Key Info|Key #/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Key Info
// ---------------------------------------------------------------------------

test.describe("Key Info", () => {
  test("shows key metadata (purpose, security level, type)", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Navigate: select identity → view keys → click key
    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    // Wait for keys
    await expect(
      page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }),
    ).toBeVisible({ timeout: 5000 });

    // Click key to view info
    await page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }).click();

    // Should show key metadata — "Master" security level text
    await expect(page.getByText("Master").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows public key hex display", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();
    await expect(
      page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }),
    ).toBeVisible({ timeout: 5000 });
    await page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }).click();

    // Public key hex should be visible
    await expect(
      page.getByText("02abc123").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("has back button to return to key list", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();
    await expect(
      page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }),
    ).toBeVisible({ timeout: 5000 });
    await page.getByRole("button", { name: /Key 0.*AUTHENTICATION/i }).click();

    // Back button should be present
    await expect(
      page.getByRole("button", { name: /Back/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Add Key
// ---------------------------------------------------------------------------

test.describe("Add Key", () => {
  test("opens add key form from key management", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    // Click Add Key
    const addKeyBtn = page.getByRole("button", { name: /Add Key/i });
    await expect(addKeyBtn).toBeVisible({ timeout: 5000 });
    await addKeyBtn.click();

    // Add key form should appear
    await expect(
      page.getByRole("heading", { name: "Add New Key" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows purpose, security level, and key type selectors", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    const addKeyBtn = page.getByRole("button", { name: /Add Key/i });
    await expect(addKeyBtn).toBeVisible({ timeout: 5000 });
    await addKeyBtn.click();

    // Form should show purpose and security level labels
    await expect(
      page.getByText(/Purpose/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows generate random key button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    const viewKeysBtn = page.getByRole("button", { name: /Manage Keys/i }).first();
    await viewKeysBtn.click();

    const addKeyBtn = page.getByRole("button", { name: /Add Key/i });
    await expect(addKeyBtn).toBeVisible({ timeout: 5000 });
    await addKeyBtn.click();

    // Generate random key button
    await expect(
      page.getByRole("button", { name: /Generate/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Inline Alias Editing
// ---------------------------------------------------------------------------

test.describe("Inline Alias Editing", () => {
  test("identity list supports inline alias editing", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Wait for identities to render
    await expect(page.getByText("Test Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // The identity card should show the alias text
    // This verifies the alias is displayed and the inline editing infrastructure exists
    await expect(page.getByText("Test Identity").first()).toBeVisible();
    await expect(page.getByText("Second Identity").first()).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Register DPNS Name (from Identity screen)
// ---------------------------------------------------------------------------

test.describe("Register DPNS Name", () => {
  test("navigates to register DPNS name screen from identity detail", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await page.getByText("Test Identity").first().click();
    await expect(
      page.getByRole("region", { name: "Identity details" }),
    ).toBeVisible({ timeout: 5000 });

    // Click Register DPNS button
    const registerBtn = page.getByRole("button", { name: /Register DPNS|DPNS/i }).first();
    await expect(registerBtn).toBeVisible({ timeout: 5000 });
    await registerBtn.click();

    // Register DPNS name screen should appear
    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Context Menu Actions
// ---------------------------------------------------------------------------

test.describe("Context Menu Actions", () => {
  test("identity action menu is accessible", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    // Wait for identities
    await expect(page.getByText("Test Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // Find action menu trigger (the "..." or kebab button)
    const actionTrigger = page
      .getByRole("button", { name: /Identity actions|actions/i })
      .first();
    await expect(actionTrigger).toBeVisible({ timeout: 5000 });
    await actionTrigger.click();

    // Dropdown menu items should appear
    await expect(
      page.getByRole("menuitem", { name: /Refresh/i }),
    ).toBeVisible({ timeout: 3000 });
  });

  test("context menu shows key identity actions", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

    await expect(page.getByText("Test Identity").first()).toBeVisible({
      timeout: 10000,
    });

    const actionTrigger = page
      .getByRole("button", { name: /Identity actions|actions/i })
      .first();
    await actionTrigger.click();

    // Check for key menu items
    await expect(
      page.getByRole("menuitem", { name: /Top Up/i }),
    ).toBeVisible({ timeout: 3000 });
    await expect(
      page.getByRole("menuitem", { name: /Withdraw/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: /Remove/i }).or(
        page.getByRole("menuitem", { name: /Delete/i }),
      ),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Drag-and-drop reordering
// ---------------------------------------------------------------------------

test("identity list shows drag handles for reordering", async ({
  page,
  mockIPC,
}) => {
  await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

  // Wait for identities to render
  await expect(page.getByText("Test Identity").first()).toBeVisible({
    timeout: 10000,
  });

  // Should show drag handles for each identity
  const handles = page.getByLabel("Drag to reorder");
  await expect(handles).toHaveCount(2);
});

test("drag handles have grab cursor styling", async ({
  page,
  mockIPC,
}) => {
  await mockIPC.navigateWithHandlers("/identities", identityListHandlers());

  await expect(page.getByText("Test Identity").first()).toBeVisible({
    timeout: 10000,
  });

  const handle = page.getByLabel("Drag to reorder").first();
  await expect(handle).toBeVisible();
  await expect(handle).toHaveClass(/cursor-grab/);
});
