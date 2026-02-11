import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenPurchaseScreen } from "./TokenPurchaseScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-purchase-111",
  contractId: "contract-purchase-111",
  tokenPosition: "0",
  name: "BuyToken",
  balance: "2000000000",
  decimals: "8",
  identityId: "id-purchase-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenPurchase = vi.fn();
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
    id: "id-purchase-identity",
    alias: "BuyerIdentity",
    balance: 5000000000,
    keys: [
      {
        keyId: 3,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "purchase-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-purchase"],
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
          seedHash: "seed-hash-purchase",
          alias: "BuyerWallet",
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

/** Emit to all registered task result listeners. */
function emitTaskResult(payload: unknown) {
  const calls = vi.mocked(events.taskResultEvent.listen).mock.calls;
  for (const [cb] of calls) {
    cb?.({ payload } as { payload: unknown });
  }
}

/** Emit to all registered task error listeners. */
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
    ...render(<TokenPurchaseScreen />),
  };
}

/** Simulate fetching pricing — clicks button and delivers result event. */
async function fetchPricingWithSinglePrice(
  user: ReturnType<typeof userEvent.setup>,
  pricePerSmallestUnit: number,
) {
  await user.click(screen.getByTestId("fetch-pricing-button"));

  await act(async () => {
    emitTaskResult({
      taskId: "task-pricing-1",
      resultType: "Token",
      payload: {
        token_id: "token-purchase-111",
        prices: { SinglePrice: pricePerSmallestUnit },
      },
    });
  });
}

