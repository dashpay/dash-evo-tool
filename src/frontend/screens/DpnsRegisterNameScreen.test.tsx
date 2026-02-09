import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DpnsRegisterNameScreen } from "./DpnsRegisterNameScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContestStore } from "@/stores/contestStore";
import { renderWithProviders } from "@/test/router-utils";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Mock navigation ────────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

// ─── Mock Tauri bindings ────────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    identityListLocal: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    identityLoadOrder: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    identityGetById: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    identityRegisterDpnsName: vi.fn().mockResolvedValue({
      status: "ok",
      data: { taskId: "task-1" },
    }),
    identityRefreshDpnsNames: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    identityLocalDpnsNames: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    contestedQueryDpnsContests: vi.fn().mockResolvedValue({
      status: "ok",
      data: null,
    }),
    contestedGetScheduledVotes: vi.fn().mockResolvedValue({
      status: "ok",
      data: [],
    }),
  };
  const mockEvents = {
    taskResultEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    taskErrorEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    walletUpdatedEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
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

// ─── Test Fixtures ──────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    purpose: "AUTHENTICATION",
    securityLevel: "HIGH",
    keyType: "ECDSA_SECP256K1",
    data: "aabb".repeat(16),
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: true,
    ...overrides,
  };
}

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Test Identity",
    balance: 10_000_000_000, // 100 DASH in credits
    keys: [makeKey()],
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

// ─── Helpers ────────────────────────────────────────────────────────

function resetStores() {
  useIdentityStore.setState({
    identities: [],
    selectedIdentityId: null,
    loading: false,
    refreshingIds: new Set(),
    refreshingAll: false,
    error: null,
    sortColumn: "alias",
    sortOrder: "ascending",
    useCustomOrder: true,
  });
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

/**
 * Pre-populate the store with identities AND set up the mock so
 * loadIdentities resolves with them. This avoids the loading spinner.
 */
function setupWithIdentities(identities: QualifiedIdentityDto[]) {
  mockCommands.identityListLocal.mockResolvedValue({
    status: "ok",
    data: identities,
  });
  // Pre-populate so component doesn't show loading spinner
  useIdentityStore.setState({ identities, loading: false });
}

function setup() {
  const user = userEvent.setup();
  const result = renderWithProviders(<DpnsRegisterNameScreen />);
  return { user, ...result };
}

function resetMockDefaults() {
  mockCommands.identityListLocal.mockResolvedValue({ status: "ok", data: [] });
  mockCommands.identityLoadOrder.mockResolvedValue({ status: "ok", data: [] });
  mockCommands.identityGetById.mockResolvedValue({ status: "ok", data: null });
  mockCommands.identityRegisterDpnsName.mockResolvedValue({
    status: "ok",
    data: { taskId: "task-1" },
  });
  mockCommands.identityRefreshDpnsNames.mockResolvedValue({
    status: "ok",
    data: null,
  });
  mockCommands.identityLocalDpnsNames.mockResolvedValue({ status: "ok", data: [] });
  mockCommands.contestedQueryDpnsContests.mockResolvedValue({
    status: "ok",
    data: null,
  });
  mockCommands.contestedGetScheduledVotes.mockResolvedValue({
    status: "ok",
    data: [],
  });
  mockEvents.taskResultEvent.listen.mockResolvedValue(() => {});
  mockEvents.taskErrorEvent.listen.mockResolvedValue(() => {});
  mockEvents.walletUpdatedEvent.listen.mockResolvedValue(() => {});
  mockEvents.scheduledVoteExecutedEvent.listen.mockResolvedValue(() => {});
}

beforeEach(() => {
  vi.clearAllMocks();
  resetMockDefaults();
  resetStores();
});

// ─── Rendering ──────────────────────────────────────────────────────

describe("DpnsRegisterNameScreen — rendering", () => {
  it("renders the Register DPNS Name heading", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByText("Register DPNS Name")).toBeInTheDocument();
    });
  });

  it("shows loading spinner when identities are loading with no data", () => {
    useIdentityStore.setState({ loading: true, identities: [] });
    setup();

    expect(screen.getByText("Loading identities...")).toBeInTheDocument();
  });

  it("does not show loading spinner when identities exist even if loading", () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    useIdentityStore.setState({ loading: true });
    setup();

    expect(screen.queryByText("Loading identities...")).not.toBeInTheDocument();
    expect(screen.getByText("Register DPNS Name")).toBeInTheDocument();
  });

  it("shows DPNS breadcrumb (source=dpns)", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByText("DPNS")).toBeInTheDocument();
    });
  });

  it("shows no identities warning when no identities loaded", async () => {
    // Mock returns empty list so after loadIdentities completes, identities is []
    mockCommands.identityListLocal.mockResolvedValue({ status: "ok", data: [] });
    setup();

    // Wait for loadIdentities to complete and form to render
    await waitFor(() => {
      expect(screen.getByText(/No identities loaded/)).toBeInTheDocument();
    });
  });

  it("shows identity balance display", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByTestId("identity-balance")).toBeInTheDocument();
    });
  });

  it("shows name input field", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });
  });

  it("shows register button", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByTestId("register-btn")).toBeInTheDocument();
    });
  });
});

