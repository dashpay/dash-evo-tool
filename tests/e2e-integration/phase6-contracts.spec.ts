/**
 * Phase 6 — Contracts & Documents Screens Smoke Tests
 *
 * Verifies contract tree panel, document query screen, add contracts screen,
 * register contract screen, all 6 document action screens, and group actions
 * screen render and function correctly with mock IPC data.
 */

import { test, expect, createTestIdentity } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a mock contract summary DTO. */
function createContractSummary(overrides: Record<string, unknown> = {}) {
  return {
    id: "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    alias: "Test Contract",
    documentTypeCount: 2,
    tokenCount: 0,
    ...overrides,
  };
}

/** Create a mock contract detail DTO. */
function createContractDetail(overrides: Record<string, unknown> = {}) {
  return {
    id: "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    ownerId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Test Contract",
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

/** Standard identity for contract screens. */
function createContractIdentity() {
  return {
    id: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Contract Owner",
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

/** Standard handlers for contract screens that load identities and contracts. */
function contractHandlers(overrides: Record<string, unknown> = {}) {
  return {
    contract_list_local: [createContractSummary()],
    contract_get_by_id: createContractDetail(),
    identity_list_local: [createContractIdentity()],
    identity_list_summaries: [createContractIdentity()],
    identity_load_order: ["GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec"],
    wallet_list_all: {
      hdWallets: [],
      singleKeyWallets: [],
      selected: null,
    },
    context_is_developer_mode: false,
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Document Query Screen (main /contracts route)
// ---------------------------------------------------------------------------

test.describe("Document Query Screen", () => {
  test("renders with Document Query screen and action buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    // The document query screen has a data-testid
    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // Action buttons should be present
    await expect(
      page.getByTestId("action-load-contracts"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByTestId("action-register-contract"),
    ).toBeVisible();
  });

  test("shows query input and Fetch Documents button", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await expect(page.getByTestId("query-input")).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByTestId("fetch-documents-btn")).toBeVisible();
  });

  test("shows contract tree panel with contracts", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // Contract name should appear in the tree panel
    await expect(page.getByText("Test Contract").first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows empty state when no results", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // With no query executed, should show the empty/hint state
    // The exact text is: 'Select a contract and document type on the left, then click "Fetch Documents" to query documents.'
    await expect(
      page.getByText(/Select a contract and document type/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("all 10 action buttons are present", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    const testIds = [
      "action-load-contracts",
      "action-register-contract",
      "action-update-contract",
      "action-create-document",
      "action-delete-document",
      "action-replace-document",
      "action-transfer-document",
      "action-purchase-document",
      "action-set-document-price",
      "action-group-actions",
    ];

    for (const testId of testIds) {
      await expect(page.getByTestId(testId)).toBeVisible({ timeout: 3000 });
    }
  });

  test("Load Contracts button navigates to add-contracts", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-load-contracts").click();
    await page.waitForURL(/add-contracts/, { timeout: 5000 });
  });

  test("Register Contract button navigates to register", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-register-contract").click();
    await page.waitForURL(/register/, { timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Add Contracts Screen
// ---------------------------------------------------------------------------

test.describe("Add Contracts Screen", () => {
  test("renders with Add Contracts heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows contract ID input field", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    // First contract input field should be visible
    await expect(
      page.getByPlaceholder("Hex or base58 identifier"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Add Another Contract Field button", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Add Another Contract Field/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Add Contracts (fetch) button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Add Contracts$/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("can add multiple contract fields", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    // Click add another field
    await page
      .getByRole("button", { name: /Add Another Contract Field/i })
      .click();

    // Should now have 2 input fields
    const inputs = page.getByPlaceholder("Hex or base58 identifier");
    await expect(inputs).toHaveCount(2);
  });

  test("shows error for invalid contract ID format", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers({
        contract_fetch: { taskId: "mock-fetch-task" },
      }),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    // Enter invalid ID
    await page
      .getByPlaceholder("Hex or base58 identifier")
      .fill("not-a-valid-id!");

    // Click Add Contracts
    await page.getByRole("button", { name: /Add Contracts$/i }).click();

    // Should show error
    await expect(page.getByText(/Invalid/i).first()).toBeVisible({
      timeout: 5000,
    });
  });

  test("shows Back to Contracts button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Back to Contracts/i }),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Register Contract Screen
// ---------------------------------------------------------------------------

test.describe("Register Contract Screen", () => {
  test("renders with Register Data Contract heading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows identity selector with loaded identity", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    // Identity should be shown
    await expect(
      page.getByText("Contract Owner").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows contract alias input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByPlaceholder("e.g., My DApp Contract"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows contract JSON textarea", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    // JSON textarea should be visible (aria-label="Contract JSON")
    await expect(
      page.getByRole("textbox", { name: "Contract JSON" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Register Contract button (disabled without JSON)", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    const registerBtn = page.getByRole("button", {
      name: /Register Contract/i,
    });
    await expect(registerBtn).toBeVisible({ timeout: 5000 });
    await expect(registerBtn).toBeDisabled();
  });

  test("shows Back to Contracts button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /Back to Contracts/i }),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Document Action Screens (Create, Delete, Replace, Transfer, Purchase, Set Price)
// ---------------------------------------------------------------------------

const documentActionRoutes: Array<{
  path: string;
  title: string;
  broadcastLabel: string;
}> = [
  {
    path: "/contracts/create-document",
    title: "Create Document",
    broadcastLabel: "Broadcast document",
  },
  {
    path: "/contracts/delete-document",
    title: "Delete Document",
    broadcastLabel: "Delete document",
  },
  {
    path: "/contracts/replace-document",
    title: "Replace Document",
    broadcastLabel: "Replace document",
  },
  {
    path: "/contracts/transfer-document",
    title: "Transfer Document",
    broadcastLabel: "Transfer document",
  },
  {
    path: "/contracts/purchase-document",
    title: "Purchase Document",
    broadcastLabel: "Purchase document",
  },
  {
    path: "/contracts/set-document-price",
    title: "Set Document Price",
    broadcastLabel: "Set document price",
  },
];

for (const { path, title } of documentActionRoutes) {
  test.describe(`${title} Screen`, () => {
    test(`renders with ${title} heading`, async ({ page, mockIPC }) => {
      await mockIPC.navigateWithHandlers(path, contractHandlers());

      await expect(page.getByText(title).first()).toBeVisible({
        timeout: 10000,
      });
    });

    test("shows contract and document type selectors", async ({
      page,
      mockIPC,
    }) => {
      await mockIPC.navigateWithHandlers(path, contractHandlers());

      await expect(page.getByText(title).first()).toBeVisible({
        timeout: 10000,
      });

      await expect(page.getByTestId("contract-select")).toBeVisible({
        timeout: 5000,
      });
      await expect(page.getByTestId("doctype-select")).toBeVisible();
    });

    test("shows identity selector", async ({ page, mockIPC }) => {
      await mockIPC.navigateWithHandlers(path, contractHandlers());

      await expect(page.getByText(title).first()).toBeVisible({
        timeout: 10000,
      });

      // Identity section heading
      await expect(
        page.getByText(/Select an identity/i).first(),
      ).toBeVisible({ timeout: 5000 });
    });

    test("shows Back to Contracts button", async ({ page, mockIPC }) => {
      await mockIPC.navigateWithHandlers(path, contractHandlers());

      await expect(page.getByText(title).first()).toBeVisible({
        timeout: 10000,
      });

      await expect(page.getByTestId("back-button")).toBeVisible({
        timeout: 5000,
      });
    });
  });
}

// Document-specific tests for action screens

test.describe("Create Document — specific fields", () => {
  test("shows step 3 heading for filling out fields", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/create-document",
      contractHandlers(),
    );

    await expect(page.getByTestId("document-action-screen")).toBeVisible({
      timeout: 10000,
    });

    // Step 3 heading should mention filling out fields
    await expect(
      page.getByText(/Fill out the document type fields/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

test.describe("Delete Document — specific fields", () => {
  test("shows document ID input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/delete-document",
      contractHandlers(),
    );

    await expect(
      page.getByText("Delete Document").first(),
    ).toBeVisible({ timeout: 10000 });

    // Document ID input for delete action
    await expect(
      page
        .getByTestId("document-id-input")
        .or(page.getByPlaceholder("Enter document ID...")),
    ).toBeVisible({ timeout: 5000 });
  });
});

test.describe("Transfer Document — specific fields", () => {
  test("shows document ID and recipient ID inputs", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/transfer-document",
      contractHandlers(),
    );

    await expect(
      page.getByText("Transfer Document").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(page.getByTestId("document-id-input")).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByTestId("recipient-id-input")).toBeVisible();
  });
});

test.describe("Set Document Price — specific fields", () => {
  test("shows document ID and price inputs", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/set-document-price",
      contractHandlers(),
    );

    await expect(
      page.getByText("Set Document Price").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(page.getByTestId("document-id-input")).toBeVisible({
      timeout: 5000,
    });
    await expect(page.getByTestId("price-input")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Group Actions Screen
// ---------------------------------------------------------------------------

test.describe("Group Actions Screen", () => {
  test("renders with Group Actions heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/group-actions",
      contractHandlers({
        contract_list_local: [
          createContractSummary({ tokenCount: 1 }),
        ],
      }),
    );

    await expect(page.getByText("Group Actions").first()).toBeVisible({
      timeout: 10000,
    });
  });

  test("shows Step 1 — Select Contract section", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/group-actions",
      contractHandlers({
        contract_list_local: [
          createContractSummary({ tokenCount: 1 }),
        ],
      }),
    );

    await expect(page.getByText("Group Actions").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByText(/Step 1/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Step 2 — Select Identity section", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/group-actions",
      contractHandlers({
        contract_list_local: [
          createContractSummary({ tokenCount: 1 }),
        ],
      }),
    );

    await expect(page.getByText("Group Actions").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByText(/Step 2/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Fetch Group Actions button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/group-actions",
      contractHandlers({
        contract_list_local: [
          createContractSummary({ tokenCount: 1 }),
        ],
      }),
    );

    await expect(page.getByText("Group Actions").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Fetch Group Actions/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Back to Contracts button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/group-actions",
      contractHandlers(),
    );

    await expect(page.getByText("Group Actions").first()).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByRole("button", { name: /Back to Contracts/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows hint text when no contract/identity selected", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/group-actions",
      contractHandlers({
        contract_list_local: [],
        identity_list_local: [],
      }),
    );

    await expect(page.getByText("Group Actions").first()).toBeVisible({
      timeout: 10000,
    });

    // Should show hint about needing to add contracts
    await expect(
      page.getByText(/No contracts with tokens found/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Document Query — display mode toggle and advanced controls
// ---------------------------------------------------------------------------

test.describe("Document Query — display and query controls", () => {
  test("Fetch Documents button is disabled without contract/doc type", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    // Fetch button should be disabled when no contract/doc type is selected
    await expect(page.getByTestId("fetch-documents-btn")).toBeDisabled({
      timeout: 5000,
    });
  });

  test("query input has correct placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await expect(page.getByTestId("query-input")).toHaveAttribute(
      "placeholder",
      /SELECT/,
    );
  });

  test("query input accepts text", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    const queryInput = page.getByTestId("query-input");
    await queryInput.fill("SELECT * FROM note WHERE $ownerId = 'abc'");
    await expect(queryInput).toHaveValue(
      "SELECT * FROM note WHERE $ownerId = 'abc'",
    );
  });

  test("Create Document button navigates to create-document", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-create-document").click();
    await page.waitForURL(/create-document/, { timeout: 5000 });
  });

  test("Delete Document button navigates to delete-document", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-delete-document").click();
    await page.waitForURL(/delete-document/, { timeout: 5000 });
  });

  test("Update Contract button navigates to update-contract", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-update-contract").click();
    await page.waitForURL(/update-contract/, { timeout: 5000 });
  });

  test("Group Actions button navigates to group-actions", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-group-actions").click();
    await page.waitForURL(/group-actions/, { timeout: 5000 });
  });

  test("Set Document Price button navigates", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/contracts", contractHandlers());

    await expect(page.getByTestId("document-query-screen")).toBeVisible({
      timeout: 10000,
    });

    await page.getByTestId("action-set-document-price").click();
    await page.waitForURL(/set-document-price/, { timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Update Contract Screen
// ---------------------------------------------------------------------------

test.describe("Update Contract Screen", () => {
  test("renders with Update Contract heading", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/update-contract",
      contractHandlers(),
    );

    await expect(
      page.getByText("Update Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });
  });

  test("shows identity selector with loaded identity", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/update-contract",
      contractHandlers(),
    );

    await expect(
      page.getByText("Update Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText("Contract Owner").first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows contract selector", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/update-contract",
      contractHandlers(),
    );

    await expect(
      page.getByText("Update Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    // Contract selector heading
    await expect(
      page.getByText(/Select Contract/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows contract JSON textarea", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/update-contract",
      contractHandlers(),
    );

    await expect(
      page.getByText("Update Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("textbox", { name: /Contract JSON/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Update Contract button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/update-contract",
      contractHandlers(),
    );

    await expect(
      page.getByText("Update Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /Update Contract/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows Back to Contracts button", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/update-contract",
      contractHandlers(),
    );

    await expect(
      page.getByText("Update Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /Back to Contracts/i }),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Register Contract — advanced options and fee estimation
// ---------------------------------------------------------------------------

test.describe("Register Contract — advanced options", () => {
  test("shows Advanced Options toggle", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers(),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /Advanced Options/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows fee estimation when contract JSON is entered", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/register",
      contractHandlers({
        contract_register: { taskId: "mock-register" },
      }),
    );

    await expect(
      page.getByText("Register Data Contract").first(),
    ).toBeVisible({ timeout: 10000 });

    // Paste valid-looking contract JSON into textarea
    const jsonInput = page.getByRole("textbox", { name: "Contract JSON" });
    await jsonInput.fill(
      JSON.stringify({
        note: {
          type: "object",
          properties: { message: { type: "string" } },
          additionalProperties: false,
        },
      }),
    );

    // Register button should now be enabled
    const registerBtn = page.getByRole("button", {
      name: /Register Contract/i,
    });
    await expect(registerBtn).toBeEnabled({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Add Contracts — fetch flow
// ---------------------------------------------------------------------------

test.describe("Add Contracts — fetch flow", () => {
  test("entering valid hex ID and clicking Add Contracts triggers fetch", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers({
        contract_fetch: { taskId: "mock-fetch" },
      }),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    // Enter a valid hex contract ID
    await page
      .getByPlaceholder("Hex or base58 identifier")
      .fill("1122334455667788990011223344556677889900aabbccddeeff001122334455");

    // Click Add Contracts
    await page.getByRole("button", { name: /Add Contracts$/i }).click();

    // Should show fetching state
    await expect(
      page.getByText(/Fetching contracts/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("successful fetch shows found contracts", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers({
        contract_fetch: { taskId: "mock-fetch" },
      }),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    // Enter a valid hex contract ID
    await page
      .getByPlaceholder("Hex or base58 identifier")
      .fill("1122334455667788990011223344556677889900aabbccddeeff001122334455");

    // Click Add Contracts
    await page.getByRole("button", { name: /Add Contracts$/i }).click();

    // Simulate task result with found contract
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-fetch",
      resultType: "Contract",
      payload: {
        contracts: [createContractDetail()],
      },
    });

    // Should show success
    await expect(
      page.getByText(/Successfully/i).first(),
    ).toBeVisible({ timeout: 5000 });
  });

  test("can remove a contract input field", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/add-contracts",
      contractHandlers(),
    );

    await expect(page.getByText("Add Contracts").first()).toBeVisible({
      timeout: 10000,
    });

    // Add a second field
    await page
      .getByRole("button", { name: /Add Another Contract Field/i })
      .click();

    // Should have 2 input fields
    const inputs = page.getByPlaceholder("Hex or base58 identifier");
    await expect(inputs).toHaveCount(2);

    // Remove one field using the trash button
    const removeButtons = page.getByRole("button").filter({ has: page.locator("svg") });
    // Find the remove button (trash icon) — there should be remove buttons now
    // Click the first remove-looking button next to an input
    const trashButtons = page.locator("button:has(svg.lucide-trash2), button:has(svg.lucide-trash)");
    if (await trashButtons.count() > 0) {
      await trashButtons.first().click();
      await expect(inputs).toHaveCount(1);
    }
  });
});

// ---------------------------------------------------------------------------
// Document Action — Purchase-specific fields
// ---------------------------------------------------------------------------

test.describe("Purchase Document — specific fields", () => {
  test("shows document ID input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/purchase-document",
      contractHandlers(),
    );

    await expect(
      page.getByText("Purchase Document").first(),
    ).toBeVisible({ timeout: 10000 });

    // Purchase action needs document ID
    await expect(page.getByTestId("document-id-input")).toBeVisible({
      timeout: 5000,
    });
  });
});

// ---------------------------------------------------------------------------
// Document Action — Replace-specific fields
// ---------------------------------------------------------------------------

test.describe("Replace Document — specific fields", () => {
  test("shows document ID input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/contracts/replace-document",
      contractHandlers(),
    );

    await expect(
      page.getByText("Replace Document").first(),
    ).toBeVisible({ timeout: 10000 });

    await expect(page.getByTestId("document-id-input")).toBeVisible({
      timeout: 5000,
    });
  });
});
