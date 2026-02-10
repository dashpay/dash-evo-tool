import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenUnfreezeScreen } from "./TokenUnfreezeScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-unfreeze-222",
  contractId: "contract-unfreeze-222",
  tokenPosition: "0",
  name: "UnfreezeToken",
  balance: "8000000",
  decimals: "8",
  identityId: "id-unfreeze-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenUnfreeze = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-unfreeze-1" },
});

let mockTaskResultListener: ((event: { payload: unknown }) => void) | null =
  null;
let mockTaskErrorListener: ((event: { payload: unknown }) => void) | null =
  null;

vi.mock("@/bindings", () => ({
  commands: {
    tokenUnfreeze: (...args: unknown[]) => mockTokenUnfreeze(...args),
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
    id: "id-unfreeze-identity",
    alias: "UnfreezeIdentity",
    balance: 2500000000,
    keys: [
      {
        keyId: 7,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "unfreeze-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-unfreeze"],
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
        seedHash: "seed-hash-unfreeze",
        alias: "UnfreezeWallet",
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
    ...render(<TokenUnfreezeScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenUnfreezeScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockTaskResultListener = null;
    mockTaskErrorListener = null;
    currentSearch = {
      tokenId: "token-unfreeze-222",
      contractId: "contract-unfreeze-222",
      tokenPosition: "0",
      name: "UnfreezeToken",
      balance: "8000000",
      decimals: "8",
      identityId: "id-unfreeze-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Unfreeze action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /unfreeze/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("UnfreezeToken")).toBeInTheDocument();
    });

    it("shows identity ID input field", () => {
      setup();
      expect(
        screen.getByLabelText("Identity ID to Unfreeze"),
      ).toBeInTheDocument();
    });

    it("shows helper text about entering frozen identity", () => {
      setup();
      expect(
        screen.getByText(
          /enter the identity id of the frozen identity you want to unfreeze/i,
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
    it("disables unfreeze button when identity ID is empty", () => {
      setup();
      const button = screen.getByRole("button", { name: /unfreeze/i });
      expect(button).toBeDisabled();
    });

    it("enables unfreeze button when identity ID is entered", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Unfreeze");
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");
      const button = screen.getByRole("button", { name: /unfreeze/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenUnfreeze with correct params after confirmation", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Unfreeze");
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const unfreezeButton = screen.getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(unfreezeButton);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUnfreeze).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-unfreeze-222",
            tokenPosition: 0,
          }),
          unfreezeIdentityId:
            "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq",
        }),
      );
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Unfreeze");
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const unfreezeButton = screen.getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(unfreezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      expect(screen.getByText("Unfreeze...")).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Unfreeze");
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const unfreezeButton = screen.getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(unfreezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskResultListener?.({
          payload: {
            taskId: "task-unfreeze-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(screen.getByText("Unfreeze Successful")).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Unfreeze");
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const unfreezeButton = screen.getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(unfreezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskErrorListener?.({
          payload: {
            taskId: "task-unfreeze-1",
            message: "Unfreeze failed",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(screen.getByText("Unfreeze failed")).toBeInTheDocument();
    });

    it("shows Unfreeze Another button on success", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Unfreeze");
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const unfreezeButton = screen.getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(unfreezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskResultListener?.({
          payload: {
            taskId: "task-unfreeze-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByRole("button", { name: /unfreeze another/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Group action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows read-only identity when group signing", () => {
      setup({
        groupActionId: "group-action-456",
        details: JSON.stringify({
          unfreezeIdentityId: "target-frozen-id",
        }),
      });

      expect(
        screen.getByText("Identity to unfreeze:"),
      ).toBeInTheDocument();
      expect(screen.getByText("target-frozen-id")).toBeInTheDocument();
      expect(
        screen.queryByLabelText("Identity ID to Unfreeze"),
      ).not.toBeInTheDocument();
    });

    it("passes group info in IPC call when group signing", async () => {
      const { user } = setup({
        groupActionId: "group-action-456",
        details: JSON.stringify({
          unfreezeIdentityId: "target-frozen-id",
        }),
      });

      const signButton = screen.getByRole("button", {
        name: /sign unfreeze/i,
      });
      await user.click(signButton);

      const confirmButton = screen.getByRole("button", {
        name: /sign unfreeze/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUnfreeze).toHaveBeenCalledWith(
        expect.objectContaining({
          unfreezeIdentityId: "target-frozen-id",
          groupInfo: expect.objectContaining({
            type: "other_signer",
            action_id: "group-action-456",
          }),
        }),
      );
    });
  });
});
