import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenTransferScreen } from "./TokenTransferScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
const mockRouterSearch: Record<string, string> = {
  tokenId: "token111122223333",
  contractId: "contract111122223333",
  tokenPosition: "0",
  name: "TestToken",
  balance: "500000000",
  decimals: "8",
  identityId: "id-abc123def456",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: mockRouterSearch } }),
}));

const mockTokenTransfer = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-transfer-1" },
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
    id: "id-abc123def456",
    alias: "TestIdentity",
    balance: 1000000000,
    keys: [
      {
        keyId: 1,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "deadbeef",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-1"],
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
        seedHash: "seed-hash-1",
        alias: "TestWallet",
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

function setup() {
  return {
    user: userEvent.setup(),
    ...render(<TokenTransferScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenTransferScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenTransfer).mockImplementation(mockTokenTransfer);
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders TokenOperationForm with Transfer action name", () => {
      setup();
      expect(screen.getByRole("button", { name: /transfer/i })).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("TestToken")).toBeInTheDocument();
    });

    it("shows amount input with correct label", () => {
      setup();
      expect(screen.getByText("Amount to Transfer")).toBeInTheDocument();
    });

    it("shows recipient input with correct label", () => {
      setup();
      expect(screen.getByText("Recipient Identity ID")).toBeInTheDocument();
    });

    it("shows identity selector", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("reads token context from route search params", () => {
      setup();
      // Token name should be visible
      expect(screen.getByText("TestToken")).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables transfer button when form is empty", () => {
      setup();
      const button = screen.getByRole("button", { name: /transfer/i });
      expect(button).toBeDisabled();
    });

    it("disables transfer button when only amount is entered", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const button = screen.getByRole("button", { name: /transfer/i });
      expect(button).toBeDisabled();
    });

    it("disables transfer button when only recipient is entered", async () => {
      const { user } = setup();
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "some-recipient-id");
      const button = screen.getByRole("button", { name: /transfer/i });
      expect(button).toBeDisabled();
    });

    it("enables transfer button when amount and recipient are valid", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");
      const button = screen.getByRole("button", { name: /transfer/i });
      expect(button).toBeEnabled();
    });

    it("shows validation message when amount exceeds balance", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "999999999");
      expect(screen.getByText(/exceeds available balance/i)).toBeInTheDocument();
    });

    it("shows validation message when amount is zero", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "0");
      expect(screen.getByText(/must be greater than 0/i)).toBeInTheDocument();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenTransfer with correct parameters", async () => {
      const { user } = setup();

      // Fill form
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");

      // Click transfer
      const button = screen.getByRole("button", { name: /transfer/i });
      await user.click(button);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /transfer/i });
      await user.click(confirmBtn);

      expect(mockTokenTransfer).toHaveBeenCalledWith({
        operation: {
          identityId: "id-abc123def456",
          contractId: "contract111122223333",
          tokenPosition: 0,
          keyId: 1,
          publicNote: null,
        },
        recipientId: "recipient-id-123",
        amount: "100",
      });
    });

    it("shows confirmation dialog before submitting", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");

      const button = screen.getByRole("button", { name: /transfer/i });
      await user.click(button);

      expect(screen.getByText("Confirm Transfer")).toBeInTheDocument();
    });

    it("shows broadcasting state after confirmation", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");

      const button = screen.getByRole("button", { name: /transfer/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /transfer/i });
      await user.click(confirmBtn);

      expect(screen.getByText("Transfer...")).toBeInTheDocument();
    });
  });

  // ── Success and Error ──────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result event", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");

      const button = screen.getByRole("button", { name: /transfer/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /transfer/i });
      await user.click(confirmBtn);

      // Simulate task result
      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-transfer-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByText(/transfer successful/i)).toBeInTheDocument();
    });

    it("shows error screen on task error event", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");

      const button = screen.getByRole("button", { name: /transfer/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /transfer/i });
      await user.click(confirmBtn);

      // Simulate task error
      await act(async () => {
        const listener = vi.mocked(events.taskErrorEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-transfer-1",
            message: "Network timeout",
          },
        });
      });

      expect(screen.getByText(/network timeout/i)).toBeInTheDocument();
    });

    it("shows Transfer More button on success", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "100");
      const recipientInput = screen.getByPlaceholderText(/identity/i);
      await user.type(recipientInput, "recipient-id-123");

      const button = screen.getByRole("button", { name: /transfer/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /transfer/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-transfer-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByRole("button", { name: /transfer more/i })).toBeInTheDocument();
    });
  });

  // ── No group action support ────────────────────────────────────────────

  describe("group action", () => {
    it("does not show group action info (transfer has no group support)", () => {
      setup();
      expect(screen.queryByText(/group/i)).not.toBeInTheDocument();
    });
  });
});
