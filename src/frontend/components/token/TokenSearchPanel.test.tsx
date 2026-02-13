import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TokenSearchPanel } from "./TokenSearchPanel";
import type {
  TokenSearchPanelProps,
  SearchResult,
  ContractDetail,
} from "./TokenSearchPanel";
import { renderWithProviders } from "@/test/router-utils";
import { displayId } from "@/lib/utils";

// ─── Mock ThemeProvider ──────────────────────────────────────────────

vi.mock("@/components/theme/ThemeProvider", () => ({
  useTheme: () => ({
    resolvedTheme: "light",
    theme: "light",
    setTheme: vi.fn(),
  }),
}));

// ─── Mock sonner ─────────────────────────────────────────────────────

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

// ─── Fixtures ────────────────────────────────────────────────────────

const mockResults: SearchResult[] = [
  {
    contractId: "aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb",
    description: "A cool token for testing",
  },
  {
    contractId: "ccdd1122334455667788ccdd1122334455667788ccdd1122334455667788ccdd",
    description: "Another great token",
  },
  {
    contractId: "eeff1122334455667788eeff1122334455667788eeff1122334455667788eeff",
    description: "",
  },
];

const mockContractDetail: ContractDetail = {
  contractId: "aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb",
  description: "A cool token for testing",
  tokens: [
    {
      tokenId: "1111222233334444555566667777888899990000aaaabbbbccccddddeeee1111",
      name: "CoolToken",
      description: "The coolest token on Dash",
      configurationJson: { decimals: 8, maxSupply: "1000000" },
    },
    {
      tokenId: "2222333344445555666677778888999900001111aaaabbbbccccddddeeee2222",
      name: "AnotherToken",
      description: null,
    },
  ],
};

// ─── Helpers ─────────────────────────────────────────────────────────

const defaultProps: TokenSearchPanelProps = {
  results: [],
  searching: false,
  hasMore: false,
  currentPage: 0,
  onSearch: vi.fn(),
  onNextPage: vi.fn(),
  onPreviousPage: vi.fn(),
  onClear: vi.fn(),
  onMoreInfo: vi.fn(),
  contractDetail: null,
  contractDetailLoading: false,
  onAddToken: vi.fn(),
  onBackToResults: vi.fn(),
  addingTokenName: null,
};

function renderPanel(overrides: Partial<TokenSearchPanelProps> = {}) {
  const props = { ...defaultProps, ...overrides };
  return renderWithProviders(<TokenSearchPanel {...props} />);
}

// ─── Tests ───────────────────────────────────────────────────────────

describe("TokenSearchPanel — initial state", () => {
  it("renders search input and search button", () => {
    renderPanel();
    expect(
      screen.getByLabelText("Token search keyword"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /search/i }),
    ).toBeInTheDocument();
  });

  it("shows initial empty state when no search has been performed", () => {
    renderPanel({ currentPage: 0 });
    expect(screen.getByText("Search for tokens")).toBeInTheDocument();
    expect(
      screen.getByText(/Enter a keyword above/),
    ).toBeInTheDocument();
  });

  it("disables search button when keyword is empty", () => {
    renderPanel();
    const searchBtn = screen.getByRole("button", { name: /search/i });
    expect(searchBtn).toBeDisabled();
  });
});

describe("TokenSearchPanel — search interaction", () => {
  it("calls onSearch when search button is clicked", async () => {
    const onSearch = vi.fn();
    const user = userEvent.setup();
    renderPanel({ onSearch });

    await user.type(screen.getByLabelText("Token search keyword"), "dash");
    await user.click(screen.getByRole("button", { name: /search/i }));
    expect(onSearch).toHaveBeenCalledWith("dash");
  });

  it("calls onSearch when Enter is pressed in input", async () => {
    const onSearch = vi.fn();
    const user = userEvent.setup();
    renderPanel({ onSearch });

    const input = screen.getByLabelText("Token search keyword");
    await user.type(input, "token{Enter}");
    expect(onSearch).toHaveBeenCalledWith("token");
  });

  it("trims whitespace from keyword before searching", async () => {
    const onSearch = vi.fn();
    const user = userEvent.setup();
    renderPanel({ onSearch });

    await user.type(
      screen.getByLabelText("Token search keyword"),
      "  hello  ",
    );
    await user.click(screen.getByRole("button", { name: /search/i }));
    expect(onSearch).toHaveBeenCalledWith("hello");
  });

  it("does not call onSearch for empty/whitespace keyword", async () => {
    const onSearch = vi.fn();
    const user = userEvent.setup();
    renderPanel({ onSearch });

    await user.type(screen.getByLabelText("Token search keyword"), "   ");
    await user.click(screen.getByRole("button", { name: /search/i }));
    expect(onSearch).not.toHaveBeenCalled();
  });
});

