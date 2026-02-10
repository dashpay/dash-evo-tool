import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TokenMyTokensScreen } from "./TokenMyTokensScreen";
import { useTokenStore } from "@/stores/tokenStore";
import { renderWithProviders } from "@/test/router-utils";
import type { TokenEntry } from "@/stores/tokenStore";

// ─── Mock Tauri bindings ──────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    tokenQueryMyBalances: vi.fn().mockResolvedValue({ taskId: "task-1" }),
    tokenRemove: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    tokenLoadOrder: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    tokenSaveOrder: vi.fn().mockResolvedValue({ status: "ok", data: null }),
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

// ─── Fixtures ────────────────────────────────────────────────────

function makeToken(overrides: Partial<TokenEntry> = {}): TokenEntry {
  return {
    identityId: "aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb",
    tokenId: "ccdd1122334455667788ccdd1122334455667788ccdd1122334455667788ccdd",
    contractId: "eeff1122334455667788eeff1122334455667788eeff1122334455667788eeff",
    tokenPosition: 0,
    name: "TestToken",
    ownerAlias: "Alice",
    balance: "100000000",
    decimals: 8,
    ...overrides,
  };
}

const defaultTokens = [
  makeToken(),
  makeToken({
    tokenId: "aaaa1122334455667788aaaa1122334455667788aaaa1122334455667788aaaa",
    name: "SecondToken",
    ownerAlias: "Bob",
    balance: "50000000000",
    decimals: 8,
  }),
];

// ─── Helpers ─────────────────────────────────────────────────────

function resetStore(tokens: TokenEntry[] = []) {
  useTokenStore.setState({
    tokens,
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
  return renderWithProviders(<TokenMyTokensScreen />);
}

// ─── Tests ───────────────────────────────────────────────────────

describe("TokenMyTokensScreen — rendering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("renders page title and description", () => {
    renderScreen();
    expect(screen.getByText("My Tokens")).toBeInTheDocument();
    expect(
      screen.getByText(/Manage your token balances/),
    ).toBeInTheDocument();
  });

  it("renders action buttons in header", () => {
    renderScreen();
    expect(screen.getByText("Refresh")).toBeInTheDocument();
    expect(screen.getByText("Add Token by ID")).toBeInTheDocument();
    expect(screen.getByText("Search Tokens")).toBeInTheDocument();
    expect(screen.getByText("Create Token")).toBeInTheDocument();
  });

  it("renders token table with token entries", () => {
    renderScreen();
    expect(screen.getByText("TestToken")).toBeInTheDocument();
    expect(screen.getByText("SecondToken")).toBeInTheDocument();
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("Bob")).toBeInTheDocument();
  });

  it("renders column headers", () => {
    renderScreen();
    expect(screen.getByText("Owner Identity")).toBeInTheDocument();
    expect(screen.getByText("Token Name")).toBeInTheDocument();
    expect(screen.getByText("Balance")).toBeInTheDocument();
  });
});

describe("TokenMyTokensScreen — loading state", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows loading spinner when loading with no tokens", () => {
    resetStore([]);
    useTokenStore.setState({ loading: true });
    renderScreen();
    expect(screen.getByText("Loading tokens...")).toBeInTheDocument();
  });

  it("shows table when loading but tokens already exist", () => {
    resetStore(defaultTokens);
    useTokenStore.setState({ loading: true });
    renderScreen();
    expect(screen.queryByText("Loading tokens...")).not.toBeInTheDocument();
    expect(screen.getByText("TestToken")).toBeInTheDocument();
  });
});

describe("TokenMyTokensScreen — empty state", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore([]);
  });

  it("shows empty state when no tokens and not loading", async () => {
    renderScreen();
    // Wait for the initial load to fire, then simulate loading complete
    await waitFor(() => {
      expect(mockCommands.tokenQueryMyBalances).toHaveBeenCalled();
    });
    // Simulate loading finished with no tokens (as if the event came back empty)
    useTokenStore.setState({ loading: false, tokens: [] });
    await waitFor(() => {
      expect(screen.getByText("No tokens yet")).toBeInTheDocument();
    });
  });
});

describe("TokenMyTokensScreen — data loading", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore([]);
  });

  it("calls loadMyTokenBalances on mount", async () => {
    renderScreen();
    await waitFor(() => {
      expect(mockCommands.tokenQueryMyBalances).toHaveBeenCalled();
    });
  });

  it("subscribes to task events on mount", async () => {
    renderScreen();
    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });
  });
});

describe("TokenMyTokensScreen — error handling", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore([]);
  });

  it("shows error toast when store has error", async () => {
    renderScreen();
    useTokenStore.setState({ error: "Something went wrong" });
    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Something went wrong");
    });
  });
});

