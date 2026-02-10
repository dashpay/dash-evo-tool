// jsdom polyfills for Radix Select
if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
}
if (!Element.prototype.setPointerCapture) {
  Element.prototype.setPointerCapture = () => {};
}
if (!Element.prototype.releasePointerCapture) {
  Element.prototype.releasePointerCapture = () => {};
}
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DocumentActionScreen } from "./DocumentActionScreen";
import type { DocumentActionType } from "./DocumentActionScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { useWalletStore } from "@/stores/walletStore";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Centralized mock bindings ──────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("sonner", () => ({
  toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
  Toaster: () => null,
}));

import { commands, events } from "@/bindings";

// ─── Test fixtures ──────────────────────────────────────────────────

const TEST_KEY: IdentityKeyDto = {
  keyId: 1,
  keyType: "ECDSA_SECP256K1",
  purpose: "AUTHENTICATION",
  securityLevel: "HIGH",
  isDisabled: false,
  publicKeyHex: "03aabb",
  hasPrivateKey: true,
  contractBounds: null,
};

const TEST_IDENTITY: QualifiedIdentityDto = {
  id: "aabbccdd11223344aabbccdd11223344aabbccdd11223344aabbccdd11223344",
  alias: "TestIdentity",
  balance: 5_000_000_000_000,
  identityType: "User",
  keys: [TEST_KEY],
  associatedWalletHashes: [],
  dpnsNames: [],
  isLocal: true,
};

const TEST_CONTRACT_SUMMARY = {
  id: "1122334455667788112233445566778811223344556677881122334455667788",
  alias: "TestContract",
  documentTypeCount: 2,
  tokenCount: 0,
};

// ─── Helper ─────────────────────────────────────────────────────────

function setupStores(
  identities: QualifiedIdentityDto[] = [TEST_IDENTITY],
  contracts = [TEST_CONTRACT_SUMMARY],
) {
  // Mock the IPC calls that the stores make on load
  vi.mocked(commands.identityListLocal).mockResolvedValue({
    status: "ok",
    data: identities,
  });
  vi.mocked(commands.contractListLocal).mockResolvedValue({
    status: "ok",
    data: contracts,
  });

  useIdentityStore.setState({
    identities,
    loading: false,
    error: null,
  });
  useContractStore.setState({
    contracts,
    loading: false,
    error: null,
  });
  useWalletStore.setState({
    hdWallets: [],
    singleKeyWallets: [],
    loading: false,
  });
}

function renderAction(actionType: DocumentActionType) {
  return render(<DocumentActionScreen actionType={actionType} />);
}

function getLastTaskResultListener() {
  const calls = vi.mocked(events.taskResultEvent.listen).mock.calls;
  return calls[calls.length - 1]?.[0];
}

function getLastTaskErrorListener() {
  const calls = vi.mocked(events.taskErrorEvent.listen).mock.calls;
  return calls[calls.length - 1]?.[0];
}

// ─── Tests ──────────────────────────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  useIdentityStore.setState({ identities: [], loading: false, error: null });
  useContractStore.setState({ contracts: [], loading: false, error: null });
  useWalletStore.setState({ hdWallets: [], singleKeyWallets: [], loading: false });
});