describe("TokenSearchPanel — searching state", () => {
  it("shows searching spinner when searching", () => {
    renderPanel({ searching: true, currentPage: 1 });
    expect(screen.getByText("Searching...")).toBeInTheDocument();
  });

  it("disables search button when searching", () => {
    renderPanel({ searching: true });
    const searchBtn = screen.getByRole("button", { name: /search/i });
    expect(searchBtn).toBeDisabled();
  });
});

describe("TokenSearchPanel — results display", () => {
  it("renders results table with contract IDs and descriptions", () => {
    renderPanel({ results: mockResults, currentPage: 1 });
    expect(screen.getByText("A cool token for testing")).toBeInTheDocument();
    expect(screen.getByText("Another great token")).toBeInTheDocument();
    expect(screen.getByText("No description")).toBeInTheDocument();
  });

  it("renders column headers", () => {
    renderPanel({ results: mockResults, currentPage: 1 });
    expect(screen.getByText("Contract ID")).toBeInTheDocument();
    expect(screen.getByText("Description")).toBeInTheDocument();
    expect(screen.getByText("Action")).toBeInTheDocument();
  });

  it("renders More Info button for each result", () => {
    renderPanel({ results: mockResults, currentPage: 1 });
    const buttons = screen.getAllByText("More Info");
    expect(buttons).toHaveLength(3);
  });

  it("calls onMoreInfo with correct contract ID when More Info is clicked", async () => {
    const onMoreInfo = vi.fn();
    const user = userEvent.setup();
    renderPanel({ results: mockResults, currentPage: 1, onMoreInfo });

    const buttons = screen.getAllByText("More Info");
    await user.click(buttons[0]);
    expect(onMoreInfo).toHaveBeenCalledWith(mockResults[0].contractId);
  });

  it("truncates long contract IDs in display", () => {
    renderPanel({ results: mockResults, currentPage: 1 });
    // The full 64-char hex should be truncated
    expect(
      screen.queryByText(mockResults[0].contractId),
    ).not.toBeInTheDocument();
    // But truncated version should be visible
    expect(screen.getByText(displayId("aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb"))).toBeInTheDocument();
  });
});

describe("TokenSearchPanel — empty search results", () => {
  it("shows no-results message after search completes with empty results", () => {
    renderPanel({ results: [], currentPage: 1 });
    expect(
      screen.getByText("No tokens match your keyword"),
    ).toBeInTheDocument();
  });
});

describe("TokenSearchPanel — pagination", () => {
  it("shows pagination controls when there are more results", () => {
    renderPanel({
      results: mockResults,
      currentPage: 1,
      hasMore: true,
    });
    expect(screen.getByText("Page 1")).toBeInTheDocument();
    expect(screen.getByText("Next")).toBeInTheDocument();
  });

  it("hides pagination when only one page", () => {
    renderPanel({
      results: mockResults,
      currentPage: 1,
      hasMore: false,
    });
    expect(screen.queryByText("Next")).not.toBeInTheDocument();
    expect(screen.queryByText("Previous")).not.toBeInTheDocument();
  });

  it("disables Previous on first page", () => {
    renderPanel({
      results: mockResults,
      currentPage: 1,
      hasMore: true,
    });
    const prevBtn = screen.getByText("Previous").closest("button");
    expect(prevBtn).toBeDisabled();
  });

  it("enables Previous on page 2+", () => {
    renderPanel({
      results: mockResults,
      currentPage: 2,
      hasMore: true,
    });
    const prevBtn = screen.getByText("Previous").closest("button");
    expect(prevBtn).not.toBeDisabled();
  });

  it("calls onNextPage when Next is clicked", async () => {
    const onNextPage = vi.fn();
    const user = userEvent.setup();
    renderPanel({
      results: mockResults,
      currentPage: 1,
      hasMore: true,
      onNextPage,
    });

    await user.click(screen.getByText("Next"));
    expect(onNextPage).toHaveBeenCalled();
  });

  it("calls onPreviousPage when Previous is clicked", async () => {
    const onPreviousPage = vi.fn();
    const user = userEvent.setup();
    renderPanel({
      results: mockResults,
      currentPage: 2,
      hasMore: true,
      onPreviousPage,
    });

    await user.click(screen.getByText("Previous"));
    expect(onPreviousPage).toHaveBeenCalled();
  });

  it("disables Next when no more results", () => {
    renderPanel({
      results: mockResults,
      currentPage: 2,
      hasMore: false,
    });
    const nextBtn = screen.getByText("Next").closest("button");
    expect(nextBtn).toBeDisabled();
  });

  it("displays correct page number", () => {
    renderPanel({
      results: mockResults,
      currentPage: 3,
      hasMore: true,
    });
    expect(screen.getByText("Page 3")).toBeInTheDocument();
  });
});

