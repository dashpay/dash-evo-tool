import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DpnsOwnedNamesScreen } from "./DpnsOwnedNamesScreen";
import { useContestStore } from "@/stores/contestStore";
import { useIdentityStore } from "@/stores/identityStore";
import { renderWithProviders } from "@/test/router-utils";
import { createMockIdentity } from "@/test/fixtures";
import type { DpnsNameEntryDto } from "@/bindings";

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

function makeName(
  overrides: Partial<DpnsNameEntryDto> = {},
): DpnsNameEntryDto {
  return {
    identityId: "abcdef1234567890abcdef1234567890",
    name: "alice.dash",
    acquiredAt: Math.floor(Date.now() / 1000) - 86400,
    ...overrides,
  };
}

const defaultNames = [makeName()];

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

/** Set up mock to return names and pre-populate the store. */
function setupWithNames(names: DpnsNameEntryDto[] = defaultNames) {
  vi.mocked(commands.identityLocalDpnsNames).mockResolvedValue({
    status: "ok",
    data: names,
  });
  useContestStore.setState({ localDpnsNames: names });
}

function setup() {
  const user = userEvent.setup();
  const result = renderWithProviders(<DpnsOwnedNamesScreen />);
  return { user, ...result };
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
});

// ─── Rendering ───────────────────────────────────────────────────

describe("DpnsOwnedNamesScreen — rendering", () => {
  it("renders the screen header", async () => {
    setupWithNames();
    setup();

    await waitFor(() => {
      expect(screen.getByText("My Usernames")).toBeInTheDocument();
    });
    expect(
      screen.getByText("DPNS names owned by your loaded identities."),
    ).toBeInTheDocument();
  });

  it("shows loading spinner when loading with no data", () => {
    useContestStore.setState({ loading: true, localDpnsNames: [] });
    setup();

    expect(screen.getByText("Loading owned names...")).toBeInTheDocument();
  });

  it("does not show loading spinner when data exists even if loading", () => {
    setupWithNames();
    useContestStore.setState({ loading: true });
    setup();

    expect(screen.queryByText("Loading owned names...")).not.toBeInTheDocument();
    expect(screen.getByText("My Usernames")).toBeInTheDocument();
  });

  it("renders refresh button", async () => {
    setupWithNames();
    setup();

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /refresh owned names/i }),
      ).toBeInTheDocument();
    });
  });

  it("displays the OwnedNamesPanel with name data", async () => {
    setupWithNames();
    setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });
  });

  it("shows empty state when no owned names", async () => {
    setup();

    await waitFor(() => {
      expect(screen.getByText("No owned usernames.")).toBeInTheDocument();
    });
  });
});

// ─── Data loading ────────────────────────────────────────────────

describe("DpnsOwnedNamesScreen — data loading", () => {
  it("calls loadLocalNames on mount", async () => {
    setup();

    await waitFor(() => {
      expect(commands.identityLocalDpnsNames).toHaveBeenCalled();
    });
  });

  it("subscribes to update events on mount", async () => {
    setup();

    await waitFor(() => {
      expect(events.taskResultEvent.listen).toHaveBeenCalled();
      expect(events.taskErrorEvent.listen).toHaveBeenCalled();
      expect(events.scheduledVoteExecutedEvent.listen).toHaveBeenCalled();
    });
  });
});

// ─── Action buttons ──────────────────────────────────────────────

describe("DpnsOwnedNamesScreen — action buttons", () => {
  it("refresh button triggers refreshDpnsNames", async () => {
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /refresh owned names/i }),
    );

    await waitFor(() => {
      expect(commands.identityRefreshDpnsNames).toHaveBeenCalled();
    });
  });

  it("refresh button is disabled when refreshing", () => {
    setupWithNames();
    useContestStore.setState({ refreshing: true });
    setup();

    const refreshButton = screen.getByRole("button", {
      name: /refresh owned names/i,
    });
    expect(refreshButton).toBeDisabled();
  });
});

// ─── Set Alias ───────────────────────────────────────────────────

