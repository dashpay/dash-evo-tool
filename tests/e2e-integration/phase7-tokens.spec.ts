/**
 * Phase 7 — Token Screens Smoke Tests
 *
 * Verifies My Tokens table, token search, add by ID, token creator wizard,
 * and all token action screens (transfer, mint, burn, freeze, unfreeze,
 * destroy frozen, pause, resume) render and function correctly with mock IPC data.
 */

import { test, expect } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a mock token entry matching TokenEntry from tokenStore. */
function createTokenEntry(overrides: Record<string, unknown> = {}) {
  return {
    identityId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    tokenId:
      "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233",
    contractId:
      "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    tokenPosition: 0,
    name: "TestToken",
    ownerAlias: "Token Owner",
    balance: "1000000000000",
    decimals: 8,
    ...overrides,
  };
}

/** Create a second token entry for multi-row tests. */
function createSecondToken(overrides: Record<string, unknown> = {}) {
  return createTokenEntry({
    tokenId:
      "bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff0011223344aa",
    contractId:
      "2233445566778899001122334455667788990011aabbccddeeff001122334466",
    name: "OtherToken",
    ownerAlias: "Other Owner",
    balance: "500000000",
    ...overrides,
  });
}

/** Create a mock token search result. */
function createSearchResult(overrides: Record<string, unknown> = {}) {
  return {
    contractId:
      "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    description: "A test token for keyword search",
    ...overrides,
  };
}

