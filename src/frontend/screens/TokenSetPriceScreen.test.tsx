import { render, screen, within, act, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenSetPriceScreen } from "./TokenSetPriceScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-setprice-111",
  contractId: "contract-setprice-111",
  tokenPosition: "0",
  name: "PriceToken",
  balance: "5000000000",
  decimals: "8",
  identityId: "id-setprice-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenSetDirectPurchasePrice = vi.fn();
const mockTokenQueryPricing = vi.fn();

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

const mockIdentities = [
  {
    id: "id-setprice-identity",
    alias: "PriceIdentity",
    balance: 3000000000,
    keys: [
      {
        keyId: 7,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "setprice-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-setprice"],
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
          seedHash: "seed-hash-setprice",
          alias: "PriceWallet",
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

/** Emit a task result event to all registered listeners. */
function emitTaskResult(payload: unknown) {
  const calls = vi.mocked(events.taskResultEvent.listen).mock.calls;
  for (const [cb] of calls) {
    cb?.({ payload } as { payload: unknown });
  }
}

/** Emit a task error event to all registered listeners. */
function emitTaskError(payload: unknown) {
  const calls = vi.mocked(events.taskErrorEvent.listen).mock.calls;
  for (const [cb] of calls) {
    cb?.({ payload } as { payload: unknown });
  }
}

function setup(searchOverrides?: Partial<Record<string, string>>) {
  if (searchOverrides) {
    currentSearch = { ...currentSearch, ...searchOverrides };
  }
  return {
    user: userEvent.setup(),
    ...render(<TokenSetPriceScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenSetPriceScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentSearch = {
      tokenId: "token-setprice-111",
      contractId: "contract-setprice-111",
      tokenPosition: "0",
      name: "PriceToken",
      balance: "5000000000",
      decimals: "8",
      identityId: "id-setprice-identity",
    };
    vi.mocked(commands.tokenSetDirectPurchasePrice).mockImplementation(
      mockTokenSetDirectPurchasePrice,
    );
    vi.mocked(commands.tokenQueryPricing).mockImplementation(
      mockTokenQueryPricing,
    );
    mockTokenSetDirectPurchasePrice.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-setprice-1" },
    });
    mockTokenQueryPricing.mockResolvedValue({
      status: "ok",
      data: { taskId: "pricing-query-1" },
    });
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Set Price action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /set price/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("PriceToken")).toBeInTheDocument();
    });

    it("shows pricing type buttons", () => {
      setup();
      expect(screen.getByTestId("pricing-type-buttons")).toBeInTheDocument();
      expect(screen.getByTestId("pricing-type-single")).toBeInTheDocument();
      expect(screen.getByTestId("pricing-type-tiered")).toBeInTheDocument();
      expect(screen.getByTestId("pricing-type-remove")).toBeInTheDocument();
    });

    it("shows identity selector for signing identity", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("defaults to remove pricing type", () => {
      setup();
      expect(screen.getByTestId("remove-pricing-warning")).toBeInTheDocument();
    });

    it("shows remove pricing warning by default", () => {
      setup();
      expect(
        screen.getByText(/will remove the pricing schedule/i),
      ).toBeInTheDocument();
    });
  });

  // ── Current pricing display ────────────────────────────────────────────

  describe("current pricing display", () => {
    it("fetches current pricing on mount", async () => {
      setup();
      await waitFor(() => {
        expect(mockTokenQueryPricing).toHaveBeenCalledWith({
          tokenId: "token-setprice-111",
        });
      });
    });

    it("shows loading state while fetching pricing", async () => {
      // Use a never-resolving promise to keep loading state visible
      mockTokenQueryPricing.mockReturnValueOnce(new Promise(() => {}));
      setup();

      await waitFor(() => {
        expect(screen.getByTestId("current-pricing-loading")).toBeInTheDocument();
      });
      expect(screen.getByText(/fetching current pricing/i)).toBeInTheDocument();
    });

    it("displays single price when fetched", async () => {
      setup();

      await waitFor(() => {
        expect(mockTokenQueryPricing).toHaveBeenCalled();
        expect(
          vi.mocked(events.taskResultEvent.listen).mock.calls.length,
        ).toBeGreaterThan(0);
      });

      await act(async () => {
        emitTaskResult({
            taskId: "pricing-query-1",
            resultType: "Token",
            payload: {
              token_id: "token-setprice-111",
              prices: { SinglePrice: 1 },
            },
        });
      });

      await waitFor(() => {
        expect(screen.getByTestId("current-pricing-display")).toBeInTheDocument();
      });
      // SinglePrice: 1 (per smallest unit), with 8 decimals → 1 * 10^8 = 100000000 credits = 1 DASH
      expect(screen.getByText(/fixed price: 1 DASH per token/i)).toBeInTheDocument();
    });

    it("displays tiered pricing when fetched", async () => {
      setup();

      await waitFor(() => {
        expect(mockTokenQueryPricing).toHaveBeenCalled();
        expect(
          vi.mocked(events.taskResultEvent.listen).mock.calls.length,
        ).toBeGreaterThan(0);
      });

      await act(async () => {
        emitTaskResult({
          taskId: "pricing-query-1",
          resultType: "Token",
          payload: {
            token_id: "token-setprice-111",
            prices: {
              SetPrices: {
                "100000000": 1,
                "1000000000": 0,
              },
            },
          },
        });
      });

      await waitFor(() => {
        expect(screen.getByTestId("current-pricing-display")).toBeInTheDocument();
      });
      const pricingDisplay = screen.getByTestId("current-pricing-display");
      expect(within(pricingDisplay).getByText(/tiered pricing/i)).toBeInTheDocument();
      // 100000000 smallest units / 10^8 = 1 token, price 1 per smallest = 1 * 10^8 = 100000000 credits = 1 DASH
      expect(within(pricingDisplay).getByText(/1\+ tokens: 1 DASH each/i)).toBeInTheDocument();
      // 1000000000 smallest units / 10^8 = 10 tokens, price 0 = FREE
      expect(within(pricingDisplay).getByText(/10\+ tokens: FREE/i)).toBeInTheDocument();
    });

    it("shows 'no pricing' message when token has no pricing set", async () => {
      setup();

      await waitFor(() => {
        expect(mockTokenQueryPricing).toHaveBeenCalled();
        expect(
          vi.mocked(events.taskResultEvent.listen).mock.calls.length,
        ).toBeGreaterThan(0);
      });

      await act(async () => {
        emitTaskResult({
          taskId: "pricing-query-1",
          resultType: "Token",
          payload: {
            token_id: "token-setprice-111",
            prices: null,
          },
        });
      });

      await waitFor(() => {
        expect(screen.getByTestId("current-pricing-none")).toBeInTheDocument();
      });
      expect(
        screen.getByText(/no pricing schedule is currently set/i),
      ).toBeInTheDocument();
    });

    it("shows error when pricing fetch fails", async () => {
      setup();

      await waitFor(() => {
        expect(mockTokenQueryPricing).toHaveBeenCalled();
        expect(
          vi.mocked(events.taskErrorEvent.listen).mock.calls.length,
        ).toBeGreaterThan(0);
      });

      await act(async () => {
        emitTaskError({
          taskId: "pricing-query-1",
          message: "Network error",
          details: null,
          recoverable: false,
        });
      });

      await waitFor(() => {
        expect(screen.getByTestId("current-pricing-error")).toBeInTheDocument();
      });
      expect(screen.getByText("Network error")).toBeInTheDocument();
    });

    it("does not fetch pricing when group signing", () => {
      setup({
        groupActionId: "group-action-456",
        details: JSON.stringify({
          pricingSchedule: { SinglePrice: 100 },
        }),
      });

      expect(mockTokenQueryPricing).not.toHaveBeenCalled();
    });

    it("shows current pricing section", () => {
      setup();
      expect(screen.getByTestId("current-pricing-section")).toBeInTheDocument();
    });
  });

  // ── Pricing type selection ─────────────────────────────────────────────

  describe("pricing type selection", () => {
    it("shows single price input when Single Price is selected", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-single"));

      expect(screen.getByTestId("single-price-section")).toBeInTheDocument();
      expect(screen.getByTestId("single-price-input")).toBeInTheDocument();
    });

    it("shows tiered pricing section when Tiered Pricing is selected", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-tiered"));

      expect(
        screen.getByTestId("tiered-pricing-section"),
      ).toBeInTheDocument();
      expect(screen.getByTestId("add-tier-button")).toBeInTheDocument();
    });

    it("shows remove pricing warning when Remove is selected", async () => {
      const { user } = setup();
      // Start on single price
      await user.click(screen.getByTestId("pricing-type-single"));
      // Switch to remove
      await user.click(screen.getByTestId("pricing-type-remove"));

      expect(screen.getByTestId("remove-pricing-warning")).toBeInTheDocument();
    });
  });

  // ── Single Price ───────────────────────────────────────────────────────

  describe("single price", () => {
    it("enables Set Price when valid price is entered", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-single"));

      const priceInput = screen.getByTestId("single-price-input");
      await user.clear(priceInput);
      await user.type(priceInput, "1.0");

      const setButton = screen.getByRole("button", { name: /set price/i });
      expect(setButton).toBeEnabled();
    });

    it("disables Set Price when no price is entered", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-single"));

      const setButton = screen.getByRole("button", { name: /set price/i });
      expect(setButton).toBeDisabled();
    });

    it("shows price preview for valid single price", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-single"));

      const priceInput = screen.getByTestId("single-price-input");
      await user.clear(priceInput);
      await user.type(priceInput, "1.0");

      expect(screen.getByTestId("price-preview")).toBeInTheDocument();
      expect(screen.getByText(/100000000 credits/)).toBeInTheDocument();
    });
  });

  // ── Tiered Pricing ─────────────────────────────────────────────────────

  describe("tiered pricing", () => {
    it("starts with one tier row (amount=1)", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-tiered"));

      expect(screen.getByTestId("tier-row-0")).toBeInTheDocument();
      const firstAmountInput = screen.getByTestId("tier-amount-0");
      expect(firstAmountInput).toHaveValue(1);
      // First tier's amount is disabled (always 1)
      expect(firstAmountInput).toBeDisabled();
    });

    it("adds a new tier row when Add Tier is clicked", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-tiered"));

      await user.click(screen.getByTestId("add-tier-button"));
      expect(screen.getByTestId("tier-row-1")).toBeInTheDocument();
    });

    it("removes a tier row when X is clicked (not first)", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-tiered"));

      // Add two extra tiers
      await user.click(screen.getByTestId("add-tier-button"));
      await user.click(screen.getByTestId("add-tier-button"));

      expect(screen.getByTestId("tier-row-2")).toBeInTheDocument();

      // Remove the third tier (index 2)
      await user.click(screen.getByTestId("tier-remove-2"));
      expect(screen.queryByTestId("tier-row-2")).not.toBeInTheDocument();
    });

    it("first tier remove button is disabled", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-tiered"));

      const removeBtn = screen.getByTestId("tier-remove-0");
      expect(removeBtn).toBeDisabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenSetDirectPurchasePrice with null schedule for remove pricing", async () => {
      const { user } = setup();
      // Default is "remove"

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /confirm/i,
      });
      await user.click(confirmButton);

      expect(mockTokenSetDirectPurchasePrice).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-setprice-111",
            tokenPosition: 0,
          }),
          tokenPricingSchedule: null,
        }),
      );
    });

    it("calls tokenSetDirectPurchasePrice with SinglePrice for single pricing", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("pricing-type-single"));

      const priceInput = screen.getByTestId("single-price-input");
      await user.clear(priceInput);
      await user.type(priceInput, "1.0");

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /confirm/i,
      });
      await user.click(confirmButton);

      expect(mockTokenSetDirectPurchasePrice).toHaveBeenCalledWith(
        expect.objectContaining({
          tokenPricingSchedule: { SinglePrice: 1 },
        }),
      );
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /confirm/i,
      });
      await user.click(confirmButton);

      expect(screen.getByTestId("operation-broadcasting")).toBeInTheDocument();
    });

    it("shows destructive confirmation dialog for remove pricing", async () => {
      const { user } = setup();

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      const dialog = screen.getByRole("dialog");
      expect(
        within(dialog).getByText(/remove the pricing schedule/i),
      ).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /confirm/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        emitTaskResult({
          taskId: "task-setprice-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(
        screen.getByText("Set Price Successful"),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /confirm/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        emitTaskError({
          taskId: "task-setprice-1",
          message: "Set price failed: permission denied",
          details: null,
          recoverable: false,
        });
      });

      expect(
        screen.getByText("Set price failed: permission denied"),
      ).toBeInTheDocument();
    });

    it("shows Set Price Again button on success", async () => {
      const { user } = setup();

      const setButton = screen.getByRole("button", { name: /set price/i });
      await user.click(setButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /confirm/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        emitTaskResult({
          taskId: "task-setprice-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(
        screen.getByRole("button", { name: /set price again/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Group action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows read-only schedule when group signing", () => {
      setup({
        groupActionId: "group-action-456",
        details: JSON.stringify({
          pricingSchedule: { SinglePrice: 100 },
        }),
      });

      expect(
        screen.getByText(/pricing schedule is pre-determined/i),
      ).toBeInTheDocument();
      expect(
        screen.getByTestId("group-schedule-display"),
      ).toBeInTheDocument();
      // Should not show the pricing selector
      expect(
        screen.queryByTestId("pricing-type-buttons"),
      ).not.toBeInTheDocument();
    });

    it("passes group info in IPC call when group signing", async () => {
      const { user } = setup({
        groupActionId: "group-action-456",
        details: JSON.stringify({
          pricingSchedule: { SinglePrice: 100 },
        }),
      });

      // Button text says "Sign Set Price" for groups
      const signButton = screen.getByRole("button", {
        name: /sign set price/i,
      });
      await user.click(signButton);

      // Confirm
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /sign set price/i,
      });
      await user.click(confirmButton);

      expect(mockTokenSetDirectPurchasePrice).toHaveBeenCalledWith(
        expect.objectContaining({
          groupInfo: expect.objectContaining({
            type: "other_signer",
            action_id: "group-action-456",
          }),
        }),
      );
    });
  });
});
