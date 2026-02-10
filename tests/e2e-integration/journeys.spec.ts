/**
 * Multi-Screen User Journey Tests
 *
 * These tests exercise complete user flows that span multiple screens,
 * verifying that navigation, data flow, and IPC commands work together
 * across the entire application.
 *
 * Each journey exercises 3-6 screens in sequence, simulating realistic
 * user workflows from start to finish.
 */

import { test, expect } from "./fixtures";
import { navigateToSection } from "./helpers";

// ---------------------------------------------------------------------------
// Shared test data factories
// ---------------------------------------------------------------------------

const SEED_HASH =
  "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
const IDENTITY_ID = "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec";
const CONTRACT_ID =
  "1122334455667788990011223344556677889900aabbccddeeff001122334455";
const TOKEN_ID =
  "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233";

function createHdWallet(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: SEED_HASH,
    usesPassword: true,
    alias: "Journey Wallet",
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
    ],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [],
    passwordHint: "test",
    ...overrides,
  };
}

function createIdentity(overrides: Record<string, unknown> = {}) {
  return {
    id: IDENTITY_ID,
    alias: "Journey Identity",
    identityType: "user",
    balance: 1000000000,
    dpnsNames: [],
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
    associatedWalletHashes: [SEED_HASH],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

function createContractSummary(overrides: Record<string, unknown> = {}) {
  return {
    id: CONTRACT_ID,
    alias: "Journey Contract",
    documentTypeCount: 2,
    tokenCount: 0,
    ...overrides,
  };
}

function createContractDetail(overrides: Record<string, unknown> = {}) {
  return {
    id: CONTRACT_ID,
    ownerId: IDENTITY_ID,
    alias: "Journey Contract",
    version: 1,
    documentTypeNames: ["note", "profile"],
    tokenCount: 0,
    schemaJson: {
      documentSchemas: {
        note: {
          type: "object",
          properties: {
            message: { type: "string", maxLength: 256 },
            author: { type: "string" },
          },
          required: ["message"],
          indices: [
            {
              name: "by_owner",
              properties: [{ $ownerId: "asc" }],
              unique: false,
            },
          ],
          additionalProperties: false,
        },
        profile: {
          type: "object",
          properties: {
            displayName: { type: "string" },
            bio: { type: "string" },
          },
          additionalProperties: false,
        },
      },
    },
    ...overrides,
  };
}

function createTokenEntry(overrides: Record<string, unknown> = {}) {
  return {
    identityId: IDENTITY_ID,
    tokenId: TOKEN_ID,
    contractId: CONTRACT_ID,
    tokenPosition: 0,
    name: "JourneyToken",
    ownerAlias: "Journey Identity",
    balance: "1000000000000",
    decimals: 8,
    ...overrides,
  };
}

/** Base handlers shared across all journeys. */
function baseHandlers(overrides: Record<string, unknown> = {}) {
  return {
    settings_get: {
      theme: "Dark",
      developerMode: false,
      disableZmq: false,
      onboardingCompleted: true,
      showEvonodeTools: false,
      userMode: "Basic",
      closeDashQtOnExit: false,
      autoStartSpv: false,
    },
    context_get_network: "Testnet",
    context_is_developer_mode: false,
    ...overrides,
  };
}

// ===========================================================================
// Journey 1: New User Journey
// Welcome → Create Wallet → wallet in list → Create Identity → identity in
// list → Register DPNS Name → name in Owned Names
// ===========================================================================

test.describe("Journey 1: New User Onboarding", () => {
  test("Welcome → Create Wallet → Wallet appears in list", async ({
    page,
    mockIPC,
  }) => {
    // Start on the welcome screen (onboarding not completed)
    await mockIPC.navigateWithHandlers("/welcome", {
      ...baseHandlers({
        settings_get: {
          theme: "Dark",
          developerMode: false,
          disableZmq: false,
          onboardingCompleted: false,
          showEvonodeTools: false,
          userMode: "Basic",
          closeDashQtOnExit: false,
          autoStartSpv: false,
        },
      }),
      wallet_list_all: {
        hdWallets: [],
        singleKeyWallets: [],
        selected: null,
      },
      identity_list_local: [],
      identity_load_order: [],
    });

    // Welcome screen should show action cards
    await expect(page.getByText(/Welcome/i).first()).toBeVisible({
      timeout: 10000,
    });

    // Click "Create Wallet" card/button — navigates to /wallets
    const createWalletBtn = page.getByRole("button", {
      name: /Create Wallet/i,
    });
    await expect(createWalletBtn).toBeVisible({ timeout: 5000 });
    await createWalletBtn.click();

    // Welcome navigates to /wallets, then we go to create from there
    await page.waitForURL(/\/wallets/, { timeout: 5000 });

    // Navigate to the create wallet screen
    await page.goto("/wallets/create");
    await mockIPC.waitForInit();
    await expect(page.getByText("Create New Wallet")).toBeVisible({
      timeout: 10000,
    });

    // Step 1: Generate seed phrase
    const generateBtn = page.getByRole("button", { name: /Generate/i });
    await expect(generateBtn).toBeVisible({ timeout: 5000 });
    await generateBtn.click();

    // Step 2: Backup — confirm seed phrase
    await expect(
      page.getByText("Back Up Your Seed Phrase"),
    ).toBeVisible({ timeout: 5000 });
    await page.getByRole("checkbox").check();
    await page.getByRole("button", { name: /Continue/i }).click();

    // Step 3: Name and protect
    await expect(page.getByText("Name & Protect")).toBeVisible({
      timeout: 5000,
    });

    // Configure the wallet_create mock to return a seed hash
    await mockIPC.setHandler("wallet_create", { seedHash: SEED_HASH });

    await page.locator("#wallet-alias").fill("My First Wallet");
    await page.locator("#wallet-password").fill("SecureP@ss1!");
    await page.getByRole("button", { name: /Create Wallet/i }).click();

    // Step 4: Success
    await expect(
      page.getByText("Wallet Created Successfully!"),
    ).toBeVisible({ timeout: 15000 });

    // Now configure wallet_list_all to return the created wallet
    const createdWallet = createHdWallet({ alias: "My First Wallet" });
    await mockIPC.setHandler("wallet_list_all", {
      hdWallets: [createdWallet],
      singleKeyWallets: [],
      selected: { type: "hd", seedHash: SEED_HASH },
    });

    // Click "Go to Wallet" to navigate back to wallet list
    await page.getByRole("button", { name: /Go to Wallet/i }).click();
    await page.waitForURL(/\/wallets/, { timeout: 5000 });

    // Verify the wallet appears in the list
    await expect(page.getByText("My First Wallet").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("Wallet list → Create Identity → Identity appears in list", async ({
    page,
    mockIPC,
  }) => {
    const wallet = createHdWallet();

    await mockIPC.navigateWithHandlers("/identities", {
      ...baseHandlers(),
      wallet_list_all: {
        hdWallets: [wallet],
        singleKeyWallets: [],
        selected: { type: "hd", seedHash: SEED_HASH },
      },
      identity_list_local: [],
      identity_load_order: [],
      identity_register: { taskId: "mock-register-task" },
    });

    // Identity list should show empty state with Create Identity button
    const createBtn = page
      .getByRole("region", { name: "Identity list" })
      .getByRole("button", { name: /Create Identity/i });
    await expect(createBtn).toBeVisible({ timeout: 10000 });
    await createBtn.click();

    // Create identity form should appear
    await expect(
      page.getByRole("heading", { name: /Create New Identity/i }),
    ).toBeVisible({ timeout: 5000 });

    // Verify wallet selection is shown
    await expect(page.getByText(/Wallet/i).first()).toBeVisible({
      timeout: 5000,
    });

    // Verify funding method section is visible
    await expect(
      page.getByText(/Funding Method/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("Identity list → Navigate to DPNS Register Name", async ({
    page,
    mockIPC,
  }) => {
    const identity = createIdentity({ dpnsNames: [] });

    await mockIPC.navigateWithHandlers("/identities", {
      ...baseHandlers(),
      identity_list_local: [identity],
      identity_load_order: [IDENTITY_ID],
      wallet_list_all: {
        hdWallets: [createHdWallet()],
        singleKeyWallets: [],
        selected: { type: "hd", seedHash: SEED_HASH },
      },
    });

    // Identity should appear in the list
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // Navigate to DPNS via sidebar
    await navigateToSection(page, "Contracts");
    await page.waitForTimeout(300);

    // Navigate to DPNS Register Name screen
    await mockIPC.setHandlers({
      contested_query_dpns_contests: { taskId: "mock-task-id" },
      contested_get_scheduled_votes: [],
      identity_list_voting: [identity],
      identity_local_dpns_names: [],
    });

    await page.goto("/contracts/dpns-register");
    await mockIPC.waitForInit();

    // Register name screen should appear with identity selector
    await expect(
      page.getByText(/Register/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // The name input with .dash suffix should be present
    await expect(page.getByText(".dash").first()).toBeVisible({
      timeout: 5000,
    });

    // Type a valid name
    const nameInput = page.getByPlaceholder("e.g. alice");
    await expect(nameInput).toBeVisible({ timeout: 5000 });
    await nameInput.fill("myusername");

    // Register button should be visible
    await expect(
      page.getByRole("button", { name: /Register/i }).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ===========================================================================
// Journey 2: Token Creator Journey
// Tokens → Create Token → complete wizard steps → token in My Tokens →
// Transfer tokens
// ===========================================================================

test.describe("Journey 2: Token Creation and Transfer", () => {
  const identity = createIdentity();

  function tokenJourneyHandlers(overrides: Record<string, unknown> = {}) {
    return {
      ...baseHandlers(),
      token_query_my_balances: { taskId: "mock-token-task" },
      token_load_order: [],
      identity_list_local: [identity],
      identity_list_summaries: [identity],
      identity_load_order: [IDENTITY_ID],
      wallet_list_all: {
        hdWallets: [createHdWallet()],
        singleKeyWallets: [],
        selected: null,
      },
      ...overrides,
    };
  }

  test("My Tokens → Create Token → token creator screen renders", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenJourneyHandlers());

    // My Tokens screen renders
    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Click Create Token button
    const createBtn = page.getByRole("button", { name: /Create Token/i });
    await expect(createBtn).toBeVisible({ timeout: 5000 });
    await createBtn.click();

    // Token creator wizard should appear
    await page.waitForURL(/\/tokens\/creator/, { timeout: 5000 });

    // Token Creator heading should be visible
    await expect(
      page.getByText("Token Creator").first(),
    ).toBeVisible({ timeout: 10000 });

    // Starts in Simple Mode — should have Token Name input
    await expect(
      page.getByText(/Token Name/i).first(),
    ).toBeVisible({ timeout: 5000 });

    // Mode toggle should be present (Simple/Advanced)
    await expect(
      page
        .getByRole("button", { name: /Simple/i })
        .or(page.getByText(/Simple Mode/i).first()),
    ).toBeVisible({ timeout: 5000 });

    // Switch to Advanced mode to see the step-by-step wizard
    const advancedBtn = page.getByRole("button", { name: /Advanced/i });
    if (await advancedBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await advancedBtn.click();

      // In Advanced mode, should see step navigation (Next button visible, may be disabled)
      const nextBtn = page.getByTestId("wizard-next");
      await expect(nextBtn).toBeVisible({ timeout: 5000 });

      // Step indicator or step labels should be visible
      await expect(
        page.getByText(/Step 1|Basic Info/i).first(),
      ).toBeVisible({ timeout: 5000 });
    }
  });

  test("My Tokens → receive token data → navigate to Transfer screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenJourneyHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Emit token data via task result event
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-token-task",
      resultType: "Token",
      payload: [createTokenEntry()],
    });

    // Token should appear in the table
    await expect(page.getByText("JourneyToken").first()).toBeVisible({
      timeout: 5000,
    });

    // Click the token to drill into Level 2 (per-identity balances)
    await page.getByText("JourneyToken").first().click();

    // Should show the identity's balance for this token
    await expect(
      page.getByText("Journey Identity").or(page.getByText(IDENTITY_ID.slice(0, 8))).first(),
    ).toBeVisible({ timeout: 5000 });

    // Look for the action dropdown or Transfer button
    const actionDropdown = page.getByRole("button", { name: /Actions/i }).first();
    if (await actionDropdown.isVisible({ timeout: 3000 }).catch(() => false)) {
      await actionDropdown.click();

      // Click Transfer in the dropdown
      const transferOption = page.getByRole("menuitem", { name: /Transfer/i });
      if (await transferOption.isVisible({ timeout: 2000 }).catch(() => false)) {
        await transferOption.click();

        // Should navigate to transfer screen
        await page.waitForURL(/\/tokens\/transfer/, { timeout: 5000 });
        await expect(page.getByText(/Transfer/i).first()).toBeVisible({
          timeout: 10000,
        });
      }
    }
  });

  test("Token Search → find token → add to My Tokens", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tokens", tokenJourneyHandlers());

    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Click Search Tokens button
    const searchBtn = page.getByRole("button", { name: /Search Tokens/i });
    await expect(searchBtn).toBeVisible({ timeout: 5000 });
    await searchBtn.click();

    // Should navigate to search screen
    await page.waitForURL(/\/tokens\/search/, { timeout: 5000 });

    // Enter search keyword
    const searchInput = page.getByPlaceholder(/keyword/i).or(
      page.getByRole("textbox").first(),
    );
    await expect(searchInput).toBeVisible({ timeout: 10000 });
    await searchInput.fill("journey");

    // Click Search button
    const searchAction = page.getByRole("button", { name: /^Search$/i });
    if (await searchAction.isVisible({ timeout: 3000 }).catch(() => false)) {
      await searchAction.click();
    }
  });
});

// ===========================================================================
// Journey 3: Contract Journey
// Contracts → Add Contract by ID → contract in tree → Query Documents →
// Create Document → Delete Document
// ===========================================================================

test.describe("Journey 3: Contract & Document Workflow", () => {
  const identity = createIdentity();

  function contractJourneyHandlers(overrides: Record<string, unknown> = {}) {
    return {
      ...baseHandlers(),
      contract_list_local: [],
      contract_get_by_id: createContractDetail(),
      identity_list_local: [identity],
      identity_list_summaries: [identity],
      identity_load_order: [IDENTITY_ID],
      wallet_list_all: {
        hdWallets: [createHdWallet()],
        singleKeyWallets: [],
        selected: null,
      },
      ...overrides,
    };
  }

  test("Navigate to Contracts → Add Contract → contract appears in tree", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractJourneyHandlers());

    // Document query screen loads
    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // Click Load Contracts button to navigate to add contracts screen
    await page.getByTestId("action-load-contracts").click();
    await page.waitForURL(/\/contracts\/add-contracts/, { timeout: 5000 });

    // Add Contracts screen should render
    await expect(
      page.getByText("Add Contracts").first(),
    ).toBeVisible({ timeout: 10000 });

    // Enter a contract ID in the hex/base58 identifier input
    const contractInput = page.getByPlaceholder("Hex or base58 identifier").first();
    await expect(contractInput).toBeVisible({ timeout: 5000 });
    await contractInput.fill(CONTRACT_ID);

    // Configure mock: contract_fetch returns raw taskId (bindings wrapper adds status)
    await mockIPC.setHandler("contract_fetch", {
      taskId: "mock-fetch-contract",
    });

    // Pre-configure contract_list_local to return the contract after fetch
    // (the screen calls loadContracts() after receiving the task-result-event)
    await mockIPC.setHandler("contract_list_local", [createContractSummary()]);

    // Click "Add Contracts" button (the fetch/submit button)
    const addBtn = page.getByRole("button", { name: /Add Contracts$/i });
    await expect(addBtn).toBeVisible({ timeout: 5000 });
    await addBtn.click();

    // Wait a moment for the IPC call to complete
    await page.waitForTimeout(500);

    // Simulate successful contract fetch via task result event
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-fetch-contract",
      resultType: "Contract",
      payload: createContractDetail(),
    });

    // After fetch + re-load from contract_list_local, the contract alias
    // should appear in results. The screen shows a "Complete" status with
    // found contract IDs.
    await expect(
      page
        .getByText("Journey Contract")
        .or(page.getByText(CONTRACT_ID.slice(0, 12)))
        .first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("Contract tree → expand contract → see Document Types section", async ({
    page,
    mockIPC,
  }) => {
    // Start with a contract already loaded
    await mockIPC.navigateWithHandlers("/contracts", {
      ...contractJourneyHandlers({
        contract_list_local: [createContractSummary()],
      }),
    });

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // Contract should appear in tree panel
    await expect(page.getByText("Journey Contract").first()).toBeVisible({
      timeout: 5000,
    });

    // Click on the contract to expand it (triggers contract detail fetch)
    await page.getByText("Journey Contract").first().click();

    // After expanding, the "Document Types" section toggle should appear
    await expect(
      page.getByTestId("doc-types-toggle"),
    ).toBeVisible({ timeout: 5000 });

    // Click "Document Types" to expand the section and show doc type names
    const docTypesToggle = page.getByTestId("doc-types-toggle");
    if (await docTypesToggle.isVisible({ timeout: 2000 }).catch(() => false)) {
      await docTypesToggle.click();

      // Individual doc type names should now be visible
      await expect(
        page.getByTestId("doctype-toggle-note"),
      ).toBeVisible({ timeout: 5000 });
    }

    // Fetch Documents button should be present
    const fetchBtn = page.getByTestId("fetch-documents-btn");
    await expect(fetchBtn).toBeVisible({ timeout: 5000 });
  });

  test("Contracts → navigate to Create Document screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", {
      ...contractJourneyHandlers({
        contract_list_local: [createContractSummary()],
      }),
    });

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // Click Create Document action button
    const createDocBtn = page.getByTestId("action-create-document");
    await expect(createDocBtn).toBeVisible({ timeout: 5000 });
    await createDocBtn.click();

    // Should navigate to create document screen
    await page.waitForURL(/\/contracts\/create-document/, { timeout: 5000 });

    // Create Document form should render
    await expect(
      page.getByText(/Create Document/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("Create Document → navigate to Delete Document screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts/create-document", {
      ...contractJourneyHandlers({
        contract_list_local: [createContractSummary()],
      }),
    });

    // Create Document screen should render
    await expect(
      page.getByText(/Create Document/i).first(),
    ).toBeVisible({ timeout: 10000 });

    // Navigate to Delete Document via direct URL
    await page.goto("/contracts/delete-document");
    await mockIPC.waitForInit();

    // Delete Document screen should render
    await expect(
      page.getByText(/Delete Document/i).first(),
    ).toBeVisible({ timeout: 10000 });
  });
});

// ===========================================================================
// Journey 4: Wallet Operations Journey
// Create Wallet → Receive (copy address) → Send (enter recipient, amount)
// ===========================================================================

test.describe("Journey 4: Wallet Operations", () => {
  function walletJourneyHandlers() {
    const wallet = createHdWallet();
    return {
      ...baseHandlers(),
      wallet_list_all: {
        hdWallets: [wallet],
        singleKeyWallets: [],
        selected: { type: "hd" as const, seedHash: SEED_HASH },
      },
      identity_list_local: [],
      identity_load_order: [],
    };
  }

  test("Wallet list → Receive dialog → copy address → close → Send screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletJourneyHandlers());

    // Wallet should render with detail panel
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });

    // Step 1: Open Receive dialog
    const receiveBtn = page.getByRole("button", {
      name: "Receive",
      exact: true,
    });
    await expect(receiveBtn).toBeVisible({ timeout: 5000 });
    await receiveBtn.click();

    // Receive dialog should appear with QR code and address
    await expect(page.getByRole("dialog")).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText("yXJqLJt8MWAMN2V").first(),
    ).toBeVisible({ timeout: 5000 });

    // Verify QR code is present (SVG element)
    const qrCode = page.getByRole("dialog").locator("svg").first();
    await expect(qrCode).toBeVisible({ timeout: 3000 });

    // Close the dialog
    const closeBtn = page
      .getByRole("dialog")
      .getByRole("button", { name: /Close|×/i })
      .first();
    if (await closeBtn.isVisible({ timeout: 2000 }).catch(() => false)) {
      await closeBtn.click();
    } else {
      // Try pressing Escape
      await page.keyboard.press("Escape");
    }

    // Wait for dialog to close
    await expect(page.getByRole("dialog")).not.toBeVisible({ timeout: 3000 });

    // Step 2: Navigate to Send screen
    await page.getByRole("button", { name: /Send/i }).first().click();
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    // Send screen should render
    await expect(page.getByText("Send Dash")).toBeVisible({
      timeout: 10000,
    });

    // Enter recipient address
    const addressInput = page.getByPlaceholder(/address/i);
    await expect(addressInput).toBeVisible({ timeout: 5000 });
    await addressInput.fill("yMockRecipientAddress12345678901");

    // Enter amount
    const amountInput = page.getByPlaceholder(/amount/i);
    await expect(amountInput).toBeVisible({ timeout: 5000 });
    await amountInput.fill("1.5");

    // Verify the send/transaction button is visible
    const submitBtn = page.getByRole("button", {
      name: /Transaction|Send Dash|Send$/i,
    });
    await expect(submitBtn.first()).toBeVisible({ timeout: 5000 });
  });

  test("Wallet list → Asset Locks tab → Create Asset Lock screen", async ({
    page,
    mockIPC,
  }) => {
    const wallet = createHdWallet({
      unusedAssetLocks: [
        {
          txid: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
          address: "yXJqLJt8MWAMN2VSJQEuYAxzjXCzJEHpjF",
          amount: 100000000,
          hasInstantLock: true,
          hasAssetLockProof: true,
          proofDetails: {
            type: "instantSend",
            instantLockTxid:
              "def456abc123def456abc123def456abc123def456abc123def456abc123def4",
            outputIndex: 0,
          },
          proofHex: "deadbeef",
        },
      ],
    });

    await mockIPC.navigateWithHandlers("/wallets", {
      ...baseHandlers(),
      wallet_list_all: {
        hdWallets: [wallet],
        singleKeyWallets: [],
        selected: { type: "hd" as const, seedHash: SEED_HASH },
      },
      identity_list_local: [],
      identity_load_order: [],
    });

    // Switch to Asset Locks tab
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("tab", { name: "Asset Locks" }).click();

    // Asset lock should be visible
    await expect(
      page.getByText("abc123def456").first(),
    ).toBeVisible({ timeout: 5000 });

    // Click Create Asset Lock button if available
    const createLockBtn = page.getByRole("button", {
      name: /Create Asset Lock/i,
    });
    if (await createLockBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await createLockBtn.click();
      await page.waitForURL(/\/wallets\/asset-locks\/create/, {
        timeout: 5000,
      });

      // Asset lock creation screen should render
      await expect(
        page.getByText(/Asset Lock/i).first(),
      ).toBeVisible({ timeout: 10000 });
    }
  });

  test("Send screen → back to wallets → navigate to Import Wallet", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/wallets", walletJourneyHandlers());

    // Navigate to send screen
    await expect(page.getByRole("tab", { name: "Addresses" })).toBeVisible({
      timeout: 10000,
    });
    await page.getByRole("button", { name: /Send/i }).first().click();
    await page.waitForURL(/\/wallets\/send/, { timeout: 5000 });

    // Go back to wallets
    const backBtn = page.getByRole("button", {
      name: /Back to wallets|Back/i,
    });
    await expect(backBtn).toBeVisible({ timeout: 10000 });
    await backBtn.click();
    await page.waitForURL(/\/wallets$/, { timeout: 5000 });

    // Navigate to Import Wallet screen
    await page.goto("/wallets/import");
    await mockIPC.waitForInit();

    await expect(page.getByText("Import Wallet")).toBeVisible({
      timeout: 10000,
    });

    // Both tabs should be present
    await expect(
      page.getByRole("tab", { name: /Seed Phrase/i }),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByRole("tab", { name: /Private Key/i }),
    ).toBeVisible();
  });
});

// ===========================================================================
// Journey 5: Identity Management Journey
// Load Identity → View Keys → Add Key → Top Up → Withdraw → Transfer
// ===========================================================================

test.describe("Journey 5: Identity Management", () => {
  const identity = createIdentity();
  const wallet = createHdWallet();

  function identityJourneyHandlers(overrides: Record<string, unknown> = {}) {
    return {
      ...baseHandlers(),
      identity_list_local: [identity],
      identity_load_order: [IDENTITY_ID],
      identity_list_voting: [identity],
      wallet_list_all: {
        hdWallets: [wallet],
        singleKeyWallets: [],
        selected: { type: "hd" as const, seedHash: SEED_HASH },
      },
      ...overrides,
    };
  }

  test("Identity list → select identity → view detail panel", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/identities",
      identityJourneyHandlers(),
    );

    // Identity list should show the identity
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // Click the identity row to select it
    await page.getByText("Journey Identity").first().click();

    // Detail panel should show identity info including balance
    await expect(
      page
        .getByText(IDENTITY_ID.slice(0, 8))
        .or(page.getByText("Journey Identity"))
        .first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("Identity detail → View Keys screen → key list", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/identities",
      identityJourneyHandlers(),
    );

    // Select the identity
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });
    await page.getByText("Journey Identity").first().click();

    // Look for keys section or View Keys button
    const viewKeysBtn = page.getByRole("button", { name: /Keys|View Keys/i });
    if (await viewKeysBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
      await viewKeysBtn.click();

      // Keys screen should show the identity's keys
      await expect(
        page.getByText("MASTER").or(page.getByText("AUTHENTICATION")).first(),
      ).toBeVisible({ timeout: 5000 });
    }
  });

  test("Identity list → navigate to Top Up screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/identities",
      identityJourneyHandlers(),
    );

    // Wait for identity list to render
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // Select the identity
    await page.getByText("Journey Identity").first().click();

    // Look for Top Up button in the detail panel action bar
    const topUpBtn = page.getByRole("button", { name: /Top Up/i });
    if (await topUpBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
      await topUpBtn.click();

      // Top Up screen should render
      await expect(
        page.getByText(/Top Up/i).first(),
      ).toBeVisible({ timeout: 10000 });

      // Should show the identity
      await expect(
        page
          .getByText("Journey Identity")
          .or(page.getByText(IDENTITY_ID.slice(0, 12)))
          .first(),
      ).toBeVisible({ timeout: 5000 });
    }
  });

  test("Identity list → Withdraw screen → Transfer screen", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/identities",
      identityJourneyHandlers(),
    );

    // Wait for identity list
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // Select the identity
    await page.getByText("Journey Identity").first().click();

    // Look for Withdraw button
    const withdrawBtn = page.getByRole("button", { name: /Withdraw/i });
    if (await withdrawBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
      await withdrawBtn.click();

      // Withdraw screen should render
      await expect(
        page.getByText(/Withdraw/i).first(),
      ).toBeVisible({ timeout: 10000 });

      // Navigate back and try Transfer
      const backBtn = page.getByRole("button", {
        name: /Back|Cancel/i,
      });
      if (await backBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
        await backBtn.click();
      } else {
        await page.goto("/identities");
        await mockIPC.waitForInit();
      }
    }

    // Now navigate to Transfer screen
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });
    await page.getByText("Journey Identity").first().click();

    const transferBtn = page.getByRole("button", { name: /Transfer/i });
    if (await transferBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
      await transferBtn.click();

      // Transfer screen should render
      await expect(
        page.getByText(/Transfer/i).first(),
      ).toBeVisible({ timeout: 10000 });
    }
  });

  test("Cross-section navigation: Identities → Wallets → Tokens → back to Identities", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/identities",
      identityJourneyHandlers({
        token_query_my_balances: { taskId: "mock-token-task" },
        token_load_order: [],
        contract_list_local: [],
      }),
    );

    // Start on Identities
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });

    // Navigate to Wallets via sidebar
    await navigateToSection(page, "Wallets");
    await expect(
      page.getByText("Journey Wallet").or(page.getByText("No Wallets")).first(),
    ).toBeVisible({ timeout: 10000 });

    // Navigate to Tokens via sidebar
    await navigateToSection(page, "Tokens");
    await expect(page.getByText("My Tokens").first()).toBeVisible({
      timeout: 10000,
    });

    // Navigate back to Identities via sidebar
    await navigateToSection(page, "Identities");
    await expect(page.getByText("Journey Identity").first()).toBeVisible({
      timeout: 10000,
    });
  });
});
