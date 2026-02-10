/**
 * Phase 9 — Tools Screens Smoke Tests
 *
 * Verifies tools landing page, Platform Info, Address Balance, Contract
 * Visualizer, and Document Visualizer screens render and function correctly
 * with mock IPC data. Placeholder screens (Proof Log, Transition Visualizer,
 * Proof Visualizer, Masternode List Diff, GroveSTARK) get basic render tests.
 */

import { test, expect } from "./fixtures";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Standard IPC handlers for tools screens. */
function toolsHandlers(overrides: Record<string, unknown> = {}) {
  return {
    settings_get: {
      theme: "Dark",
      developerMode: false,
      disableZmq: false,
      onboardingCompleted: true,
    },
    context_get_network: "Testnet",
    ...overrides,
  };
}

/** Create a mock contract summary DTO. */
function createContractSummary(overrides: Record<string, unknown> = {}) {
  return {
    id: "aabb112233445566778899aabb112233445566778899aabb112233445566aabb",
    alias: "Test Contract",
    documentTypeCount: 2,
    tokenCount: 0,
    ...overrides,
  };
}

/** Create a mock contract detail DTO. */
function createContractDetail(overrides: Record<string, unknown> = {}) {
  return {
    id: "aabb112233445566778899aabb112233445566778899aabb112233445566aabb",
    ownerId: "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
    alias: "Test Contract",
    version: 1,
    documentTypeNames: ["note", "profile"],
    tokenCount: 0,
    schemaJson: {},
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Tools Landing Page
// ---------------------------------------------------------------------------

test.describe("Tools Landing Page", () => {
  test("renders the Tools heading and subtitle", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText("Platform utilities and data inspection tools"),
    ).toBeVisible();
  });

  test("renders all three category headings", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(page.getByText("Query & Inspection")).toBeVisible({
      timeout: 10000,
    });
    await expect(page.getByText("Deserializers")).toBeVisible();
    await expect(page.getByText("Advanced")).toBeVisible();
  });

  test("renders all 9 tool cards", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    const cardNames = [
      "Platform Info",
      "Address Balance",
      "Proof Log",
      "Masternode List Diff",
      "Transition Visualizer",
      "Contract Visualizer",
      "Document Visualizer",
      "Proof Visualizer",
      "GroveSTARK",
    ];

    for (const name of cardNames) {
      await expect(
        page.getByRole("button", { name: new RegExp(name, "i") }),
      ).toBeVisible();
    }
  });

  test("renders tool descriptions on cards", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText(/fetch platform data/i),
    ).toBeVisible();
    await expect(
      page.getByText(/look up the balance/i),
    ).toBeVisible();
    await expect(
      page.getByText(/zero-knowledge proofs/i),
    ).toBeVisible();
  });

  test("navigates to Platform Info when card is clicked", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByRole("button", { name: /Platform Info/i })
      .click();

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 5000,
    });
  });

  test("navigates to Address Balance when card is clicked", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByRole("button", { name: /Address Balance/i })
      .click();

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to Contract Visualizer when card is clicked", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByRole("button", { name: /Contract Visualizer/i })
      .click();

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("navigates to Document Visualizer when card is clicked", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers("/tools", toolsHandlers());

    await expect(
      page.getByRole("heading", { name: "Tools" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByRole("button", { name: /Document Visualizer/i })
      .click();

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Platform Info Screen
// ---------------------------------------------------------------------------

test.describe("Platform Info Screen", () => {
  test("renders page title and empty state", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers(),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });
    await expect(
      page.getByText(/select a query from the left/i),
    ).toBeVisible();
  });

  test("renders all 7 query type cards", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers(),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    const queryTitles = [
      "Basic Platform Info",
      "Current Epoch Info",
      "Total Credits on Platform",
      "Version Voting State",
      "Validator Set Info",
      "Withdrawals in Queue",
      "Recently Completed Withdrawals",
    ];

    for (const title of queryTitles) {
      await expect(page.getByText(title)).toBeVisible();
    }
  });

  test("query cards have descriptions", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers(),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    await expect(
      page.getByText(/protocol version, fee version/i),
    ).toBeVisible();
    await expect(
      page.getByText(/current epoch protocol version/i),
    ).toBeVisible();
  });

  test("clicking a query card dispatches IPC and shows loading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers({
        platform_basic_info: { taskId: "mock-platform-task" },
      }),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    // Click the Basic Platform Info card
    await page.getByText("Basic Platform Info").click();

    // Loading indicator should appear
    await expect(page.getByText("Loading...")).toBeVisible({ timeout: 5000 });
  });

  test("shows result text when task result event is received", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers({
        platform_basic_info: { taskId: "mock-platform-task" },
      }),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    // Click query card
    await page.getByText("Basic Platform Info").click();
    await expect(page.getByText("Loading...")).toBeVisible({ timeout: 5000 });

    // Emit task result
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-platform-task",
      resultType: "Platform",
      payload: {
        type: "text",
        title: "Basic Platform Info",
        data: "Protocol Version: 7\nFee Version: 1\nChain Lock Height: 123456",
      },
    });

    // Result should be displayed
    await expect(
      page.getByText("Protocol Version: 7"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText("Chain Lock Height: 123456"),
    ).toBeVisible();
  });

  test("shows error when task error event is received", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers({
        platform_current_epoch_info: { taskId: "mock-epoch-task" },
      }),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    // Click epoch info card
    await page.getByText("Current Epoch Info").click();

    // Emit error
    await mockIPC.emitEvent("task-error-event", {
      taskId: "mock-epoch-task",
      message: "Network connection failed",
      details: "",
      retryable: false,
    });

    // Error should be visible
    await expect(
      page.getByText("Network connection failed"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("dismiss error button clears the error", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers({
        platform_total_credits: { taskId: "mock-credits-task" },
      }),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    await page.getByText("Total Credits on Platform").click();

    await mockIPC.emitEvent("task-error-event", {
      taskId: "mock-credits-task",
      message: "Some error occurred",
      details: "",
      retryable: false,
    });

    await expect(
      page.getByText("Some error occurred"),
    ).toBeVisible({ timeout: 5000 });

    // Click dismiss
    await page.getByRole("button", { name: /dismiss/i }).click();

    await expect(
      page.getByText("Some error occurred"),
    ).not.toBeVisible({ timeout: 3000 });
  });

  test("cards are disabled while a query is loading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/platform-info",
      toolsHandlers({
        platform_basic_info: { taskId: "mock-platform-task" },
      }),
    );

    await expect(page.getByText("Platform Information")).toBeVisible({
      timeout: 10000,
    });

    await page.getByText("Basic Platform Info").click();

    // Other cards should be disabled
    const epochCard = page.getByText("Current Epoch Info").locator("../..");
    await expect(epochCard).toBeDisabled({ timeout: 3000 });
  });
});