describe("DocumentActionScreen", () => {
  // ─── Rendering ──────────────────────────────────────────────────

  it.each([
    ["create", "Create Document"],
    ["delete", "Delete Document"],
    ["replace", "Replace Document"],
    ["transfer", "Transfer Document"],
    ["purchase", "Purchase Document"],
    ["setPrice", "Set Document Price"],
  ] as [DocumentActionType, string][])(
    "renders correct title for %s action",
    (actionType, expectedTitle) => {
      setupStores();
      renderAction(actionType);
      expect(
        screen.getByRole("heading", { name: expectedTitle }),
      ).toBeInTheDocument();
    },
  );

  it("renders breadcrumbs with Contracts link", () => {
    setupStores();
    renderAction("create");
    expect(screen.getByText("Contracts")).toBeInTheDocument();
  });

  it("renders back button", () => {
    setupStores();
    renderAction("create");
    expect(screen.getByTestId("back-button")).toBeInTheDocument();
  });

  it("shows step headings", () => {
    setupStores();
    renderAction("create");
    expect(screen.getByText("1. Select a contract and document type")).toBeInTheDocument();
    expect(screen.getByText("2. Select an identity")).toBeInTheDocument();
  });

  it("renders contract and document type selectors", () => {
    setupStores();
    renderAction("create");
    expect(screen.getByTestId("contract-select")).toBeInTheDocument();
    expect(screen.getByTestId("doctype-select")).toBeInTheDocument();
  });

  it("shows no identities warning when none loaded", () => {
    setupStores([]);
    renderAction("create");
    expect(screen.getByText(/No identities loaded/)).toBeInTheDocument();
  });

  // ─── Identity Selection ─────────────────────────────────────────

  it("auto-selects first identity", () => {
    setupStores();
    renderAction("create");
    expect(screen.getByText("TestIdentity")).toBeInTheDocument();
  });

  it("shows identity balance", () => {
    setupStores();
    renderAction("create");
    expect(screen.getByText(/50\.00000000 DASH/)).toBeInTheDocument();
  });

  // ─── Advanced Options ───────────────────────────────────────────

  it("toggles advanced options", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("create");

    expect(screen.queryByTestId("key-select")).not.toBeInTheDocument();
    await user.click(screen.getByTestId("advanced-toggle"));
    expect(screen.getByTestId("key-select")).toBeInTheDocument();
    await user.click(screen.getByTestId("advanced-toggle"));
    expect(screen.queryByTestId("key-select")).not.toBeInTheDocument();
  });

  // ─── Step 3 Labels ─────────────────────────────────────────────

  it.each([
    ["create", "3. Fill out the document type fields"],
    ["delete", "3. Enter document ID to delete"],
    ["replace", "3. Enter document ID and fetch existing document"],
    ["transfer", "3. Transfer document"],
    ["purchase", "3. Enter document ID to purchase"],
    ["setPrice", "3. Set document price"],
  ] as [DocumentActionType, string][])(
    "shows correct step 3 label for %s",
    (actionType, expectedLabel) => {
      setupStores();
      renderAction(actionType);
      expect(screen.getByText(expectedLabel)).toBeInTheDocument();
    },
  );

  // ─── Delete Action Fields ───────────────────────────────────────

  it("renders document ID input for delete action", () => {
    setupStores();
    renderAction("delete");
    expect(screen.getByTestId("document-id-input")).toBeInTheDocument();
  });

  it("does not show broadcast button without contract/doctype selected", () => {
    setupStores();
    renderAction("delete");
    expect(screen.queryByTestId("broadcast-button")).not.toBeInTheDocument();
  });

  // ─── Transfer Action Fields ─────────────────────────────────────

  it("renders both ID inputs for transfer action", () => {
    setupStores();
    renderAction("transfer");
    expect(screen.getByTestId("document-id-input")).toBeInTheDocument();
    expect(screen.getByTestId("recipient-id-input")).toBeInTheDocument();
  });

  // ─── Purchase Action Fields ─────────────────────────────────────

  it("renders fetch price button for purchase action", () => {
    setupStores();
    renderAction("purchase");
    expect(screen.getByTestId("fetch-price-button")).toBeInTheDocument();
  });

  it("renders document ID input for purchase action", () => {
    setupStores();
    renderAction("purchase");
    expect(screen.getByTestId("document-id-input")).toBeInTheDocument();
  });

  // ─── SetPrice Action Fields ─────────────────────────────────────

  it("renders document ID and price inputs for setPrice", () => {
    setupStores();
    renderAction("setPrice");
    expect(screen.getByTestId("document-id-input")).toBeInTheDocument();
    expect(screen.getByTestId("price-input")).toBeInTheDocument();
  });

  // ─── Replace Action Fields ──────────────────────────────────────

  it("renders document ID input and fetch button for replace", () => {
    setupStores();
    renderAction("replace");
    expect(screen.getByTestId("document-id-input")).toBeInTheDocument();
    expect(screen.getByTestId("fetch-document-button")).toBeInTheDocument();
  });

  it("fetch button is disabled without document ID for replace", () => {
    setupStores();
    renderAction("replace");
    expect(screen.getByTestId("fetch-document-button")).toBeDisabled();
  });

  it("fetch button is enabled with document ID for replace", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("replace");
    await user.type(screen.getByTestId("document-id-input"), "abc");
    expect(screen.getByTestId("fetch-document-button")).not.toBeDisabled();
  });

  // ─── Navigation ─────────────────────────────────────────────────

  it("navigates back to contracts on back button click", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("create");
    await user.click(screen.getByTestId("back-button"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
  });

  // ─── Wallet Lock ────────────────────────────────────────────────

  it("shows wallet locked warning when wallet requires password", () => {
    useIdentityStore.setState({
      identities: [{ ...TEST_IDENTITY, associatedWalletHashes: ["wallet-hash-1"] }],
      loading: false,
      error: null,
    });
    useContractStore.setState({ contracts: [TEST_CONTRACT_SUMMARY], loading: false, error: null });
    useWalletStore.setState({
      hdWallets: [{
        seedHash: "wallet-hash-1",
        alias: "MyWallet",
        usesPassword: true,
        passwordHint: "hint",
        accounts: [],
        balance: 100000,
        pendingBalance: 0,
      }],
      singleKeyWallets: [],
      loading: false,
    });

    renderAction("create");
    expect(screen.getByText(/Wallet is locked/)).toBeInTheDocument();
    expect(screen.getByTestId("unlock-wallet")).toBeInTheDocument();
  });

  // ─── Event Subscription ─────────────────────────────────────────

  it("subscribes to task events on mount", async () => {
    setupStores();
    renderAction("create");
    await waitFor(() => {
      expect(events.taskResultEvent.listen).toHaveBeenCalled();
      expect(events.taskErrorEvent.listen).toHaveBeenCalled();
    });
  });

  // ─── Loading on mount ───────────────────────────────────────────

  it("loads contracts and identities on mount", async () => {
    setupStores();
    renderAction("create");
    await waitFor(() => {
      expect(commands.contractListLocal).toHaveBeenCalled();
      expect(commands.identityListLocal).toHaveBeenCalled();
    });
  });

  // ─── All action types render without errors ─────────────────────

  it("renders all 6 action types without errors", () => {
    const actionTypes: DocumentActionType[] = [
      "create", "delete", "replace", "transfer", "purchase", "setPrice",
    ];
    for (const actionType of actionTypes) {
      setupStores();
      const { unmount } = renderAction(actionType);
      expect(screen.getByTestId("document-action-screen")).toBeInTheDocument();
      unmount();
    }
  });

  // ─── Input interactions ─────────────────────────────────────────

  it("accepts text input in document ID field for delete", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("delete");
    const input = screen.getByTestId("document-id-input");
    await user.type(input, "testDocId123");
    expect(input).toHaveValue("testDocId123");
  });

  it("accepts text input in both fields for transfer", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("transfer");
    await user.type(screen.getByTestId("document-id-input"), "doc1");
    await user.type(screen.getByTestId("recipient-id-input"), "recip1");
    expect(screen.getByTestId("document-id-input")).toHaveValue("doc1");
    expect(screen.getByTestId("recipient-id-input")).toHaveValue("recip1");
  });

  it("accepts text input in price field for setPrice", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("setPrice");
    await user.type(screen.getByTestId("price-input"), "5000");
    expect(screen.getByTestId("price-input")).toHaveValue("5000");
  });

  it("accepts text input in document ID for purchase", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("purchase");
    await user.type(screen.getByTestId("document-id-input"), "docPurchase");
    expect(screen.getByTestId("document-id-input")).toHaveValue("docPurchase");
  });

  it("accepts text input in document ID for replace", async () => {
    setupStores();
    const user = userEvent.setup();
    renderAction("replace");
    await user.type(screen.getByTestId("document-id-input"), "docReplace");
    expect(screen.getByTestId("document-id-input")).toHaveValue("docReplace");
  });

  // ─── Contract selection loads detail ───────────────────────────────

  it("loads contract detail when contract is selected", async () => {
    setupStores();
    const contractDetail = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["domain", "preorder"],
      schemaJson: {
        documentSchemas: {
          domain: {
            properties: {
              label: { type: "string", maxLength: 63 },
              normalizedLabel: { type: "string" },
            },
            required: ["label"],
            indices: [
              { properties: [{ $ownerId: "asc" }, { label: "asc" }] },
            ],
          },
          preorder: {
            properties: {
              saltedDomainHash: { type: "array", byteArray: true, minItems: 32 },
            },
            required: ["saltedDomainHash"],
          },
        },
      },
    };
    vi.mocked(commands.contractGetById).mockResolvedValue({
      status: "ok",
      data: contractDetail,
    });

    const user = userEvent.setup();
    renderAction("create");

    // Open contract select and pick the test contract
    await user.click(screen.getByTestId("contract-select"));
    await user.click(screen.getByText("TestContract"));

    await waitFor(() => {
      expect(commands.contractGetById).toHaveBeenCalledWith(TEST_CONTRACT_SUMMARY.id);
    });
  });

  // ─── Delete: Fetch Owned Documents flow ────────────────────────────

  describe("Delete action - fetch owned documents", () => {
    const CONTRACT_WITH_OWNER_INDEX = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["domain"],
      schemaJson: {
        documentSchemas: {
          domain: {
            properties: {
              label: { type: "string", maxLength: 63 },
            },
            required: ["label"],
            indices: [
              { properties: [{ $ownerId: "asc" }, { label: "asc" }] },
            ],
          },
        },
      },
    };

    async function setupDeleteWithContract() {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_WITH_OWNER_INDEX,
      });
      const user = userEvent.setup();
      renderAction("delete");

      // Select contract
      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));

      await waitFor(() => {
        expect(commands.contractGetById).toHaveBeenCalled();
      });

      return user;
    }

    it("shows fetch owned documents button when doc type has $ownerId index", async () => {
      await setupDeleteWithContract();
      await waitFor(() => {
        expect(screen.getByTestId("fetch-owned-button")).toBeInTheDocument();
      });
    });

    it("dispatches fetch with $ownerId filter when fetch owned clicked", async () => {
      const user = await setupDeleteWithContract();
      await waitFor(() => {
        expect(screen.getByTestId("fetch-owned-button")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("fetch-owned-button"));

      await waitFor(() => {
        expect(commands.documentFetch).toHaveBeenCalledWith(
          expect.objectContaining({
            contractId: TEST_CONTRACT_SUMMARY.id,
            documentTypeName: "domain",
            whereClauses: expect.arrayContaining([
              expect.objectContaining({ field: "$ownerId", operator: "==" }),
            ]),
          }),
        );
      });
    });

    it("shows fetched documents list after successful fetch", async () => {
      const user = await setupDeleteWithContract();
      await waitFor(() => {
        expect(screen.getByTestId("fetch-owned-button")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("fetch-owned-button"));

      // Wait for task result listener to be registered, then fire event
      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [
              { $id: "doc-id-1", label: "alice" },
              { $id: "doc-id-2", label: "bob" },
            ],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByText("doc-id-1")).toBeInTheDocument();
        expect(screen.getByText("doc-id-2")).toBeInTheDocument();
      });
    });

    it("shows no owned documents message when fetch returns empty", async () => {
      const user = await setupDeleteWithContract();
      await waitFor(() => {
        expect(screen.getByTestId("fetch-owned-button")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("fetch-owned-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: { documents: [] },
        },
      });

      await waitFor(() => {
        expect(screen.getByText(/No owned documents found/)).toBeInTheDocument();
      });
    });

    it("sets document ID when selecting a fetched document", async () => {
      const user = await setupDeleteWithContract();
      await waitFor(() => {
        expect(screen.getByTestId("fetch-owned-button")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("fetch-owned-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [{ $id: "doc-select-id", label: "test" }],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByText("doc-select-id")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("select-doc-doc-select-id"));

      expect(screen.getByTestId("document-id-input")).toHaveValue("doc-select-id");
    });

    it("opens view dialog when clicking View on a fetched document", async () => {
      const user = await setupDeleteWithContract();
      await waitFor(() => {
        expect(screen.getByTestId("fetch-owned-button")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("fetch-owned-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [{ $id: "view-doc-id", label: "viewable" }],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByTestId("view-doc-view-doc-id")).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("view-doc-view-doc-id"));

      await waitFor(() => {
        expect(screen.getByRole("heading", { name: "Document" })).toBeInTheDocument();
      });
    });
  });

  // ─── Delete: Broadcast flow ────────────────────────────────────────

  describe("Delete action - broadcast", () => {
    const CONTRACT_DETAIL = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["note"],
      schemaJson: {
        documentSchemas: {
          note: {
            properties: { message: { type: "string" } },
            required: [],
          },
        },
      },
    };

    async function setupDeleteReady() {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("delete");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      // Enter document ID
      await user.type(screen.getByTestId("document-id-input"), "doc-to-delete");

      return user;
    }

    it("dispatches documentDelete on broadcast", async () => {
      const user = await setupDeleteReady();

      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(commands.documentDelete).toHaveBeenCalledWith(
          expect.objectContaining({
            documentId: "doc-to-delete",
            documentTypeName: "note",
            contractId: TEST_CONTRACT_SUMMARY.id,
          }),
        );
      });
    });

    it("shows broadcasting state after dispatch", async () => {
      const user = await setupDeleteReady();
      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(screen.getByTestId("broadcasting-state")).toBeInTheDocument();
      });
    });

    it("shows success on task result event", async () => {
      const user = await setupDeleteReady();
      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: { fee_result: { processing: 1000 } },
        },
      });

      await waitFor(() => {
        expect(screen.getByTestId("success-screen")).toBeInTheDocument();
      });
    });

    it("shows error on task error event", async () => {
      const user = await setupDeleteReady();
      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => expect(vi.mocked(events.taskErrorEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const errorListener = getLastTaskErrorListener();
      errorListener?.({
        payload: {
          taskId: "mock-task-id",
          message: "Document not found on platform",
        },
      });

      await waitFor(() => {
        expect(screen.getByTestId("error-banner")).toBeInTheDocument();
        expect(screen.getByText(/Document not found on platform/)).toBeInTheDocument();
      });
    });
  });

  // ─── Replace: Fetch → Edit → Broadcast ─────────────────────────────

  describe("Replace action - full flow", () => {
    const CONTRACT_DETAIL = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["note"],
      schemaJson: {
        documentSchemas: {
          note: {
            properties: {
              message: { type: "string", maxLength: 256 },
              priority: { type: "integer" },
            },
            required: ["message"],
          },
        },
      },
    };

    async function setupReplaceWithContract() {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("replace");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());
      return user;
    }

    it("dispatches fetch when clicking Fetch button", async () => {
      const user = await setupReplaceWithContract();
      await user.type(screen.getByTestId("document-id-input"), "doc-replace-id");
      await user.click(screen.getByTestId("fetch-document-button"));

      await waitFor(() => {
        expect(commands.documentFetch).toHaveBeenCalledWith(
          expect.objectContaining({
            contractId: TEST_CONTRACT_SUMMARY.id,
            documentTypeName: "note",
            whereClauses: expect.arrayContaining([
              expect.objectContaining({ field: "$id", operator: "==" }),
            ]),
          }),
        );
      });
    });

    it("populates form fields from fetched document", async () => {
      const user = await setupReplaceWithContract();
      await user.type(screen.getByTestId("document-id-input"), "doc-replace-id");
      await user.click(screen.getByTestId("fetch-document-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [
              {
                $id: "doc-replace-id",
                $ownerId: "owner-1",
                message: "original text",
                priority: 42,
              },
            ],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByText("Document fetched successfully.")).toBeInTheDocument();
        expect(screen.getByTestId("field-message")).toHaveValue("original text");
        expect(screen.getByTestId("field-priority")).toHaveValue("42");
      });
    });

    it("shows error when fetched document is not found", async () => {
      const user = await setupReplaceWithContract();
      await user.type(screen.getByTestId("document-id-input"), "nonexistent-id");
      await user.click(screen.getByTestId("fetch-document-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: { documents: [] },
        },
      });

      await waitFor(() => {
        expect(screen.getByText(/No document found with the provided ID/)).toBeInTheDocument();
      });
    });

    it("dispatches documentReplace on broadcast after edit", async () => {
      const user = await setupReplaceWithContract();
      await user.type(screen.getByTestId("document-id-input"), "doc-replace-id");
      await user.click(screen.getByTestId("fetch-document-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [
              {
                $id: "doc-replace-id",
                $ownerId: "owner-1",
                message: "original",
                priority: 1,
              },
            ],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByTestId("field-message")).toBeInTheDocument();
      });

      // Edit the message field
      const messageField = screen.getByTestId("field-message");
      await user.clear(messageField);
      await user.type(messageField, "updated text");

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(commands.documentReplace).toHaveBeenCalledWith(
          expect.objectContaining({
            documentTypeName: "note",
            contractId: TEST_CONTRACT_SUMMARY.id,
          }),
        );
      });
    });
  });

  // ─── Purchase: Fetch Price → Broadcast ──────────────────────────────

  describe("Purchase action - full flow", () => {
    const CONTRACT_DETAIL = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["listing"],
      schemaJson: {
        documentSchemas: {
          listing: {
            properties: {
              title: { type: "string" },
            },
            required: ["title"],
          },
        },
      },
    };

    async function setupPurchaseWithContract() {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("purchase");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());
      return user;
    }

    it("dispatches fetch when clicking Fetch Document Price", async () => {
      const user = await setupPurchaseWithContract();
      await user.type(screen.getByTestId("document-id-input"), "purchasable-doc");
      await user.click(screen.getByTestId("fetch-price-button"));

      await waitFor(() => {
        expect(commands.documentFetch).toHaveBeenCalledWith(
          expect.objectContaining({
            contractId: TEST_CONTRACT_SUMMARY.id,
            documentTypeName: "listing",
          }),
        );
      });
    });

    it("displays fetched price in credits", async () => {
      const user = await setupPurchaseWithContract();
      await user.type(screen.getByTestId("document-id-input"), "purchasable-doc");
      await user.click(screen.getByTestId("fetch-price-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [{ $id: "purchasable-doc", $price: 50000, title: "item" }],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByText(/50000/)).toBeInTheDocument();
        expect(screen.getByText(/credits/i)).toBeInTheDocument();
      });
    });

    it("shows error when document not found for purchase", async () => {
      const user = await setupPurchaseWithContract();
      await user.type(screen.getByTestId("document-id-input"), "missing-doc");
      await user.click(screen.getByTestId("fetch-price-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: { documents: [] },
        },
      });

      await waitFor(() => {
        expect(screen.getByText(/Document not found/)).toBeInTheDocument();
      });
    });

    it("dispatches documentPurchase on broadcast after price fetched", async () => {
      const user = await setupPurchaseWithContract();
      await user.type(screen.getByTestId("document-id-input"), "purchasable-doc");
      await user.click(screen.getByTestId("fetch-price-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {
            documents: [{ $id: "purchasable-doc", $price: 50000, title: "item" }],
          },
        },
      });

      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(commands.documentPurchase).toHaveBeenCalledWith(
          expect.objectContaining({
            price: 50000,
            documentId: "purchasable-doc",
            documentTypeName: "listing",
            contractId: TEST_CONTRACT_SUMMARY.id,
          }),
        );
      });
    });
  });

  // ─── Transfer: Broadcast flow ──────────────────────────────────────

  describe("Transfer action - broadcast", () => {
    const CONTRACT_DETAIL = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["transferable"],
      schemaJson: {
        documentSchemas: {
          transferable: {
            properties: { name: { type: "string" } },
            required: [],
          },
        },
      },
    };

    it("dispatches documentTransfer with correct params", async () => {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("transfer");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "transfer-doc-id");
      await user.type(screen.getByTestId("recipient-id-input"), "recipient-id-123");

      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(commands.documentTransfer).toHaveBeenCalledWith(
          expect.objectContaining({
            documentId: "transfer-doc-id",
            newOwnerId: "recipient-id-123",
            documentTypeName: "transferable",
            contractId: TEST_CONTRACT_SUMMARY.id,
          }),
        );
      });
    });
  });

  // ─── SetPrice: Broadcast flow ──────────────────────────────────────

  describe("SetPrice action - broadcast", () => {
    const CONTRACT_DETAIL = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["priceable"],
      schemaJson: {
        documentSchemas: {
          priceable: {
            properties: { name: { type: "string" } },
            required: [],
          },
        },
      },
    };

    it("dispatches documentSetPrice with correct params", async () => {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("setPrice");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "price-doc-id");
      await user.type(screen.getByTestId("price-input"), "75000");

      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(commands.documentSetPrice).toHaveBeenCalledWith(
          expect.objectContaining({
            price: 75000,
            documentId: "price-doc-id",
            documentTypeName: "priceable",
            contractId: TEST_CONTRACT_SUMMARY.id,
          }),
        );
      });
    });

    it("shows error for non-numeric price and returns to input", async () => {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("setPrice");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "price-doc-id");
      await user.type(screen.getByTestId("price-input"), "not-a-number");

      await waitFor(() => {
        expect(screen.getByTestId("broadcast-button")).not.toBeDisabled();
      });

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(screen.getByText(/Price must be a valid integer/)).toBeInTheDocument();
      });
    });
  });

  // ─── Create: Dynamic form + broadcast ──────────────────────────────

  describe("Create action - dynamic form", () => {
    const CONTRACT_WITH_FIELDS = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["profile"],
      schemaJson: {
        documentSchemas: {
          profile: {
            properties: {
              displayName: { type: "string", maxLength: 25 },
              bio: { type: "string", maxLength: 140 },
              age: { type: "integer" },
              active: { type: "boolean" },
            },
            required: ["displayName"],
          },
        },
      },
    };

    async function setupCreateWithFields() {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_WITH_FIELDS,
      });
      const user = userEvent.setup();
      renderAction("create");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());
      return user;
    }

    it("renders dynamic form fields from document type schema", async () => {
      await setupCreateWithFields();
      await waitFor(() => {
        expect(screen.getByTestId("field-displayName")).toBeInTheDocument();
        expect(screen.getByTestId("field-bio")).toBeInTheDocument();
        expect(screen.getByTestId("field-age")).toBeInTheDocument();
        expect(screen.getByTestId("field-active")).toBeInTheDocument();
      });
    });

    it("marks required fields with asterisk", async () => {
      await setupCreateWithFields();
      await waitFor(() => {
        expect(screen.getByText("displayName *")).toBeInTheDocument();
        // bio is not required, should not have asterisk
        expect(screen.getByText("bio")).toBeInTheDocument();
      });
    });

    it("renders boolean fields as checkboxes", async () => {
      await setupCreateWithFields();
      await waitFor(() => {
        const activeCheckbox = screen.getByTestId("field-active");
        expect(activeCheckbox).toBeInTheDocument();
        expect(activeCheckbox.getAttribute("role")).toBe("checkbox");
      });
    });

    it("dispatches documentBroadcast with field values", async () => {
      const user = await setupCreateWithFields();
      await waitFor(() => {
        expect(screen.getByTestId("field-displayName")).toBeInTheDocument();
      });

      await user.type(screen.getByTestId("field-displayName"), "Alice");
      await user.type(screen.getByTestId("field-bio"), "Hello world");
      await user.type(screen.getByTestId("field-age"), "30");

      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(commands.documentBroadcast).toHaveBeenCalledWith(
          expect.objectContaining({
            documentTypeName: "profile",
            contractId: TEST_CONTRACT_SUMMARY.id,
            documentJson: expect.objectContaining({
              displayName: "Alice",
              bio: "Hello world",
              age: 30,
            }),
          }),
        );
      });
    });

    it("shows no editable properties message when schema has no properties", async () => {
      const emptyContract = {
        ...CONTRACT_WITH_FIELDS,
        documentTypeNames: ["empty"],
        schemaJson: {
          documentSchemas: {
            empty: {
              properties: {},
              required: [],
            },
          },
        },
      };
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: emptyContract,
      });
      const user = userEvent.setup();
      renderAction("create");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));

      await waitFor(() => {
        expect(screen.getByTestId("no-properties")).toBeInTheDocument();
      });
    });
  });

  // ─── Success screen navigation ─────────────────────────────────────

  describe("Success screen", () => {
    const CONTRACT_DETAIL = {
      id: TEST_CONTRACT_SUMMARY.id,
      ownerId: "ownerhex",
      alias: "TestContract",
      version: 1,
      tokenCount: 0,
      documentTypeNames: ["note"],
      schemaJson: {
        documentSchemas: {
          note: {
            properties: { content: { type: "string" } },
            required: [],
          },
        },
      },
    };

    it("navigates back to contracts from success screen", async () => {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("delete");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "del-doc");
      await waitFor(() => expect(screen.getByTestId("broadcast-button")).not.toBeDisabled());
      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {},
        },
      });

      await waitFor(() => expect(screen.getByTestId("success-screen")).toBeInTheDocument());
      await user.click(screen.getByTestId("back-to-contracts"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
    });

    it("resets form when clicking Do Another from success screen", async () => {
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: CONTRACT_DETAIL,
      });
      const user = userEvent.setup();
      renderAction("delete");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "del-doc");
      await waitFor(() => expect(screen.getByTestId("broadcast-button")).not.toBeDisabled());
      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();
      listener?.({
        payload: {
          taskId: "mock-task-id",
          resultType: "Document",
          payload: {},
        },
      });

      await waitFor(() => expect(screen.getByTestId("success-screen")).toBeInTheDocument());
      await user.click(screen.getByTestId("do-another"));

      // Form should be visible again
      await waitFor(() => {
        expect(screen.queryByTestId("success-screen")).not.toBeInTheDocument();
        expect(screen.getByTestId("document-id-input")).toHaveValue("");
      });
    });
  });

  // ─── Delete: No $ownerId index message ─────────────────────────────

  describe("Delete action - no $ownerId index", () => {
    it("shows cannot fetch message when doc type has no $ownerId index", async () => {
      const contractNoOwnerIndex = {
        id: TEST_CONTRACT_SUMMARY.id,
        ownerId: "ownerhex",
        alias: "TestContract",
        version: 1,
        tokenCount: 0,
        documentTypeNames: ["simple"],
        schemaJson: {
          documentSchemas: {
            simple: {
              properties: { data: { type: "string" } },
              required: [],
              indices: [{ properties: [{ data: "asc" }] }],
            },
          },
        },
      };
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: contractNoOwnerIndex,
      });
      const user = userEvent.setup();
      renderAction("delete");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await waitFor(() => {
        expect(screen.getByText(/Cannot use Fetch Owned Documents/)).toBeInTheDocument();
      });
    });
  });

  // ─── Error dismissal across action types ───────────────────────────

  describe("Error handling", () => {
    it("dismisses error and returns to input state", async () => {
      const contractDetail = {
        id: TEST_CONTRACT_SUMMARY.id,
        ownerId: "ownerhex",
        alias: "TestContract",
        version: 1,
        tokenCount: 0,
        documentTypeNames: ["note"],
        schemaJson: {
          documentSchemas: {
            note: { properties: { msg: { type: "string" } }, required: [] },
          },
        },
      };
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: contractDetail,
      });
      vi.mocked(commands.documentDelete).mockResolvedValue({ status: "error", error: "Backend error" });

      const user = userEvent.setup();
      renderAction("delete");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "bad-doc");

      await waitFor(() => expect(screen.getByTestId("broadcast-button")).not.toBeDisabled());
      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(screen.getByTestId("error-banner")).toBeInTheDocument();
        expect(screen.getByText(/Backend error/)).toBeInTheDocument();
      });

      await user.click(screen.getByTestId("dismiss-error"));

      await waitFor(() => {
        expect(screen.queryByTestId("error-banner")).not.toBeInTheDocument();
      });
    });

    it("shows error when IPC command throws exception", async () => {
      const contractDetail = {
        id: TEST_CONTRACT_SUMMARY.id,
        ownerId: "ownerhex",
        alias: "TestContract",
        version: 1,
        tokenCount: 0,
        documentTypeNames: ["note"],
        schemaJson: {
          documentSchemas: {
            note: { properties: { msg: { type: "string" } }, required: [] },
          },
        },
      };
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: contractDetail,
      });
      vi.mocked(commands.documentTransfer).mockRejectedValue(new Error("Connection refused"));

      const user = userEvent.setup();
      renderAction("transfer");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "doc-1");
      await user.type(screen.getByTestId("recipient-id-input"), "recip-1");

      await waitFor(() => expect(screen.getByTestId("broadcast-button")).not.toBeDisabled());
      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => {
        expect(screen.getByText(/Connection refused/)).toBeInTheDocument();
      });
    });

    it("ignores task result events with non-matching taskId", async () => {
      const contractDetail = {
        id: TEST_CONTRACT_SUMMARY.id,
        ownerId: "ownerhex",
        alias: "TestContract",
        version: 1,
        tokenCount: 0,
        documentTypeNames: ["note"],
        schemaJson: {
          documentSchemas: {
            note: { properties: { msg: { type: "string" } }, required: [] },
          },
        },
      };
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: contractDetail,
      });
      // Ensure documentDelete returns dispatch-ok (previous test may have overridden it)
      vi.mocked(commands.documentDelete).mockResolvedValue({
        status: "ok",
        data: { taskId: "mock-task-id" },
      });
      const user = userEvent.setup();
      renderAction("delete");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await user.type(screen.getByTestId("document-id-input"), "doc-1");
      await waitFor(() => expect(screen.getByTestId("broadcast-button")).not.toBeDisabled());
      await user.click(screen.getByTestId("broadcast-button"));

      await waitFor(() => expect(screen.getByTestId("broadcasting-state")).toBeInTheDocument());
      await waitFor(() => expect(vi.mocked(events.taskResultEvent.listen).mock.calls.length).toBeGreaterThan(0));
      const listener = getLastTaskResultListener();

      // Fire event with wrong taskId — should be ignored
      listener?.({
        payload: {
          taskId: "wrong-task-id",
          resultType: "Document",
          payload: {},
        },
      });

      // Screen should still be in broadcasting state, not success
      expect(screen.getByTestId("broadcasting-state")).toBeInTheDocument();
      expect(screen.queryByTestId("success-screen")).not.toBeInTheDocument();
    });
  });

  // ─── Fee estimation display ────────────────────────────────────────

  describe("Fee estimation", () => {
    it("shows estimated fee when contract and doc type are selected", async () => {
      const contractDetail = {
        id: TEST_CONTRACT_SUMMARY.id,
        ownerId: "ownerhex",
        alias: "TestContract",
        version: 1,
        tokenCount: 0,
        documentTypeNames: ["note"],
        schemaJson: {
          documentSchemas: {
            note: { properties: { msg: { type: "string" } }, required: [] },
          },
        },
      };
      setupStores();
      vi.mocked(commands.contractGetById).mockResolvedValue({
        status: "ok",
        data: contractDetail,
      });
      const user = userEvent.setup();
      renderAction("create");

      await user.click(screen.getByTestId("contract-select"));
      await user.click(screen.getByText("TestContract"));
      await waitFor(() => expect(commands.contractGetById).toHaveBeenCalled());

      await waitFor(() => {
        expect(screen.getByText(/Estimated fee/)).toBeInTheDocument();
      });
    });
  });
});
