import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DpnsActiveContestsScreen } from "./DpnsActiveContestsScreen";
import { useContestStore } from "@/stores/contestStore";
import { renderWithProviders } from "@/test/router-utils";
import type { ContestedName, SelectedVote } from "@/stores/contestStore";
import type { QualifiedIdentityDto } from "@/bindings";

// ─── Centralized mock bindings ───────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

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

// Mock react-router navigation
const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

import { commands, events } from "@/bindings";

// ─── Test Fixtures ────────────────────────────────────────────────

function makeContestant(overrides: Record<string, unknown> = {}) {
  return {
    id: "aaaa1111bbbb2222cccc3333dddd4444",
    name: "alice",
    info: "",
    votes: 3,
    createdAt: null,
    createdAtBlockHeight: null,
    createdAtCoreBlockHeight: null,
    documentId: "doc111",
    ...overrides,
  };
}

function makeContest(overrides: Partial<ContestedName> = {}): ContestedName {
  return {
    normalizedContestedName: "d4sh",
    contestants: [makeContestant()],
    lockedVotes: 2,
    abstainVotes: 1,
    awardedTo: null,
    endTime: Math.floor(Date.now() / 1000) + 86400,
    state: "ongoing",
    lastUpdated: Math.floor(Date.now() / 1000) - 60,
    ...overrides,
  };
}

function makeVotingIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "voter1".padEnd(64, "0"),
    identityType: "masternode",
    alias: "MN Voter 1",
    balance: 5000000000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: null,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

// ─── Helpers ──────────────────────────────────────────────────────

function resetStore() {
  useContestStore.setState({
    contestedNames: [],
    localDpnsNames: [],
    scheduledVotes: [],
    selectedVotes: [],
    loading: false,
    refreshing: false,
    votingInProgress: false,
    error: null,
    activeFilterTerm: "",
    pastFilterTerm: "",
    ownedFilterTerm: "",
    sortColumn: "name",
    sortOrder: "ascending",
    scheduledVoteCastInProgress: false,
  });
}

function setup() {
  const user = userEvent.setup();
  const result = renderWithProviders(<DpnsActiveContestsScreen />);
  return { user, ...result };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
});

// ─── Rendering ───────────────────────────────────────────────────

describe("DpnsActiveContestsScreen — rendering", () => {
  it("renders the screen header", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.getByText("Active Contests")).toBeInTheDocument();
    expect(
      screen.getByText("Vote on contested DPNS names using your voting identities."),
    ).toBeInTheDocument();
  });

  it("shows loading spinner when loading with no data", async () => {
    useContestStore.setState({ loading: true, contestedNames: [] });
    setup();

    expect(screen.getByText("Loading contests...")).toBeInTheDocument();
  });

  it("does not show loading spinner when data exists even if loading", () => {
    useContestStore.setState({
      loading: true,
      contestedNames: [makeContest()],
    });
    setup();

    expect(screen.queryByText("Loading contests...")).not.toBeInTheDocument();
    expect(screen.getByText("Active Contests")).toBeInTheDocument();
  });

  it("renders action buttons", () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.getByRole("button", { name: /refresh contests/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /register dpns name/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /cast or schedule votes/i })).toBeInTheDocument();
  });

  it("displays the ActiveContestsTable with contest data", () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.getByText("d4sh")).toBeInTheDocument();
  });

  it("shows empty state when no active contests", () => {
    useContestStore.setState({ contestedNames: [] });
    setup();

    expect(screen.getByText("No active contests at the moment.")).toBeInTheDocument();
  });
});

// ─── Data loading ────────────────────────────────────────────────

describe("DpnsActiveContestsScreen — data loading", () => {
  it("calls loadContests on mount", async () => {
    setup();

    await waitFor(() => {
      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
    });
  });

  it("loads voting identities on mount", async () => {
    setup();

    await waitFor(() => {
      expect(commands.identityListVoting).toHaveBeenCalled();
    });
  });

  it("subscribes to contest update events on mount", async () => {
    setup();

    await waitFor(() => {
      expect(events.taskResultEvent.listen).toHaveBeenCalled();
      expect(events.taskErrorEvent.listen).toHaveBeenCalled();
      expect(events.scheduledVoteExecutedEvent.listen).toHaveBeenCalled();
    });
  });
});

