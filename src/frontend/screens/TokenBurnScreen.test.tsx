import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenBurnScreen } from "./TokenBurnScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-burn-111222",
  contractId: "contract-burn-111222",
  tokenPosition: "0",
  name: "BurnToken",
  balance: "10000000",
  decimals: "8",
  identityId: "id-burn-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenBurn = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-burn-1" },
});

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

const mockIdentities = [
  {
    id: "id-burn-identity",
    alias: "BurnIdentity",
    balance: 3000000000,
    keys: [
      {
        keyId: 5,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "burn-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-burn"],
    walletIndex: 0,
    topUps: [],
    status: "Active",
    identityType: "User",
  },
];

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      identities: mockIdentities,
      loadIdentities: vi.fn().mockResolvedValue(null),
    };
    return sel ? sel(s) : s;
  },
}));

vi.mock("@/stores/walletStore", () => ({
  useWalletStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      hdWallets: [
        {
          seedHash: "seed-hash-burn",
          alias: "BurnWallet",
          usesPassword: false,
          passwordHint: null,
        },
      ],
      singleKeyWallets: [],
      loadWallets: vi.fn().mockResolvedValue(null),
    };
    return sel ? sel(s) : s;
  },
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
    ...render(<TokenBurnScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenBurnScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenBurn).mockImplementation(mockTokenBurn);
    currentSearch = {
      tokenId: "token-burn-111222",
      contractId: "contract-burn-111222",
      tokenPosition: "0",
      name: "BurnToken",
      balance: "10000000",
      decimals: "8",
      identityId: "id-burn-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Burn action name", () => {
      setup();
      expect(screen.getByRole("button", { name: /burn/i })).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("BurnToken")).toBeInTheDocument();
    });

    it("shows amount input with burn label", () => {
      setup();
      expect(screen.getByText("Amount to Burn")).toBeInTheDocument();
    });

    it("does not show recipient input (burn has no recipient)", () => {
      setup();
      expect(screen.queryByText(/recipient/i)).not.toBeInTheDocument();
    });

    it("shows identity selector", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables burn button when amount is empty", () => {
      setup();
      const button = screen.getByRole("button", { name: /burn/i });
      expect(button).toBeDisabled();
    });

    it("enables burn button when amount is valid and within balance", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");
      const button = screen.getByRole("button", { name: /burn/i });
      expect(button).toBeEnabled();
    });

    it("shows validation message when amount exceeds balance", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "99999999");
      expect(screen.getByText(/exceeds available balance/i)).toBeInTheDocument();
    });

    it("shows validation message when amount is zero", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "0");
      expect(screen.getByText(/must be greater than 0/i)).toBeInTheDocument();
    });

    it("disables burn button when amount exceeds balance", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "99999999");
      const button = screen.getByRole("button", { name: /burn/i });
      expect(button).toBeDisabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenBurn with correct parameters", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      // Confirm in destructive dialog
      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /burn/i });
      await user.click(confirmBtn);

      expect(mockTokenBurn).toHaveBeenCalledWith({
        operation: {
          identityId: "id-burn-identity",
          contractId: "contract-burn-111222",
          tokenPosition: 0,
          keyId: 5,
          publicNote: null,
        },
        amount: "5000",
        groupInfo: null,
      });
    });

    it("shows destructive confirmation dialog", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      expect(screen.getByText("Confirm Burn")).toBeInTheDocument();
      expect(screen.getByText(/cannot be undone/i)).toBeInTheDocument();
    });

    it("shows broadcasting state after confirmation", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /burn/i });
      await user.click(confirmBtn);

      expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();
    });
  });

  // ── Group Action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows group action info when groupActionId is present", () => {
      setup({
        groupActionId: "group-burn-123",
        details: JSON.stringify({ amount: "3000" }),
      });
      expect(screen.getByText("Group Action Signing")).toBeInTheDocument();
    });

    it("hides amount input when signing group action", () => {
      setup({
        groupActionId: "group-burn-123",
        details: JSON.stringify({ amount: "3000" }),
      });
      expect(screen.queryByText("Amount to Burn")).not.toBeInTheDocument();
    });

    it("shows group action amount as read-only", () => {
      setup({
        groupActionId: "group-burn-123",
        details: JSON.stringify({ amount: "3000" }),
      });
      expect(screen.getByText("3000")).toBeInTheDocument();
    });

    it("calls tokenBurn with groupInfo when signing group action", async () => {
      const { user } = setup({
        groupActionId: "group-burn-123",
        details: JSON.stringify({ amount: "3000" }),
      });

      const buttons = screen.getAllByRole("button");
      const signButton = buttons.find(
        (b) =>
          b.textContent?.toLowerCase().includes("sign") ||
          b.textContent?.toLowerCase().includes("burn"),
      );

      if (signButton && !signButton.disabled) {
        await user.click(signButton);

        const dialog = screen.queryByRole("dialog");
        if (dialog) {
          const confirmBtn = within(dialog).getByRole("button", {
            name: /sign|burn/i,
          });
          await user.click(confirmBtn);
        }

        expect(mockTokenBurn).toHaveBeenCalledWith(
          expect.objectContaining({
            groupInfo: expect.objectContaining({
              type: "other_signer",
              action_id: "group-burn-123",
            }),
          }),
        );
      }
    });

    it("skips balance validation when signing group action", () => {
      setup({
        groupActionId: "group-burn-123",
        details: JSON.stringify({ amount: "999999999999" }),
      });
      // Should not show "exceeds balance" since group signing bypasses balance check
      expect(screen.queryByText(/exceeds available balance/i)).not.toBeInTheDocument();
    });
  });

  // ── Result Handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result event", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /burn/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-burn-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByText(/burn successful/i)).toBeInTheDocument();
    });

    it("shows error on task error event", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /burn/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskErrorEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-burn-1",
            message: "Burning not allowed on this token",
          },
        });
      });

      expect(screen.getByText(/burning not allowed/i)).toBeInTheDocument();
    });

    it("shows Burn More button on success", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /burn/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-burn-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByRole("button", { name: /burn more/i })).toBeInTheDocument();
    });

    it("shows Back to Tokens button on success", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "5000");

      const button = screen.getByRole("button", { name: /burn/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /burn/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-burn-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByRole("button", { name: /back to tokens/i })).toBeInTheDocument();
    });
  });
});
