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

import { render, screen, within, act, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenUpdateConfigScreen } from "./TokenUpdateConfigScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-config-111",
  contractId: "contract-config-111",
  tokenPosition: "0",
  name: "ConfigToken",
  balance: "10000000",
  decimals: "8",
  identityId: "id-config-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenUpdateConfig = vi.fn();

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

const mockIdentities = [
  {
    id: "id-config-identity",
    alias: "ConfigIdentity",
    balance: 5000000000,
    keys: [
      {
        keyId: 7,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "config-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-config"],
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
          seedHash: "seed-hash-config",
          alias: "ConfigWallet",
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
    ...render(<TokenUpdateConfigScreen />),
  };
}

/**
 * Opens the change item selector, selects an option, and returns the user
 * event instance. We use a custom implementation because the shadcn Select
 * is built on Radix which uses portals.
 *
 * We target the option by role "option" since the group label text is the
 * same as some option labels (e.g., "Max Supply" group contains "Max Supply" item).
 */
async function selectChangeItem(
  user: ReturnType<typeof userEvent.setup>,
  label: string,
) {
  const trigger = screen.getByTestId("change-item-selector");
  await user.click(trigger);
  // Radix Select renders options with role="option"
  const option = await screen.findByRole("option", { name: label });
  await user.click(option);
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenUpdateConfigScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentSearch = {
      tokenId: "token-config-111",
      contractId: "contract-config-111",
      tokenPosition: "0",
      name: "ConfigToken",
      balance: "10000000",
      decimals: "8",
      identityId: "id-config-identity",
    };
    vi.mocked(commands.tokenUpdateConfig).mockImplementation(
      mockTokenUpdateConfig,
    );
    mockTokenUpdateConfig.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-config-1" },
    });
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Update Config action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /update config/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("ConfigToken")).toBeInTheDocument();
    });

    it("shows configuration item selector", () => {
      setup();
      expect(screen.getByTestId("change-item-selector")).toBeInTheDocument();
    });

    it("shows signing identity selector", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("does not show amount input", () => {
      setup();
      expect(screen.queryByLabelText(/^amount$/i)).not.toBeInTheDocument();
    });

    it("button is disabled by default when no change item is selected", () => {
      setup();
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeDisabled();
    });
  });

  // ── Change Item Selection ─────────────────────────────────────────────

  describe("change item selection", () => {
    it("shows MaxSupply input after selecting Max Supply", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");
      expect(screen.getByTestId("max-supply-input")).toBeInTheDocument();
    });

    it("shows checkbox for Minting Allow Choosing Destination", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Minting Allow Choosing Destination");
      expect(screen.getByTestId("minting-allow-choose")).toBeInTheDocument();
    });

    it("shows perpetual distribution warning", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Perpetual Distribution");
      expect(
        screen.getByTestId("perpetual-distribution-warning"),
      ).toBeInTheDocument();
    });

    it("shows conventions JSON editor for Conventions", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Conventions");
      expect(
        screen.getByTestId("conventions-json-editor"),
      ).toBeInTheDocument();
    });

    it("shows destination identity input for New Tokens Destination", async () => {
      const { user } = setup();
      await selectChangeItem(user, "New Tokens Destination");
      expect(
        screen.getByTestId("destination-identity-input"),
      ).toBeInTheDocument();
    });

    it("shows main control group input", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Main Control Group");
      expect(
        screen.getByTestId("main-control-group-input"),
      ).toBeInTheDocument();
    });

    it("shows action taker selector for Manual Minting", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Manual Minting");
      expect(screen.getByTestId("action-taker-selector")).toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables button when no change item is selected", () => {
      setup();
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeDisabled();
    });

    it("enables button when MaxSupply is selected (even empty value)", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeEnabled();
    });

    it("disables button when Perpetual Distribution is selected", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Perpetual Distribution");
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeDisabled();
    });

    it("enables button when Conventions has valid JSON", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Conventions");
      const editor = screen.getByTestId("conventions-json-editor");
      // Use fireEvent.change since userEvent.type treats { as a modifier key
      fireEvent.change(editor, {
        target: { value: '{"localizedName":"Test"}' },
      });
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeEnabled();
    });

    it("shows conventions error for invalid JSON", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Conventions");
      const editor = screen.getByTestId("conventions-json-editor");
      await user.type(editor, "not valid json");
      expect(screen.getByTestId("conventions-error")).toBeInTheDocument();
    });

    it("enables button for MintingAllowChoosingDestination", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Minting Allow Choosing Destination");
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenUpdateConfig with MaxSupply change item", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");
      const input = screen.getByTestId("max-supply-input");
      await user.type(input, "1000000");

      const button = screen.getByRole("button", { name: /update config/i });
      await user.click(button);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /update config/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUpdateConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          contractId: "contract-config-111",
          tokenId: "token-config-111",
          tokenPosition: 0,
          changeItemJson: { MaxSupply: 1000000 },
        }),
      );
    });

    it("calls tokenUpdateConfig with MintingAllowChoosingDestination", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Minting Allow Choosing Destination");
      const checkbox = screen.getByTestId("minting-allow-choose");
      await user.click(checkbox);

      const button = screen.getByRole("button", { name: /update config/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /update config/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUpdateConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          changeItemJson: { MintingAllowChoosingDestination: true },
        }),
      );
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");

      const button = screen.getByRole("button", { name: /update config/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /update config/i,
      });
      await user.click(confirmButton);

      expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();
    });
  });

  // ── Result Handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");

      const button = screen.getByRole("button", { name: /update config/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /update config/i,
      });
      await user.click(confirmButton);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-config-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByText("Update Config Successful"),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");

      const button = screen.getByRole("button", { name: /update config/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /update config/i,
      });
      await user.click(confirmButton);

      const listener = vi.mocked(events.taskErrorEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-config-1",
            message: "Config update failed",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(screen.getByText("Config update failed")).toBeInTheDocument();
    });

    it("shows Update Another Config button on success", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Max Supply");

      const button = screen.getByRole("button", { name: /update config/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /update config/i,
      });
      await user.click(confirmButton);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-config-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByRole("button", { name: /update another config/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Group Action ──────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows read-only change item when group signing", () => {
      setup({
        groupActionId: "group-action-abc",
        details: JSON.stringify({ changeItemType: "MaxSupply" }),
      });
      // Should show as read-only text (in the config item display), not a selector
      expect(screen.getByText("Config item:")).toBeInTheDocument();
      // The selector trigger should not be present
      expect(
        screen.queryByTestId("change-item-selector"),
      ).not.toBeInTheDocument();
    });

    it("sends group info when group signing", async () => {
      const { user } = setup({
        groupActionId: "group-action-abc",
        details: JSON.stringify({ changeItemType: "MaxSupply" }),
      });

      // MaxSupply should be auto-selected and editor visible
      expect(screen.getByTestId("max-supply-input")).toBeInTheDocument();

      const button = screen.getByRole("button", { name: /sign update/i });
      await user.click(button);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /sign update/i,
      });
      await user.click(confirmButton);

      expect(mockTokenUpdateConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          groupInfo: expect.objectContaining({
            type: "other_signer",
            action_id: "group-action-abc",
          }),
        }),
      );
    });
  });

  // ── AuthorizedActionTakers Editor ─────────────────────────────────────

  describe("authorized action takers editor", () => {
    it("shows action taker selector for Freeze", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Freeze");
      expect(screen.getByTestId("action-taker-selector")).toBeInTheDocument();
    });

    it("shows identity input when Identity taker is selected", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Manual Minting");

      // Change action taker to Identity
      const aatSelector = screen.getByTestId("action-taker-selector");
      await user.click(aatSelector);
      // Use role="option" to disambiguate from "Identity" label in the form
      const identityOption = await screen.findByRole("option", {
        name: "Identity",
      });
      await user.click(identityOption);

      expect(screen.getByTestId("aat-identity-input")).toBeInTheDocument();
    });

    it("shows group input when Group taker is selected", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Manual Burning");

      // Change action taker to Group
      const aatSelector = screen.getByTestId("action-taker-selector");
      await user.click(aatSelector);
      const groupOption = await screen.findByText("Group");
      await user.click(groupOption);

      expect(screen.getByTestId("aat-group-input")).toBeInTheDocument();
    });
  });

  // ── Main Control Group ────────────────────────────────────────────────

  describe("main control group", () => {
    it("renders group position input", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Main Control Group");
      expect(
        screen.getByTestId("main-control-group-input"),
      ).toBeInTheDocument();
    });

    it("enables button for Main Control Group", async () => {
      const { user } = setup();
      await selectChangeItem(user, "Main Control Group");
      const button = screen.getByRole("button", { name: /update config/i });
      expect(button).toBeEnabled();
    });
  });
});
