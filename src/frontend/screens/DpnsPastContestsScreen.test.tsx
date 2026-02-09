import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DpnsPastContestsScreen } from "./DpnsPastContestsScreen";
import { useContestStore } from "@/stores/contestStore";
import { renderWithProviders } from "@/test/router-utils";
import type { ContestedName } from "@/stores/contestStore";

// ─── Mock Tauri bindings ──────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    contestedQueryDpnsContests: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    contestedGetScheduledVotes: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    identityLocalDpnsNames: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
  };
  const mockEvents = {
    taskResultEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    taskErrorEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    scheduledVoteExecutedEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
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

// ─── Test Fixtures ────────────────────────────────────────────────

function makeContest(overrides: Partial<ContestedName> = {}): ContestedName {
  return {
    normalizedContestedName: "d4sh",
    contestants: null,
    lockedVotes: 5,
    abstainVotes: 2,
    awardedTo: "winner1111222233334444",
    endTime: Math.floor(Date.now() / 1000) - 86400, // 1 day ago
    state: { wonBy: "winner1111222233334444" },
    lastUpdated: Math.floor(Date.now() / 1000) - 60,
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
  const result = renderWithProviders(<DpnsPastContestsScreen />);
  return { user, ...result };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
});

// ─── Rendering ───────────────────────────────────────────────────

describe("DpnsPastContestsScreen — rendering", () => {
  it("renders the screen header", () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.getByText("Past Contests")).toBeInTheDocument();
    expect(
      screen.getByText("Historical DPNS name contests that have ended."),
    ).toBeInTheDocument();
  });

  it("shows loading spinner when loading with no data", () => {
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
    expect(screen.getByText("Past Contests")).toBeInTheDocument();
  });

  it("renders refresh button", () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.getByRole("button", { name: /refresh contests/i })).toBeInTheDocument();
  });

  it("does not render vote-related buttons", () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.queryByRole("button", { name: /cast or schedule/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /register/i })).not.toBeInTheDocument();
  });

  it("displays the PastContestsTable with contest data", () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    setup();

    expect(screen.getByText("d4sh")).toBeInTheDocument();
  });

  it("shows empty state when no past contests", () => {
    useContestStore.setState({ contestedNames: [] });
    setup();

    expect(screen.getByText("No past contests yet.")).toBeInTheDocument();
  });

  it("shows empty state when only active contests exist", () => {
    useContestStore.setState({
      contestedNames: [
        {
          ...makeContest(),
          state: "ongoing",
          normalizedContestedName: "active-one",
        },
      ],
    });
    setup();

    expect(screen.getByText("No past contests yet.")).toBeInTheDocument();
    expect(screen.queryByText("active-one")).not.toBeInTheDocument();
  });
});

// ─── Data loading ────────────────────────────────────────────────

describe("DpnsPastContestsScreen — data loading", () => {
  it("calls loadContests on mount", async () => {
    setup();

    await waitFor(() => {
      expect(mockCommands.contestedQueryDpnsContests).toHaveBeenCalled();
    });
  });

  it("subscribes to contest update events on mount", async () => {
    setup();

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
      expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
      expect(mockEvents.scheduledVoteExecutedEvent.listen).toHaveBeenCalled();
    });
  });
});

// ─── Action buttons ──────────────────────────────────────────────

describe("DpnsPastContestsScreen — action buttons", () => {
  it("refresh button triggers loadContests", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    await waitFor(() => {
      expect(mockCommands.contestedQueryDpnsContests).toHaveBeenCalled();
    });
    mockCommands.contestedQueryDpnsContests.mockClear();

    useContestStore.setState({ refreshing: false });

    await user.click(screen.getByRole("button", { name: /refresh contests/i }));

    await waitFor(() => {
      expect(mockCommands.contestedQueryDpnsContests).toHaveBeenCalled();
    });
  });

  it("refresh button is disabled when refreshing", () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      refreshing: true,
    });
    setup();

    const refreshButton = screen.getByRole("button", { name: /refresh contests/i });
    expect(refreshButton).toBeDisabled();
  });
});

// ─── Error handling ──────────────────────────────────────────────

describe("DpnsPastContestsScreen — error handling", () => {
  it("shows error toast when store has error", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      error: "Network connection failed",
    });
    setup();

    await waitFor(() => {
      expect(mockToast.error).toHaveBeenCalledWith("Network connection failed");
    });
  });

  it("clears error after showing toast", async () => {
    useContestStore.setState({
      contestedNames: [makeContest()],
      error: "Test error",
    });
    setup();

    await waitFor(() => {
      expect(mockToast.error).toHaveBeenCalled();
    });

    const state = useContestStore.getState();
    expect(state.error).toBeNull();
  });
});

// ─── Filter integration ─────────────────────────────────────────

describe("DpnsPastContestsScreen — filter integration", () => {
  it("typing in the filter input updates store past filter term", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    const filterInput = screen.getByRole("textbox", { name: /filter contests/i });
    await user.type(filterInput, "dash");

    const state = useContestStore.getState();
    expect(state.pastFilterTerm).toBe("dash");
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

describe("DpnsPastContestsScreen — sort integration", () => {
  it("clicking a sort column header updates sort state", async () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "alpha" }),
        makeContest({ normalizedContestedName: "beta", state: "locked" }),
      ],
    });
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: /sort by ended time/i }));

    const state = useContestStore.getState();
    expect(state.sortColumn).toBe("endingTime");
  });

  it("clicking awarded to sort column updates sort state", async () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "alpha" }),
        makeContest({ normalizedContestedName: "beta", state: "locked" }),
      ],
    });
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: /sort by awarded to/i }));

    const state = useContestStore.getState();
    expect(state.sortColumn).toBe("awardedTo");
  });
});

// ─── Multiple past contests ─────────────────────────────────────

describe("DpnsPastContestsScreen — multiple contests", () => {
  it("renders multiple past contests in the table", () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "d4sh" }),
        makeContest({ normalizedContestedName: "test", state: "locked" }),
        makeContest({
          normalizedContestedName: "name",
          state: { wonBy: "otherwinner1234" },
        }),
      ],
    });
    setup();

    expect(screen.getByText("d4sh")).toBeInTheDocument();
    expect(screen.getByText("test")).toBeInTheDocument();
    expect(screen.getByText("name")).toBeInTheDocument();
  });

  it("shows both locked and wonBy contests together", () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "locked-one", state: "locked" }),
        makeContest({
          normalizedContestedName: "won-one",
          state: { wonBy: "winner9876543210abcdef" },
        }),
      ],
    });
    setup();

    expect(screen.getByText("Locked")).toBeInTheDocument();
    expect(screen.getByText("winner98...")).toBeInTheDocument();
  });

  it("filters out active contests while showing past", () => {
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "past-one", state: { wonBy: "id1" } }),
        makeContest({ normalizedContestedName: "active-one", state: "ongoing" }),
        makeContest({ normalizedContestedName: "past-two", state: "locked" }),
      ],
    });
    setup();

    expect(screen.getByText("past-one")).toBeInTheDocument();
    expect(screen.getByText("past-two")).toBeInTheDocument();
    expect(screen.queryByText("active-one")).not.toBeInTheDocument();
  });
});
