import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TokenAddByIdScreen } from "./TokenAddByIdScreen";
import { useTokenStore } from "@/stores/tokenStore";
import { renderWithProviders } from "@/test/router-utils";

// ─── Mock Tauri bindings ──────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    tokenFetchByContractId: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "fetch-contract-1" },
    }),
    tokenFetchByTokenId: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "fetch-token-1" },
    }),
    tokenSaveLocally: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "save-task-1" },
    }),
    tokenQueryMyBalances: vi.fn().mockResolvedValue({ taskId: "load-1" }),
    tokenRemove: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    tokenLoadOrder: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    tokenSaveOrder: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    tokenQueryDescriptionsByKeyword: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "search-1" },
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
      return vi
        .fn()
        .mockResolvedValue({ status: "error", error: "not mocked" });
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
  return renderWithProviders(<TokenAddByIdScreen />);
}

const VALID_HEX_64 =
  "aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb";

// ─── Tests ───────────────────────────────────────────────────────────

describe("TokenAddByIdScreen — rendering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("renders page title and description", () => {
    renderScreen();
    expect(screen.getByText("Add Token by ID")).toBeInTheDocument();
    expect(
      screen.getByText(/Look up a token by its contract ID/),
    ).toBeInTheDocument();
  });

  it("renders back button to My Tokens", () => {
    renderScreen();
    expect(screen.getByLabelText("Back to My Tokens")).toBeInTheDocument();
  });

  it("renders search input with placeholder", () => {
    renderScreen();
    const input = screen.getByLabelText("Contract or token ID");
    expect(input).toBeInTheDocument();
    expect(input).toHaveAttribute(
      "placeholder",
      "Enter contract ID or token ID (hex)...",
    );
  });

  it("renders Search button", () => {
    renderScreen();
    expect(screen.getByLabelText("Search")).toBeInTheDocument();
  });

  it("Search button is disabled when input is empty", () => {
    renderScreen();
    expect(screen.getByLabelText("Search")).toBeDisabled();
  });

  it("shows idle state with instruction message", () => {
    renderScreen();
    expect(
      screen.getByText(/Enter a contract ID or token ID/),
    ).toBeInTheDocument();
  });

  it("does not show Clear button in idle state with empty input", () => {
    renderScreen();
    expect(screen.queryByLabelText("Clear")).not.toBeInTheDocument();
  });
});

describe("TokenAddByIdScreen — navigation", () => {
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

describe("TokenAddByIdScreen — search interaction", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("enables Search button when input has text", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    expect(screen.getByLabelText("Search")).toBeEnabled();
  });

  it("shows Clear button when input has text", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      "abc123",
    );
    expect(screen.getByLabelText("Clear")).toBeInTheDocument();
  });

  it("dispatches tokenFetchByContractId on Search click", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));
    expect(mockCommands.tokenFetchByContractId).toHaveBeenCalledWith({
      contractId: VALID_HEX_64,
    });
  });

  it("dispatches search on Enter key press", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      `${VALID_HEX_64}{Enter}`,
    );
    expect(mockCommands.tokenFetchByContractId).toHaveBeenCalledWith({
      contractId: VALID_HEX_64,
    });
  });

  it("trims whitespace from input before searching", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      `  ${VALID_HEX_64}  {Enter}`,
    );
    expect(mockCommands.tokenFetchByContractId).toHaveBeenCalledWith({
      contractId: VALID_HEX_64,
    });
  });

  it("shows error for invalid hex input", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      "not-hex!{Enter}",
    );
    expect(screen.getByTestId("error-message")).toHaveTextContent(
      /Invalid ID format/,
    );
  });

  it("shows error for hex input that is too short", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      "aabb{Enter}",
    );
    expect(screen.getByTestId("error-message")).toHaveTextContent(
      /Invalid ID format/,
    );
  });

  it("does not dispatch when input is empty and Enter is pressed", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      "{Enter}",
    );
    expect(mockCommands.tokenFetchByContractId).not.toHaveBeenCalled();
  });
});

