import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DpnsScheduledVotesScreen } from "./DpnsScheduledVotesScreen";
import { useContestStore } from "@/stores/contestStore";
import { renderWithProviders } from "@/test/router-utils";
import type { ScheduledVoteWithStatus } from "@/stores/contestStore";
import type { VoteChoiceDto } from "@/bindings";

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

function makeVote(
  overrides: Partial<{
    contestedName: string;
    voterId: string;
    choice: VoteChoiceDto;
    unixTimestamp: number;
    executedSuccessfully: boolean;
    castingStatus: "notStarted" | "inProgress" | "completed" | "failed";
  }> = {},
): ScheduledVoteWithStatus {
  const castingStatus = overrides.castingStatus ?? "notStarted";
  return {
    vote: {
      contestedName: overrides.contestedName ?? "test-name",
      voterId: overrides.voterId ?? "abcdef1234567890abcdef1234567890",
      choice: overrides.choice ?? "lock",
      unixTimestamp: overrides.unixTimestamp ?? Date.now() + 3_600_000,
      executedSuccessfully:
        overrides.executedSuccessfully ?? castingStatus === "completed",
    },
    castingStatus,
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

function setupWithVotes(votes: ScheduledVoteWithStatus[] = [makeVote()]) {
  vi.mocked(commands.contestedGetScheduledVotes).mockResolvedValue({
    status: "ok",
    data: votes.map((sv) => sv.vote),
  });
  useContestStore.setState({ scheduledVotes: votes });
}

function setup() {
  const user = userEvent.setup();
  const result = renderWithProviders(<DpnsScheduledVotesScreen />);
  return { user, ...result };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
});

// ─── Rendering ───────────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — rendering", () => {
  it("renders the screen header", async () => {
    setupWithVotes();
    setup();

    await waitFor(() => {
      expect(screen.getByText("Scheduled Votes")).toBeInTheDocument();
    });
    expect(
      screen.getByText(/Votes scheduled for future execution/),
    ).toBeInTheDocument();
  });

  it("shows loading spinner when loading with no data", () => {
    useContestStore.setState({ loading: true, scheduledVotes: [] });
    setup();

    expect(
      screen.getByText("Loading scheduled votes..."),
    ).toBeInTheDocument();
  });

  it("does not show loading spinner when data exists even if loading", () => {
    setupWithVotes();
    useContestStore.setState({ loading: true });
    setup();

    expect(
      screen.queryByText("Loading scheduled votes..."),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Scheduled Votes")).toBeInTheDocument();
  });

  it("renders all action buttons", async () => {
    setupWithVotes();
    setup();

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /refresh scheduled votes/i }),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByRole("button", { name: /clear all scheduled votes/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /clear casted votes/i }),
    ).toBeInTheDocument();
  });

  it("displays scheduled votes in the table", async () => {
    setupWithVotes();
    setup();

    await waitFor(() => {
      expect(screen.getByText("test-name")).toBeInTheDocument();
    });
  });

  it("shows empty state when no scheduled votes", async () => {
    setup();

    await waitFor(() => {
      expect(screen.getByText("No scheduled votes.")).toBeInTheDocument();
    });
  });
});

// ─── Data loading ────────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — data loading", () => {
  it("calls loadScheduledVotes on mount", async () => {
    setup();

    await waitFor(() => {
      expect(
        commands.contestedGetScheduledVotes,
      ).toHaveBeenCalled();
    });
  });

  it("subscribes to update events on mount", async () => {
    setup();

    await waitFor(() => {
      expect(events.taskResultEvent.listen).toHaveBeenCalled();
      expect(events.taskErrorEvent.listen).toHaveBeenCalled();
      expect(
        events.scheduledVoteExecutedEvent.listen,
      ).toHaveBeenCalled();
    });
  });
});