// ---------------------------------------------------------------------------
// Address Balance Screen
// ---------------------------------------------------------------------------

test.describe("Address Balance Screen", () => {
  test("renders page title and empty state", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers(),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/enter a platform address above/i),
    ).toBeVisible();
  });

  test("renders address input and Fetch Balance button", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers(),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByPlaceholder("evo1... or tevo1..."),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /fetch balance/i }),
    ).toBeVisible();
  });

  test("Fetch Balance button is disabled when input is empty", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers(),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("button", { name: /fetch balance/i }),
    ).toBeDisabled();
  });

  test("shows validation error for invalid address prefix", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers(),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    // Type invalid address
    await page
      .getByPlaceholder("evo1... or tevo1...")
      .fill("invalidaddress123");

    await expect(
      page.getByText(/address must start with/i),
    ).toBeVisible({ timeout: 3000 });
  });

  test("enables Fetch Balance for valid tevo1 address", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers(),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder("evo1... or tevo1...")
      .fill("tevo1abc123def456");

    await expect(
      page.getByRole("button", { name: /fetch balance/i }),
    ).toBeEnabled();
  });

  test("clicking Fetch Balance dispatches IPC and shows loading", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers({
        platform_fetch_address_balance: { taskId: "mock-balance-task" },
      }),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder("evo1... or tevo1...")
      .fill("tevo1abc123def456");
    await page
      .getByRole("button", { name: /fetch balance/i })
      .click();

    await expect(page.getByText("Fetching balance...")).toBeVisible({
      timeout: 5000,
    });
  });

  test("displays result when task result event is received", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers({
        platform_fetch_address_balance: { taskId: "mock-balance-task" },
      }),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder("evo1... or tevo1...")
      .fill("tevo1abc123def456");
    await page
      .getByRole("button", { name: /fetch balance/i })
      .click();

    // Emit result
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-balance-task",
      resultType: "Platform",
      payload: {
        type: "addressBalance",
        address: "tevo1abc123def456",
        balance: 500000000000,
        nonce: 42,
      },
    });

    // Result fields should appear
    await expect(page.getByText("Result")).toBeVisible({ timeout: 5000 });
    await expect(page.getByText("tevo1abc123def456")).toBeVisible();
    await expect(page.getByText(/42/)).toBeVisible();
  });

  test("shows error when task error event is received", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers({
        platform_fetch_address_balance: { taskId: "mock-balance-task" },
      }),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder("evo1... or tevo1...")
      .fill("tevo1abc123def456");
    await page
      .getByRole("button", { name: /fetch balance/i })
      .click();

    await mockIPC.emitEvent("task-error-event", {
      taskId: "mock-balance-task",
      message: "Address not found on platform",
      details: "",
      retryable: false,
    });

    await expect(
      page.getByText("Address not found on platform"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("label describes evo1/tevo1 prefix requirement", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/address-balance",
      toolsHandlers(),
    );

    await expect(
      page.getByText("Platform Address Balance Lookup"),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText(/enter a platform address.*evo1.*tevo1/i),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Contract Visualizer Screen
// ---------------------------------------------------------------------------

test.describe("Contract Visualizer Screen", () => {
  test("renders page title and awaiting input state", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(page.getByText(/awaiting input/i)).toBeVisible();
  });

  test("renders subtitle", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByText(/deserialize and inspect/i),
    ).toBeVisible({ timeout: 10000 });
  });

  test("renders HexInput textarea with label and placeholder", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText(/enter hex, base64, or comma-separated/i),
    ).toBeVisible();
    await expect(
      page.getByPlaceholder(/paste serialized contract bytes/i),
    ).toBeVisible();
  });

  test("renders Result label area", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(page.getByText("Result")).toBeVisible();
  });

  test("shows error for invalid input format", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    // Type something that is not valid hex, base64, or CSV bytes
    // "zzz!!!" is not valid in any format, so decodeToHex returns null
    await page
      .getByPlaceholder(/paste serialized contract bytes/i)
      .fill("zzz!!!");

    // Wait for debounced parse — client-side error should appear
    await expect(page.getByRole("alert")).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/unable to decode input/i),
    ).toBeVisible();
  });

  test("shows parsed JSON for valid contract input", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers({
        parse_data_contract: {
          json: '{"$id": "abc123", "ownerId": "def456", "documentSchemas": {"note": {"type": "object"}}}',
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized contract bytes/i)
      .fill("aabbccdd");

    // JSON viewer should render with contract data
    await expect(page.getByText("$id")).toBeVisible({ timeout: 5000 });
  });

  test("dismiss error button clears error state", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/contract-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Contract Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    // Enter invalid format to trigger client-side error
    await page
      .getByPlaceholder(/paste serialized contract bytes/i)
      .fill("zzz!!!");

    await expect(page.getByRole("alert")).toBeVisible({ timeout: 5000 });

    // Click dismiss
    await page.getByRole("button", { name: /dismiss error/i }).click();

    await expect(page.getByRole("alert")).not.toBeVisible({ timeout: 3000 });
  });
});

