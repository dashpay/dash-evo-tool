import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RegisterContractScreen } from "./RegisterContractScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useContractStore } from "@/stores/contractStore";
import { useWalletStore } from "@/stores/walletStore";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Hoisted mocks ─────────────────────────────────────────────────

type EventCallback = (event: { payload: unknown }) => void;

const { mockCommands, mockEvents, eventListeners, mockNavigate } = vi.hoisted(
  () => {
    const eventListeners: Record<string, EventCallback[]> = {
      taskResultEvent: [],
      taskErrorEvent: [],
    };

    const mockEvents = {
      taskResultEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskResultEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskResultEvent =
              eventListeners.taskResultEvent.filter((l) => l !== cb);
          });
        }),
      },
      taskErrorEvent: {
        listen: vi.fn().mockImplementation((cb: EventCallback) => {
          eventListeners.taskErrorEvent.push(cb);
          return Promise.resolve(() => {
            eventListeners.taskErrorEvent =
              eventListeners.taskErrorEvent.filter((l) => l !== cb);
          });
        }),
      },
      walletUpdatedEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    };

    const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
      identityListLocal: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
      identityLoadOrder: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
      identityGetById: vi.fn().mockResolvedValue({ status: "ok", data: null }),
      contractRegister: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: { taskId: "task-1" } }),
      contractListLocal: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: [] }),
      walletListAll: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: { hdWallets: [], singleKeyWallets: [] } }),
      walletNotifyUnlocked: vi
        .fn()
        .mockResolvedValue({ status: "ok", data: null }),
    };

    const mockNavigate = vi.fn();

    return { mockCommands, mockEvents, eventListeners, mockNavigate };
  },
);

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("@/bindings", () => ({
  commands: new Proxy({} as Record<string, unknown>, {
    get: (_target, prop: string) => {
      if (prop in mockCommands) return mockCommands[prop];
      return vi.fn().mockResolvedValue({ status: "error", error: "not mocked" });
    },
  }),
  events: mockEvents,
}));

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

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

// ─── Test Fixtures ─────────────────────────────────────────────────

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
    balance: 10_000_000_000,
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

const VALID_RAW_SCHEMAS = JSON.stringify({
  note: {
    type: "object",
    properties: { message: { type: "string" } },
    additionalProperties: false,
  },
});

const VALID_FULL_CONTRACT = JSON.stringify({
  $format_version: "0",
  id: "bb".repeat(32),
  ownerId: "aa".repeat(32),
  version: 1,
  documentSchemas: {
    note: {
      type: "object",
      properties: { message: { type: "string" } },
    },
  },
  config: {
    $format_version: "0",
    canBeDeleted: false,
    readonly: false,
    keepsHistory: false,
    documentsKeepHistoryContractDefault: false,
    documentsMutableContractDefault: true,
    documentsCanBeDeletedContractDefault: false,
    requiresIdentityEncryptionBoundedKey: null,
    requiresIdentityDecryptionBoundedKey: null,
  },
});

function makeWalletDto(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: "wallet-hash-1",
    usesPassword: true,
    alias: "My Wallet",
    isMain: false,
    confirmedBalance: 1000000,
    unconfirmedBalance: 0,
    totalBalance: 1000000,
    addresses: [],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [],
    passwordHint: "hint",
    ...overrides,
  };
}

// ─── Helpers ───────────────────────────────────────────────────────

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
  useContractStore.setState({
    contracts: [],
    selectedContractId: null,
    selectedContractDetail: null,
    loading: false,
    fetching: false,
    error: null,
  });
  useWalletStore.setState({
    hdWallets: [],
    singleKeyWallets: [],
    selectedWallet: null,
    loading: false,
    refreshing: false,
    error: null,
  });
}

function setupWithIdentities(identities: QualifiedIdentityDto[]) {
  mockCommands.identityListLocal.mockResolvedValue({
    status: "ok",
    data: identities,
  });
  useIdentityStore.setState({ identities, loading: false });
}