// ─── Action buttons ──────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — action buttons", () => {
  it("refresh button triggers loadScheduledVotes", async () => {
    setupWithVotes();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("test-name")).toBeInTheDocument();
    });

    // Clear mock to isolate the click action
    vi.mocked(commands.contestedGetScheduledVotes).mockClear();

    await user.click(
      screen.getByRole("button", { name: /refresh scheduled votes/i }),
    );

    await waitFor(() => {
      expect(
        commands.contestedGetScheduledVotes,
      ).toHaveBeenCalled();
    });
  });

  it("Clear All triggers clearAllScheduledVotes", async () => {
    setupWithVotes();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("test-name")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /clear all scheduled votes/i }),
    );

    await waitFor(() => {
      expect(
        commands.contestedClearAllScheduledVotes,
      ).toHaveBeenCalled();
    });
  });

  it("Clear All button is disabled when no votes", () => {
    setup();

    const clearAllBtn = screen.getByRole("button", {
      name: /clear all scheduled votes/i,
    });
    expect(clearAllBtn).toBeDisabled();
  });

  it("Clear Casted triggers clearExecutedScheduledVotes", async () => {
    setupWithVotes([makeVote({ castingStatus: "completed" })]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("test-name")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /clear casted votes/i }),
    );

    await waitFor(() => {
      expect(
        commands.contestedClearExecutedScheduledVotes,
      ).toHaveBeenCalled();
    });
  });

  it("Clear Casted button is disabled when no completed votes", () => {
    setupWithVotes([makeVote({ castingStatus: "notStarted" })]);
    setup();

    const clearCastedBtn = screen.getByRole("button", {
      name: /clear casted votes/i,
    });
    expect(clearCastedBtn).toBeDisabled();
  });

  it("Clear Casted button is enabled when completed votes exist", () => {
    setupWithVotes([makeVote({ castingStatus: "completed" })]);
    setup();

    const clearCastedBtn = screen.getByRole("button", {
      name: /clear casted votes/i,
    });
    expect(clearCastedBtn).not.toBeDisabled();
  });
});

// ─── Cast Now ───────────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — Cast Now", () => {
  it("calls castScheduledVote when Cast Now is clicked", async () => {
    const vote = makeVote({ castingStatus: "notStarted" });
    setupWithVotes([vote]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("test-name")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /cast now test-name/i }),
    );

    await waitFor(() => {
      expect(
        commands.contestedCastScheduledVote,
      ).toHaveBeenCalledWith({ vote: vote.vote });
    });
  });

  it("Cast Now is disabled during a cast operation", () => {
    setupWithVotes([makeVote({ castingStatus: "notStarted" })]);
    useContestStore.setState({ scheduledVoteCastInProgress: true });
    setup();

    expect(
      screen.getByRole("button", { name: /cast now test-name/i }),
    ).toBeDisabled();
  });
});

// ─── Remove ─────────────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — Remove", () => {
  it("calls deleteScheduledVote when Remove is clicked", async () => {
    setupWithVotes();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("test-name")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /remove test-name/i }),
    );

    await waitFor(() => {
      expect(
        commands.contestedDeleteScheduledVote,
      ).toHaveBeenCalledWith({
        voterId: "abcdef1234567890abcdef1234567890",
        contestedName: "test-name",
      });
    });
  });
});

// ─── Error handling ──────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — error handling", () => {
  it("shows error toast when store has error", async () => {
    setupWithVotes();
    useContestStore.setState({ error: "Network connection failed" });
    setup();

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith(
        "Network connection failed",
      );
    });
  });

  it("clears error after showing toast", async () => {
    setupWithVotes();
    useContestStore.setState({ error: "Test error" });
    setup();

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalled();
    });

    const state = useContestStore.getState();
    expect(state.error).toBeNull();
  });
});

// ─── Multiple votes ─────────────────────────────────────────────

describe("DpnsScheduledVotesScreen — multiple votes", () => {
  it("renders multiple scheduled votes in the table", async () => {
    const votes = [
      makeVote({ contestedName: "alice" }),
      makeVote({ contestedName: "bob", voterId: "111122223333444455556666" }),
      makeVote({
        contestedName: "charlie",
        voterId: "aaaabbbbccccddddeeee",
        castingStatus: "completed",
      }),
    ];
    setupWithVotes(votes);
    setup();

    await waitFor(() => {
      expect(screen.getByText("alice")).toBeInTheDocument();
    });
    expect(screen.getByText("bob")).toBeInTheDocument();
    expect(screen.getByText("charlie")).toBeInTheDocument();
  });

  it("shows mixed statuses correctly", async () => {
    // Only notStarted and completed survive a store reload from DB
    const votes = [
      makeVote({ contestedName: "pending-vote", castingStatus: "notStarted" }),
      makeVote({
        contestedName: "done-vote",
        voterId: "111122223333",
        castingStatus: "completed",
      }),
    ];
    setupWithVotes(votes);
    setup();

    await waitFor(() => {
      expect(screen.getByText("Pending")).toBeInTheDocument();
    });
    expect(screen.getByText("Casted")).toBeInTheDocument();
  });
});