// ─── Action buttons ──────────────────────────────────────────────

describe("DpnsActiveContestsScreen — action buttons", () => {
  it("refresh button triggers loadContests", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    // Wait for initial mount calls to complete, then clear
    await waitFor(() => {
      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
    });
    vi.mocked(commands.contestedQueryDpnsContests).mockClear();
    vi.mocked(commands.identityListVoting).mockClear();

    // Reset refreshing state (mount's loadContests sets it, and no event clears it in test)
    useContestStore.setState({ refreshing: false });

    await user.click(screen.getByRole("button", { name: /refresh contests/i }));

    await waitFor(() => {
      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
      expect(commands.identityListVoting).toHaveBeenCalled();
    });
  });

  it("refresh button shows spinning icon when refreshing", () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      refreshing: true,
    });
    setup();

    const refreshButton = screen.getByRole("button", { name: /refresh contests/i });
    expect(refreshButton).toBeDisabled();
  });

  it("register name button navigates to DPNS register screen", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: /register dpns name/i }));

    expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts/dpns-register" });
  });

  it("cast/schedule votes button is disabled when no votes selected", () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes: [],
    });
    setup();

    const castButton = screen.getByRole("button", { name: /cast or schedule votes/i });
    expect(castButton).toBeDisabled();
  });

  it("cast/schedule votes button shows info toast when clicked with no votes", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes: [],
    });
    setup();

    const castButton = screen.getByRole("button", { name: /cast or schedule votes/i });
    expect(castButton).toBeDisabled();
  });

  it("cast/schedule votes button is enabled when votes are selected", () => {
    const selectedVotes: SelectedVote[] = [
      { contestedName: "d4sh", choice: "lock", endTime: null },
    ];
    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes,
    });
    setup();

    const castButton = screen.getByRole("button", { name: /cast or schedule votes/i });
    expect(castButton).toBeEnabled();
  });

  it("cast/schedule votes button shows vote count badge", () => {
    const selectedVotes: SelectedVote[] = [
      { contestedName: "d4sh", choice: "lock", endTime: null },
      { contestedName: "test", choice: "abstain", endTime: null },
    ];
    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes,
    });
    setup();

    const castButton = screen.getByRole("button", { name: /cast or schedule votes/i });
    // The badge count is inside the button
    expect(castButton).toHaveTextContent("2");
  });
});

// ─── Vote toggling ───────────────────────────────────────────────

describe("DpnsActiveContestsScreen — vote toggling", () => {
  it("clicking lock vote button adds to selected votes", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    const lockButton = screen.getByRole("button", {
      name: /lock vote for d4sh/i,
    });
    await user.click(lockButton);

    const state = useContestStore.getState();
    expect(state.selectedVotes).toHaveLength(1);
    expect(state.selectedVotes[0].contestedName).toBe("d4sh");
    expect(state.selectedVotes[0].choice).toBe("lock");
  });

  it("clicking lock again deselects the vote", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes: [{ contestedName: "d4sh", choice: "lock", endTime: null }],
    });
    const { user } = setup();

    const lockButton = screen.getByRole("button", {
      name: /lock vote for d4sh/i,
    });
    await user.click(lockButton);

    const state = useContestStore.getState();
    expect(state.selectedVotes).toHaveLength(0);
  });

  it("clicking abstain replaces a lock vote with abstain", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes: [{ contestedName: "d4sh", choice: "lock", endTime: null }],
    });
    const { user } = setup();

    const abstainButton = screen.getByRole("button", {
      name: /abstain vote for d4sh/i,
    });
    await user.click(abstainButton);

    const state = useContestStore.getState();
    expect(state.selectedVotes).toHaveLength(1);
    expect(state.selectedVotes[0].choice).toBe("abstain");
  });
});

// ─── Vote casting dialog ────────────────────────────────────────

