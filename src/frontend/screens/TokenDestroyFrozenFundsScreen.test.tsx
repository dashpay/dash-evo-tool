import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenDestroyFrozenFundsScreen } from "./TokenDestroyFrozenFundsScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-destroy-333",
  contractId: "contract-destroy-333",
  tokenPosition: "0",
  name: "DestroyToken",
  balance: "3000000",
  decimals: "8",
  identityId: "id-destroy-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenDestroyFrozenFunds = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-destroy-1" },
});

let mockTaskResultListener: ((event: { payload: unknown }) => void) | null =
  null;
let mockTaskErrorListener: ((event: { payload: unknown }) => void) | null =
  null;

vi.mock("@/bindings", () => ({
  commands: {
    tokenDestroyFrozenFunds: (...args: unknown[]) =>
      mockTokenDestroyFrozenFunds(...args),
    walletNotifyUnlocked: vi.fn().mockResolvedValue({ status: "ok" }),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockImplementation((cb) => {
        mockTaskResultListener = cb;
        return Promise.resolve(() => {
          mockTaskResultListener = null;
        });
      }),
    },
    taskErrorEvent: {
      listen: vi.fn().mockImplementation((cb) => {
        mockTaskErrorListener = cb;
        return Promise.resolve(() => {
          mockTaskErrorListener = null;
        });
      }),
    },
  },
}));

const mockIdentities = [
  {
    id: "id-destroy-identity",
    alias: "DestroyIdentity",
    balance: 5000000000,
    keys: [
      {
        keyId: 9,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "destroy-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-destroy"],
    walletIndex: 0,
    topUps: [],
    status: "Active",
    identityType: "User",
  },
];

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: () => ({
    identities: mockIdentities,
    loadIdentities: vi.fn().mockResolvedValue(null),
  }),
}));

vi.mock("@/stores/walletStore", () => ({
  useWalletStore: () => ({
    hdWallets: [
      {
        seedHash: "seed-hash-destroy",
        alias: "DestroyWallet",
        usesPassword: false,
        passwordHint: null,
      },
    ],
    singleKeyWallets: [],
    loadWallets: vi.fn().mockResolvedValue(null),
  }),
}));

vi.mock("@/stores/tokenStore", () => ({
  useTokenStore: () => ({
    loadMyTokenBalances: vi.fn().mockResolvedValue(null),
  }),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

vi.mock("@/lib/toastError", () => ({
  toastError: vi.fn(),
}));

// ─── Helpers ─────────────────────────────────────────────────────────────────

function setup(searchOverrides?: Partial<Record<string, string>>) {
  if (searchOverrides) {
    currentSearch = { ...currentSearch, ...searchOverrides };
  }
  return {
    user: userEvent.setup(),
    ...render(<TokenDestroyFrozenFundsScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenDestroyFrozenFundsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockTaskResultListener = null;
    mockTaskErrorListener = null;
    currentSearch = {
      tokenId: "token-destroy-333",
      contractId: "contract-destroy-333",
      tokenPosition: "0",
      name: "DestroyToken",
      balance: "3000000",
      decimals: "8",
      identityId: "id-destroy-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Destroy Frozen Funds action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /destroy frozen funds/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("DestroyToken")).toBeInTheDocument();
    });

    it("shows frozen identity ID input field", () => {
      setup();
      expect(
        screen.getByLabelText("Frozen Identity ID"),
      ).toBeInTheDocument();
    });

    it("shows helper text about permanent destruction", () => {
      setup();
      expect(
        screen.getByText(
          /enter the identity id of the frozen identity whose funds will be permanently destroyed/i,
        ),
      ).toBeInTheDocument();
    });

    it("shows identity selector for signing identity", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables destroy button when identity ID is empty", () => {
      setup();
      const button = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      expect(button).toBeDisabled();
    });

    it("enables destroy button when identity ID is entered", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");
      const button = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenDestroyFrozenFunds with correct params after confirmation", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      // Click destroy
      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      // Confirm in destructive dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /destroy/i,
      });
      await user.click(confirmButton);

      expect(mockTokenDestroyFrozenFunds).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-destroy-333",
            tokenPosition: 0,
          }),
          frozenIdentityId:
            "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq",
        }),
      );
    });

    it("shows destructive confirmation dialog", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      // Dialog should show title and mention "cannot be undone"
      expect(
        screen.getByText("Confirm Destroy Frozen Funds"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/this action cannot be undone/i),
      ).toBeInTheDocument();
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /destroy/i,
      });
      await user.click(confirmButton);

      expect(
        screen.getByText("Destroy Frozen Funds..."),
      ).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      const dialog1 = screen.getByRole("dialog");
      const confirmButton = within(dialog1).getByRole("button", {
        name: /destroy/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskResultListener?.({
          payload: {
            taskId: "task-destroy-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByText("Destroy Frozen Funds Successful"),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      const dialog2 = screen.getByRole("dialog");
      const confirmButton2 = within(dialog2).getByRole("button", {
        name: /destroy/i,
      });
      await user.click(confirmButton2);

      await act(async () => {
        mockTaskErrorListener?.({
          payload: {
            taskId: "task-destroy-1",
            message: "Destroy operation failed",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(screen.getByText("Destroy operation failed")).toBeInTheDocument();
    });

    it("shows Destroy More button on success", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Frozen Identity ID");
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      const dialog3 = screen.getByRole("dialog");
      const confirmButton3 = within(dialog3).getByRole("button", {
        name: /destroy/i,
      });
      await user.click(confirmButton3);

      await act(async () => {
        mockTaskResultListener?.({
          payload: {
            taskId: "task-destroy-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByRole("button", { name: /destroy more/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Group action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows read-only frozen identity when group signing", () => {
      setup({
        groupActionId: "group-action-789",
        details: JSON.stringify({
          frozenIdentityId: "frozen-victim-id",
        }),
      });

      expect(screen.getByText("Frozen identity:")).toBeInTheDocument();
      expect(screen.getByText("frozen-victim-id")).toBeInTheDocument();
      expect(
        screen.queryByLabelText("Frozen Identity ID"),
      ).not.toBeInTheDocument();
    });

    it("passes group info in IPC call when group signing", async () => {
      const { user } = setup({
        groupActionId: "group-action-789",
        details: JSON.stringify({
          frozenIdentityId: "frozen-victim-id",
        }),
      });

      const signButton = screen.getByRole("button", {
        name: /sign destroy/i,
      });
      await user.click(signButton);

      const confirmButton = screen.getByRole("button", {
        name: /sign destroy/i,
      });
      await user.click(confirmButton);

      expect(mockTokenDestroyFrozenFunds).toHaveBeenCalledWith(
        expect.objectContaining({
          frozenIdentityId: "frozen-victim-id",
          groupInfo: expect.objectContaining({
            type: "other_signer",
            action_id: "group-action-789",
          }),
        }),
      );
    });
  });
});