function fireTaskResult(payload: unknown) {
  act(() => {
    for (const listener of eventListeners.taskResultEvent) {
      listener({ payload });
    }
  });
}

function fireTaskError(payload: unknown) {
  act(() => {
    for (const listener of eventListeners.taskErrorEvent) {
      listener({ payload });
    }
  });
}

function setup() {
  const user = userEvent.setup();
  const result = render(<RegisterContractScreen />);
  return { user, ...result };
}

// ─── Tests ─────────────────────────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  eventListeners.taskResultEvent = [];
  eventListeners.taskErrorEvent = [];
  mockCommands.contractRegister.mockResolvedValue({
    status: "ok",
    data: { taskId: "task-1" },
  });
});

describe("RegisterContractScreen", () => {
  describe("rendering", () => {
    it("renders the page title", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Register Data Contract")).toBeInTheDocument();
    });

    it("renders breadcrumbs", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Contracts")).toBeInTheDocument();
      // "Register Contract" appears in both breadcrumb and button
      expect(screen.getAllByText(/Register Contract/).length).toBeGreaterThanOrEqual(2);
    });

    it("renders back button", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByRole("button", { name: /Back to Contracts/i }),
      ).toBeInTheDocument();
    });

    it("renders step headings", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
      expect(
        screen.getByText("2. Contract Alias (optional)"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("3. Paste the contract JSON below"),
      ).toBeInTheDocument();
    });

    it("renders identity selector", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Contract Owner")).toBeInTheDocument();
    });

    it("renders alias input", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByPlaceholderText("e.g., My DApp Contract"),
      ).toBeInTheDocument();
    });

    it("renders contract JSON textarea", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByLabelText("Contract JSON"),
      ).toBeInTheDocument();
    });

    it("renders dashpay.io link", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(
        screen.getByText("Create a contract on dashpay.io"),
      ).toBeInTheDocument();
    });

    it("renders register button (disabled initially)", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      const btn = screen.getByRole("button", { name: /Register Contract/i });
      expect(btn).toBeInTheDocument();
      expect(btn).toBeDisabled();
    });

    it("renders advanced options toggle", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText("Advanced Options")).toBeInTheDocument();
    });

    it("shows identity balance", () => {
      const identity = makeIdentity({ balance: 10_000_000_000 });
      setupWithIdentities([identity]);
      setup();

      expect(screen.getByText(/Balance:/)).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner when identities are loading", () => {
      useIdentityStore.setState({ loading: true, identities: [] });
      setup();

      expect(
        screen.getByText("Loading identities..."),
      ).toBeInTheDocument();
    });

    it("does not show loading spinner when identities exist", () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      useIdentityStore.setState({ loading: true });
      setup();

      expect(
        screen.queryByText("Loading identities..."),
      ).not.toBeInTheDocument();
    });
  });

  describe("no identities", () => {
    it("shows warning when no identities are loaded", async () => {
      setupWithIdentities([]);
      setup();

      await waitFor(() => {
        expect(
          screen.getByText(/No identities loaded/),
        ).toBeInTheDocument();
      });
    });
  });

  describe("JSON parsing", () => {
    it("shows parse error for invalid JSON", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste("not valid json {{{");

      await waitFor(() => {
        expect(screen.getByText(/Invalid JSON:/)).toBeInTheDocument();
      });
    });

    it("shows parse error for non-object JSON", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste("[1, 2, 3]");

      await waitFor(() => {
        expect(
          screen.getByText("Contract JSON must be an object."),
        ).toBeInTheDocument();
      });
    });

    it("shows fee estimation for valid full contract", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(screen.getByText(/Estimated Fee:/)).toBeInTheDocument();
      });
    });

    it("enables register button with valid contract and identity", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        const btn = screen.getByRole("button", {
          name: /Register Contract/i,
        });
        expect(btn).not.toBeDisabled();
      });
    });

    it("clears parse state when textarea is emptied", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(screen.getByText(/Estimated Fee:/)).toBeInTheDocument();
      });

      await user.clear(textarea);

      await waitFor(() => {
        expect(screen.queryByText(/Estimated Fee:/)).not.toBeInTheDocument();
      });
    });
  });

  describe("raw schema auto-wrap", () => {
    it("shows auto-wrap notification for raw schemas", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_RAW_SCHEMAS);

      await waitFor(() => {
        expect(
          screen.getByText(/Raw document schemas detected/),
        ).toBeInTheDocument();
      });
    });

    it("enables register button for auto-wrapped schemas", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_RAW_SCHEMAS);

      await waitFor(() => {
        const btn = screen.getByRole("button", {
          name: /Register Contract/i,
        });
        expect(btn).not.toBeDisabled();
      });
    });

    it("shows error when pasting raw schemas with no identity selected", async () => {
      setupWithIdentities([]);
      const { user } = setup();

      // Wait for the form to render (loading state clears)
      await waitFor(() => {
        expect(screen.getByLabelText("Contract JSON")).toBeInTheDocument();
      });

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_RAW_SCHEMAS);

      await waitFor(() => {
        expect(
          screen.getByText(
            /Please select an identity before pasting raw document schemas/,
          ),
        ).toBeInTheDocument();
      });
    });

    it("does not show auto-wrap for full contract JSON", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(screen.getByText(/Estimated Fee:/)).toBeInTheDocument();
      });

      expect(
        screen.queryByText(/Raw document schemas detected/),
      ).not.toBeInTheDocument();
    });
  });

  describe("advanced options", () => {
    it("shows key selector when advanced options is toggled", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({ keyId: 0, securityLevel: "HIGH" }),
          makeKey({ keyId: 1, securityLevel: "CRITICAL" }),
        ],
      });
      setupWithIdentities([identity]);
      const { user } = setup();

      await user.click(screen.getByText("Advanced Options"));

      await waitFor(() => {
        expect(screen.getByText("Signing Key")).toBeInTheDocument();
      });
    });

    it("hides key selector when advanced options is toggled off", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      // Open
      await user.click(screen.getByText("Advanced Options"));
      await waitFor(() => {
        expect(screen.getByText("Signing Key")).toBeInTheDocument();
      });

      // Close
      await user.click(screen.getByText("Advanced Options"));
      await waitFor(() => {
        expect(screen.queryByText("Signing Key")).not.toBeInTheDocument();
      });
    });
  });

  describe("contract registration", () => {
    it("dispatches contractRegister on button click", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      // Fill contract JSON
      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      // Wait for button to be enabled
      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      expect(mockCommands.contractRegister).toHaveBeenCalledWith(
        expect.objectContaining({
          identityId: identity.id,
          keyId: 0,
          alias: "",
        }),
      );
    });

    it("passes alias in registration call", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      // Fill alias
      const aliasInput = screen.getByPlaceholderText("e.g., My DApp Contract");
      await user.type(aliasInput, "My Contract");

      // Fill contract JSON
      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      expect(mockCommands.contractRegister).toHaveBeenCalledWith(
        expect.objectContaining({
          alias: "My Contract",
        }),
      );
    });

    it("shows broadcasting state after clicking register", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(
          screen.getByText(/Broadcasting contract registration/),
        ).toBeInTheDocument();
      });
    });

    it("shows success screen after task result event", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      // Wait for broadcasting state
      await waitFor(() => {
        expect(
          screen.getByText(/Broadcasting contract registration/),
        ).toBeInTheDocument();
      });

      // Fire success event
      fireTaskResult({
        taskId: "task-1",
        resultType: "Contract",
        payload: null,
      });

      await waitFor(() => {
        expect(
          screen.getByText("Contract Registered Successfully"),
        ).toBeInTheDocument();
      });
    });

    it("shows error when task error event fires", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(
          screen.getByText(/Broadcasting contract registration/),
        ).toBeInTheDocument();
      });

      fireTaskError({
        taskId: "task-1",
        message: "Insufficient balance for contract registration",
        details: "",
        recoverable: false,
      });

      await waitFor(() => {
        expect(screen.getByText("Registration Failed")).toBeInTheDocument();
        expect(
          screen.getByText("Insufficient balance for contract registration"),
        ).toBeInTheDocument();
      });
    });

    it("shows error when IPC command returns error", async () => {
      mockCommands.contractRegister.mockResolvedValue({
        status: "error",
        error: "Network error",
      });

      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(screen.getByText("Registration Failed")).toBeInTheDocument();
        expect(screen.getByText("Network error")).toBeInTheDocument();
      });
    });

    it("shows error when IPC command throws", async () => {
      mockCommands.contractRegister.mockRejectedValue(
        new Error("Connection refused"),
      );

      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(screen.getByText("Registration Failed")).toBeInTheDocument();
        expect(screen.getByText("Connection refused")).toBeInTheDocument();
      });
    });

    it("ignores task result events with non-matching taskId", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(
          screen.getByText(/Broadcasting contract registration/),
        ).toBeInTheDocument();
      });

      // Fire event with wrong task ID
      fireTaskResult({
        taskId: "wrong-task-id",
        resultType: "Contract",
        payload: null,
      });

      // Should still be broadcasting
      expect(
        screen.getByText(/Broadcasting contract registration/),
      ).toBeInTheDocument();
    });

    it("ignores task result events with non-Contract result type", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(
          screen.getByText(/Broadcasting contract registration/),
        ).toBeInTheDocument();
      });

      fireTaskResult({
        taskId: "task-1",
        resultType: "Document",
        payload: null,
      });

      // Should still be broadcasting
      expect(
        screen.getByText(/Broadcasting contract registration/),
      ).toBeInTheDocument();
    });
  });

  describe("error dismissal", () => {
    it("returns to input phase when dismiss is clicked", async () => {
      mockCommands.contractRegister.mockResolvedValue({
        status: "error",
        error: "Some error",
      });

      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(screen.getByText("Registration Failed")).toBeInTheDocument();
      });

      await user.click(screen.getByRole("button", { name: /Dismiss/i }));

      await waitFor(() => {
        expect(
          screen.queryByText("Registration Failed"),
        ).not.toBeInTheDocument();
        expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
      });
    });
  });

  describe("success screen", () => {
    async function goToSuccess() {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      await waitFor(() => {
        expect(
          screen.getByText(/Broadcasting contract registration/),
        ).toBeInTheDocument();
      });

      fireTaskResult({
        taskId: "task-1",
        resultType: "Contract",
        payload: null,
      });

      await waitFor(() => {
        expect(
          screen.getByText("Contract Registered Successfully"),
        ).toBeInTheDocument();
      });

      return { user };
    }

    it("shows success message and buttons", async () => {
      await goToSuccess();

      expect(
        screen.getByText("Contract Registered Successfully"),
      ).toBeInTheDocument();
      // Two "Back to Contracts" buttons: one in header, one in success
      const backButtons = screen.getAllByRole("button", {
        name: /Back to Contracts/i,
      });
      expect(backButtons.length).toBeGreaterThanOrEqual(1);
      expect(
        screen.getByRole("button", { name: /Register Another Contract/i }),
      ).toBeInTheDocument();
    });

    it("navigates back on Back to Contracts click", async () => {
      const { user } = await goToSuccess();

      // Click the second "Back to Contracts" button (in success area)
      const backButtons = screen.getAllByRole("button", {
        name: /Back to Contracts/i,
      });
      await user.click(backButtons[backButtons.length - 1]);

      expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
    });

    it("resets form on Register Another Contract click", async () => {
      const { user } = await goToSuccess();

      await user.click(
        screen.getByRole("button", { name: /Register Another Contract/i }),
      );

      await waitFor(() => {
        expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
        const textarea = screen.getByLabelText("Contract JSON");
        expect(textarea).toHaveValue("");
      });
    });
  });

  describe("navigation", () => {
    it("navigates back on back button click", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      const { user } = setup();

      await user.click(
        screen.getByRole("button", { name: /Back to Contracts/i }),
      );

      expect(mockNavigate).toHaveBeenCalledWith({ to: "/contracts" });
    });
  });

  describe("wallet lock state", () => {
    function setupWithLockedWallet(usesPassword = true) {
      const wallet = makeWalletDto({ usesPassword, passwordHint: usesPassword ? "hint" : null });
      mockCommands.walletListAll.mockResolvedValue({
        status: "ok",
        data: { hdWallets: [wallet], singleKeyWallets: [], selectedWallet: null },
      });
      useWalletStore.setState({
        hdWallets: [wallet as never],
        singleKeyWallets: [],
      });
    }

    it("shows wallet locked warning when wallet has password", async () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      setupWithLockedWallet();
      setup();

      await waitFor(() => {
        expect(
          screen.getByText(/Wallet is locked/),
        ).toBeInTheDocument();
      });
    });

    it("disables register button when wallet is locked", async () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      setupWithLockedWallet();
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      // Even with valid JSON, button should be disabled
      await waitFor(() => {
        expect(screen.getByText(/Estimated Fee:/)).toBeInTheDocument();
      });

      expect(
        screen.getByRole("button", { name: /Register Contract/i }),
      ).toBeDisabled();
    });

    it("shows unlock wallet button", async () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      setupWithLockedWallet();
      setup();

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Unlock Wallet/i }),
        ).toBeInTheDocument();
      });
    });

    it("does not show wallet locked warning for wallets without password", async () => {
      const identity = makeIdentity({
        associatedWalletHashes: ["wallet-hash-1"],
      });
      setupWithIdentities([identity]);
      setupWithLockedWallet(false);
      setup();

      // Wait for form to render
      await waitFor(() => {
        expect(screen.getByText("1. Select Identity")).toBeInTheDocument();
      });

      expect(screen.queryByText(/Wallet is locked/)).not.toBeInTheDocument();
    });
  });

  describe("event subscription", () => {
    it("subscribes to task result and error events", async () => {
      const identity = makeIdentity();
      setupWithIdentities([identity]);
      setup();

      await waitFor(() => {
        expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
        expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
      });
    });
  });

  describe("key auto-selection", () => {
    it("auto-selects HIGH auth key", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({ keyId: 0, purpose: "OWNER", securityLevel: "MASTER" }),
          makeKey({
            keyId: 1,
            purpose: "AUTHENTICATION",
            securityLevel: "HIGH",
          }),
          makeKey({
            keyId: 2,
            purpose: "AUTHENTICATION",
            securityLevel: "CRITICAL",
          }),
        ],
      });
      setupWithIdentities([identity]);
      const { user } = setup();

      // Fill valid JSON and register — check which keyId is passed
      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      expect(mockCommands.contractRegister).toHaveBeenCalledWith(
        expect.objectContaining({
          keyId: 1, // HIGH key
        }),
      );
    });

    it("falls back to CRITICAL when no HIGH key exists", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({
            keyId: 2,
            purpose: "AUTHENTICATION",
            securityLevel: "CRITICAL",
          }),
        ],
      });
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: /Register Contract/i }),
        ).not.toBeDisabled();
      });

      await user.click(
        screen.getByRole("button", { name: /Register Contract/i }),
      );

      expect(mockCommands.contractRegister).toHaveBeenCalledWith(
        expect.objectContaining({
          keyId: 2,
        }),
      );
    });

    it("disables register when no eligible keys exist", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({ keyId: 0, purpose: "OWNER", securityLevel: "MASTER" }),
        ],
      });
      setupWithIdentities([identity]);
      const { user } = setup();

      const textarea = screen.getByLabelText("Contract JSON");
      await user.click(textarea);
      await user.paste(VALID_FULL_CONTRACT);

      // Even with valid JSON, no eligible key means disabled
      await waitFor(() => {
        expect(screen.getByText(/Estimated Fee:/)).toBeInTheDocument();
      });

      expect(
        screen.getByRole("button", { name: /Register Contract/i }),
      ).toBeDisabled();
    });
  });
});
