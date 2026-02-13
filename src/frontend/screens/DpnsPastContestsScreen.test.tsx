import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DpnsPastContestsScreen } from "./DpnsPastContestsScreen";
import { useContestStore } from "@/stores/contestStore";
import { renderWithProviders } from "@/test/router-utils";
import type { ContestedName } from "@/stores/contestStore";
import { displayId } from "@/lib/utils";

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

import { commands, events } from "@/bindings";

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
      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
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

describe("DpnsPastContestsScreen — action buttons", () => {
  it("refresh button triggers loadContests", async () => {
    useContestStore.setState({ contestedNames: [makeContest()] });
    const { user } = setup();

    await waitFor(() => {
      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
    });
    vi.mocked(commands.contestedQueryDpnsContests).mockClear();

    useContestStore.setState({ refreshing: false });

    await user.click(screen.getByRole("button", { name: /refresh contests/i }));

    await waitFor(() => {
      expect(commands.contestedQueryDpnsContests).toHaveBeenCalled();
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
    const winnerId = "winner9876543210abcdef";
    useContestStore.setState({
      contestedNames: [
        makeContest({ normalizedContestedName: "locked-one", state: "locked" }),
        makeContest({
          normalizedContestedName: "won-one",
          state: { wonBy: winnerId },
        }),
      ],
    });
    setup();

    expect(screen.getByText("Locked")).toBeInTheDocument();
    expect(screen.getByText(displayId(winnerId))).toBeInTheDocument();
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
