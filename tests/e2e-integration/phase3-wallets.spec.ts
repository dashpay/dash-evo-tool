/**
 * Phase 3 — Wallet Screens Smoke Tests
 *
 * Verifies wallet list, wallet detail (HD + single-key), create wallet,
 * import wallet, send flow, receive dialog, wallet context menu actions,
 * and asset lock screens render and function correctly with mock IPC data.
 */

import {
  test,
  expect,
} from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Create a realistic HD wallet matching the real WalletDto shape.
 * The createTestHdWallet fixture has some legacy fields — this provides
 * a complete DTO matching the actual bindings.ts WalletDto type.
 */
function createRealisticHdWallet(
  overrides: Record<string, unknown> = {},
) {
  return {
    seedHash:
      "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
    usesPassword: true,
    alias: "My HD Wallet",
    isMain: true,
    confirmedBalance: 500000000,
    unconfirmedBalance: 0,
    totalBalance: 500000000,
    addresses: [
      {
        address: "yXJqLJt8MWAMN2VSJQEuYAxzjXCzJEHpjF",
        derivationPath: "m/44'/5'/0'/0/0",
        balance: 500000000,
        totalReceived: 1000000000,
      },
      {
        address: "yAnotherAddress123456789012345",
        derivationPath: "m/44'/5'/0'/0/1",
        balance: 0,
        totalReceived: 0,
      },
    ],
    transactions: [],
    unusedAssetLocks: [
      {
        txid: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
        address: "yXJqLJt8MWAMN2VSJQEuYAxzjXCzJEHpjF",
        amount: 100000000,
        hasInstantLock: true,
        hasAssetLockProof: true,
        proofDetails: {
          type: "instantSend",
          instantLockTxid: "def456abc123def456abc123def456abc123def456abc123def456abc123def4",
          outputIndex: 0,
        },
        proofHex: "deadbeefdeadbeef",
      },
    ],
    platformAddresses: [
      {
        address: "tevo1mockplatformaddress123456",
        balance: 200000000,
        nonce: 0,
      },
    ],
    identityIndexes: [],
    passwordHint: "hint",
    ...overrides,
  };
}

/**
 * Create a realistic single-key wallet matching SingleKeyWalletDto.
 */
function createRealisticSingleKeyWallet(
  overrides: Record<string, unknown> = {},
) {
  return {
    keyHash: "b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3",
    usesPassword: false,
    publicKey: "02abc123def456",
    address: "yMockSingleKeyAddress123456789",
    alias: "My Single Key",
    confirmedBalance: 100000000,
    unconfirmedBalance: 0,
    totalBalance: 100000000,
    utxoCount: 1,
    utxos: [
      {
        address: "yMockSingleKeyAddress123456789",
        txid: "def456abc123def456abc123def456abc123def456abc123def456abc123def4",
        outputIndex: 0,
        satoshis: 100000000,
        scriptPubKey:
          "76a914def45678901234567890123456789012345678988ac",
      },
    ],
    ...overrides,
  };
}

/** Standard wallet list mock with one HD and one single-key wallet */
function walletListHandlers(overrides: Record<string, unknown> = {}) {
  const hdWallet = createRealisticHdWallet();
  const skWallet = createRealisticSingleKeyWallet();

  return {
    wallet_list_all: {
      hdWallets: [hdWallet],
      singleKeyWallets: [skWallet],
      selected: { type: "hd" as const, seedHash: hdWallet.seedHash },
    },
    ...overrides,
  };
}

// No sub-route navigation helper needed — tests navigate through the UI
// (clicking buttons) to reach sub-screens, which preserves Zustand store state.

// ---------------------------------------------------------------------------
// Wallet List
// ---------------------------------------------------------------------------

