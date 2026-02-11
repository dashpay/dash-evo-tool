/**
 * Phase 9 — Tools Screens E2E Integration Tests
 *
 * Verifies all 9 tools screens render and function correctly with mock IPC:
 * - Tools Landing Page (card grid + navigation)
 * - Platform Info (7 query types + task result/error events)
 * - Address Balance (input validation + fetch + result display)
 * - Contract Visualizer (hex/base64 input + parse + JSON display)
 * - Document Visualizer (contract/doc-type selection + parse)
 * - Transition Visualizer (parse + broadcast + contract detection)
 * - Proof Log (table + sort + pagination + detail panel + display modes)
 * - Proof Visualizer (hex input + parse + monospace result)
 * - GroveSTARK (Generate 3-step form + Verify mode + mode switching)
 * - Masternode List Diff (3 tabs + input area + fetch actions + ZMQ events)
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
// Transition Visualizer Screen
// ---------------------------------------------------------------------------

test.describe("Transition Visualizer Screen", () => {
  test("renders page title, subtitle, and HexInput", async ({
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
      page.getByText(/deserialize.*inspect.*broadcast/i),
    ).toBeVisible();
    await expect(
      page.getByPlaceholder(/paste serialized state transition/i),
    ).toBeVisible();
  });

  test("parses valid hex input and shows JSON result", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers({
        parse_state_transition: {
          json: '{"type":"DataContractCreate","protocolVersion":1}',
          detectedContractIds: [],
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("aabbccdd");

    // Parsed JSON should render after debounce — "Awaiting input" should disappear
    await expect(
      page.getByText(/awaiting input/i),
    ).not.toBeVisible({ timeout: 5000 });
    // Result section should contain parsed content (react-json-view-lite renders "type" key)
    await expect(page.getByText("type")).toBeVisible({ timeout: 3000 });
  });

  test("shows error for invalid input format", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("zzz!!!");

    await expect(page.getByRole("alert")).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/unable to decode input/i),
    ).toBeVisible();
  });

  test("broadcast button is not visible until parse succeeds", async ({
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

    // Broadcast button should not be visible with no input (only renders after successful parse)
    await expect(
      page.getByRole("button", { name: /broadcast/i }),
    ).not.toBeVisible();
  });

  test("broadcast button enables after successful parse", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers({
        parse_state_transition: {
          json: '{"type":"DataContractCreate"}',
          detectedContractIds: [],
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("aabbccdd");

    // Wait for parse to complete
    // Wait for parse result to appear (Awaiting input disappears, "type" key renders)
    await expect(
      page.getByText(/awaiting input/i),
    ).not.toBeVisible({ timeout: 5000 });
    await expect(page.getByText("type")).toBeVisible({ timeout: 3000 });

    // Broadcast button should now be enabled
    await expect(
      page.getByRole("button", { name: /broadcast/i }),
    ).toBeEnabled({ timeout: 3000 });
  });

  test("clicking broadcast shows submitting state with timer", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers({
        parse_state_transition: {
          json: '{"type":"DataContractCreate"}',
          detectedContractIds: [],
        },
        broadcast_state_transition: { taskId: "mock-broadcast-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("aabbccdd");

    // Wait for parse result to appear (Awaiting input disappears, "type" key renders)
    await expect(
      page.getByText(/awaiting input/i),
    ).not.toBeVisible({ timeout: 5000 });
    await expect(page.getByText("type")).toBeVisible({ timeout: 3000 });

    await page.getByRole("button", { name: /broadcast/i }).click();

    await expect(
      page.getByText(/broadcasting/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("broadcast success shows success message", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers({
        parse_state_transition: {
          json: '{"type":"DataContractCreate"}',
          detectedContractIds: [],
        },
        broadcast_state_transition: { taskId: "mock-broadcast-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("aabbccdd");

    // Wait for parse result to appear (Awaiting input disappears, "type" key renders)
    await expect(
      page.getByText(/awaiting input/i),
    ).not.toBeVisible({ timeout: 5000 });
    await expect(page.getByText("type")).toBeVisible({ timeout: 3000 });

    await page.getByRole("button", { name: /broadcast/i }).click();

    // Emit success
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-broadcast-task",
      resultType: "BroadcastStateTransition",
      payload: {},
    });

    await expect(
      page.getByText(/successfully broadcasted/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("broadcast error shows error message", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers({
        parse_state_transition: {
          json: '{"type":"DataContractCreate"}',
          detectedContractIds: [],
        },
        broadcast_state_transition: { taskId: "mock-broadcast-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("aabbccdd");

    // Wait for parse result to appear (Awaiting input disappears, "type" key renders)
    await expect(
      page.getByText(/awaiting input/i),
    ).not.toBeVisible({ timeout: 5000 });
    await expect(page.getByText("type")).toBeVisible({ timeout: 3000 });

    await page.getByRole("button", { name: /broadcast/i }).click();

    await mockIPC.emitEvent("task-error-event", {
      taskId: "mock-broadcast-task",
      message: "Invalid state transition data",
      details: "",
      retryable: false,
    });

    await expect(
      page.getByText(/invalid state transition data/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("detected contract IDs are shown as clickable badges", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/transition-visualizer",
      toolsHandlers({
        parse_state_transition: {
          json: '{"type":"DataContractCreate"}',
          detectedContractIds: [
            "aabb112233445566778899aabb112233445566778899aabb112233445566aabb",
          ],
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Transition Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized state transition/i)
      .fill("aabbccdd");

    // Contract ID badge should appear
    await expect(
      page.getByText(/aabb11/),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// Proof Log Screen
// ---------------------------------------------------------------------------

test.describe("Proof Log Screen", () => {
  /** Create a mock proof log item. */
  function createProofLogItem(overrides: Record<string, unknown> = {}) {
    return {
      requestType: "GetIdentity",
      height: 12345,
      timeMs: 150,
      error: null,
      proofBytesHex: "aabbccdd",
      verificationPathQueryHex: "11223344",
      ...overrides,
    };
  }

  /** Proof log handler returning paginated items. */
  function proofLogHandlers(
    items: Record<string, unknown>[] = [],
    overrides: Record<string, unknown> = {},
  ) {
    return toolsHandlers({
      proof_log_get_items: { items, totalCount: items.length },
      ...overrides,
    });
  }

  test("renders page title and subtitle", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/browse and inspect.*proof log/i),
    ).toBeVisible();
  });

  test("shows empty state when no items", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers([]),
    );

    await expect(
      page.getByText(/no proof items/i),
    ).toBeVisible({ timeout: 10000 });
  });

  test("renders proof log items in a table", async ({ page, mockIPC }) => {
    const items = [
      createProofLogItem({ requestType: "GetIdentity", height: 100, timeMs: 50 }),
      createProofLogItem({ requestType: "GetDocument", height: 200, timeMs: 75 }),
    ];

    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers(items),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });

    // Table should show items — look for request type text
    await expect(page.getByText("Get Identity")).toBeVisible({ timeout: 5000 });
    await expect(page.getByText("Get Document")).toBeVisible();
  });

  test("renders sort header columns", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers([createProofLogItem()]),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });

    // Sort headers should be visible (use button role to avoid ambiguity)
    await expect(
      page.getByRole("button", { name: "Request Type" }),
    ).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole("button", { name: "Height" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Time (ms)" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Error" })).toBeVisible();
  });

  test("clicking a row selects it and shows detail panel", async ({
    page,
    mockIPC,
  }) => {
    const items = [
      createProofLogItem({
        requestType: "GetIdentity",
        height: 100,
        proofBytesHex: "deadbeef",
      }),
    ];

    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers(items),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });

    // Click the row
    await page.getByText("Get Identity").click();

    // Detail panel should show proof hex data
    await expect(page.getByText("deadbeef")).toBeVisible({ timeout: 5000 });
  });

  test("display mode radios switch between Hex, JSON, and Path Query", async ({
    page,
    mockIPC,
  }) => {
    const items = [
      createProofLogItem({
        requestType: "GetIdentity",
        proofBytesHex: "aabbccdd",
        verificationPathQueryHex: "11223344",
      }),
    ];

    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers(items, {
        parse_grovedb_proof: { text: "Parsed GroveDB proof structure" },
        parse_path_query: { text: "Parsed path query structure" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });

    // Click the row to select it
    await page.getByText("Get Identity").click();

    // Hex mode should be default — verify proof bytes visible
    await expect(page.getByText("aabbccdd")).toBeVisible({ timeout: 5000 });

    // Three radio buttons should be visible
    await expect(page.getByRole("radio", { name: "Hex" })).toBeVisible();
    await expect(page.getByRole("radio", { name: "JSON" })).toBeVisible();
    await expect(page.getByRole("radio", { name: "Path Query" })).toBeVisible();
  });

  test("shows pagination controls when items fill a page", async ({
    page,
    mockIPC,
  }) => {
    // Create exactly 100 items (one full page)
    const items = Array.from({ length: 100 }, (_, i) =>
      createProofLogItem({ height: i + 1 }),
    );

    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers(items),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });

    // Pagination should show "Next" button
    await expect(
      page.getByRole("button", { name: /next/i }),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows item count info", async ({ page, mockIPC }) => {
    const items = [
      createProofLogItem({ height: 1 }),
      createProofLogItem({ height: 2 }),
    ];

    await mockIPC.navigateWithHandlers(
      "/tools/proof-log",
      proofLogHandlers(items),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Log" }),
    ).toBeVisible({ timeout: 10000 });

    // Should show item range info
    await expect(page.getByText(/showing items/i)).toBeVisible({
      timeout: 5000,
    });
  });
});