// ---------------------------------------------------------------------------
// Document Visualizer Screen
// ---------------------------------------------------------------------------

test.describe("Document Visualizer Screen", () => {
  test("renders page title and waiting-for-selection state", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [createContractSummary()],
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/select a contract and document type/i),
    ).toBeVisible();
  });

  test("renders subtitle", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [],
      }),
    );

    await expect(
      page.getByText(/deserialize and inspect.*document/i),
    ).toBeVisible({ timeout: 10000 });
  });

  test("renders Contract and Document Type selectors", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [createContractSummary()],
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByRole("combobox", { name: /select contract/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("combobox", { name: /select document type/i }),
    ).toBeVisible();
  });

  test("renders contract filter input", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [createContractSummary()],
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByPlaceholder(/filter contracts/i),
    ).toBeVisible();
  });

  test("renders HexInput area for document bytes", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [createContractSummary()],
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByPlaceholder(/paste serialized document bytes/i),
    ).toBeVisible();
  });

  test("document type selector is disabled when no contract selected", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [createContractSummary()],
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    // The document type select trigger should show "Pick a contract first"
    await expect(
      page.getByText(/pick a contract first/i),
    ).toBeVisible();
  });

  test("shows error for invalid input format", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/document-visualizer",
      toolsHandlers({
        contract_list_local: [createContractSummary()],
        contract_get_by_id: createContractDetail(),
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Document Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    // Select contract from dropdown
    await page
      .getByRole("combobox", { name: /select contract/i })
      .click();
    await page.getByText("Test Contract").click();

    // Wait for document types to load, then select
    await expect(
      page.getByRole("combobox", { name: /select document type/i }),
    ).toBeEnabled({ timeout: 5000 });
    await page
      .getByRole("combobox", { name: /select document type/i })
      .click();
    await page.getByRole("option", { name: "note" }).click();

    // Enter invalid data (not hex, base64, or CSV)
    await page
      .getByPlaceholder(/paste serialized document bytes/i)
      .fill("zzz!!!");

    // Client-side error should appear after debounce
    await expect(page.getByRole("alert")).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/unable to decode input/i),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Placeholder Screen Tests — verify navigation works for unimplemented tools
// ---------------------------------------------------------------------------

test.describe("Placeholder Tool Screens", () => {
  test("Proof Log renders placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });

  test("Transition Visualizer renders placeholder", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });

  test("Proof Visualizer renders placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });

  test("Masternode List Diff renders placeholder", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });

  test("GroveSTARK renders placeholder", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/will be implemented/i),
    ).toBeVisible();
  });
});
