// Radix Select uses hasPointerCapture/scrollIntoView which jsdom doesn't support
if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false;
}
if (!Element.prototype.setPointerCapture) {
  Element.prototype.setPointerCapture = () => {};
}
if (!Element.prototype.releasePointerCapture) {
  Element.prototype.releasePointerCapture = () => {};
}
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

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

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

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
  {
    id: "frozen-victim-aabb",
    alias: "FrozenVictim",
    balance: 100000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
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

/** Helper to emit a task result to all registered listeners. */
function emitTaskResult(payload: unknown) {
  const calls = vi.mocked(events.taskResultEvent.listen).mock.calls;
  for (const [cb] of calls) {
    cb?.({ payload });
  }
}

/** Helper to emit a task error to all registered listeners. */
function emitTaskError(payload: unknown) {
  const calls = vi.mocked(events.taskErrorEvent.listen).mock.calls;
  for (const [cb] of calls) {
    cb?.({ payload });
  }
}

/** Simulate frozen identity query result arriving. */
async function completeFrozenQuery(frozenIds: string[]) {
  // Wait for the dispatch to complete (sets taskIdRef.current)
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  // Now emit the result to all listeners
  await act(async () => {
    emitTaskResult({
      taskId: "frozen-query-1",
      resultType: "Token",
      payload: frozenIds,
    });
  });
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenDestroyFrozenFundsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenDestroyFrozenFunds).mockImplementation(mockTokenDestroyFrozenFunds);
    vi.mocked(commands.tokenQueryFrozenIdentities).mockResolvedValue({
      status: "ok",
      data: { taskId: "frozen-query-1" },
    });
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

  // ── Loading state ─────────────────────────────────────────────────────

  describe("loading state", () => {
    it("shows loading spinner while fetching frozen identities", () => {
      setup();
      expect(
        screen.getByText("Loading frozen identities from Platform..."),
      ).toBeInTheDocument();
    });
  });

  // ── Dropdown (frozen identities found) ────────────────────────────────

  describe("dropdown mode", () => {
    it("shows dropdown with frozen identities after query completes", async () => {
      setup();
      await completeFrozenQuery(["frozen-victim-aabb"]);

      expect(
        screen.getByTestId("frozen-identity-select"),
      ).toBeInTheDocument();
    });

    it("enables destroy button when a frozen identity is selected", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-victim-aabb"]);

      const trigger = screen.getByTestId("frozen-identity-select");
      await user.click(trigger);

      const option = screen.getByText("FrozenVictim");
      await user.click(option);

      const button = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      expect(button).toBeEnabled();
    });

    it("calls tokenDestroyFrozenFunds with selected frozen identity ID", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-victim-aabb"]);

      // Select frozen identity
      const trigger = screen.getByTestId("frozen-identity-select");
      await user.click(trigger);
      const option = screen.getByText("FrozenVictim");
      await user.click(option);

      // Click destroy
      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      // Confirm
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /destroy/i,
      });
      await user.click(confirmButton);

      expect(mockTokenDestroyFrozenFunds).toHaveBeenCalledWith(
        expect.objectContaining({
          frozenIdentityId: "frozen-victim-aabb",
        }),
      );
    });

    it("switches to manual input when Other is selected", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-victim-aabb"]);

      const trigger = screen.getByTestId("frozen-identity-select");
      await user.click(trigger);

      const otherOption = screen.getByText("Other (enter manually)");
      await user.click(otherOption);

      expect(
        screen.getByPlaceholderText("Enter frozen identity ID (Base58 or Hex)"),
      ).toBeInTheDocument();
    });

    it("shows Back to dropdown link in manual mode", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-victim-aabb"]);

      const trigger = screen.getByTestId("frozen-identity-select");
      await user.click(trigger);

      const otherOption = screen.getByText("Other (enter manually)");
      await user.click(otherOption);

      expect(screen.getByText("Back to dropdown")).toBeInTheDocument();
    });
  });

  // ── Fallback text input (no frozen identities) ────────────────────────

  describe("fallback text input", () => {
    it("shows text input when no frozen identities found", async () => {
      setup();
      await completeFrozenQuery([]);

      expect(
        screen.getByPlaceholderText("Enter frozen identity ID (Base58 or Hex)"),
      ).toBeInTheDocument();
    });

    it("shows informative message when no frozen identities found", async () => {
      setup();
      await completeFrozenQuery([]);

      expect(
        screen.getByText(
          /no frozen identities found among loaded identities/i,
        ),
      ).toBeInTheDocument();
    });

    it("enables destroy button when manual ID is entered", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const button = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      expect(button).toBeEnabled();
    });
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Destroy Frozen Funds action button", async () => {
      setup();
      await completeFrozenQuery([]);
      expect(
        screen.getByRole("button", { name: /destroy frozen funds/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("DestroyToken")).toBeInTheDocument();
    });

    it("shows identity selector for signing identity", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables destroy button when identity ID is empty", async () => {
      setup();
      await completeFrozenQuery([]);
      const button = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      expect(button).toBeDisabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("shows destructive confirmation dialog", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
      await user.type(input, "8vNLqz5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const destroyButton = screen.getByRole("button", {
        name: /destroy frozen funds/i,
      });
      await user.click(destroyButton);

      expect(
        screen.getByText("Confirm Destroy Frozen Funds"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/this action cannot be undone/i),
      ).toBeInTheDocument();
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
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
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
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
        emitTaskResult({
          taskId: "task-destroy-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(
        screen.getByText("Destroy Frozen Funds Successful"),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
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
        emitTaskError({
          taskId: "task-destroy-1",
          message: "Destroy operation failed",
          details: null,
          recoverable: false,
        });
      });

      expect(screen.getByText("Destroy operation failed")).toBeInTheDocument();
    });

    it("shows Destroy More button on success", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
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
        emitTaskResult({
          taskId: "task-destroy-1",
          resultType: "Token",
          payload: null,
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
