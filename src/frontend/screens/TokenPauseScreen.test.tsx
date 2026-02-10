import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenPauseScreen } from "./TokenPauseScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-pause-111",
  contractId: "contract-pause-111",
  tokenPosition: "0",
  name: "PauseToken",
  balance: "5000000",
  decimals: "8",
  identityId: "id-pause-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenPause = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-pause-1" },
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
    id: "id-pause-identity",
    alias: "PauseIdentity",
    balance: 3000000000,
    keys: [
      {
        keyId: 5,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "pause-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-pause"],
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
        seedHash: "seed-hash-pause",
        alias: "PauseWallet",
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
    ...render(<TokenPauseScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenPauseScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenPause).mockImplementation(mockTokenPause);
    currentSearch = {
      tokenId: "token-pause-111",
      contractId: "contract-pause-111",
      tokenPosition: "0",
      name: "PauseToken",
      balance: "5000000",
      decimals: "8",
      identityId: "id-pause-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Pause action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /pause/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("PauseToken")).toBeInTheDocument();
    });

    it("shows signing identity selector", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("does not show amount input (pause has no amount)", () => {
      setup();
      expect(screen.queryByText(/amount/i)).not.toBeInTheDocument();
    });

    it("does not show recipient input (pause has no recipient)", () => {
      setup();
      expect(screen.queryByText(/recipient/i)).not.toBeInTheDocument();
    });

    it("pause button is enabled by default (no additional input needed)", () => {
      setup();
      const button = screen.getByRole("button", { name: /pause/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenPause with correct params after confirmation", async () => {
      const { user } = setup();

      const pauseButton = screen.getByRole("button", { name: /pause/i });
      await user.click(pauseButton);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /pause/i,
      });
      await user.click(confirmButton);

      expect(mockTokenPause).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-pause-111",
            tokenPosition: 0,
          }),
        }),
      );
    });

    it("shows confirmation dialog with emergency warning", async () => {
      const { user } = setup();

      const pauseButton = screen.getByRole("button", { name: /pause/i });
      await user.click(pauseButton);

      expect(screen.getByText(/emergency action/i)).toBeInTheDocument();
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();

      const pauseButton = screen.getByRole("button", { name: /pause/i });
      await user.click(pauseButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /pause/i,
      });
      await user.click(confirmButton);

      expect(screen.getByText("Pause...")).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();

      const pauseButton = screen.getByRole("button", { name: /pause/i });
      await user.click(pauseButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /pause/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-pause-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(screen.getByText("Pause Successful")).toBeInTheDocument();
    });

    it("shows success message about paused transfers", async () => {
      const { user } = setup();

      const pauseButton = screen.getByRole("button", { name: /pause/i });
      await user.click(pauseButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /pause/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        const listener = vi.mocked(events.taskResultEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-pause-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByText(/token transfers have been paused/i),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();

      const pauseButton = screen.getByRole("button", { name: /pause/i });
      await user.click(pauseButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /pause/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        const listener = vi.mocked(events.taskErrorEvent.listen).mock.calls[0]?.[0];
        listener?.({
          payload: {
            taskId: "task-pause-1",
            message: "Pause failed: not authorized",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(
        screen.getByText("Pause failed: not authorized"),
      ).toBeInTheDocument();
    });
  });

  // ── Group action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows Sign Pause button when group signing", () => {
      setup({ groupActionId: "group-action-pause-1" });

      expect(
        screen.getByRole("button", { name: /sign pause/i }),
      ).toBeInTheDocument();
    });

    it("passes group info in IPC call when group signing", async () => {
      const { user } = setup({ groupActionId: "group-action-pause-1" });

      const signButton = screen.getByRole("button", {
        name: /sign pause/i,
      });
      await user.click(signButton);

      // Confirm
      const confirmButton = screen.getByRole("button", {
        name: /sign pause/i,
      });
      await user.click(confirmButton);

      expect(mockTokenPause).toHaveBeenCalledWith(
        expect.objectContaining({
          groupInfo: expect.objectContaining({
            type: "other_signer",
            action_id: "group-action-pause-1",
          }),
        }),
      );
    });
  });
});