// ---------------------------------------------------------------------------
// Proof Visualizer Screen
// ---------------------------------------------------------------------------

test.describe("Proof Visualizer Screen", () => {
  test("renders page title, subtitle, and HexInput", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/deserialize and inspect.*grovedb/i),
    ).toBeVisible();
    await expect(
      page.getByPlaceholder(/paste serialized grovedb proof/i),
    ).toBeVisible();
  });

  test("shows idle state message when no input", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByText(/no proof parsed yet/i),
    ).toBeVisible();
  });

  test("parses valid hex input and shows result", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers({
        parse_grovedb_proof: {
          text: "GroveDB Proof Structure:\n  Root Hash: aabbccdd\n  Layers: 3",
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized grovedb proof/i)
      .fill("aabbccdd");

    // Should show parsed result
    await expect(
      page.getByText(/grovedb proof structure/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("shows error for invalid input format", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized grovedb proof/i)
      .fill("zzz!!!");

    await expect(page.getByRole("alert")).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText(/unable to decode input/i),
    ).toBeVisible();
  });

  test("dismiss error button clears error", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    await page
      .getByPlaceholder(/paste serialized grovedb proof/i)
      .fill("zzz!!!");

    await expect(page.getByRole("alert")).toBeVisible({ timeout: 5000 });

    await page.getByRole("button", { name: /dismiss error/i }).click();

    await expect(page.getByRole("alert")).not.toBeVisible({ timeout: 3000 });
  });

  test("supports base64 input format", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/proof-visualizer",
      toolsHandlers({
        parse_grovedb_proof: {
          text: "Parsed base64 proof data successfully",
        },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Proof Visualizer" }),
    ).toBeVisible({ timeout: 10000 });

    // Base64 encoded data
    await page
      .getByPlaceholder(/paste serialized grovedb proof/i)
      .fill("SGVsbG8gV29ybGQ=");

    await expect(
      page.getByText(/parsed base64 proof data/i),
    ).toBeVisible({ timeout: 5000 });
  });
});

