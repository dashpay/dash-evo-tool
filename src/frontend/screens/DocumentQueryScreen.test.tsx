import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DocumentQueryScreen } from "./DocumentQueryScreen";
import { useContractStore } from "@/stores/contractStore";
import { useDocumentStore } from "@/stores/documentStore";
import { renderWithProviders } from "@/test/router-utils";
import type { ContractSummaryDto } from "@/bindings";
import type { DocumentPageEntry } from "@/stores/documentStore";

// ─── Mock Tauri bindings ──────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    contractListLocal: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    contractGetById: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    contractRemove: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    contractSetAlias: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    contractFetch: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t1" } }),
    contractFetchWithDescriptions: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t2" } }),
    contractSave: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t3" } }),
    documentFetchPage: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t4" } }),
  };
  const mockEvents = {
    taskResultEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    taskErrorEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
  };
  return { mockCommands, mockEvents };
});

vi.mock("@/bindings", () => ({
  commands: new Proxy({} as Record<string, unknown>, {
    get: (_target, prop: string) => {
      if (prop in mockCommands) return mockCommands[prop];
      return vi.fn().mockResolvedValue({ status: "error", error: "not mocked" });
    },
  }),
  events: mockEvents,
}));

// Mock sonner toast
const { mockToast } = vi.hoisted(() => ({
  mockToast: {
    error: vi.fn(),
    success: vi.fn(),
    info: vi.fn(),
  },
}));
vi.mock("sonner", () => ({
  toast: mockToast,
  Toaster: () => null,
}));

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
}));

// ─── Test Fixtures ────────────────────────────────────────────────

const CONTRACT_ID = "abc123def456abc123def456abc123def456abc123def456abc123def456abcd";

function makeSummary(overrides: Partial<ContractSummaryDto> = {}): ContractSummaryDto {
  return {
    id: CONTRACT_ID,
    alias: "dpns",
    documentTypeCount: 2,
    tokenCount: 0,
    ...overrides,
  };
}

function makeDocEntry(overrides: Partial<DocumentPageEntry> = {}): DocumentPageEntry {
  return {
    id: "doc001",
    document: {
      id: "doc001",
      ownerId: "owner001",
      documentType: "domain",
      data: { label: "alice", normalizedLabel: "alice" },
      revision: 1,
      createdAt: 1700000000000,
      updatedAt: null,
      transferredAt: null,
    },
    ...overrides,
  };
}

// ─── Store Reset ─────────────────────────────────────────────────