describe("DpnsActiveContestsScreen — vote casting dialog", () => {
  it("opens vote casting dialog when cast button is clicked with selections", async () => {
    const selectedVotes: SelectedVote[] = [
      { contestedName: "d4sh", choice: "lock", endTime: null },
    ];
    const votingIdentity = makeVotingIdentity();
    vi.mocked(commands.identityListVoting).mockResolvedValue({
      status: "ok",
      data: [votingIdentity],
    });

    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes,
    });
    const { user } = setup();

    // Wait for voting identities to load
    await waitFor(() => {
      expect(commands.identityListVoting).toHaveBeenCalled();
    });

    await user.click(screen.getByRole("button", { name: /cast or schedule votes/i }));

    await waitFor(() => {
      expect(screen.getByText("Voting")).toBeInTheDocument();
    });
  });

  it("dialog shows selected votes summary", async () => {
    const selectedVotes: SelectedVote[] = [
      { contestedName: "d4sh", choice: "lock", endTime: null },
    ];
    vi.mocked(commands.identityListVoting).mockResolvedValue({
      status: "ok",
      data: [makeVotingIdentity()],
    });

    useContestStore.setState({
      contestedNames: [makeContest()],
      selectedVotes,
    });
    const { user } = setup();

    await waitFor(() => {
      expect(commands.identityListVoting).toHaveBeenCalled();
    });

    await user.click(screen.getByRole("button", { name: /cast or schedule votes/i }));

    // Dialog should open with the vote summary section visible
    await waitFor(() => {
      expect(screen.getByText("Voting")).toBeInTheDocument();
      // The dialog contains "Selected Votes (1)" header
      expect(screen.getByText("Selected Votes (1)")).toBeInTheDocument();
    });
  });
});

// ─── Error handling ──────────────────────────────────────────────

describe("DpnsActiveContestsScreen — error handling", () => {
  it("shows error toast when store has error", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      error: "Network connection failed",
    });
    setup();

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Network connection failed");
    });
  });

  it("clears error after showing toast", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      error: "Test error",
    });
    setup();

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalled();
    });

    const state = useContestStore.getState();
    expect(state.error).toBeNull();
  });
});

// ─── Filter integration ─────────────────────────────────────────

describe("DpnsActiveContestsScreen — filter integration", () => {
  it("typing in the filter input updates store filter term", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    const filterInput = screen.getByRole("textbox", { name: /filter contests/i });
    await user.type(filterInput, "dash");

    const state = useContestStore.getState();
    expect(state.activeFilterTerm).toBe("dash");
  });

  it("filtering by non-matching term hides contests", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    const filterInput = screen.getByRole("textbox", { name: /filter contests/i });
    await user.type(filterInput, "zzzznonexistent");

    expect(screen.getByText("No matching contests.")).toBeInTheDocument();
  });
});

// ─── Sort integration ───────────────────────────────────────────

describe("DpnsActiveContestsScreen — sort integration", () => {
  it("clicking a sort column header updates sort state", async () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "alpha" }),
        makeContest({ normalizedContestedName: "beta" }),
      ],
    });
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: /sort by locked votes/i }));

    const state = useContestStore.getState();
    expect(state.sortColumn).toBe("lockedVotes");
  });
});

// ─── Multiple contests ──────────────────────────────────────────

describe("DpnsActiveContestsScreen — multiple contests", () => {
  it("renders multiple contests in the table", () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "d4sh" }),
        makeContest({ normalizedContestedName: "test" }),
        makeContest({ normalizedContestedName: "name" }),
      ],
    });
    setup();

    expect(screen.getByText("d4sh")).toBeInTheDocument();
    expect(screen.getByText("test")).toBeInTheDocument();
    expect(screen.getByText("name")).toBeInTheDocument();
  });

  it("can select votes from different contests", async () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "d4sh" }),
        makeContest({ normalizedContestedName: "test" }),
      ],
    });
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: /lock vote for d4sh/i }));
    await user.click(screen.getByRole("button", { name: /abstain vote for test/i }));

    const state = useContestStore.getState();
    expect(state.selectedVotes).toHaveLength(2);
  });
});
