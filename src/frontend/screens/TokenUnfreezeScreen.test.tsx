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

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

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
  {
    id: "frozen-target-aabb",
    alias: "FrozenAlice",
    balance: 500000,
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

describe("TokenUnfreezeScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(commands.tokenUnfreeze).mockImplementation(mockTokenUnfreeze);
    vi.mocked(commands.tokenQueryFrozenIdentities).mockResolvedValue({
      status: "ok",
      data: { taskId: "frozen-query-1" },
    });
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
      await completeFrozenQuery(["frozen-target-aabb"]);

      expect(
        screen.getByTestId("unfreeze-identity-select"),
      ).toBeInTheDocument();
    });

    it("enables unfreeze button when a frozen identity is selected", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-target-aabb"]);

      // Open select and pick the frozen identity
      const trigger = screen.getByTestId("unfreeze-identity-select");
      await user.click(trigger);

      const option = screen.getByText("FrozenAlice");
      await user.click(option);

      const button = screen.getByRole("button", { name: /unfreeze/i });
      expect(button).toBeEnabled();
    });

    it("calls tokenUnfreeze with selected frozen identity ID", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-target-aabb"]);

      // Select frozen identity
      const trigger = screen.getByTestId("unfreeze-identity-select");
      await user.click(trigger);
      const option = screen.getByText("FrozenAlice");
      await user.click(option);

      // Click unfreeze
      const unfreezeButton = screen.getByRole("button", { name: /unfreeze/i });
      await user.click(unfreezeButton);

      // Confirm
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUnfreeze).toHaveBeenCalledWith(
        expect.objectContaining({
          unfreezeIdentityId: "frozen-target-aabb",
        }),
      );
    });

    it("switches to manual input when Other is selected", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-target-aabb"]);

      const trigger = screen.getByTestId("unfreeze-identity-select");
      await user.click(trigger);

      const otherOption = screen.getByText("Other (enter manually)");
      await user.click(otherOption);

      // Should now show text input
      expect(
        screen.getByPlaceholderText("Enter frozen identity ID (Base58 or Hex)"),
      ).toBeInTheDocument();
    });

    it("shows Back to dropdown link in manual mode", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-target-aabb"]);

      const trigger = screen.getByTestId("unfreeze-identity-select");
      await user.click(trigger);

      const otherOption = screen.getByText("Other (enter manually)");
      await user.click(otherOption);

      expect(screen.getByText("Back to dropdown")).toBeInTheDocument();
    });

    it("switches back to dropdown when Back to dropdown is clicked", async () => {
      const { user } = setup();
      await completeFrozenQuery(["frozen-target-aabb"]);

      // Switch to manual
      const trigger = screen.getByTestId("unfreeze-identity-select");
      await user.click(trigger);
      const otherOption = screen.getByText("Other (enter manually)");
      await user.click(otherOption);

      // Switch back
      const backLink = screen.getByText("Back to dropdown");
      await user.click(backLink);

      expect(
        screen.getByTestId("unfreeze-identity-select"),
      ).toBeInTheDocument();
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

    it("enables unfreeze button when manual ID is entered", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const button = screen.getByRole("button", { name: /unfreeze/i });
      expect(button).toBeEnabled();
    });

    it("calls tokenUnfreeze with manually entered ID", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
      await user.type(input, "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq");

      const unfreezeButton = screen.getByRole("button", { name: /unfreeze/i });
      await user.click(unfreezeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /unfreeze/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUnfreeze).toHaveBeenCalledWith(
        expect.objectContaining({
          unfreezeIdentityId:
            "5A5g3y5kAEz1NTXF2fSm5BNfHv3Dp2yHQdwrpXNDTGWq",
        }),
      );
    });
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Unfreeze action button", async () => {
      setup();
      await completeFrozenQuery([]);
      expect(
        screen.getByRole("button", { name: /unfreeze/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("UnfreezeToken")).toBeInTheDocument();
    });

    it("shows identity selector for signing identity", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables unfreeze button when identity ID is empty", async () => {
      setup();
      await completeFrozenQuery([]);
      const button = screen.getByRole("button", { name: /unfreeze/i });
      expect(button).toBeDisabled();
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
        emitTaskResult({
          taskId: "task-unfreeze-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(screen.getByText("Unfreeze Successful")).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
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
        emitTaskError({
          taskId: "task-unfreeze-1",
          message: "Unfreeze failed",
          details: null,
          recoverable: false,
        });
      });

      expect(screen.getByText("Unfreeze failed")).toBeInTheDocument();
    });

    it("shows Unfreeze Another button on success", async () => {
      const { user } = setup();
      await completeFrozenQuery([]);

      const input = screen.getByPlaceholderText(
        "Enter frozen identity ID (Base58 or Hex)",
      );
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
        emitTaskResult({
          taskId: "task-unfreeze-1",
          resultType: "Token",
          payload: null,
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