describe("TokenAddByIdScreen — searching state", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("shows searching state with loader and elapsed time", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    expect(screen.getByText("Searching for token...")).toBeInTheDocument();
    expect(screen.getByTestId("elapsed-time")).toBeInTheDocument();
  });

  it("disables input and Search button during search", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    expect(screen.getByLabelText("Contract or token ID")).toBeDisabled();
    expect(screen.getByLabelText("Search")).toBeDisabled();
  });
});

describe("TokenAddByIdScreen — clear flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("clears input and resets to idle on Clear click", async () => {
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      "test-hex",
    );
    await user.click(screen.getByLabelText("Clear"));

    expect(screen.getByLabelText("Contract or token ID")).toHaveValue("");
    expect(
      screen.getByText(/Enter a contract ID or token ID/),
    ).toBeInTheDocument();
  });

  it("resets error state on Clear click", async () => {
    const user = userEvent.setup();
    renderScreen();
    // Trigger error first
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      "not-hex!{Enter}",
    );
    expect(screen.getByTestId("error-message")).toBeInTheDocument();

    // Click "Try Again" which is the clear button in error state
    await user.click(screen.getByText("Try Again"));
    expect(screen.queryByTestId("error-message")).not.toBeInTheDocument();
  });
});

describe("TokenAddByIdScreen — fallback to tokenFetchByTokenId", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("falls back to tokenFetchByTokenId when contract fetch fails", async () => {
    mockCommands.tokenFetchByContractId.mockResolvedValueOnce({
      status: "error",
      error: "Contract not found",
    });
    const user = userEvent.setup();
    renderScreen();
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    await waitFor(() => {
      expect(mockCommands.tokenFetchByTokenId).toHaveBeenCalledWith({
        tokenId: VALID_HEX_64,
      });
    });
  });
});

describe("TokenAddByIdScreen — results display", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("displays found tokens when event arrives with results", async () => {
    // Set up the event listener to capture the callback
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    // Wait for listener registration
    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    // Trigger search
    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    // Simulate backend result
    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            {
              tokenId: "token123abc",
              contractId: VALID_HEX_64,
              name: "Test Token",
              description: "A test token",
              decimals: 8,
              tokenPosition: 0,
            },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("Found 1 token:")).toBeInTheDocument();
      expect(screen.getByText("Test Token")).toBeInTheDocument();
      expect(screen.getByText("A test token")).toBeInTheDocument();
    });
  });

  it("displays multiple found tokens", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            { tokenId: "tok1", contractId: VALID_HEX_64, name: "Token A" },
            { tokenId: "tok2", contractId: VALID_HEX_64, name: "Token B" },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("Found 2 tokens:")).toBeInTheDocument();
      expect(screen.getByText("Token A")).toBeInTheDocument();
      expect(screen.getByText("Token B")).toBeInTheDocument();
    });
  });

  it("shows error when no tokens found in result", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: { tokens: [] },
      },
    });

    await waitFor(() => {
      expect(screen.getByTestId("error-message")).toHaveTextContent(
        /No tokens found/,
      );
    });
  });
});

describe("TokenAddByIdScreen — Add to My Tokens", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("renders Add button for each found token", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            {
              tokenId: "tok1",
              contractId: VALID_HEX_64,
              name: "My Token",
            },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(
        screen.getByLabelText("Add My Token to My Tokens"),
      ).toBeInTheDocument();
    });
  });

  it("calls tokenSaveLocally when Add button is clicked", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            {
              tokenId: "tok1",
              contractId: VALID_HEX_64,
              name: "My Token",
              description: "test",
              tokenPosition: 0,
            },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("My Token")).toBeInTheDocument();
    });

    await user.click(screen.getByLabelText("Add My Token to My Tokens"));

    await waitFor(() => {
      expect(mockCommands.tokenSaveLocally).toHaveBeenCalledWith({
        tokenInfoJson: expect.objectContaining({
          token_id: "tok1",
          token_name: "My Token",
          data_contract_id: VALID_HEX_64,
        }),
      });
    });
  });

  it("shows info toast when token already in My Tokens", async () => {
    // Pre-add the token to the store
    useTokenStore.setState({
      tokens: [
        {
          tokenId: "tok1",
          contractId: VALID_HEX_64,
          name: "My Token",
          identityId: "id1",
          ownerAlias: null,
          balance: "0",
          decimals: 8,
          tokenPosition: 0,
        },
      ],
    });

    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            { tokenId: "tok1", contractId: VALID_HEX_64, name: "My Token" },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("My Token")).toBeInTheDocument();
    });

    await user.click(screen.getByLabelText("Add My Token to My Tokens"));

    expect(mockToast.info).toHaveBeenCalledWith("Token already in My Tokens");
    expect(mockCommands.tokenSaveLocally).not.toHaveBeenCalled();
  });
});