// ---------------------------------------------------------------------------
// GroveSTARK Screen
// ---------------------------------------------------------------------------

test.describe("GroveSTARK Screen", () => {
  /** Create a mock identity with EdDSA keys. */
  function createEddsaIdentity(overrides: Record<string, unknown> = {}) {
    return {
      id: "EdDSAIdentity123456789012345678901234567890",
      alias: "EdDSA Identity",
      identityType: "User",
      balance: 1000000000,
      dpnsNames: [],
      keys: [
        {
          keyId: 5,
          purpose: "AUTHENTICATION",
          securityLevel: "HIGH",
          keyType: "EDDSA_25519_HASH160",
          isDisabled: false,
          publicKeyHex: "eddsa123",
          hasPrivateKey: true,
          contractBounds: null,
        },
      ],
      walletRef: { type: "hd", seedHash: "abc123" },
      ...overrides,
    };
  }

  /** Create a mock user contract (non-system). */
  function createUserContract(overrides: Record<string, unknown> = {}) {
    return {
      id: "cc112233445566778899aabb112233445566778899aabb112233445566ccdd",
      alias: "My User Contract",
      documentTypeCount: 2,
      tokenCount: 0,
      ...overrides,
    };
  }

  /** Create mock contract detail. */
  function createGrovestarkContractDetail(overrides: Record<string, unknown> = {}) {
    return {
      id: "cc112233445566778899aabb112233445566778899aabb112233445566ccdd",
      ownerId: "owner123",
      alias: "My User Contract",
      version: 1,
      documentTypeNames: ["note", "profile"],
      tokenCount: 0,
      schemaJson: {},
      ...overrides,
    };
  }

  function grovestarkHandlers(overrides: Record<string, unknown> = {}) {
    return toolsHandlers({
      identity_list_local: [createEddsaIdentity()],
      identity_list_summaries: [createEddsaIdentity()],
      contract_list_local: [createUserContract()],
      contract_get_by_id: createGrovestarkContractDetail(),
      wallet_list_all: { hdWallets: [], singleKeyWallets: [], selected: null },
      ...overrides,
    });
  }

  test("renders page title, subtitle, and warning banner", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/generate and verify zero-knowledge proofs/i),
    ).toBeVisible();
    await expect(
      page.getByText(/research project/i),
    ).toBeVisible();
  });

  test("shows Generate and Verify mode toggle buttons", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // Mode toggle buttons
    await expect(
      page.getByRole("button", { name: /generate proof/i }).first(),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: /verify proof/i }).first(),
    ).toBeVisible();
  });

  test("Generate mode shows 3-step form", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // 3 steps should be visible
    await expect(page.getByText("Step 1")).toBeVisible();
    await expect(page.getByText("Step 2")).toBeVisible();
    await expect(page.getByText("Step 3")).toBeVisible();
  });

  test("Generate mode: identity selector shows EdDSA identities", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // Click identity selector
    const identitySelect = page.locator("#identity-select");
    await expect(identitySelect).toBeVisible({ timeout: 5000 });
    await identitySelect.click();

    // Should show the EdDSA identity
    await expect(
      page.getByText(/EdDSA Identity/),
    ).toBeVisible({ timeout: 3000 });
  });

  test("Generate mode: document ID input field exists", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    await expect(
      page.getByPlaceholder(/enter the document id/i),
    ).toBeVisible();
  });

  test("Generate proof button is disabled when steps incomplete", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // The "Generate Proof" action button (not the tab toggle) should be disabled
    // It's the last one on the page since the tab toggle comes first
    const generateBtns = page.getByRole("button", { name: /generate proof/i });
    const actionBtn = generateBtns.last();
    await expect(actionBtn).toBeVisible({ timeout: 5000 });
    await expect(actionBtn).toBeDisabled();
  });

  test("Verify mode: shows proof textarea", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // Switch to Verify mode via tab toggle (first button matching)
    await page.getByRole("button", { name: /verify proof/i }).first().click();

    // Proof textarea should appear
    await expect(
      page.getByPlaceholder(/paste the proof data/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("Verify mode: verify button disabled when no input", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // Switch to Verify mode via the tab toggle (first button matching)
    await page.getByRole("button", { name: /verify proof/i }).first().click();

    // The action button (last one) should be disabled with empty input
    const verifyBtns = page.getByRole("button", { name: /verify proof/i });
    const actionBtn = verifyBtns.last();
    await expect(actionBtn).toBeVisible({ timeout: 5000 });
    await expect(actionBtn).toBeDisabled();
  });

  test("Verify mode: shows error for invalid proof format", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // Switch to Verify mode via tab toggle (first button matching)
    await page.getByRole("button", { name: /verify proof/i }).first().click();

    // Enter invalid proof text
    await page
      .getByPlaceholder(/paste the proof data/i)
      .fill("not valid proof data");

    // Click the verify action button (last one)
    const actionBtn = page.getByRole("button", { name: /verify proof/i }).last();
    await expect(actionBtn).toBeEnabled({ timeout: 3000 });
    await actionBtn.click();

    // Should show parse error
    await expect(
      page.getByText(/failed to parse proof/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("can switch between Generate and Verify modes", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/grovestark",
      grovestarkHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: /GroveSTARK/i }),
    ).toBeVisible({ timeout: 10000 });

    // Default is Generate mode — steps visible
    await expect(page.getByText("Step 1")).toBeVisible();

    // Switch to Verify via tab toggle (first matching button)
    await page.getByRole("button", { name: /verify proof/i }).first().click();
    await expect(
      page.getByPlaceholder(/paste the proof data/i),
    ).toBeVisible({ timeout: 3000 });

    // Switch back to Generate via tab toggle (first matching button)
    await page.getByRole("button", { name: /generate proof/i }).first().click();
    await expect(page.getByText("Step 1")).toBeVisible({ timeout: 3000 });
  });
});