describe("DpnsOwnedNamesScreen — set alias", () => {
  it("calls identitySetAlias when Set Alias is clicked", async () => {
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for alice.dash/i }),
    );

    await waitFor(() => {
      expect(commands.identitySetAlias).toHaveBeenCalledWith({
        identityId: "abcdef1234567890abcdef1234567890",
        alias: "alice.dash",
      });
    });
  });

  it("appends .dash suffix if missing", async () => {
    const names = [makeName({ name: "alice" })];
    setupWithNames(names);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for alice/i }),
    );

    await waitFor(() => {
      expect(commands.identitySetAlias).toHaveBeenCalledWith({
        identityId: "abcdef1234567890abcdef1234567890",
        alias: "alice.dash",
      });
    });
  });

  it("shows success toast on successful alias set", async () => {
    vi.mocked(commands.identitySetAlias).mockResolvedValue({ status: "ok", data: null });
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for alice.dash/i }),
    );

    await waitFor(() => {
      expect(mockToast.success).toHaveBeenCalledWith(
        'Alias set to "alice.dash"',
      );
    });
  });

  it("updates identity store state on successful alias set", async () => {
    vi.mocked(commands.identitySetAlias).mockResolvedValue({ status: "ok", data: null });
    // Pre-populate the identity store with an identity matching the owned name
    useIdentityStore.setState({
      identities: [
        createMockIdentity({
          id: "abcdef1234567890abcdef1234567890",
          alias: null,
        }),
      ],
    });
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for alice.dash/i }),
    );

    await waitFor(() => {
      const identity = useIdentityStore
        .getState()
        .identities.find(
          (i) => i.id === "abcdef1234567890abcdef1234567890",
        );
      expect(identity?.alias).toBe("alice.dash");
    });
  });

  it("shows error toast on failed alias set", async () => {
    vi.mocked(commands.identitySetAlias).mockResolvedValue({
      status: "error",
      error: "Database error",
    });
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    await user.click(
      screen.getByRole("button", { name: /set alias for alice.dash/i }),
    );

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Database error");
    });
  });
});

// ─── Error handling ──────────────────────────────────────────────

describe("DpnsOwnedNamesScreen — error handling", () => {
  it("shows error toast when store has error", async () => {
    setupWithNames();
    useContestStore.setState({ error: "Network connection failed" });
    setup();

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalledWith("Network connection failed");
    });
  });

  it("clears error after showing toast", async () => {
    setupWithNames();
    useContestStore.setState({ error: "Test error" });
    setup();

    await waitFor(() => {
      expect(mockToastError).toHaveBeenCalled();
    });

    const state = useContestStore.getState();
    expect(state.error).toBeNull();
  });
});

// ─── Filter integration ─────────────────────────────────────────

describe("DpnsOwnedNamesScreen — filter integration", () => {
  it("typing in the filter input updates store owned filter term", async () => {
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    const filterInput = screen.getByRole("textbox", {
      name: /filter owned names/i,
    });
    await user.type(filterInput, "dash");

    const state = useContestStore.getState();
    expect(state.ownedFilterTerm).toBe("dash");
  });

  it("filtering by non-matching term hides names", async () => {
    setupWithNames();
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    const filterInput = screen.getByRole("textbox", {
      name: /filter owned names/i,
    });
    await user.type(filterInput, "zzzznonexistent");

    expect(screen.getByText("No matching names.")).toBeInTheDocument();
  });
});

// ─── Multiple owned names ────────────────────────────────────────

describe("DpnsOwnedNamesScreen — multiple names", () => {
  it("renders multiple owned names in the table", async () => {
    const names = [
      makeName({ name: "alice.dash" }),
      makeName({ name: "bob.dash", identityId: "111122223333444455556666" }),
      makeName({ name: "charlie.dash", identityId: "aaaabbbbccccddddeeee" }),
    ];
    setupWithNames(names);
    setup();

    await waitFor(() => {
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });
    expect(screen.getByText("bob.dash")).toBeInTheDocument();
    expect(screen.getByText("charlie.dash")).toBeInTheDocument();
  });

  it("each name has its own Set Alias button", async () => {
    const names = [
      makeName({ name: "alice.dash" }),
      makeName({ name: "bob.dash" }),
    ];
    setupWithNames(names);
    setup();

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /set alias for alice.dash/i }),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByRole("button", { name: /set alias for bob.dash/i }),
    ).toBeInTheDocument();
  });
});
