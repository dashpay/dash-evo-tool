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
import { TokenClaimScreen } from "./TokenClaimScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-claim-111",
  contractId: "contract-claim-111",
  tokenPosition: "0",
  name: "ClaimToken",
  balance: "50000000",
  decimals: "8",
  identityId: "id-claim-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenClaim = vi.fn();
const mockEstimatePerpetualRewards = vi.fn();

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

const mockIdentities = [
  {
    id: "id-claim-identity",
    alias: "ClaimIdentity",
    balance: 3000000000,
    keys: [
      {
        keyId: 5,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "claim-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-claim"],
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
          seedHash: "seed-hash-claim",
          alias: "ClaimWallet",
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

function fireTaskResult(payload: unknown) {
  const listeners = vi.mocked(events.taskResultEvent.listen).mock.calls;
  for (const [cb] of listeners) {
    cb?.({ payload } as { payload: unknown });
  }
}

function fireTaskError(payload: unknown) {
  const listeners = vi.mocked(events.taskErrorEvent.listen).mock.calls;
  for (const [cb] of listeners) {
    cb?.({ payload } as { payload: unknown });
  }
}

function setup(searchOverrides?: Partial<Record<string, string>>) {
  if (searchOverrides) {
    currentSearch = { ...currentSearch, ...searchOverrides };
  }
  return {
    user: userEvent.setup(),
    ...render(<TokenClaimScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenClaimScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentSearch = {
      tokenId: "token-claim-111",
      contractId: "contract-claim-111",
      tokenPosition: "0",
      name: "ClaimToken",
      balance: "50000000",
      decimals: "8",
      identityId: "id-claim-identity",
    };
    vi.mocked(commands.tokenClaim).mockImplementation(mockTokenClaim);
    vi.mocked(commands.tokenEstimatePerpetualRewards).mockImplementation(
      mockEstimatePerpetualRewards,
    );
    mockTokenClaim.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-claim-1" },
    });
    mockEstimatePerpetualRewards.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-estimate-1" },
    });
  });

  // ── Rendering ────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders token name from search params", () => {
      setup();
      expect(screen.getByText("ClaimToken")).toBeInTheDocument();
    });

    it("renders distribution type selector", () => {
      setup();
      expect(
        screen.getByTestId("distribution-type-select"),
      ).toBeInTheDocument();
    });

    it("renders Claim action button", () => {
      setup();
      expect(
        screen.getByTestId("operation-submit"),
      ).toBeInTheDocument();
    });

    it("renders signing identity selector", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("renders View Previous Claims link", () => {
      setup();
      expect(
        screen.getByTestId("view-claims-link"),
      ).toBeInTheDocument();
    });

    it("claim button is disabled when no distribution type selected", () => {
      setup();
      const submitBtn = screen.getByTestId("operation-submit");
      expect(submitBtn).toBeDisabled();
    });

    it("shows validation message when distribution type not selected", () => {
      setup();
      expect(
        screen.getByText(/please select a distribution type/i),
      ).toBeInTheDocument();
    });
  });

  // ── Distribution type selection ──────────────────────────────────────

  describe("distribution type selection", () => {
    it("enables claim button after selecting Perpetual", async () => {
      const { user } = setup();

      // Open select
      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);

      // Select Perpetual
      const option = screen.getByText("Perpetual");
      await user.click(option);

      const submitBtn = screen.getByTestId("operation-submit");
      expect(submitBtn).toBeEnabled();
    });

    it("shows perpetual info panel when Perpetual selected", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);

      const option = screen.getByText("Perpetual");
      await user.click(option);

      expect(screen.getByTestId("perpetual-info")).toBeInTheDocument();
      expect(
        screen.getByText(/understanding claim limitations/i),
      ).toBeInTheDocument();
    });

    it("shows estimate rewards button for perpetual", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      expect(
        screen.getByTestId("estimate-rewards-button"),
      ).toBeInTheDocument();
    });

    it("does not show perpetual info for Pre-Programmed", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Pre-Programmed"));

      expect(screen.queryByTestId("perpetual-info")).not.toBeInTheDocument();
    });
  });

  // ── Estimate rewards ─────────────────────────────────────────────────

  describe("estimate rewards", () => {
    it("calls tokenEstimatePerpetualRewards on estimate click", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      const estimateBtn = screen.getByTestId("estimate-rewards-button");
      await user.click(estimateBtn);

      expect(mockEstimatePerpetualRewards).toHaveBeenCalledWith({
        identityId: "id-claim-identity",
        tokenId: "token-claim-111",
      });
    });

    it("displays estimated rewards on result", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      const estimateBtn = screen.getByTestId("estimate-rewards-button");
      await user.click(estimateBtn);

      await act(async () => {
        fireTaskResult({
          taskId: "task-estimate-1",
          resultType: "Token",
          payload: "Estimated 500 tokens over 3 epochs",
        });
      });

      expect(screen.getByTestId("estimated-rewards")).toBeInTheDocument();
      expect(
        screen.getByText(/estimated 500 tokens over 3 epochs/i),
      ).toBeInTheDocument();
    });
  });

  // ── Submit ───────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenClaim with correct params after confirmation", async () => {
      const { user } = setup();

      // Select distribution type
      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      // Click claim button
      const submitBtn = screen.getByTestId("operation-submit");
      await user.click(submitBtn);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", {
        name: /claim/i,
      });
      await user.click(confirmBtn);

      expect(mockTokenClaim).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-claim-111",
            tokenPosition: 0,
          }),
          distributionType: "Perpetual",
        }),
      );
    });

    it("shows confirmation dialog with claim message", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      const submitBtn = screen.getByTestId("operation-submit");
      await user.click(submitBtn);

      expect(
        screen.getByText(/are you sure you want to claim tokens/i),
      ).toBeInTheDocument();
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      const submitBtn = screen.getByTestId("operation-submit");
      await user.click(submitBtn);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", {
        name: /claim/i,
      });
      await user.click(confirmBtn);

      expect(screen.getByText("Claim...")).toBeInTheDocument();
    });
  });

  // ── Result handling ──────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      const submitBtn = screen.getByTestId("operation-submit");
      await user.click(submitBtn);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", {
        name: /claim/i,
      });
      await user.click(confirmBtn);

      await act(async () => {
        fireTaskResult({
          taskId: "task-claim-1",
          resultType: "Token",
          payload: null,
        });
      });

      expect(screen.getByText("Claim Successful")).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();

      const selectTrigger = screen.getByTestId("distribution-type-select");
      await user.click(selectTrigger);
      await user.click(screen.getByText("Perpetual"));

      const submitBtn = screen.getByTestId("operation-submit");
      await user.click(submitBtn);

      const dialog = screen.getByRole("dialog");
      const confirmBtn = within(dialog).getByRole("button", {
        name: /claim/i,
      });
      await user.click(confirmBtn);

      await act(async () => {
        fireTaskError({
          taskId: "task-claim-1",
          message: "Claim failed: insufficient balance",
          details: null,
          recoverable: false,
        });
      });

      expect(
        screen.getByText("Claim failed: insufficient balance"),
      ).toBeInTheDocument();
    });
  });

  // ── Navigation ───────────────────────────────────────────────────────

  describe("navigation", () => {
    it("navigates to view claims when link clicked", async () => {
      const { user } = setup();

      const viewClaimsLink = screen.getByTestId("view-claims-link");
      await user.click(viewClaimsLink);

      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({
          to: "/tokens/view-claims",
        }),
      );
    });
  });
});
