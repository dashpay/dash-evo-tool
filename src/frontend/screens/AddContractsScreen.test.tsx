import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AddContractsScreen } from "./AddContractsScreen";
import { useContractStore } from "@/stores/contractStore";

// ─── Hoisted mocks (required for vi.mock factory) ─────────────────

type EventCallback = (event: { payload: unknown }) => void;

const { mockCommands, mockEvents, eventListeners, mockNavigate } = vi.hoisted(
  () => {
    const eventListeners: Record<string, EventCallback[]> = {
      taskResultEvent: [],
      taskErrorEvent: [],
    };

    const mockEvents = {
      taskResultEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskResultEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskResultEvent =
              eventListeners.taskResultEvent.filter((l) => l !== cb);
          });
        }),
      },
      taskErrorEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskErrorEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskErrorEvent =
              eventListeners.taskErrorEvent.filter((l) => l !== cb);
          });
        }),
      },
    };

    const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
      contractFetch: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: { taskId: "task-1" } }),
      contractSetAlias: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: null }),
      contractListLocal: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: [] }),
      contractGetById: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: null }),
    };

    const mockNavigate = vi.fn();

    return { mockCommands, mockEvents, eventListeners, mockNavigate };
  },
);

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/bindings", () => ({
  commands: new Proxy({} as Record<string, unknown>, {
    get: (_target, prop: string) => {
      if (prop in mockCommands) return mockCommands[prop];
      return vi.fn().mockResolvedValue({ status: "error", error: "not mocked" });
    },
  }),
  events: mockEvents,
}));

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

// ─── Helpers ─────────────────────────────────────────────────────

const CONTRACT_ID_1 =
  "abc123def456abc123def456abc123def456abc123def456abc123def456abcd";
const CONTRACT_ID_2 =
  "1111222233334444555566667777888899990000aaaabbbbccccddddeeee0001";

function fireTaskResult(payload: unknown) {
  act(() => {
    for (const listener of eventListeners.taskResultEvent) {
      listener({ payload });
    }
  });
}

function fireTaskError(payload: unknown) {
  act(() => {
    for (const listener of eventListeners.taskErrorEvent) {
      listener({ payload });
    }
  });
}

function resetStores() {
  useContractStore.setState({
    contracts: [],
    selectedContractId: null,
    selectedContractDetail: null,
    loading: false,
    fetching: false,
    error: null,
  });
}

// ─── Tests ───────────────────────────────────────────────────────