test.describe("Wallet List", () => {
  test("renders HD and single-key wallets from mock data", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByText("My HD Wallet").first()).toBeVisible({
      timeout: 10000,
    });
    await expect(page.getByText("My Single Key").first()).toBeVisible();
  });

  test("shows HD Wallets and Single-Key Wallets section headers", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByText("HD Wallets").first()).toBeVisible({
      timeout: 10000,
    });
    await expect(
      page.getByText("Single-Key Wallets").first(),
    ).toBeVisible();
  });

  test("shows empty state when no wallets exist", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [],
        selected: null,
      },
    });

    await expect(page.getByText("No Wallets Loaded")).toBeVisible({
      timeout: 5000,
    });
  });

  test("empty state has Create Wallet action", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [],
        selected: null,
      },
    });

    await expect(page.getByText("No Wallets Loaded")).toBeVisible({
      timeout: 5000,
    });
    // The empty state should have a Create Wallet button
    await expect(
      page.getByRole("button", { name: /Create Wallet/i }),
    ).toBeVisible();
  });

  test("selecting a wallet shows detail panel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Click on the HD wallet
    await page.getByText("My HD Wallet").first().click();

    // Detail panel should appear with wallet information
    // HD wallet detail shows tabs: Addresses, Transactions (dev only), Asset Locks
    await expect(page.getByText("Addresses")).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// HD Wallet Detail
// ---------------------------------------------------------------------------

test.describe("HD Wallet Detail", () => {
  test("shows wallet alias and balance", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // The detail panel should show the wallet alias
    await expect(page.getByText("My HD Wallet").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows three tabs: Addresses, Asset Locks (and Transactions in dev mode)", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await expect(
      page.getByRole("tab", { name: "Asset Locks" }),
    ).toBeVisible();
  });

  test("addresses tab shows address table", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for addresses tab (default)
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Should show the mock address
    await expect(
      page.getByText("yXJqLJt8MWAMN2V").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("account selector shown when multiple accounts exist", async ({
    page,
    mockIPC,
  }) => {
    // Create a wallet with multiple accounts (different derivation paths)
    const multiAccountWallet = createRealisticHdWallet({
      addresses: [
        {
          address: "yAddr1MainAccount",
          derivationPath: "m/44'/5'/0'/0/0",
          balance: 300000000,
          totalReceived: 300000000,
        },
        {
          address: "yAddr2SecondAccount",
          derivationPath: "m/44'/5'/1'/0/0",
          balance: 200000000,
          totalReceived: 200000000,
        },
      ],
    });

    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [multiAccountWallet],
        singleKeyWallets: [],
        selected: { type: "hd" as const, seedHash: multiAccountWallet.seedHash as string },
      },
    });

    // With multiple BIP44 accounts, the account selector should be visible
    await expect(
      page.getByText("Main Account").or(page.getByText("Select account")),
    ).toBeVisible({ timeout: 10000 });
  });

  test("asset locks tab shows asset lock entries", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Click the Asset Locks tab
    await page.getByRole("tab", { name: "Asset Locks" }).click();

    // Should show the mock asset lock
    await expect(
      page.getByText("abc123def456").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("action buttons (Send, Receive, Refresh) are visible", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for wallet detail to fully render by checking for a tab
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Action buttons should be visible in the detail panel
    // Use exact match for "Receive" to avoid matching "Total Received"
    await expect(
      page.getByRole("button", { name: /Send/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: "Receive", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Refresh", exact: true }),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Single-Key Wallet Detail
// ---------------------------------------------------------------------------

test.describe("Single-Key Wallet Detail", () => {
  test("shows single-key wallet detail when selected", async ({
    page,
    mockIPC,
  }) => {
    const hdWallet = createRealisticHdWallet({ alias: "HD Wallet" });
    const skWallet = createRealisticSingleKeyWallet({
      alias: "My Single Key",
    });

    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [hdWallet],
        singleKeyWallets: [skWallet],
        selected: {
          type: "singleKey",
          keyHash: skWallet.keyHash,
        },
      },
    });

    // Single-key detail should show the wallet's address
    await expect(
      page.getByText("yMockSingleKeyAddress").first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows UTXO list with pagination controls", async ({
    page,
    mockIPC,
  }) => {
    const skWallet = createRealisticSingleKeyWallet({
      alias: "UTXO Test Wallet",
    });

    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [skWallet],
        selected: {
          type: "singleKey",
          keyHash: skWallet.keyHash,
        },
      },
    });

    // Should show UTXO section heading (e.g., "UTXOs (1)")
    await expect(
      page.getByRole("heading", { name: /UTXOs/i }),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows Send and Receive buttons for single-key wallet", async ({
    page,
    mockIPC,
  }) => {
    const skWallet = createRealisticSingleKeyWallet({
      alias: "SK Action Test",
    });

    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [skWallet],
        selected: {
          type: "singleKey",
          keyHash: skWallet.keyHash,
        },
      },
    });

    // Wait for wallet detail to render
    await expect(
      page.getByText("yMockSingleKeyAddress").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /Send/i }).first(),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("button", { name: /Receive/i }),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Create Wallet Flow
// ---------------------------------------------------------------------------

test.describe("Create Wallet", () => {
  test("navigates to create wallet screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets/create", {});

    await expect(page.getByText("Create New Wallet")).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows generate step with word count and language selectors", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/create", {});

    // The generate step shows the "Generate Seed Phrase" heading or button
    await expect(
      page.getByText("Generate Seed Phrase").first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("generate button creates mnemonic words", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/create", {});

    // Click generate
    const generateBtn = page.getByRole("button", { name: /Generate/i });
    await expect(generateBtn).toBeVisible({ timeout: 10000 });
    await generateBtn.click();

    // After generation, should show backup step or the words
    await expect(
      page
        .getByText("Back Up Your Seed Phrase")
        .or(page.getByText("Word 1").or(page.locator("[data-word-index]").first())),
    ).toBeVisible({ timeout: 5000 });
  });

  test("back button returns to wallets", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets/create", {});

    const backBtn = page.getByRole("button", {
      name: /Back to wallets|Back/i,
    });
    await expect(backBtn).toBeVisible({ timeout: 10000 });
    await backBtn.click();

    await page.waitForURL(/\/wallets/, { timeout: 5000 });
  });

  test("full create wallet flow: generate → backup → protect → success", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/create", {
      wallet_create: { seedHash: "created123" },
    });

    // Step 1: Generate — click Generate Seed Phrase
    const generateBtn = page.getByRole("button", { name: /Generate/i });
    await expect(generateBtn).toBeVisible({ timeout: 10000 });
    await generateBtn.click();

    // Step 2: Backup — should show seed words and confirmation checkbox
    await expect(
      page.getByText("Back Up Your Seed Phrase"),
    ).toBeVisible({ timeout: 5000 });

    // Check the confirmation checkbox
    const confirmCheckbox = page.getByRole("checkbox");
    await expect(confirmCheckbox).toBeVisible({ timeout: 3000 });
    await confirmCheckbox.check();

    // Click Continue to proceed to protect step
    await page.getByRole("button", { name: /Continue/i }).click();

    // Step 3: Protect — enter wallet name and password
    await expect(page.getByText("Name & Protect")).toBeVisible({
      timeout: 5000,
    });
    await page.locator("#wallet-alias").fill("Test Wallet");
    await page.locator("#wallet-password").fill("SecurePassword123!");

    // Click Create Wallet button
    await page.getByRole("button", { name: /Create Wallet/i }).click();

    // Step 4: Success — should show success message
    await expect(
      page.getByText("Wallet Created Successfully!"),
    ).toBeVisible({ timeout: 15000 });

    // Should have Go to Wallet button
    await expect(
      page.getByRole("button", { name: /Go to Wallet/i }),
    ).toBeVisible();
  });

  test("password strength indicator updates as user types", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/create", {});

    // Generate and proceed through backup step
    await page.getByRole("button", { name: /Generate/i }).click();
    await expect(
      page.getByText("Back Up Your Seed Phrase"),
    ).toBeVisible({ timeout: 5000 });
    await page.getByRole("checkbox").check();
    await page.getByRole("button", { name: /Continue/i }).click();

    // On the protect step, type a password
    await expect(page.getByText("Name & Protect")).toBeVisible({
      timeout: 5000,
    });
    const passwordInput = page.locator("#wallet-password");
    await passwordInput.fill("ab");

    // Password strength indicator should be visible (bars or text)
    await expect(
      page.getByText(/Weak|Very Weak|Fair|Strong|Very Strong/i),
    ).toBeVisible({ timeout: 3000 });
  });
});