async function fetchPricingWithTieredPricing(
  user: ReturnType<typeof userEvent.setup>,
  tiers: Record<string, number>,
) {
  await user.click(screen.getByTestId("fetch-pricing-button"));

  await act(async () => {
    emitTaskResult({
      taskId: "task-pricing-1",
      resultType: "Token",
      payload: {
        token_id: "token-purchase-111",
        prices: { SetPrices: tiers },
      },
    });
  });
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenPurchaseScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentSearch = {
      tokenId: "token-purchase-111",
      contractId: "contract-purchase-111",
      tokenPosition: "0",
      name: "BuyToken",
      balance: "2000000000",
      decimals: "8",
      identityId: "id-purchase-identity",
    };
    vi.mocked(commands.tokenPurchase).mockImplementation(mockTokenPurchase);
    vi.mocked(commands.tokenQueryPricing).mockImplementation(
      mockTokenQueryPricing,
    );
    mockTokenPurchase.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-purchase-1" },
    });
    mockTokenQueryPricing.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-pricing-1" },
    });
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Purchase action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /purchase/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("BuyToken")).toBeInTheDocument();
    });

    it("shows amount input field", () => {
      setup();
      expect(screen.getByTestId("purchase-amount-input")).toBeInTheDocument();
    });

    it("shows Fetch Token Price button", () => {
      setup();
      expect(screen.getByTestId("fetch-pricing-button")).toBeInTheDocument();
    });

    it("shows hint to fetch pricing initially", () => {
      setup();
      expect(screen.getByTestId("fetch-hint")).toBeInTheDocument();
    });

    it("shows identity selector for signing identity", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });
  });

  // ── Fetch pricing ─────────────────────────────────────────────────────

  describe("fetch pricing", () => {
    it("calls tokenQueryPricing when Fetch button is clicked", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("fetch-pricing-button"));

      expect(mockTokenQueryPricing).toHaveBeenCalledWith({
        tokenId: "token-purchase-111",
      });
    });

    it("shows fetching indicator while waiting", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("fetch-pricing-button"));

      expect(screen.getByTestId("fetching-indicator")).toBeInTheDocument();
    });

    it("displays single price after fetching", async () => {
      const { user } = setup();
      // SinglePrice: 1 means 1 credit per smallest unit
      // With 8 decimals, that's 1 * 10^8 credits per whole token = 1 DASH per token
      await fetchPricingWithSinglePrice(user, 1);

      expect(screen.getByTestId("pricing-display")).toBeInTheDocument();
      expect(screen.getByText(/fixed price/i)).toBeInTheDocument();
    });

    it("displays tiered pricing after fetching", async () => {
      const { user } = setup();
      // Tier: 100000000 (1 token) -> price 1, 1000000000 (10 tokens) -> price 0
      await fetchPricingWithTieredPricing(user, {
        "100000000": 1,
        "1000000000": 0,
      });

      expect(screen.getByTestId("pricing-display")).toBeInTheDocument();
      expect(screen.getByText(/tiered pricing/i)).toBeInTheDocument();
    });

    it("shows error when no pricing is set", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("fetch-pricing-button"));

      await act(async () => {
        emitTaskResult({
          taskId: "task-pricing-1",
          resultType: "Token",
          payload: {
            token_id: "token-purchase-111",
            prices: null,
          },
        });
      });

      const pricingError = screen.getByTestId("pricing-error");
      expect(pricingError).toBeInTheDocument();
      expect(pricingError.textContent).toMatch(/not available for direct purchase/i);
    });

    it("shows error on pricing fetch failure", async () => {
      const { user } = setup();
      await user.click(screen.getByTestId("fetch-pricing-button"));

      await act(async () => {
        emitTaskError({
          taskId: "task-pricing-1",
          message: "Network timeout",
          details: null,
          recoverable: false,
        });
      });

      const pricingError = screen.getByTestId("pricing-error");
      expect(pricingError).toBeInTheDocument();
      expect(pricingError.textContent).toMatch(/network timeout/i);
    });
  });

  // ── Price calculation ──────────────────────────────────────────────────

  describe("price calculation", () => {
    it("calculates total price for single price schedule", async () => {
      const { user } = setup();
      // SinglePrice: 1 = 1 credit per smallest unit
      // 8 decimals: 1 token = 100000000 smallest units
      // Total for 5 tokens = 5 * 100000000 * 1 = 500000000 credits = 5 DASH
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "5");

      expect(screen.getByTestId("calculated-price")).toBeInTheDocument();
      expect(screen.getByText(/500,000,000 credits/)).toBeInTheDocument();
    });

    it("calculates price using correct tier for tiered pricing", async () => {
      const { user } = setup();
      // Tiers: 1 token (100000000 units) → 2 credits/unit, 10 tokens → 1 credit/unit
      await fetchPricingWithTieredPricing(user, {
        "100000000": 2,
        "1000000000": 1,
      });

      // Enter 15 tokens -> should use the 10-token tier (1 credit/unit)
      // 15 * 10^8 smallest units * 1 = 1500000000 credits = 15 DASH
      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "15");

      expect(screen.getByTestId("calculated-price")).toBeInTheDocument();
      expect(screen.getByText(/1,500,000,000 credits/)).toBeInTheDocument();
    });

    it("does not show calculated price without amount", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      expect(
        screen.queryByTestId("calculated-price"),
      ).not.toBeInTheDocument();
    });

    it("does not show calculated price without pricing fetched", async () => {
      const { user } = setup();
      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "5");

      expect(
        screen.queryByTestId("calculated-price"),
      ).not.toBeInTheDocument();
    });
  });

  // ── Validation ─────────────────────────────────────────────────────────

  describe("validation", () => {
    it("disables Purchase button when no pricing is fetched", () => {
      setup();
      const button = screen.getByRole("button", { name: /purchase/i });
      expect(button).toBeDisabled();
    });

    it("disables Purchase button when amount is empty", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const button = screen.getByRole("button", { name: /purchase/i });
      expect(button).toBeDisabled();
    });

    it("enables Purchase button when pricing is fetched and amount is valid", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "3");

      const button = screen.getByRole("button", { name: /purchase/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenPurchase with correct params after confirmation", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "2");

      const purchaseButton = screen.getByRole("button", {
        name: /purchase/i,
      });
      await user.click(purchaseButton);

      // Confirm
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /purchase/i,
      });
      await user.click(confirmButton);

      expect(mockTokenPurchase).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-purchase-111",
            tokenPosition: 0,
          }),
          amount: "200000000", // 2 tokens * 10^8
          totalAgreedPrice: 200000000, // 200000000 smallest units * 1 credit/unit
        }),
      );
    });

    it("shows confirmation dialog with price details", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "1");

      const purchaseButton = screen.getByRole("button", {
        name: /purchase/i,
      });
      await user.click(purchaseButton);

      const dialog = screen.getByRole("dialog");
      expect(
        within(dialog).getByText(/purchase 1 tokens/i),
      ).toBeInTheDocument();
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "1");

      const purchaseButton = screen.getByRole("button", {
        name: /purchase/i,
      });
      await user.click(purchaseButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /purchase/i,
      });
      await user.click(confirmButton);

      expect(screen.getByText("Purchase...")).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "1");

      await user.click(screen.getByRole("button", { name: /purchase/i }));
      const dialog = screen.getByRole("dialog");
      await user.click(
        within(dialog).getByRole("button", { name: /purchase/i }),
      );

      await act(async () => {
        emitTaskResult({
          taskId: "task-purchase-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(
        screen.getByText("Purchase Successful"),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "1");

      await user.click(screen.getByRole("button", { name: /purchase/i }));
      const dialog = screen.getByRole("dialog");
      await user.click(
        within(dialog).getByRole("button", { name: /purchase/i }),
      );

      await act(async () => {
        emitTaskError({
          taskId: "task-purchase-1",
          message: "Insufficient balance",
          details: null,
          recoverable: false,
        });
      });

      expect(screen.getByText("Insufficient balance")).toBeInTheDocument();
    });

    it("shows Purchase More button on success", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 1);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "1");

      await user.click(screen.getByRole("button", { name: /purchase/i }));
      const dialog = screen.getByRole("dialog");
      await user.click(
        within(dialog).getByRole("button", { name: /purchase/i }),
      );

      await act(async () => {
        emitTaskResult({
          taskId: "task-purchase-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(
        screen.getByRole("button", { name: /purchase more/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Edge cases ─────────────────────────────────────────────────────────

  describe("edge cases", () => {
    it("handles zero decimals token correctly", async () => {
      const { user } = setup({ decimals: "0" });
      // SinglePrice: 5000000 = 0.05 DASH per token (with 0 decimals, unit = token)
      await fetchPricingWithSinglePrice(user, 5000000);

      const amountInput = screen.getByTestId("purchase-amount-input");
      await user.type(amountInput, "10");

      // 10 tokens * 5000000 credits/token = 50000000 credits = 0.5 DASH
      expect(screen.getByTestId("calculated-price")).toBeInTheDocument();
      expect(screen.getByText(/50,000,000 credits/)).toBeInTheDocument();
    });

    it("handles FREE pricing (price = 0)", async () => {
      const { user } = setup();
      await fetchPricingWithSinglePrice(user, 0);

      expect(screen.getByText(/free/i)).toBeInTheDocument();
    });
  });
});
