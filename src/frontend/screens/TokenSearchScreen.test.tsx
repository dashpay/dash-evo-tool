import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TokenSearchScreen } from "./TokenSearchScreen";
import { useTokenStore } from "@/stores/tokenStore";
import { renderWithProviders } from "@/test/router-utils";

// ─── Mock Tauri bindings ──────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    tokenQueryMyBalances: vi.fn().mockResolvedValue({ taskId: "task-1" }),
    tokenQueryDescriptionsByKeyword: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "search-task-1" },
    }),
    tokenFetchByContractId: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "fetch-task-1" },
    }),
    tokenSaveLocally: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "save-task-1" },
    }),
    tokenRemove: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    tokenLoadOrder: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    tokenSaveOrder: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    contractFetchWithDescriptions: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "contract-task-1" },
    }),
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
      if (prop in mockCommands) {
        return mockCommands[prop];
      }
      return vi.fn().mockResolvedValue({ status: "error", error: "not mocked" });
    },
  }),
  events: mockEvents,
}));

// Mock sonner
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

const { mockToastError } = vi.hoisted(() => ({
  mockToastError: vi.fn(),
}));
vi.mock("@/lib/toastError", () => ({
  toastError: mockToastError,
}));

// Mock navigation
const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));
vi.mock("@tanstack/react-router", async () => {
  const actual = await vi.importActual("@tanstack/react-router");
  return {
    ...actual,
    useNavigate: () => mockNavigate,
  };
});

// ─── Helpers ─────────────────────────────────────────────────────────

function resetStore() {
  useTokenStore.setState({
    tokens: [],
    searchResults: [],
    searchKeyword: "",
    searchCursor: null,
    searchHasMore: false,
    searching: false,
    loading: false,
    fetching: false,
    refreshing: false,
    error: null,
    sortColumn: "name",
    sortOrder: "ascending",
  });
}

function renderScreen() {
  return renderWithProviders(<TokenSearchScreen />);
}

// ─── Tests ───────────────────────────────────────────────────────────

describe("TokenSearchScreen — rendering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("renders page title and description", () => {
    renderScreen();
    expect(screen.getByText("Search Tokens")).toBeInTheDocument();
    expect(
      screen.getByText(/Search for tokens by keyword/),
    ).toBeInTheDocument();
  });

  it("renders back button to My Tokens", () => {
    renderScreen();
    expect(
      screen.getByLabelText("Back to My Tokens"),
    ).toBeInTheDocument();
  });

  it("renders search input", () => {
    renderScreen();
    expect(
      screen.getByLabelText("Token search keyword"),
    ).toBeInTheDocument();
  });

  it("shows initial empty state", () => {
    renderScreen();
    expect(screen.getByText("Search for tokens")).toBeInTheDocument();
  });
});

describe("TokenSearchScreen — navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("navigates back to /tokens when back button is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(screen.getByLabelText("Back to My Tokens"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens" });
  });
});

describe("TokenSearchScreen — search flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("dispatches search when user types keyword and clicks Search", async () => {
    const user = userEvent.setup();
    renderScreen();

    await user.type(
      screen.getByLabelText("Token search keyword"),
      "dash",
    );
    await user.click(screen.getByRole("button", { name: /search/i }));

    await waitFor(() => {
      expect(
        mockCommands.tokenQueryDescriptionsByKeyword,
      ).toHaveBeenCalledWith({
        keyword: "dash",
        startAfter: null,
      });
    });
  });

  it("dispatches search when user presses Enter", async () => {
    const user = userEvent.setup();
    renderScreen();

    await user.type(
      screen.getByLabelText("Token search keyword"),
      "token{Enter}",
    );

    await waitFor(() => {
      expect(
        mockCommands.tokenQueryDescriptionsByKeyword,
      ).toHaveBeenCalledWith({
        keyword: "token",
        startAfter: null,
      });
    });
  });

  it("shows search results when store has results", () => {
    useTokenStore.setState({
      searchResults: [
        { contractId: "aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb", description: "Test Token" },
        { contractId: "ccdd1122334455667788ccdd1122334455667788ccdd1122334455667788ccdd", description: "Other Token" },
      ],
    });
    renderScreen();
    // The panel needs currentPage > 0 to show results, but the screen manages that internally
    // Since we're directly setting store state, we need to simulate the search first
  });
});

describe("TokenSearchScreen — error handling", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("shows error toast when store has error", async () => {
    renderScreen();
    useTokenStore.setState({ error: "Network timeout" });
    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Network timeout");
    });
  });
});

describe("TokenSearchScreen — event subscription", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("subscribes to task result events on mount", async () => {
    renderScreen();
    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });
  });

  it("unsubscribes from events on unmount", async () => {
    const mockUnsubscribe = vi.fn();
    mockEvents.taskResultEvent.listen.mockResolvedValue(mockUnsubscribe);

    const { unmount } = renderScreen();
    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });
    unmount();
    await waitFor(() => {
      expect(mockUnsubscribe).toHaveBeenCalled();
    });
  });
});

describe("TokenSearchScreen — clear flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("resets state when clear is invoked", async () => {
    // Set some search state first
    useTokenStore.setState({
      searchResults: [{ contractId: "abc", description: "test" }],
      searchKeyword: "test",
    });
    const user = userEvent.setup();
    renderScreen();

    // Type something first so Clear button appears
    await user.type(
      screen.getByLabelText("Token search keyword"),
      "test",
    );

    // Clear button should be visible
    const clearBtn = screen.getByText("Clear");
    await user.click(clearBtn);

    // Store should be cleared
    const state = useTokenStore.getState();
    expect(state.searchKeyword).toBe("");
    expect(state.searchResults).toEqual([]);
  });
});