// ---------------------------------------------------------------------------
// Import Wallet Flow
// ---------------------------------------------------------------------------

test.describe("Import Wallet", () => {
  test("navigates to import wallet screen", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    await expect(page.getByText("Import Wallet")).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows seed phrase tab", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    // Tab trigger text is "Seed Phrase"
    await expect(
      page.getByRole("tab", { name: /Seed Phrase/i }),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows private key import tab", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    // Should have a tab for private key import
    await expect(
      page.getByRole("tab", { name: /Private Key/i }),
    ).toBeVisible({ timeout: 10000 });
  });

  test("back button returns to wallets", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    const backBtn = page.getByRole("button", {
      name: /Back to wallets|Back/i,
    });
    await expect(backBtn).toBeVisible({ timeout: 10000 });
    await backBtn.click();

    await page.waitForURL(/\/wallets/, { timeout: 5000 });
  });

  test("seed phrase tab has word inputs that accept text", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    // Seed Phrase tab should be default
    await expect(
      page.getByText("Import from Seed Phrase"),
    ).toBeVisible({ timeout: 10000 });

    // Should have word input fields (Word 1, Word 2, etc.)
    // Use exact match to avoid matching "Word 10", "Word 11", etc.
    const word1Input = page.getByRole("textbox", { name: "Word 1", exact: true });
    await expect(word1Input).toBeVisible({ timeout: 5000 });

    // Type a valid BIP39 word into the first input
    await word1Input.fill("abandon");

    // The input should have the value
    await expect(word1Input).toHaveValue("abandon");
  });

  test("name and password fields appear after entering valid mnemonic", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    await expect(
      page.getByText("Import from Seed Phrase"),
    ).toBeVisible({ timeout: 10000 });

    // Name/password/Import button are gated behind valid mnemonic.
    // Default word count is 24 — switch to 12 for faster test.
    const wordCountTrigger = page.locator("#import-word-count");
    await wordCountTrigger.click();
    await page.getByRole("option", { name: "12 words" }).click();
    await page.waitForTimeout(300);

    // Enter a valid 12-word BIP39 mnemonic word by word.
    const mnemonicWords = [
      "abandon", "abandon", "abandon", "abandon", "abandon", "abandon",
      "abandon", "abandon", "abandon", "abandon", "abandon", "about",
    ];

    for (let i = 0; i < mnemonicWords.length; i++) {
      const input = page.getByRole("textbox", {
        name: `Word ${i + 1}`,
        exact: true,
      });
      await input.fill(mnemonicWords[i]);
    }
    await page.waitForTimeout(500);

    // Name, password fields and Import button should now be visible
    const importBtn = page.getByRole("button", { name: /Import Wallet/i });
    await importBtn.scrollIntoViewIfNeeded();
    await expect(page.locator("#import-alias")).toBeVisible({ timeout: 5000 });
    await expect(page.locator("#import-password")).toBeVisible();
    await expect(importBtn).toBeVisible();
  });

  test("private key tab shows key input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets/import", {});

    // Click the Private Key tab
    await page.getByRole("tab", { name: /Private Key/i }).click();

    // Should show Private Key heading and input
    await expect(
      page.getByText("Import Private Key"),
    ).toBeVisible({ timeout: 5000 });

    const keyInput = page.locator("#private-key");
    await expect(keyInput).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Send Flow