describe("TokenAddByIdScreen — More Info dialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("opens TokenInfoDialog on More Info click", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            {
              tokenId: "tok1",
              contractId: VALID_HEX_64,
              name: "My Token",
              description: "desc",
              decimals: 8,
            },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("My Token")).toBeInTheDocument();
    });

    await user.click(screen.getByLabelText("More info about My Token"));

    // TokenInfoDialog should open
    await waitFor(() => {
      // The dialog title should contain the token name
      const dialogs = screen.getAllByRole("dialog");
      expect(dialogs.length).toBeGreaterThan(0);
    });
  });
});

describe("TokenAddByIdScreen — error event handling", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("shows error when task error event is received", async () => {
    let errorCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskErrorEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        errorCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(errorCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    errorCallback!({
      payload: { taskId: "fetch-contract-1", message: "Network error" },
    });

    await waitFor(() => {
      expect(screen.getByTestId("error-message")).toHaveTextContent(
        "Network error",
      );
    });
  });
});

describe("TokenAddByIdScreen — event subscription lifecycle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("subscribes to task events on mount", async () => {
    renderScreen();
    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
    });
  });

  it("unsubscribes from events on unmount", async () => {
    const mockUnsub1 = vi.fn();
    const mockUnsub2 = vi.fn();
    mockEvents.taskResultEvent.listen.mockResolvedValue(mockUnsub1);
    mockEvents.taskErrorEvent.listen.mockResolvedValue(mockUnsub2);

    const { unmount } = renderScreen();
    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });
    unmount();
    await waitFor(() => {
      expect(mockUnsub1).toHaveBeenCalled();
      expect(mockUnsub2).toHaveBeenCalled();
    });
  });
});

describe("TokenAddByIdScreen — token result card display", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it("shows 'Unnamed Token' for tokens with no name", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            { tokenId: "tok1", contractId: VALID_HEX_64, name: null },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("Unnamed Token")).toBeInTheDocument();
    });
  });

  it("shows Paused badge for paused tokens", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            {
              tokenId: "tok1",
              contractId: VALID_HEX_64,
              name: "Paused Token",
              paused: true,
            },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("Paused")).toBeInTheDocument();
    });
  });

  it("shows token ID and contract ID in result card", async () => {
    let eventCallback: ((event: { payload: unknown }) => void) | null = null;
    mockEvents.taskResultEvent.listen.mockImplementation(
      (cb: (event: { payload: unknown }) => void) => {
        eventCallback = cb;
        return Promise.resolve(() => {});
      },
    );

    const user = userEvent.setup();
    renderScreen();

    await waitFor(() => {
      expect(eventCallback).not.toBeNull();
    });

    await user.type(
      screen.getByLabelText("Contract or token ID"),
      VALID_HEX_64,
    );
    await user.click(screen.getByLabelText("Search"));

    eventCallback!({
      payload: {
        resultType: "Token",
        taskId: "fetch-contract-1",
        payload: {
          tokens: [
            {
              tokenId: VALID_HEX_64,
              contractId: VALID_HEX_64,
              name: "Display Token",
            },
          ],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByTestId("token-id-display")).toBeInTheDocument();
      expect(screen.getByTestId("contract-id-display")).toBeInTheDocument();
    });
  });
});