describe("TokenSearchPanel — clear", () => {
  it("shows Clear button when there are results", () => {
    renderPanel({ results: mockResults, currentPage: 1 });
    expect(screen.getByText("Clear")).toBeInTheDocument();
  });

  it("calls onClear when Clear is clicked", async () => {
    const onClear = vi.fn();
    const user = userEvent.setup();
    renderPanel({ results: mockResults, currentPage: 1, onClear });

    await user.click(screen.getByText("Clear"));
    expect(onClear).toHaveBeenCalled();
  });
});

describe("TokenSearchPanel — contract detail view", () => {
  it("shows loading state when contract detail is loading", () => {
    renderPanel({ contractDetailLoading: true });
    expect(
      screen.getByText("Loading contract details..."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Back to Search Results"),
    ).toBeInTheDocument();
  });

  it("renders contract detail with tokens", () => {
    renderPanel({ contractDetail: mockContractDetail });
    expect(screen.getByText("Contract Details")).toBeInTheDocument();
    expect(screen.getByText("CoolToken")).toBeInTheDocument();
    expect(screen.getByText("AnotherToken")).toBeInTheDocument();
    expect(
      screen.getByText("The coolest token on Dash"),
    ).toBeInTheDocument();
  });

  it("shows Add to My Tokens button for each token", () => {
    renderPanel({ contractDetail: mockContractDetail });
    const addButtons = screen.getAllByText("Add to My Tokens");
    expect(addButtons).toHaveLength(2);
  });

  it("calls onAddToken when Add to My Tokens is clicked", async () => {
    const onAddToken = vi.fn();
    const user = userEvent.setup();
    renderPanel({ contractDetail: mockContractDetail, onAddToken });

    const addButtons = screen.getAllByText("Add to My Tokens");
    await user.click(addButtons[0]);
    expect(onAddToken).toHaveBeenCalledWith(mockContractDetail.tokens[0]);
  });

  it("shows View Schema button for tokens with configuration", () => {
    renderPanel({ contractDetail: mockContractDetail });
    const schemaButtons = screen.getAllByText("View Schema");
    // Only CoolToken has configurationJson
    expect(schemaButtons).toHaveLength(1);
  });

  it("shows JSON viewer when View Schema is clicked", async () => {
    const user = userEvent.setup();
    renderPanel({ contractDetail: mockContractDetail });

    await user.click(screen.getByText("View Schema"));
    expect(
      screen.getByText("CoolToken — Configuration Schema"),
    ).toBeInTheDocument();
  });

  it("calls onBackToResults when Back is clicked", async () => {
    const onBackToResults = vi.fn();
    const user = userEvent.setup();
    renderPanel({ contractDetail: mockContractDetail, onBackToResults });

    await user.click(screen.getByText("Back to Search Results"));
    expect(onBackToResults).toHaveBeenCalled();
  });

  it("hides search bar when contract detail is shown", () => {
    renderPanel({ contractDetail: mockContractDetail });
    expect(
      screen.queryByLabelText("Token search keyword"),
    ).not.toBeInTheDocument();
  });

  it("shows No description for tokens without description", () => {
    renderPanel({ contractDetail: mockContractDetail });
    // AnotherToken has null description
    expect(screen.getByText("No description")).toBeInTheDocument();
  });

  it("shows adding state with spinner when adding a token", () => {
    renderPanel({
      contractDetail: mockContractDetail,
      addingTokenName: "CoolToken",
    });
    // All add buttons should be disabled
    const addButtons = screen.getAllByText("Add to My Tokens");
    addButtons.forEach((btn) => {
      expect(btn.closest("button")).toBeDisabled();
    });
  });

  it("shows contract description in detail view", () => {
    renderPanel({ contractDetail: mockContractDetail });
    expect(
      screen.getByText("A cool token for testing"),
    ).toBeInTheDocument();
  });

  it("renders empty tokens message when contract has no tokens", () => {
    const emptyDetail: ContractDetail = {
      contractId: "aabb1122334455667788aabb1122334455667788aabb1122334455667788aabb",
      description: "Empty contract",
      tokens: [],
    };
    renderPanel({ contractDetail: emptyDetail });
    expect(
      screen.getByText("No tokens found in this contract."),
    ).toBeInTheDocument();
  });
});