describe("TokenMyTokensScreen — refresh", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("calls loadMyTokenBalances when refresh is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    // Wait for the initial load to complete
    await waitFor(() => {
      expect(mockCommands.tokenQueryMyBalances).toHaveBeenCalled();
    });
    // Simulate loading complete so button is enabled
    useTokenStore.setState({ loading: false });
    mockCommands.tokenQueryMyBalances.mockClear();
    await user.click(screen.getByText("Refresh"));
    await waitFor(() => {
      expect(mockCommands.tokenQueryMyBalances).toHaveBeenCalled();
    });
  });

  it("disables refresh button when loading", () => {
    useTokenStore.setState({ loading: true });
    renderScreen();
    const btn = screen.getByText("Refresh").closest("button");
    expect(btn).toBeDisabled();
  });

  it("disables refresh button when refreshing", () => {
    useTokenStore.setState({ refreshing: true });
    renderScreen();
    const btn = screen.getByText("Refresh").closest("button");
    expect(btn).toBeDisabled();
  });
});

describe("TokenMyTokensScreen — navigation buttons", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("navigates to add-by-id when Add Token by ID is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(screen.getByText("Add Token by ID"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens/add-by-id" });
  });

  it("navigates to search when Search Tokens is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(screen.getByText("Search Tokens"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens/search" });
  });

  it("navigates to creator when Create Token is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(screen.getByText("Create Token"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens/creator" });
  });
});

describe("TokenMyTokensScreen — sort interaction", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("calls setSortColumn when column header is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.click(screen.getByText("Owner Identity"));
    // Store sort should have changed
    const { sortColumn } = useTokenStore.getState();
    expect(sortColumn).toBe("ownerAlias");
  });
});

describe("TokenMyTokensScreen — action routing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("navigates to transfer route with token context", async () => {
    const user = userEvent.setup();
    renderScreen();
    // Open dropdown for first token
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    // Click Transfer
    const transferItem = screen.getByText("Transfer");
    await user.click(transferItem);
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/tokens/transfer",
        search: expect.objectContaining({
          tokenId: defaultTokens[0].tokenId,
          contractId: defaultTokens[0].contractId,
        }),
      }),
    );
  });

  it("navigates to mint route with token context", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    const mintItem = screen.getByText("Mint");
    await user.click(mintItem);
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/tokens/mint",
      }),
    );
  });

  it("navigates to burn route with token context", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    const burnItem = screen.getByText("Burn");
    await user.click(burnItem);
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({
        to: "/tokens/burn",
      }),
    );
  });

  it("navigates to freeze route", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    await user.click(screen.getByText("Freeze"));
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ to: "/tokens/freeze" }),
    );
  });

  it("navigates to purchase route", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    await user.click(screen.getByText("Purchase"));
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.objectContaining({ to: "/tokens/purchase" }),
    );
  });
});

describe("TokenMyTokensScreen — More Info dialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("opens TokenInfoDialog when More Info is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    const infoItem = screen.getByText("More Info");
    await user.click(infoItem);
    // Dialog should be open with token name as title
    await waitFor(() => {
      // Dialog renders with token name
      const dialogTitle = screen.getByRole("heading", { name: "TestToken" });
      expect(dialogTitle).toBeInTheDocument();
    });
  });

  it("does not navigate when More Info is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    await user.click(screen.getByText("More Info"));
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});

describe("TokenMyTokensScreen — remove flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore(defaultTokens);
  });

  it("shows confirmation dialog when Remove is clicked", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    await user.click(screen.getByText("Remove"));
    await waitFor(() => {
      expect(screen.getByText("Confirm Remove Token")).toBeInTheDocument();
    });
  });

  it("calls removeToken after confirming removal", async () => {
    const user = userEvent.setup();
    renderScreen();
    const actionButtons = screen.getAllByLabelText(/^Actions for/);
    await user.click(actionButtons[0]);
    await user.click(screen.getByText("Remove"));
    // Click confirm in the dialog
    await waitFor(() => {
      expect(screen.getByText("Confirm")).toBeInTheDocument();
    });
    await user.click(screen.getByText("Confirm"));
    expect(mockCommands.tokenRemove).toHaveBeenCalledWith({
      tokenId: defaultTokens[0].tokenId,
    });
  });
});

describe("TokenMyTokensScreen — event subscription cleanup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore([]);
  });

  it("unsubscribes from events on unmount", async () => {
    const mockUnsubscribe = vi.fn();
    mockEvents.taskResultEvent.listen.mockResolvedValue(mockUnsubscribe);

    const { unmount } = renderScreen();
    // Wait for subscription to be set up
    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });
    unmount();
    // The unsubscribe might be called asynchronously
    await waitFor(() => {
      expect(mockUnsubscribe).toHaveBeenCalled();
    });
  });
});