// ─── Data loading ───────────────────────────────────────────────────

describe("DpnsRegisterNameScreen — data loading", () => {
  it("loads identities on mount", async () => {
    setup();

    await waitFor(() => {
      expect(mockCommands.identityListLocal).toHaveBeenCalled();
    });
  });

  it("subscribes to identity update events on mount", async () => {
    setup();

    await waitFor(() => {
      expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
    });
  });
});

// ─── Name validation ────────────────────────────────────────────────

describe("DpnsRegisterNameScreen — name validation", () => {
  it("shows valid feedback for a valid name", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    expect(screen.getByText("Valid name format")).toBeInTheDocument();
  });

  it("shows contested name warning for short alpha name", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("contested-warning")).toBeInTheDocument();
  });

  it("shows not contested message for name with digit > 1", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "alice99");
    expect(screen.getByTestId("not-contested")).toBeInTheDocument();
  });

  it("shows error for too short name", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "ab");
    expect(screen.getByText(/at least 3 characters/)).toBeInTheDocument();
  });

  it("shows fee estimate for valid name", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "alice");
    expect(screen.getByTestId("fee-estimate")).toBeInTheDocument();
  });
});

// ─── Registration submission ────────────────────────────────────────

describe("DpnsRegisterNameScreen — registration", () => {
  it("calls identityRegisterDpnsName on submit", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(mockCommands.identityRegisterDpnsName).toHaveBeenCalledWith({
        identityId: identity.id,
        name: "testname99",
      });
    });
  });

  it("shows registering state after submit", async () => {
    // Make the command hang (never resolve)
    mockCommands.identityRegisterDpnsName.mockReturnValue(new Promise(() => {}));

    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-registering")).toBeInTheDocument();
    });
    expect(screen.getByText("Registering...")).toBeInTheDocument();
  });

  it("shows success state after successful registration", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });
    expect(screen.getByText("DPNS Name Registered!")).toBeInTheDocument();
  });

  it("shows contested success message when registering a contested name", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    // "alice" is contested: < 20 chars, no digits except 0/1
    await user.type(screen.getByTestId("name-input"), "alice");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });
    expect(
      screen.getByText("DPNS Name Submitted (Contested)"),
    ).toBeInTheDocument();
  });

  it("shows non-contested success message when registering a non-contested name", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    // "testname99" is NOT contested: contains digits 9
    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });
    expect(screen.getByText("DPNS Name Registered!")).toBeInTheDocument();
    expect(
      screen.queryByText("DPNS Name Submitted (Contested)"),
    ).not.toBeInTheDocument();
  });

  it("refreshes DPNS names after successful registration", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });

    // refreshDpnsNames calls identityRefreshDpnsNames
    expect(mockCommands.identityRefreshDpnsNames).toHaveBeenCalled();
  });

  it("reloads identity after successful registration", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });

    // reloadIdentity calls identityGetById with the identity ID string
    expect(mockCommands.identityGetById).toHaveBeenCalledWith(identity.id);
  });

  it("shows error state on failed registration", async () => {
    mockCommands.identityRegisterDpnsName.mockResolvedValue({
      status: "error",
      error: "Platform error: insufficient funds",
    });

    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-error")).toBeInTheDocument();
    });
    expect(screen.getByText("Platform error: insufficient funds")).toBeInTheDocument();
  });

  it("shows error state on exception during registration", async () => {
    mockCommands.identityRegisterDpnsName.mockRejectedValue(
      new Error("Network timeout"),
    );

    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-error")).toBeInTheDocument();
    });
    expect(screen.getByText("Network timeout")).toBeInTheDocument();
  });
});