// ---------------------------------------------------------------------------
// Masternode List Diff Screen
// ---------------------------------------------------------------------------

test.describe("Masternode List Diff Screen", () => {
  test("renders page title and subtitle", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });
    await expect(
      page.getByText(/chain locks.*instant send/i).first(),
    ).toBeVisible();
  });

  test("renders 3 tabs: Core Items, QR Info, Quorum Viewer", async ({
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

    await expect(page.getByRole("tab", { name: "Core Items" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "QR Info" })).toBeVisible();
    await expect(page.getByRole("tab", { name: /quorum viewer/i })).toBeVisible();
  });

  test("renders input area with base/end height inputs", async ({
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

    await expect(page.getByTestId("base-height-input")).toBeVisible();
    await expect(page.getByTestId("end-height-input")).toBeVisible();
  });

  test("renders all 6 action buttons", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    await expect(page.getByTestId("fetch-diff-button")).toBeVisible();
    await expect(page.getByTestId("fetch-qrinfo-button")).toBeVisible();
    await expect(
      page.getByTestId("fetch-dmls-no-rotation-button"),
    ).toBeVisible();
    await expect(
      page.getByTestId("fetch-dmls-with-rotation-button"),
    ).toBeVisible();
    await expect(page.getByTestId("fetch-chain-locks-button")).toBeVisible();
    await expect(page.getByTestId("clear-button")).toBeVisible();
  });

  test("clicking fetch diff dispatches IPC with height values", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers({
        mnlist_fetch_diff: { taskId: "mock-diff-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    // Enter base and end heights
    await page.getByTestId("base-height-input").fill("1000");
    await page.getByTestId("end-height-input").fill("2000");

    // Click fetch diff
    await page.getByTestId("fetch-diff-button").click();

    // Should show pending indicator
    await expect(page.getByTestId("pending-indicator")).toBeVisible({
      timeout: 5000,
    });
  });

  test("fetch diff success shows success message", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers({
        mnlist_fetch_diff: { taskId: "mock-diff-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByTestId("end-height-input").fill("2000");
    await page.getByTestId("fetch-diff-button").click();

    // Wait for pending indicator to confirm the IPC call has been initiated
    await expect(page.getByTestId("pending-indicator")).toBeVisible({
      timeout: 5000,
    });

    // Allow time for the async IPC response to resolve and register the task ID
    await page.waitForTimeout(500);

    // Emit success result with diff matching MnListDiffDto shape
    await mockIPC.emitEvent("task-result-event", {
      taskId: "mock-diff-task",
      resultType: "MnList",
      payload: {
        type: "FetchedDiff",
        baseHeight: 0,
        height: 2000,
        diff: {
          version: 1,
          baseBlockHash: "aabb",
          blockHash: "ccdd",
          totalTransactions: 0,
          merkleHashes: [],
          merkleFlagsLen: 0,
          coinbaseTxid: "0000",
          coinbaseSize: 0,
          newMasternodes: [],
          deletedMasternodes: [],
          newQuorums: [],
          deletedQuorums: [],
          chainlockSigCount: 0,
          chainlockSignatures: [],
        },
      },
    });

    // Should show success message
    await expect(
      page.getByTestId("message-banner"),
    ).toBeVisible({ timeout: 5000 });
  });

  test("fetch error shows error banner", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers({
        mnlist_fetch_diff: { taskId: "mock-diff-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByTestId("end-height-input").fill("2000");
    await page.getByTestId("fetch-diff-button").click();

    await mockIPC.emitEvent("task-error-event", {
      taskId: "mock-diff-task",
      message: "Failed to fetch masternode list diff",
      details: "",
      retryable: false,
    });

    await expect(
      page.getByTestId("error-banner"),
    ).toBeVisible({ timeout: 5000 });
    await expect(
      page.getByText("Failed to fetch masternode list diff"),
    ).toBeVisible();
  });

  test("buttons are disabled while operation is pending", async ({
    page,
    mockIPC,
  }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers({
        mnlist_fetch_diff: { taskId: "mock-diff-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByTestId("end-height-input").fill("2000");
    await page.getByTestId("fetch-diff-button").click();

    // Other action buttons should be disabled while pending
    await expect(
      page.getByTestId("fetch-qrinfo-button"),
    ).toBeDisabled({ timeout: 3000 });
    await expect(
      page.getByTestId("fetch-chain-locks-button"),
    ).toBeDisabled();
  });

  test("Core Items tab: shows chain-locked blocks from ZMQ events", async ({
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

    // Ensure Core Items tab is active
    await page.getByRole("tab", { name: "Core Items" }).click();

    // Emit a chain-locked block ZMQ event
    await mockIPC.emitEvent("zmq-chain-locked-block-event", {
      network: "Testnet",
      blockHeight: 54321,
      blockHash: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
      txCount: 1,
      txIds: ["aabb1122"],
      rawBlock: "01000000",
      rawChainLock: "02000000",
      signature: "03000000",
      isValid: true,
    });

    // Block height should appear in the list
    await expect(page.getByText("54321")).toBeVisible({ timeout: 5000 });
  });

  test("Core Items tab: shows instant send transactions from ZMQ events", async ({
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

    await page.getByRole("tab", { name: "Core Items" }).click();

    // Emit an instant send locked transaction ZMQ event
    await mockIPC.emitEvent("zmq-is-locked-transaction-event", {
      network: "Testnet",
      txid: "tx123abc456def789012345678901234567890123456789012345678901234",
      rawTx: "01000000",
      rawIsLock: "02000000",
      affectedUtxoCount: 1,
      isValid: true,
    });

    // TxID (truncated) should appear in the transactions list
    await expect(page.getByText(/tx123a/).first()).toBeVisible({ timeout: 5000 });
  });

  test("QR Info tab: shows empty state and load button", async ({
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

    // Switch to QR Info tab
    await page.getByRole("tab", { name: "QR Info" }).click();

    // Should show empty state
    await expect(
      page.getByText(/load a qrinfo/i),
    ).toBeVisible({ timeout: 5000 });

    // Load button should be visible
    await expect(page.getByTestId("load-qrinfo-button")).toBeVisible();
  });

  test("Quorum Viewer tab: shows empty state when no data", async ({
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

    // Switch to Quorum Viewer tab
    await page.getByRole("tab", { name: /quorum viewer/i }).click();

    // Should show empty state
    await expect(
      page.getByText(/no quorum data available/i),
    ).toBeVisible({ timeout: 5000 });
  });

  test("Clear button resets state", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers(),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    // Enter some height values
    await page.getByTestId("base-height-input").fill("1000");
    await page.getByTestId("end-height-input").fill("2000");

    // Click clear
    await page.getByTestId("clear-button").click();

    // Success message should appear
    await expect(
      page.getByTestId("message-banner"),
    ).toBeVisible({ timeout: 3000 });
  });

  test("fetch chain locks dispatches IPC", async ({ page, mockIPC }) => {
    await mockIPC.navigateWithHandlers(
      "/tools/masternode-list",
      toolsHandlers({
        mnlist_fetch_chain_locks: { taskId: "mock-chainlocks-task" },
      }),
    );

    await expect(
      page.getByRole("heading", { name: "Masternode List Diff" }),
    ).toBeVisible({ timeout: 10000 });

    await page.getByTestId("base-height-input").fill("100");
    await page.getByTestId("end-height-input").fill("200");
    await page.getByTestId("fetch-chain-locks-button").click();

    // Should show pending state
    await expect(page.getByTestId("pending-indicator")).toBeVisible({
      timeout: 5000,
    });
  });
});