function resetStores() {
  useContractStore.setState({
    contracts: [],
    selectedContractId: null,
    selectedContractDetail: null,
    loading: false,
    fetching: false,
    error: null,
  });
  useDocumentStore.setState({
    queryText: "",
    whereClauses: [],
    orderByClauses: [],
    documents: [],
    queryStatus: "idle",
    queryStartedAt: null,
    queryError: null,
    activeTaskId: null,
    displayMode: "json",
    searchFilter: "",
    fieldSelection: {},
    currentPage: 1,
    nextCursors: [null],
    hasNextPage: false,
    queryContractId: null,
    queryDocumentType: null,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
});

// ─── Tests ───────────────────────────────────────────────────────

describe("DocumentQueryScreen", () => {
  it("renders the screen with tree panel and query area", async () => {
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("document-query-screen")).toBeInTheDocument();
    expect(screen.getByTestId("contract-tree-panel")).toBeInTheDocument();
    expect(screen.getByTestId("query-input")).toBeInTheDocument();
    expect(screen.getByTestId("fetch-documents-btn")).toBeInTheDocument();
  });

  it("shows page header with title", async () => {
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByText("Document Query")).toBeInTheDocument();
  });

  it("shows action buttons in toolbar", async () => {
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("action-load-contracts")).toBeInTheDocument();
    expect(screen.getByTestId("action-register-contract")).toBeInTheDocument();
    expect(screen.getByTestId("action-update-contract")).toBeInTheDocument();
    expect(screen.getByTestId("action-create-document")).toBeInTheDocument();
    expect(screen.getByTestId("action-delete-document")).toBeInTheDocument();
    expect(screen.getByTestId("action-replace-document")).toBeInTheDocument();
    expect(screen.getByTestId("action-transfer-document")).toBeInTheDocument();
    expect(screen.getByTestId("action-purchase-document")).toBeInTheDocument();
    expect(screen.getByTestId("action-set-document-price")).toBeInTheDocument();
    expect(screen.getByTestId("action-group-actions")).toBeInTheDocument();
  });

  it("shows empty state when no query has been run", async () => {
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByText("Query Documents")).toBeInTheDocument();
  });

  it("loads contracts on mount", async () => {
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [makeSummary()],
    });
    renderWithProviders(<DocumentQueryScreen />);
    await waitFor(() => {
      expect(mockCommands.contractListLocal).toHaveBeenCalled();
    });
  });

  it("shows fetch button disabled when no contract/doctype selected", () => {
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("fetch-documents-btn")).toBeDisabled();
  });

  it("shows error toast when fetch clicked without contract selected", async () => {
    // Set queryContractId/queryDocumentType to null (default)
    renderWithProviders(<DocumentQueryScreen />);
    // Button should be disabled, but let's verify
    expect(screen.getByTestId("fetch-documents-btn")).toBeDisabled();
  });

  it("updates query text in the input field", async () => {
    const user = userEvent.setup();
    renderWithProviders(<DocumentQueryScreen />);
    const input = screen.getByTestId("query-input");
    await user.clear(input);
    await user.type(input, "SELECT * FROM domain");
    expect(input).toHaveValue("SELECT * FROM domain");
  });

  it("enables fetch button when contract and doc type are selected", async () => {
    // Pre-set store state as if user selected a doc type
    useDocumentStore.setState({
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
      queryText: "SELECT * FROM domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("fetch-documents-btn")).not.toBeDisabled();
  });

  it("dispatches fetch when fetch button is clicked", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
      queryText: "SELECT * FROM domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    await user.click(screen.getByTestId("fetch-documents-btn"));
    await waitFor(() => {
      expect(mockCommands.documentFetchPage).toHaveBeenCalledWith({
        contractId: CONTRACT_ID,
        documentTypeName: "domain",
        whereClauses: [],
        orderByClauses: [],
        startAfter: null,
      });
    });
  });

  it("shows loading state when query is waiting", () => {
    useDocumentStore.setState({
      queryStatus: "waiting",
      queryStartedAt: Date.now(),
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("query-loading")).toBeInTheDocument();
    expect(screen.getByText(/Fetching documents/)).toBeInTheDocument();
  });

  it("shows error message when query fails", () => {
    useDocumentStore.setState({
      queryStatus: "error",
      queryError: "Network timeout",
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("query-error")).toBeInTheDocument();
    expect(screen.getByText("Network timeout")).toBeInTheDocument();
  });

  it("shows no documents message when query completes with empty results", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [],
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("no-documents")).toBeInTheDocument();
    expect(screen.getByText("No documents found.")).toBeInTheDocument();
  });

  it("renders document results in JSON mode", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      displayMode: "json",
      fieldSelection: { label: true, normalizedLabel: true, $id: true, $ownerId: false },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const results = screen.getByTestId("document-results");
    expect(results).toBeInTheDocument();
    expect(results.textContent).toContain('"label"');
    expect(results.textContent).toContain('"alice"');
    expect(results.textContent).toContain('"$id"');
  });

  it("renders document results in YAML mode", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      displayMode: "yaml",
      fieldSelection: { label: true, normalizedLabel: false },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const results = screen.getByTestId("document-results");
    expect(results.textContent).toContain("label:");
    expect(results.textContent).toContain('"alice"');
  });

  it("shows controls row when documents exist", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("search-filter-input")).toBeInTheDocument();
    expect(screen.getByTestId("select-properties-btn")).toBeInTheDocument();
    expect(screen.getByTestId("display-mode-yaml")).toBeInTheDocument();
    expect(screen.getByTestId("display-mode-json")).toBeInTheDocument();
  });

  it("toggles display mode between JSON and YAML", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      displayMode: "json",
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    // Switch to YAML
    await user.click(screen.getByTestId("display-mode-yaml"));
    expect(useDocumentStore.getState().displayMode).toBe("yaml");
    // Switch back to JSON
    await user.click(screen.getByTestId("display-mode-json"));
    expect(useDocumentStore.getState().displayMode).toBe("json");
  });

  it("filters documents by search term", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [
        makeDocEntry({ id: "d1", document: { id: "d1", ownerId: "o1", documentType: "domain", data: { label: "alice" }, revision: 1, createdAt: null, updatedAt: null, transferredAt: null } }),
        makeDocEntry({ id: "d2", document: { id: "d2", ownerId: "o2", documentType: "domain", data: { label: "bob" }, revision: 1, createdAt: null, updatedAt: null, transferredAt: null } }),
      ],
      displayMode: "json",
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const filterInput = screen.getByTestId("search-filter-input");
    await user.type(filterInput, "bob");
    const results = screen.getByTestId("document-results");
    expect(results.textContent).toContain("bob");
    expect(results.textContent).not.toContain("alice");
  });

  it("shows pagination controls when results exist", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      currentPage: 1,
      hasNextPage: true,
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("pagination-controls")).toBeInTheDocument();
    expect(screen.getByTestId("page-indicator")).toHaveTextContent("Page 1");
    expect(screen.getByTestId("previous-page-btn")).toBeDisabled();
    expect(screen.getByTestId("next-page-btn")).not.toBeDisabled();
  });

  it("disables next page button when no more pages", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      currentPage: 1,
      hasNextPage: false,
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("next-page-btn")).toBeDisabled();
  });

  it("shows page 2 indicator after navigating forward", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      currentPage: 2,
      hasNextPage: false,
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("page-indicator")).toHaveTextContent("Page 2");
    expect(screen.getByTestId("previous-page-btn")).not.toBeDisabled();
  });

  it("opens field selection dialog when Select Properties is clicked", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      fieldSelection: { label: true, normalizedLabel: false, $id: true, $ownerId: false },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    await user.click(screen.getByTestId("select-properties-btn"));
    expect(screen.getByTestId("field-selection-dialog")).toBeInTheDocument();
    expect(screen.getByText("Document Properties")).toBeInTheDocument();
    expect(screen.getByText("Universal Properties")).toBeInTheDocument();
  });

  it("toggles field visibility in field selection dialog", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      fieldSelection: { label: true, normalizedLabel: false, $id: true, $ownerId: false },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    await user.click(screen.getByTestId("select-properties-btn"));
    // Toggle normalizedLabel on
    const checkbox = screen.getByTestId("field-checkbox-normalizedLabel");
    await user.click(checkbox);
    expect(useDocumentStore.getState().fieldSelection.normalizedLabel).toBe(true);
  });

  it("select all / deselect all in field dialog", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      fieldSelection: { label: true, normalizedLabel: false, $id: false },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    await user.click(screen.getByTestId("select-properties-btn"));

    // Deselect all
    await user.click(screen.getByTestId("deselect-all-fields"));
    const state1 = useDocumentStore.getState().fieldSelection;
    expect(Object.values(state1).every((v) => v === false)).toBe(true);

    // Select all
    await user.click(screen.getByTestId("select-all-fields"));
    const state2 = useDocumentStore.getState().fieldSelection;
    expect(Object.values(state2).every((v) => v === true)).toBe(true);
  });

  it("closes field selection dialog", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    await user.click(screen.getByTestId("select-properties-btn"));
    expect(screen.getByTestId("field-selection-dialog")).toBeInTheDocument();
    await user.click(screen.getByTestId("close-field-dialog"));
    await waitFor(() => {
      expect(screen.queryByTestId("field-selection-dialog")).not.toBeInTheDocument();
    });
  });

  it("renders contract tree panel with loaded contracts", async () => {
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [makeSummary({ alias: "dpns" }), makeSummary({ id: "bbb", alias: "dashpay" })],
    });
    renderWithProviders(<DocumentQueryScreen />);
    await waitFor(() => {
      expect(screen.getByText("DPNS")).toBeInTheDocument();
    });
  });

  it("shows no filtered documents message when search term matches nothing", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      displayMode: "json",
      fieldSelection: { label: true },
      searchFilter: "nonexistent_term_xyz",
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.getByTestId("no-filtered-documents")).toBeInTheDocument();
  });

  it("fetch is triggered on Enter key in query input", async () => {
    const user = userEvent.setup();
    useDocumentStore.setState({
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
      queryText: "SELECT * FROM domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const input = screen.getByTestId("query-input");
    await user.click(input);
    await user.keyboard("{Enter}");
    await waitFor(() => {
      expect(mockCommands.documentFetchPage).toHaveBeenCalled();
    });
  });

  it("does not show pagination when no documents", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [],
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    expect(screen.queryByTestId("pagination-controls")).not.toBeInTheDocument();
  });

  it("multiple documents are separated by dividers", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [
        makeDocEntry({ id: "d1", document: { id: "d1", ownerId: "o1", documentType: "domain", data: { label: "alice" }, revision: 1, createdAt: null, updatedAt: null, transferredAt: null } }),
        makeDocEntry({ id: "d2", document: { id: "d2", ownerId: "o2", documentType: "domain", data: { label: "bob" }, revision: 1, createdAt: null, updatedAt: null, transferredAt: null } }),
      ],
      displayMode: "json",
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const results = screen.getByTestId("document-results");
    expect(results.textContent).toContain("---");
  });

  it("filters out $ownerId when field is deselected", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      displayMode: "json",
      fieldSelection: { label: true, $id: true, $ownerId: false },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const results = screen.getByTestId("document-results");
    expect(results.textContent).toContain('"$id"');
    expect(results.textContent).not.toContain('"$ownerId"');
  });

  it("shows $id and $ownerId system fields when selected", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [makeDocEntry()],
      displayMode: "json",
      fieldSelection: { $id: true, $ownerId: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const results = screen.getByTestId("document-results");
    expect(results.textContent).toContain('"doc001"');
    expect(results.textContent).toContain('"owner001"');
  });

  it("handles null document entries gracefully", () => {
    useDocumentStore.setState({
      queryStatus: "complete",
      documents: [
        makeDocEntry(),
        { id: "deleted", document: null },
      ],
      displayMode: "json",
      fieldSelection: { label: true },
      queryContractId: CONTRACT_ID,
      queryDocumentType: "domain",
    });
    renderWithProviders(<DocumentQueryScreen />);
    const results = screen.getByTestId("document-results");
    expect(results.textContent).toContain("alice");
    // Null doc entry should just be skipped
    expect(results.textContent).not.toContain("deleted");
  });
});