// ─── Navigation ─────────────────────────────────────────────────────

describe("DpnsRegisterNameScreen — navigation", () => {
  it("navigates back to active contests on back button", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("back-btn")).toBeInTheDocument();
    });

    await user.click(screen.getByTestId("back-btn"));

    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/contracts/dpns-active",
    });
  });

  it("navigates back to active contests from success screen", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });

    await user.click(screen.getByTestId("back-btn"));

    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/contracts/dpns-active",
    });
  });

  it("resets to form state on register another click", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-success")).toBeInTheDocument();
    });

    await user.click(screen.getByTestId("register-another-btn"));

    await waitFor(() => {
      expect(screen.getByText("Register DPNS Name")).toBeInTheDocument();
    });
  });
});

// ─── Error dismissal ────────────────────────────────────────────────

describe("DpnsRegisterNameScreen — error dismissal", () => {
  it("returns to form state when error is dismissed", async () => {
    mockCommands.identityRegisterDpnsName.mockResolvedValue({
      status: "error",
      error: "Some error",
    });

    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    await user.click(screen.getByTestId("register-btn"));

    await waitFor(() => {
      expect(screen.getByTestId("register-dpns-error")).toBeInTheDocument();
    });

    await user.click(screen.getByTestId("dismiss-error-btn"));

    await waitFor(() => {
      expect(screen.getByText("Register DPNS Name")).toBeInTheDocument();
    });
  });
});

// ─── Button state ───────────────────────────────────────────────────

describe("DpnsRegisterNameScreen — button state", () => {
  it("disables register button when name is empty", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByTestId("register-btn")).toBeInTheDocument();
    });

    expect(screen.getByTestId("register-btn")).toBeDisabled();
  });

  it("enables register button when valid name entered", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    expect(screen.getByTestId("register-btn")).toBeEnabled();
  });

  it("disables register button when balance is insufficient", async () => {
    const identity = makeIdentity({ balance: 100 });
    setupWithIdentities([identity]);
    const { user } = setup();

    await waitFor(() => {
      expect(screen.getByTestId("name-input")).toBeInTheDocument();
    });

    await user.type(screen.getByTestId("name-input"), "testname99");
    expect(screen.getByTestId("register-btn")).toBeDisabled();
    expect(screen.getByTestId("insufficient-balance")).toBeInTheDocument();
  });
});

// ─── Multiple identities ───────────────────────────────────────────

describe("DpnsRegisterNameScreen — multiple identities", () => {
  it("shows identity selector when multiple identities", async () => {
    const id1 = makeIdentity({ alias: "Alice" });
    const id2 = makeIdentity({
      id: "bb".repeat(32),
      alias: "Bob",
    });
    setupWithIdentities([id1, id2]);
    setup();

    await waitFor(() => {
      expect(screen.getByLabelText("Identity")).toBeInTheDocument();
    });
  });

  it("shows identity badge when only one identity", async () => {
    const identity = makeIdentity();
    setupWithIdentities([identity]);
    setup();

    await waitFor(() => {
      expect(screen.getByTestId("identity-display")).toBeInTheDocument();
    });
  });
});
