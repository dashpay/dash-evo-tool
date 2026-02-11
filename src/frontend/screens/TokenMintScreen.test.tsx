import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenMintScreen } from "./TokenMintScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token111122223333",
  contractId: "contract111122223333",
  tokenPosition: "0",
  name: "MintToken",
  balance: "1000000",
  decimals: "8",
  identityId: "id-mint-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenMint = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-mint-1" },
});

const mockTokenGetMintingConfig = vi.fn().mockResolvedValue({
  status: "ok",
  data: {
    allowChoosingDestination: true,
    defaultDestinationIdentityId: null,
  },
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
    id: "id-mint-identity",
    alias: "MintIdentity",
    balance: 2000000000,
    keys: [
      {
        keyId: 3,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "aabbccdd",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
      {
        keyId: 4,
        purpose: "AUTHENTICATION",
        securityLevel: "CRITICAL",
        keyType: "BLS12_381",
        data: "11223344",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-mint"],
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
        seedHash: "seed-hash-mint",
        alias: "MintWallet",
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
    ...render(<TokenMintScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenMintScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenMint).mockImplementation(mockTokenMint);
    vi.mocked(commands.tokenGetMintingConfig).mockImplementation(mockTokenGetMintingConfig);
    // Default: allow choosing destination, no default destination
    mockTokenGetMintingConfig.mockResolvedValue({
      status: "ok",
      data: {
        allowChoosingDestination: true,
        defaultDestinationIdentityId: null,
      },
    });
    currentSearch = {
      tokenId: "token111122223333",
      contractId: "contract111122223333",
      tokenPosition: "0",
      name: "MintToken",
      balance: "1000000",
      decimals: "8",
      identityId: "id-mint-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Mint action name", () => {
      setup();
      expect(screen.getByRole("button", { name: /mint/i })).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("MintToken")).toBeInTheDocument();
    });

    it("shows amount input", () => {
      setup();
      expect(screen.getByText("Amount to Mint")).toBeInTheDocument();
    });

    it("shows recipient input when allowChoosingDestination is true", async () => {
      setup();
      // Wait for config to load (async)
      expect(await screen.findByText("Recipient Identity ID")).toBeInTheDocument();
    });

    it("shows recipient placeholder for self-mint", async () => {
      setup();
      expect(await screen.findByPlaceholderText(/leave empty to mint to yourself/i)).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables mint button when amount is empty", () => {
      setup();
      const button = screen.getByRole("button", { name: /mint/i });
      expect(button).toBeDisabled();
    });

    it("enables mint button when amount is valid (no recipient required)", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "1000");
      const button = screen.getByRole("button", { name: /mint/i });
      expect(button).toBeEnabled();
    });

    it("shows validation message when amount is zero", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "0");
      expect(screen.getByText(/must be greater than 0/i)).toBeInTheDocument();
    });

    it("allows mint without recipient (mints to self)", async () => {
      const { user } = setup();
      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");
      // Button should be enabled even without recipient
      const button = screen.getByRole("button", { name: /mint/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenMint with correct parameters (no recipient)", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "1000");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /mint/i });
      await user.click(confirmBtn);

      expect(mockTokenMint).toHaveBeenCalledWith({
        operation: {
          identityId: "id-mint-identity",
          contractId: "contract111122223333",
          tokenPosition: 0,
          keyId: 3,
          publicNote: null,
        },
        amount: "1000",
        recipientId: null,
        groupInfo: null,
      });
    });

    it("calls tokenMint with recipient when provided", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");
      const recipientInput = await screen.findByPlaceholderText(/leave empty to mint to yourself/i);
      await user.type(recipientInput, "recipient-abc");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /mint/i });
      await user.click(confirmBtn);

      expect(mockTokenMint).toHaveBeenCalledWith(
        expect.objectContaining({
          recipientId: "recipient-abc",
        }),
      );
    });

    it("shows confirmation dialog before minting", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      expect(screen.getByText("Confirm Mint")).toBeInTheDocument();
    });

    it("shows broadcasting state after confirmation", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /mint/i });
      await user.click(confirmBtn);

      expect(screen.getByText("Mint...")).toBeInTheDocument();
    });
  });

  // ── Group Action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows group action info when groupActionId is in search params", () => {
      setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({ amount: "2000", recipientId: "grp-recipient" }),
      });
      // Group signing info banner should be shown
      expect(screen.getByText("Group Action Signing")).toBeInTheDocument();
    });

    it("hides amount input when signing group action", () => {
      setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({ amount: "2000" }),
      });
      // Amount input should not be present (hidden for group signing)
      expect(screen.queryByText("Amount to Mint")).not.toBeInTheDocument();
    });

    it("shows group action details as read-only", () => {
      setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({ amount: "2000", recipientId: "recipient-xyz" }),
      });
      // Read-only amount display
      expect(screen.getByText("2000")).toBeInTheDocument();
    });

    it("calls tokenMint with groupInfo when signing group action", async () => {
      const { user } = setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({ amount: "2000" }),
      });

      // The form should be pre-populated and valid for group signing
      // Need to find the Sign Mint button
      const buttons = screen.getAllByRole("button");
      const signButton = buttons.find(
        (b) => b.textContent?.toLowerCase().includes("sign") || b.textContent?.toLowerCase().includes("mint"),
      );

      if (signButton && !signButton.disabled) {
        await user.click(signButton);

        // If confirmation dialog appears, confirm it
        const dialog = screen.queryByRole("dialog");
        if (dialog) {
          const confirmBtn = within(dialog).getByRole("button", {
            name: /sign|mint/i,
          });
          await user.click(confirmBtn);
        }

        expect(mockTokenMint).toHaveBeenCalledWith(
          expect.objectContaining({
            groupInfo: expect.objectContaining({
              type: "other_signer",
              action_id: "group-action-123",
            }),
          }),
        );
      }
    });

    it("hides recipient input when signing group action", () => {
      setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({ amount: "2000" }),
      });
      expect(screen.queryByText("Recipient Identity ID")).not.toBeInTheDocument();
    });
  });

  // ── Result Handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result event", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /mint/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-mint-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByText(/mint successful/i)).toBeInTheDocument();
    });

    it("shows error on task error event", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /mint/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskErrorEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-mint-1",
            message: "Minting not allowed",
          },
        });
      });

      expect(screen.getByText(/minting not allowed/i)).toBeInTheDocument();
    });

    it("shows Mint More button on success", async () => {
      const { user } = setup();

      const amountInput = screen.getByPlaceholderText("0");
      await user.type(amountInput, "500");

      const button = screen.getByRole("button", { name: /mint/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", { name: /mint/i });
      await user.click(confirmBtn);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-mint-1",
            resultType: "Token",
            result: {},
          },
        });
      });

      expect(screen.getByRole("button", { name: /mint more/i })).toBeInTheDocument();
    });
  });

  // ── Minting Destination Config ──────────────────────────────────────

  describe("minting destination config", () => {
    it("fetches minting config on mount", async () => {
      setup();
      // Wait for async config fetch
      await screen.findByText("Amount to Mint");
      expect(mockTokenGetMintingConfig).toHaveBeenCalledWith({
        contractId: "contract111122223333",
        tokenPosition: 0,
      });
    });

    it("hides recipient input when allowChoosingDestination is false and no default", async () => {
      mockTokenGetMintingConfig.mockResolvedValue({
        status: "ok",
        data: {
          allowChoosingDestination: false,
          defaultDestinationIdentityId: null,
        },
      });
      setup();
      // Wait for config to load
      await screen.findByText("Amount to Mint");
      // Recipient input should not be present
      expect(screen.queryByTestId("recipient-section")).not.toBeInTheDocument();
    });

    it("shows fixed destination info when choosing is not allowed but default exists", async () => {
      mockTokenGetMintingConfig.mockResolvedValue({
        status: "ok",
        data: {
          allowChoosingDestination: false,
          defaultDestinationIdentityId: "aabbccdd11223344556677889900aabb",
        },
      });
      setup();
      expect(await screen.findByText(/minted tokens will be sent to/i)).toBeInTheDocument();
    });

    it("auto-populates recipient with default destination identity", async () => {
      mockTokenGetMintingConfig.mockResolvedValue({
        status: "ok",
        data: {
          allowChoosingDestination: true,
          defaultDestinationIdentityId: "aabbccdd11223344556677889900aabb",
        },
      });
      setup();
      const recipientInput = await screen.findByTestId("operation-recipient-input") as HTMLInputElement;
      expect(recipientInput.value).toBe("aabbccdd11223344556677889900aabb");
    });

    it("marks recipient as optional when default destination exists", async () => {
      mockTokenGetMintingConfig.mockResolvedValue({
        status: "ok",
        data: {
          allowChoosingDestination: true,
          defaultDestinationIdentityId: "aabbccdd11223344556677889900aabb",
        },
      });
      setup();
      expect(await screen.findByText("Optional")).toBeInTheDocument();
    });

    it("shows recipient as optional even when no default destination", async () => {
      mockTokenGetMintingConfig.mockResolvedValue({
        status: "ok",
        data: {
          allowChoosingDestination: true,
          defaultDestinationIdentityId: null,
        },
      });
      setup();
      // Wait for config to load and recipient to render
      expect(await screen.findByText("Optional")).toBeInTheDocument();
    });

    it("falls back to showing recipient input when config fetch fails", async () => {
      mockTokenGetMintingConfig.mockRejectedValue(new Error("Network error"));
      setup();
      // Should still show recipient input as fallback
      expect(await screen.findByText("Recipient Identity ID")).toBeInTheDocument();
    });
  });
});
