import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenFreezeScreen } from "./TokenFreezeScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-freeze-111",
  contractId: "contract-freeze-111",
  tokenPosition: "0",
  name: "FreezeToken",
  balance: "5000000",
  decimals: "8",
  identityId: "id-freeze-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenFreeze = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-freeze-1" },
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
    id: "id-freeze-identity",
    alias: "FreezeIdentity",
    balance: 3000000000,
    keys: [
      {
        keyId: 5,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "freeze-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-freeze"],
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
          seedHash: "seed-hash-freeze",
          alias: "FreezeWallet",
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
    ...render(<TokenFreezeScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenFreezeScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenFreeze).mockImplementation(mockTokenFreeze);
    currentSearch = {
      tokenId: "token-freeze-111",
      contractId: "contract-freeze-111",
      tokenPosition: "0",
      name: "FreezeToken",
      balance: "5000000",
      decimals: "8",
      identityId: "id-freeze-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Freeze action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /freeze/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("FreezeToken")).toBeInTheDocument();
    });

    it("shows identity ID input field", () => {
      setup();
      expect(
        screen.getByLabelText("Identity ID to Freeze"),
      ).toBeInTheDocument();
    });

    it("shows identity selector for signing identity", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("does not show amount input (freeze has no amount)", () => {
      setup();
      expect(screen.queryByText(/amount/i)).not.toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables freeze button when identity ID is empty", () => {
      setup();
      const button = screen.getByRole("button", { name: /freeze/i });
      expect(button).toBeDisabled();
    });

    it("enables freeze button when identity ID is entered", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Freeze");
      await user.type(input, "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec");
      const button = screen.getByRole("button", { name: /freeze/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenFreeze with correct params after confirmation", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Freeze");
      await user.type(input, "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec");

      // Click freeze
      const freezeButton = screen.getByRole("button", { name: /freeze/i });
      await user.click(freezeButton);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /freeze/i,
      });
      await user.click(confirmButton);

      expect(mockTokenFreeze).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-freeze-111",
            tokenPosition: 0,
          }),
          freezeIdentityId:
            "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
        }),
      );
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Freeze");
      await user.type(input, "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec");

      const freezeButton = screen.getByRole("button", { name: /freeze/i });
      await user.click(freezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /freeze/i,
      });
      await user.click(confirmButton);

      expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Freeze");
      await user.type(input, "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec");

      const freezeButton = screen.getByRole("button", { name: /freeze/i });
      await user.click(freezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /freeze/i,
      });
      await user.click(confirmButton);

      // Simulate success event
      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-freeze-1",
            result: { type: "tokenCompleted" },
            payload: null,
          },
        });
      });

      expect(screen.getByText("Freeze Successful")).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Freeze");
      await user.type(input, "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec");

      const freezeButton = screen.getByRole("button", { name: /freeze/i });
      await user.click(freezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /freeze/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        const listener = vi.mocked(events.taskErrorEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-freeze-1",
            message: "Freeze failed",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(screen.getByText("Freeze failed")).toBeInTheDocument();
    });

    it("shows Freeze Another button on success", async () => {
      const { user } = setup();
      const input = screen.getByLabelText("Identity ID to Freeze");
      await user.type(input, "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec");

      const freezeButton = screen.getByRole("button", { name: /freeze/i });
      await user.click(freezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /freeze/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-freeze-1",
            result: { type: "tokenCompleted" },
            payload: null,
          },
        });
      });

      expect(
        screen.getByRole("button", { name: /freeze another/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Group action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows read-only identity when group signing", () => {
      setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({
          freezeIdentityId: "frozen-target-id",
        }),
      });

      expect(
        screen.getByText("Identity to freeze:"),
      ).toBeInTheDocument();
      expect(screen.getByText("frozen-target-id")).toBeInTheDocument();
      // Should not show the text input
      expect(
        screen.queryByLabelText("Identity ID to Freeze"),
      ).not.toBeInTheDocument();
    });

    it("passes group info in IPC call when group signing", async () => {
      const { user } = setup({
        groupActionId: "group-action-123",
        details: JSON.stringify({
          freezeIdentityId: "frozen-target-id",
        }),
      });

      // Button should say "Sign Freeze"
      const signButton = screen.getByRole("button", {
        name: /sign freeze/i,
      });
      await user.click(signButton);

      // Confirm
      const confirmButton = screen.getByRole("button", {
        name: /sign freeze/i,
      });
      await user.click(confirmButton);

      expect(mockTokenFreeze).toHaveBeenCalledWith(
        expect.objectContaining({
          freezeIdentityId: "frozen-target-id",
          groupInfo: expect.objectContaining({
            type: "otherSigner",
            groupContractPosition: 0,
            actionId: "group-action-123",
            actionIsProposer: false,
          }),
        }),
      );
    });
  });
});