describe("AddContractsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    eventListeners.taskResultEvent = [];
    eventListeners.taskErrorEvent = [];
    resetStores();
    // Default: contractListLocal returns empty list
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [],
    });
  });

  // --- Rendering ---

  it("renders the page header with title", () => {
    render(<AddContractsScreen />);
    expect(
      screen.getByRole("heading", { name: /add contracts/i }),
    ).toBeInTheDocument();
  });

  it("renders breadcrumbs", () => {
    render(<AddContractsScreen />);
    expect(screen.getByText("Contracts")).toBeInTheDocument();
  });

  it("renders back to contracts button in header", () => {
    render(<AddContractsScreen />);
    expect(
      screen.getByRole("button", { name: /back to contracts/i }),
    ).toBeInTheDocument();
  });

  it("renders one contract input field initially", () => {
    render(<AddContractsScreen />);
    const inputs = screen.getAllByPlaceholderText(/hex or base58/i);
    expect(inputs).toHaveLength(1);
  });

  it("renders 'Contract 1:' label", () => {
    render(<AddContractsScreen />);
    expect(screen.getByText("Contract 1:")).toBeInTheDocument();
  });

  it("renders add another field button", () => {
    render(<AddContractsScreen />);
    expect(
      screen.getByRole("button", { name: /add another contract field/i }),
    ).toBeInTheDocument();
  });

  it("renders add contracts (fetch) button", () => {
    render(<AddContractsScreen />);
    expect(
      screen.getByRole("button", { name: /add contracts/i }),
    ).toBeInTheDocument();
  });

  it("renders description text", () => {
    render(<AddContractsScreen />);
    expect(
      screen.getByText(/enter up to 10 contract ids/i),
    ).toBeInTheDocument();
  });

  // --- Adding/removing fields ---

  it("adds a new input field when 'Add Another' is clicked", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await user.click(
      screen.getByRole("button", { name: /add another contract field/i }),
    );

    const inputs = screen.getAllByPlaceholderText(/hex or base58/i);
    expect(inputs).toHaveLength(2);
    expect(screen.getByText("Contract 2:")).toBeInTheDocument();
  });

  it("hides add button when max fields reached", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    // Add 9 more fields (total 10)
    for (let i = 0; i < 9; i++) {
      await user.click(
        screen.getByRole("button", { name: /add another contract field/i }),
      );
    }

    expect(screen.getAllByPlaceholderText(/hex or base58/i)).toHaveLength(10);
    expect(
      screen.queryByRole("button", { name: /add another contract field/i }),
    ).not.toBeInTheDocument();
  });

  it("removes a field when trash button is clicked", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    // Add second field
    await user.click(
      screen.getByRole("button", { name: /add another contract field/i }),
    );
    expect(screen.getAllByPlaceholderText(/hex or base58/i)).toHaveLength(2);

    // Remove first field
    const removeButtons = screen.getAllByRole("button", {
      name: /remove contract field/i,
    });
    await user.click(removeButtons[0]);

    expect(screen.getAllByPlaceholderText(/hex or base58/i)).toHaveLength(1);
  });

  it("does not show remove button for single field", () => {
    render(<AddContractsScreen />);
    expect(
      screen.queryByRole("button", { name: /remove contract field/i }),
    ).not.toBeInTheDocument();
  });

  // --- Input validation and fetch ---

  it("disables fetch button when all inputs are empty", () => {
    render(<AddContractsScreen />);
    const button = screen.getByRole("button", { name: /add contracts$/i });
    expect(button).toBeDisabled();
  });

  it("enables fetch button when at least one input has text", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    const button = screen.getByRole("button", { name: /add contracts$/i });
    expect(button).not.toBeDisabled();
  });

  it("dispatches contractFetch with hex ID", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(mockCommands.contractFetch).toHaveBeenCalledWith({
      contractIds: [CONTRACT_ID_1],
    });
  });

  it("dispatches contractFetch with base58 ID", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, "7TmjsEviZVDaGkCN3Rnz5D");

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(mockCommands.contractFetch).toHaveBeenCalledWith({
      contractIds: ["7TmjsEviZVDaGkCN3Rnz5D"],
    });
  });

  it("dispatches contractFetch with multiple IDs", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    // Add second field
    await user.click(
      screen.getByRole("button", { name: /add another contract field/i }),
    );

    const inputs = screen.getAllByPlaceholderText(/hex or base58/i);
    await user.type(inputs[0], CONTRACT_ID_1);
    await user.type(inputs[1], CONTRACT_ID_2);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(mockCommands.contractFetch).toHaveBeenCalledWith({
      contractIds: [CONTRACT_ID_1, CONTRACT_ID_2],
    });
  });

  it("skips empty fields when fetching", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    // Add second field, only fill the first
    await user.click(
      screen.getByRole("button", { name: /add another contract field/i }),
    );

    const inputs = screen.getAllByPlaceholderText(/hex or base58/i);
    await user.type(inputs[0], CONTRACT_ID_1);
    // Leave inputs[1] empty

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(mockCommands.contractFetch).toHaveBeenCalledWith({
      contractIds: [CONTRACT_ID_1],
    });
  });

  it("shows error for invalid ID format", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, "not-a-valid-id!!!!");

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(screen.getByText(/invalid id/i)).toBeInTheDocument();
  });

  it("keeps fetch button disabled when only whitespace is entered", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, "   ");

    const button = screen.getByRole("button", { name: /add contracts$/i });
    expect(button).toBeDisabled();
  });

  it("dismisses error and returns to input", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, "invalid-id!!!");

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(screen.getByText(/invalid id/i)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /dismiss/i }));

    expect(screen.queryByText(/invalid id/i)).not.toBeInTheDocument();
    // Input fields should still be visible
    expect(screen.getByPlaceholderText(/hex or base58/i)).toBeInTheDocument();
  });

  it("dispatches fetch on Enter key press", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);
    await user.keyboard("{Enter}");

    expect(mockCommands.contractFetch).toHaveBeenCalledWith({
      contractIds: [CONTRACT_ID_1],
    });
  });

  // --- Fetching / loading state ---

  it("shows loading state while fetching", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(screen.getByText(/fetching contracts/i)).toBeInTheDocument();
  });

  it("shows elapsed time during loading", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    expect(screen.getByText(/time taken so far/i)).toBeInTheDocument();
  });

  // --- Success state ---

  it("shows success screen with found contracts", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    // Set up store to contain the found contract after reload
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 3, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(
        screen.getByText(/successfully queried contracts/i),
      ).toBeInTheDocument();
    });
  });

  it("shows found contract ID on success", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(screen.getByText(CONTRACT_ID_1)).toBeInTheDocument();
    });
  });

  it("shows not-found contracts in red", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    // No contracts returned after fetch
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(
        screen.getByText(/the following contracts were not found/i),
      ).toBeInTheDocument();
    });
  });

  it("shows both found and not-found sections for mixed results", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    // Only first contract found
    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    // Add second field
    await user.click(
      screen.getByRole("button", { name: /add another contract field/i }),
    );

    const inputs = screen.getAllByPlaceholderText(/hex or base58/i);
    await user.type(inputs[0], CONTRACT_ID_1);
    await user.type(inputs[1], CONTRACT_ID_2);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(
        screen.getByText(/found and added the following contracts/i),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/the following contracts were not found/i),
      ).toBeInTheDocument();
    });
  });

  // --- Alias editing ---

  it("renders alias input for found contracts", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/enter alias/i)).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /set alias/i }),
      ).toBeInTheDocument();
    });
  });

  it("sets alias via IPC command", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/enter alias/i)).toBeInTheDocument();
    });

    const aliasInput = screen.getByPlaceholderText(/enter alias/i);
    await user.type(aliasInput, "my-contract");

    await user.click(screen.getByRole("button", { name: /set alias/i }));

    expect(mockCommands.contractSetAlias).toHaveBeenCalledWith({
      contractId: CONTRACT_ID_1,
      alias: "my-contract",
    });
  });

  it("shows success message after alias is set", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/enter alias/i)).toBeInTheDocument();
    });

    const aliasInput = screen.getByPlaceholderText(/enter alias/i);
    await user.type(aliasInput, "test-alias");

    await user.click(screen.getByRole("button", { name: /set alias/i }));

    await waitFor(() => {
      expect(
        screen.getByText(/alias set successfully.*test-alias/i),
      ).toBeInTheDocument();
    });
  });

  it("shows error when alias is empty", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /set alias/i }),
      ).toBeInTheDocument();
    });

    // Click set alias without entering text
    await user.click(screen.getByRole("button", { name: /set alias/i }));

    await waitFor(() => {
      expect(screen.getByText(/alias cannot be empty/i)).toBeInTheDocument();
    });
  });

  it("sets alias on Enter key press in alias input", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/enter alias/i)).toBeInTheDocument();
    });

    const aliasInput = screen.getByPlaceholderText(/enter alias/i);
    await user.type(aliasInput, "enter-alias");
    await user.keyboard("{Enter}");

    expect(mockCommands.contractSetAlias).toHaveBeenCalledWith({
      contractId: CONTRACT_ID_1,
      alias: "enter-alias",
    });
  });

  // --- Error handling ---

  it("shows error on task error event", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskError({
      taskId: "task-1",
      message: "Network connection failed",
      details: "timeout",
    });

    await waitFor(() => {
      expect(
        screen.getByText("Network connection failed"),
      ).toBeInTheDocument();
    });
  });

  it("handles IPC dispatch error gracefully", async () => {
    const user = userEvent.setup();
    mockCommands.contractFetch.mockRejectedValueOnce(
      new Error("IPC unavailable"),
    );

    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    await waitFor(() => {
      expect(screen.getByText("IPC unavailable")).toBeInTheDocument();
    });
  });

  it("handles contractFetch returning error status", async () => {
    const user = userEvent.setup();
    mockCommands.contractFetch.mockResolvedValueOnce({
      status: "error",
      error: "Backend is not initialized",
    });

    render(<AddContractsScreen />);

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    await waitFor(() => {
      expect(
        screen.getByText("Backend is not initialized"),
      ).toBeInTheDocument();
    });
  });

  // --- Navigation ---

  it("navigates back to contracts on header button click", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await user.click(
      screen.getByRole("button", { name: /back to contracts/i }),
    );

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
  });

  it("shows 'Back to Contracts' button on success screen", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      const backButtons = screen.getAllByRole("button", {
        name: /back to contracts/i,
      });
      expect(backButtons.length).toBeGreaterThanOrEqual(1);
    });
  });

  // --- Event subscription ---

  it("subscribes to task events on mount", async () => {
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalledTimes(1);
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalledTimes(1);
    });
  });

  it("ignores results from other task types", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    // Fire an Identity result — should be ignored
    fireTaskResult({
      taskId: "task-1",
      resultType: "Identity",
      payload: null,
    });

    // Should still show loading
    expect(screen.getByText(/fetching contracts/i)).toBeInTheDocument();
  });

  it("ignores results from other task IDs", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    // Fire a Contract result with different task ID
    fireTaskResult({
      taskId: "different-task",
      resultType: "Contract",
      payload: null,
    });

    // Should still show loading
    expect(screen.getByText(/fetching contracts/i)).toBeInTheDocument();
  });

  // --- Alias error handling ---

  it("shows error when contractSetAlias fails", async () => {
    const user = userEvent.setup();
    mockCommands.contractSetAlias.mockResolvedValueOnce({
      status: "error",
      error: "DB write failed",
    });

    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/enter alias/i)).toBeInTheDocument();
    });

    const aliasInput = screen.getByPlaceholderText(/enter alias/i);
    await user.type(aliasInput, "fail-alias");

    await user.click(screen.getByRole("button", { name: /set alias/i }));

    await waitFor(() => {
      expect(screen.getByText(/failed to set alias/i)).toBeInTheDocument();
    });
  });

  // --- Copy button ---

  it("renders copy button for found contract IDs", async () => {
    const user = userEvent.setup();
    render(<AddContractsScreen />);

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });

    mockCommands.contractListLocal.mockResolvedValue({
      status: "ok",
      data: [{ id: CONTRACT_ID_1, alias: null, documentTypeCount: 1, tokenCount: 0 }],
    });

    const input = screen.getByPlaceholderText(/hex or base58/i);
    await user.type(input, CONTRACT_ID_1);

    await user.click(
      screen.getByRole("button", { name: /add contracts$/i }),
    );

    fireTaskResult({
      taskId: "task-1",
      resultType: "Contract",
      payload: null,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /copy/i }),
      ).toBeInTheDocument();
    });
  });
});