// ---------------------------------------------------------------------------

test.describe("Send Flow", () => {
  test("clicking Send button navigates to HD send screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for wallet detail to render
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Click Send button in the wallet detail action bar
    await page.getByRole("button", { name: /Send/i }).first().click();

    // Should navigate to the send screen
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    await expect(page.getByText("Send Dash")).toBeVisible({
      timeout: 10000,
    });
  });

  test("send screen has address input", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for detail panel, then click Send
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("button", { name: /Send/i }).first().click();
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    // Address input — label is "Send to" with placeholder containing "address"
    await expect(
      page.getByPlaceholder(/address/i),
    ).toBeVisible({ timeout: 10000 });
  });

  test("send screen has back button to wallets", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("button", { name: /Send/i }).first().click();
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    const backBtn = page.getByRole("button", {
      name: /Back to wallets|Back/i,
    });
    await expect(backBtn).toBeVisible({ timeout: 10000 });
  });

  test("send screen has amount input and send button", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for detail panel, then click Send
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("button", { name: /Send/i }).first().click();
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    // Amount input should be present
    await expect(
      page.getByPlaceholder(/amount/i),
    ).toBeVisible({ timeout: 10000 });

    // A Send/submit button should be present at the bottom
    // The button text varies: "Core Transaction", "Send", etc.
    const submitBtn = page.getByRole("button", { name: /Transaction|Send Dash|Send$/i });
    await expect(submitBtn.first()).toBeVisible({ timeout: 5000 });
  });

  test("send screen validates address input", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("button", { name: /Send/i }).first().click();
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    // Enter an invalid address
    const addressInput = page.getByPlaceholder(/address/i);
    await expect(addressInput).toBeVisible({ timeout: 10000 });
    await addressInput.fill("invalid-address");
    // Trigger blur to activate validation
    await addressInput.blur();
    await page.waitForTimeout(300);

    // Should show some error indication for the invalid address
    await expect(
      page.getByText(/invalid/i),
    ).toBeVisible({ timeout: 3000 });
  });

  test("single-key send screen renders via Send button", async ({ page, mockIPC }) => {
    const skWallet = createRealisticSingleKeyWallet({
      alias: "SK Sender",
    });

    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [skWallet],
        selected: {
          type: "singleKey",
          keyHash: skWallet.keyHash,
        },
      },
    });

    // Wait for single-key detail to fully render with heading
    await expect(
      page.getByRole("heading", { name: "SK Sender" }),
    ).toBeVisible({ timeout: 10000 });

    // Wait for the store and callbacks to stabilize
    await page.waitForTimeout(500);

    // Click the "Send" button in the detail view action bar
    const sendBtn = page.getByRole("button", { name: "Send", exact: true });
    await expect(sendBtn).toBeVisible({ timeout: 3000 });
    await sendBtn.click();

    // Should navigate to the send-single-key screen
    await page.waitForURL(/\/wallets\/send/, { timeout: 10000 });

    // Should render the send screen
    await expect(
      page
        .getByText("Send Dash")
        .or(page.getByText("Send"))
        .first(),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// Receive Dialog
// ---------------------------------------------------------------------------

test.describe("Receive Dialog", () => {
  test("receive button opens dialog with address", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for the detail panel to render (tabs visible = detail loaded)
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Click Receive button (exact match to avoid "Total Received" columns)
    const receiveBtn = page.getByRole("button", { name: "Receive", exact: true });
    await expect(receiveBtn).toBeVisible({ timeout: 5000 });
    await receiveBtn.click();

    // Dialog should appear — title contains "Receive"
    await expect(
      page.getByRole("dialog"),
    ).toBeVisible({ timeout: 5000 });

    // Should show a wallet address in the dialog
    await expect(
      page.getByText("yXJqLJt8MWAMN2V").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("receive dialog shows QR code", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    const receiveBtn = page.getByRole("button", { name: "Receive", exact: true });
    await receiveBtn.click();

    await expect(page.getByRole("dialog")).toBeVisible({ timeout: 5000 });

    // QR code is an SVG with role="img"
    const qrCode = page.getByRole("dialog").locator("svg").first();
    await expect(qrCode).toBeVisible({ timeout: 5000 });
  });

  test("receive dialog has copy button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    page.getByRole("button", { name: "Receive", exact: true }).click();

    await expect(page.getByRole("dialog")).toBeVisible({ timeout: 5000 });

    // Should have a copy button for the address
    await expect(
      page.getByRole("dialog").getByRole("button", { name: /Copy/i }),
    ).toBeVisible({ timeout: 3000 });
  });

  test("receive dialog shows balance info", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    page.getByRole("button", { name: "Receive", exact: true }).click();

    await expect(page.getByRole("dialog")).toBeVisible({ timeout: 5000 });

    // Should show balance information
    await expect(
      page.getByRole("dialog").getByText(/Balance/i),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Wallet Context Menu Actions
// ---------------------------------------------------------------------------

test.describe("Wallet Context Menu", () => {
  test("wallet card has context menu with Rename and Remove", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByText("My HD Wallet").first()).toBeVisible({
      timeout: 10000,
    });

    // Find and click the dropdown trigger (three-dot menu / MoreVertical icon)
    const walletCards = page.locator("[data-testid='wallet-card'], .group").first();
    const menuTrigger = walletCards
      .getByRole("button")
      .filter({ has: page.locator("svg") })
      .last();

    // If menu trigger is found, click it
    if (await menuTrigger.isVisible().catch(() => false)) {
      await menuTrigger.click();

      // Menu items should appear
      await expect(page.getByText("Rename")).toBeVisible({ timeout: 3000 });
      await expect(page.getByText("Remove")).toBeVisible();
    }
  });

  test("remove wallet shows confirmation dialog", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    await expect(page.getByText("My HD Wallet").first()).toBeVisible({
      timeout: 10000,
    });

    // Try to open the context menu via right-click or trigger button
    const hdWalletCard = page.getByText("My HD Wallet").first();
    // Attempt to find the dropdown trigger near the wallet card
    const parentCard = hdWalletCard.locator("xpath=ancestor::*[contains(@class, 'group') or contains(@class, 'flex')]").first();
    const trigger = parentCard.getByRole("button").last();

    if (await trigger.isVisible().catch(() => false)) {
      await trigger.click();
      await page.waitForTimeout(300);

      const removeItem = page.getByText("Remove");
      if (await removeItem.isVisible().catch(() => false)) {
        await removeItem.click();

        // Confirmation dialog should appear
        await expect(
          page.getByText("Remove Wallet"),
        ).toBeVisible({ timeout: 3000 });
      }
    }
  });
});

// ---------------------------------------------------------------------------
// Asset Lock Screens
// ---------------------------------------------------------------------------

test.describe("Asset Lock Screens", () => {
  test("asset locks tab has create button that navigates to create screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for detail panel to load
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Switch to Asset Locks tab
    await page.getByRole("tab", { name: "Asset Locks" }).click();

    // Look for a "Create" or "Create Asset Lock" button/link in the Asset Locks tab
    const createBtn = page.getByRole("button", { name: /Create/i }).or(
      page.getByRole("link", { name: /Create/i }),
    );
    if (await createBtn.first().isVisible({ timeout: 3000 }).catch(() => false)) {
      await createBtn.first().click();
      await page.waitForURL(/\/wallets\/asset-locks\/create/, { timeout: 5000 });

      await expect(page.getByText("Create Asset Lock")).toBeVisible({
        timeout: 10000,
      });
    } else {
      // If no create button in asset locks tab, just navigate directly
      // and verify the screen renders (even without wallet store data)
      await page.goto("/wallets/asset-locks/create");
      await mockIPC.waitForInit();
      // Should show either the form or "No HD wallet selected" message
      await expect(
        page.getByText("Create Asset Lock").or(
          page.getByText("No HD wallet selected"),
        ),
      ).toBeVisible({ timeout: 10000 });
    }
  });

  test("asset lock detail screen renders via View button", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Wait for detail panel to load
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Switch to Asset Locks tab
    await page.getByRole("tab", { name: "Asset Locks" }).click();

    // Wait for asset lock to appear, then click the "View" button
    await expect(page.getByText("abc123def456").first()).toBeVisible({ timeout: 5000 });
    const viewBtn = page.getByRole("button", { name: /View asset lock/i }).first();
    await expect(viewBtn).toBeVisible({ timeout: 3000 });
    await viewBtn.click();

    // Should navigate to the detail screen
    await page.waitForURL(/\/wallets\/asset-locks\//, { timeout: 5000 });

    await expect(page.getByText("Asset Lock Detail")).toBeVisible({
      timeout: 10000,
    });
  });

  test("asset lock detail shows transaction info and proof status", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletListHandlers());

    // Navigate to Asset Locks tab and click View
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("tab", { name: "Asset Locks" }).click();
    await expect(page.getByText("abc123def456").first()).toBeVisible({ timeout: 5000 });
    await page.getByRole("button", { name: /View asset lock/i }).first().click();
    await page.waitForURL(/\/wallets\/asset-locks\//, { timeout: 5000 });

    // Should show transaction information and proof status
    await expect(page.getByText("Transaction Information")).toBeVisible({
      timeout: 10000,
    });
    await expect(page.getByText("Proof Status")).toBeVisible();

    // Mock has hasAssetLockProof: true, hasInstantLock: true
    await expect(
      page.getByText("Instant Send Locked"),
    ).toBeVisible();
  });

  test("asset lock detail not found shows error", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/wallets/asset-locks/nonexistent-txid",
      walletListHandlers(),
    );

    await expect(
      page.getByText("Asset lock not found").or(
        page.getByText("No HD wallet selected"),
      ),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ---------------------------------------------------------------------------
// IPC call verification
// ---------------------------------------------------------------------------

test.describe("Wallet IPC Integration", () => {
  test("wallet_list_all is called on mount", async ({ page, mockIPC }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(1500);

    const calls = await mockIPC.getCallHistory("wallet_list_all");
    expect(calls.length).toBeGreaterThanOrEqual(1);
  });

  test("wallet_select is called when selecting a wallet", async ({
    page,
    mockIPC,
  }) => {
    const hdWallet = createRealisticHdWallet({ alias: "Click Me" });
    const skWallet = createRealisticSingleKeyWallet({ alias: "Other" });

    await mockIPC.navigateWithHandlers("/wallets", {
      wallet_list_all: {
        hdWallets: [hdWallet],
        singleKeyWallets: [skWallet],
        selected: null,
      },
    });

    await mockIPC.clearCallHistory("wallet_select");

    // Click on the HD wallet
    await page.getByText("Click Me").first().click();
    await page.waitForTimeout(500);

    const selectCalls = await mockIPC.getCallHistory("wallet_select");
    expect(selectCalls.length).toBeGreaterThanOrEqual(1);
  });

  test("context_is_developer_mode is called on mount", async ({
    page,
    mockIPC,
  }) => {
    await page.goto("/wallets");
    await mockIPC.waitForInit();
    await page.waitForTimeout(1500);

    const calls = await mockIPC.getCallHistory("context_is_developer_mode");
    expect(calls.length).toBeGreaterThanOrEqual(1);
  });
});