/** Standard identity for token screens. */
function createTokenIdentity() {
  return {
    id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Token Owner",
    identityType: "user",
    balance: 5000000000,
    dpnsNames: [],
    keys: [
      {
        keyId: 0,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "02abc123",
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
  };
}

/** Standard IPC handlers for token screens. */
function tokenHandlers(overrides: Record<string, unknown> = {}) {
  return {
    token_query_my_balances: { taskId: "mock-token-task" },
    token_load_order: [],
    identity_list_local: [createTokenIdentity()],
    identity_list_summaries: [createTokenIdentity()],
    identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    wallet_list_all: {
      hdWallets: [
        {
          seedHash:
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
          alias: "Test Wallet",
          usesPassword: false,
          passwordHint: null,
          identityRegistrations: [],
          accounts: [],
          utxos: [],
          assetLocks: [],
        },
      ],
      singleKeyWallets: [],
      selected: null,
    },
    context_is_developer_mode: false,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// My Tokens Screen
// ---------------------------------------------------------------------------

test.describe("My Tokens Screen", () => {
  test("renders with My Tokens heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows Refresh, Add Token by ID, Search Tokens, and Create Token buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Refresh/i }),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /Add Token by ID/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Search Tokens/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /Create Token/i }),
    ).toBeVisible();
  });

  test("renders page with heading and description", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Description text should be present
    await expect(
      page.getByText(/Manage your token balances/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows tokens in table when task result arrives", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit token data via task result event
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [createTokenEntry(), createSecondToken()],
    });

    // Token names should appear
    await expect(page.getByText("TestToken").first()).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByText("OtherToken").first()).toBeVisible();
  });

  test("shows token in Level 1 table with name", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [createTokenEntry()],
    });

    // In Level 1, the token name is a clickable button
    await expect(page.getByText("TestToken").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("Add Token by ID button navigates to add-by-id", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    await page.getByRole("button", { name: /Add Token by ID/i }).click();
    await page.waitForURL(/add-by-id/, { timeout: 5000 });
  });

  test("Search Tokens button navigates to search", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    await page.getByRole("button", { name: /Search Tokens/i }).click();
    await page.waitForURL(/search/, { timeout: 5000 });
  });

  test("Create Token button navigates to creator", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    await page.getByRole("button", { name: /Create Token/i }).click();
    await page.waitForURL(/creator/, { timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token Search Screen
// ---------------------------------------------------------------------------

test.describe("Token Search Screen", () => {
  test("renders token search with keyword input", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens/search", tokenHandlers());

    // Should have a search input
    await expect(
      page
        .getByPlaceholder(/search/i)
        .or(page.getByPlaceholder(/keyword/i))
        .or(page.getByRole("textbox").first()),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows back navigation", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tokens/search", tokenHandlers());

    // Back button or link should be present
    await expect(
      page
        .getByRole("button", { name: /Back/i })
        .or(page.getByRole("link", { name: /Back/i })),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// Token Add by ID Screen
// ---------------------------------------------------------------------------

test.describe("Token Add by ID Screen", () => {
  test("renders with input for token/contract ID", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/add-by-id",
      tokenHandlers(),
    );

    // Should have an input for token/contract ID
    await expect(
      page
        .getByPlaceholder(/contract/i)
        .or(page.getByPlaceholder(/token/i))
        .or(page.getByPlaceholder(/identifier/i))
        .or(page.getByRole("textbox").first()),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows back navigation", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/add-by-id",
      tokenHandlers(),
    );

    await expect(
      page
        .getByRole("button", { name: /Back/i })
        .or(page.getByRole("link", { name: /Back/i })),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// Token Creator Screen
// ---------------------------------------------------------------------------

test.describe("Token Creator Screen", () => {
  test("renders token creator wizard with heading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/creator",
      tokenHandlers({
        contract_list_local: [],
      }),
    );

    // PageHeader title is "Token Creator"
    await expect(
      page.getByText("Token Creator").first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Back to Tokens button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/creator",
      tokenHandlers({
        contract_list_local: [],
      }),
    );

    await expect(
      page.getByTestId("back-to-tokens"),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// Token Operation Screens (all use TokenOperationForm)
// ---------------------------------------------------------------------------

/** Token action route configurations for parameterized tests. */
const tokenActionRoutes: Array<{
  path: string;
  actionName: string;
}> = [
  { path: "/tokens/transfer", actionName: "Transfer" },
  { path: "/tokens/mint", actionName: "Mint" },
  { path: "/tokens/burn", actionName: "Burn" },
  { path: "/tokens/freeze", actionName: "Freeze" },
  { path: "/tokens/unfreeze", actionName: "Unfreeze" },
  { path: "/tokens/destroy-frozen", actionName: "Destroy Frozen" },
  { path: "/tokens/pause", actionName: "Pause" },
  { path: "/tokens/resume", actionName: "Resume" },
];

/** Build search params for token operation screens. */
function tokenSearchParams() {
  return new URLSearchParams({
    tokenId:
      "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233",
    contractId:
      "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    tokenPosition: "0",
    identityId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
  }).toString();
}

for (const { path, actionName } of tokenActionRoutes) {
  test.describe(`Token ${actionName} Screen`, () => {
    test(`renders ${actionName} screen`, async ({ page, mockIPC }) => {
      await mockIPC.navigateWithHandlers(
        `${path}?${tokenSearchParams()}`,
        tokenHandlers(),
      );

      // The screen should render with the action name
      await expect(
        page
          .getByText(new RegExp(actionName, "i"))
          .first(),
      ).toBeVisible({ timeout: 10000 });
    });

    test("shows Back to Tokens button", async ({ page, mockIPC }) => {
      await mockIPC.navigateWithHandlers(
        `${path}?${tokenSearchParams()}`,
        tokenHandlers(),
      );

      // TokenOperationForm has "Back to Tokens" button
      await expect(
        page.getByText("Back to Tokens").first(),
      ).toBeVisible({ timeout: 10000 });
    });
  });
}

// Specific tests for Transfer (has extra fields)

test.describe("Token Transfer — specific fields", () => {
  test("shows amount and recipient fields", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/transfer?${tokenSearchParams()}`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Amount field (data-testid="operation-amount-input")
    await expect(
      page.getByTestId("operation-amount-input"),
    ).toBeVisible({ timeout: 5000 });

    // Recipient field (data-testid="operation-recipient-input")
    await expect(
      page.getByTestId("operation-recipient-input"),
    ).toBeVisible();
  });
});

// Specific tests for Mint (has amount field)

test.describe("Token Mint — specific fields", () => {
  test("shows amount field", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/mint?${tokenSearchParams()}`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Mint/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("operation-amount-input"),
    ).toBeVisible({ timeout: 5000 });
  });
});

// Specific tests for Burn (has amount field)

test.describe("Token Burn — specific fields", () => {
  test("shows amount field", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/burn?${tokenSearchParams()}`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Burn/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("operation-amount-input"),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token Freeze — specific fields
// ---------------------------------------------------------------------------

test.describe("Token Freeze — specific fields", () => {
  test("shows freeze identity ID input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/freeze?${tokenSearchParams()}`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Freeze/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Freeze screen has a custom identity ID input
    await expect(
      page
        .locator("#freeze-identity-id")
        .or(page.getByPlaceholder(/identity ID/i)),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token operation form — shared elements
// ---------------------------------------------------------------------------

test.describe("Token operation form — shared elements", () => {
  test("shows token context header with name", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/transfer?${tokenSearchParams()}&name=TestToken&balance=1000000000000&decimals=8`,
      tokenHandlers(),
    );

    await expect(page.getByTestId("token-context-header")).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows identity selector in operation form", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/transfer?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("operation-identity-select"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows submit button in operation form", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/transfer?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("operation-submit"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows cancel button in operation form", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/transfer?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("operation-cancel"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows advanced toggle in operation form", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/transfer?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Transfer/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("operation-advanced-toggle"),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token Search — additional tests
// ---------------------------------------------------------------------------

test.describe("Token Search — additional tests", () => {
  test("shows Search Tokens heading and description", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens/search", tokenHandlers());

    await expect(
      page.getByText("Search Tokens").first(),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/Search for tokens by keyword/i),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token Add by ID — additional tests
// ---------------------------------------------------------------------------

test.describe("Token Add by ID — additional tests", () => {
  test("shows Add Token by ID heading and description", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/add-by-id",
      tokenHandlers(),
    );

    await expect(
      page.getByText("Add Token by ID").first(),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/Look up a token/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Search button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/add-by-id",
      tokenHandlers(),
    );

    await expect(
      page.getByText("Add Token by ID").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /Search/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows idle hint text", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/add-by-id",
      tokenHandlers(),
    );

    await expect(
      page.getByText("Add Token by ID").first(),
    ).toBeVisible({ timeout: 10000 });

    // Idle state shows hint text
    await expect(
      page.getByText(/Enter a contract ID or token ID/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("enters text in ID input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/add-by-id",
      tokenHandlers(),
    );

    await expect(
      page.getByText("Add Token by ID").first(),
    ).toBeVisible({ timeout: 10000 });

    const input = page.getByPlaceholder(/contract ID or token ID/i);
    await expect(input).toBeVisible({ timeout: 5000 });
    await input.fill("aa11bb22cc33dd44ee55ff66");
    await expect(input).toHaveValue("aa11bb22cc33dd44ee55ff66");
  });
});

// ---------------------------------------------------------------------------
// Token Creator — additional tests
// ---------------------------------------------------------------------------

test.describe("Token Creator — step navigation", () => {
  test("shows step 1 (Basic Info) content on load", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/creator",
      tokenHandlers({
        contract_list_local: [],
      }),
    );

    await expect(
      page.getByText("Token Creator").first(),
    ).toBeVisible({ timeout: 10000 });

    // Step 1 should show Basic Info fields — look for token name label
    await expect(
      page.getByText(/Token Name|Basic Info|Step 1/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows mode toggle (Simple/Advanced)", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tokens/creator",
      tokenHandlers({
        contract_list_local: [],
      }),
    );

    await expect(
      page.getByText("Token Creator").first(),
    ).toBeVisible({ timeout: 10000 });

    // Simple/Advanced mode toggle
    await expect(
      page
        .getByRole("button", { name: /Simple/i })
        .or(page.getByText(/Simple Mode/i).first()),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// My Tokens — additional interaction tests
// ---------------------------------------------------------------------------

test.describe("My Tokens — drill-down interaction", () => {
  test("clicking token name in Level 1 drills into Level 2", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit two tokens under same name but different identities
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [
        createTokenEntry(),
        createTokenEntry({
          identityId: "HXTSBVGNkYx9IqRGbOJrCV8OCiNL5cs6VFTtC5U42GFd",
          ownerAlias: "Second Owner",
          balance: "200000000",
        }),
      ],
    });

    // In Level 1, click on the token name to drill down
    const tokenNameBtn = page.getByRole("button", { name: "TestToken" }).first();
    await expect(tokenNameBtn).toBeVisible({ timeout: 5000 });
    await tokenNameBtn.click();

    // Level 2 should show per-identity rows — look for "Back" button
    await expect(
      page.getByRole("button", { name: /Back/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("empty state shows no tokens message", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit empty token list
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [],
    });

    // Should show empty state or "No tokens" message
    await expect(
      page
        .getByText(/No tokens/i)
        .or(page.getByText(/no token balances/i))
        .or(page.getByText(/Add tokens/i)),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token Set Price and Purchase screens
// ---------------------------------------------------------------------------

test.describe("Token Set Price Screen", () => {
  test("renders with Set Price heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/set-price?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Set Price/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows pricing type buttons", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/set-price?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Set Price/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("pricing-section"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Back to Tokens button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/set-price?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Set Price/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText("Back to Tokens").first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

test.describe("Token Purchase Screen", () => {
  test("renders with Purchase heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/purchase?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Purchase/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows amount input section", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/purchase?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Purchase/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByTestId("purchase-amount-section"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Back to Tokens button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/purchase?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Purchase/i).first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText("Back to Tokens").first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Token Claim and View Claims screens
// ---------------------------------------------------------------------------

test.describe("Token Claim Screen", () => {
  test("renders Claim screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/claim?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Claim/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Back to Tokens button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/claim?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText("Back to Tokens").first(),
    ).toBeVisible({ timeout: 10000 });
  });
});

test.describe("Token View Claims Screen", () => {
  test("renders View Claims screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/view-claims?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Claims/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Back to Tokens button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/view-claims?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText("Back to Tokens").first(),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// Token Update Config screen
// ---------------------------------------------------------------------------

test.describe("Token Update Config Screen", () => {
  test("renders Update Config screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/update-config?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText(/Update Config|Update Token/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Back to Tokens button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      `/tokens/update-config?${tokenSearchParams()}&name=TestToken`,
      tokenHandlers(),
    );

    await expect(
      page.getByText("Back to Tokens").first(),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// Drag-and-drop reordering
// ---------------------------------------------------------------------------

test.describe("Token List — drag-and-drop", () => {
  test("shows drag handles for each token in Level 1 list", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit token data
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [createTokenEntry(), createSecondToken()],
    });

    // Wait for tokens to render
    await expect(page.getByText("TestToken").first()).toBeVisible({
      timeout: 5000,
    });

    // Drag handles should be visible
    const handles = page.getByLabel("Drag to reorder");
    await expect(handles).toHaveCount(2);
  });

  test("drag handles have Reorder column header", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [createTokenEntry()],
    });

    await expect(page.getByText("TestToken").first()).toBeVisible({
      timeout: 5000,
    });

    // sr-only header for accessibility
    await expect(page.getByText("Reorder")).toBeAttached();
  });
});
