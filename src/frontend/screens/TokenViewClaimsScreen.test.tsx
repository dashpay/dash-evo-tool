import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenViewClaimsScreen } from "./TokenViewClaimsScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-vc-111",
  contractId: "contract-vc-111",
  tokenPosition: "0",
  name: "ViewClaimsToken",
  balance: "80000000",
  decimals: "8",
  identityId: "id-vc-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenQueryClaims = vi.fn();

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

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
    ...render(<TokenViewClaimsScreen />),
  };
}

const sampleClaims = {
  documents: [
    {
      amount: 1000,
      $createdAt: 1700000000000,
      $createdAtBlockHeight: 12345,
      note: "First claim",
    },
    {
      amount: 2500,
      $createdAt: 1700100000000,
      $createdAtBlockHeight: 12400,
      note: "",
    },
    {
      amount: 500,
      $createdAt: 1700200000000,
      $createdAtBlockHeight: 12500,
      note: "Third claim note",
    },
  ],
  hasMore: false,
};

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenViewClaimsScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentSearch = {
      tokenId: "token-vc-111",
      contractId: "contract-vc-111",
      tokenPosition: "0",
      name: "ViewClaimsToken",
      balance: "80000000",
      decimals: "8",
      identityId: "id-vc-identity",
    };
    vi.mocked(commands.tokenQueryClaims).mockImplementation(
      mockTokenQueryClaims,
    );
    mockTokenQueryClaims.mockResolvedValue({
      status: "ok",
      data: { taskId: "task-vc-1" },
    });
  });

  // ── Rendering ────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders view claims screen", () => {
      setup();
      expect(screen.getByTestId("view-claims-screen")).toBeInTheDocument();
    });

    it("displays token name", () => {
      setup();
      expect(screen.getByText("ViewClaimsToken")).toBeInTheDocument();
    });

    it("renders Fetch Claims button", () => {
      setup();
      expect(screen.getByTestId("fetch-claims-button")).toBeInTheDocument();
    });

    it("renders Claim Tokens button", () => {
      setup();
      expect(screen.getByTestId("claim-tokens-button")).toBeInTheDocument();
    });

    it("renders Back to Tokens button", () => {
      setup();
      expect(screen.getByTestId("back-to-tokens")).toBeInTheDocument();
    });

    it("renders Refresh button", () => {
      setup();
      expect(screen.getByTestId("refresh-claims-button")).toBeInTheDocument();
    });

    it("shows initial state message", () => {
      setup();
      expect(screen.getByTestId("claims-initial")).toBeInTheDocument();
    });

    it("displays token header with name", () => {
      setup({ name: "MySpecialToken" });
      expect(screen.getByText("MySpecialToken")).toBeInTheDocument();
    });
  });

  // ── Fetch claims ─────────────────────────────────────────────────────

  describe("fetch claims", () => {
    it("calls tokenQueryClaims on Fetch click", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      expect(mockTokenQueryClaims).toHaveBeenCalledWith({
        tokenId: "token-vc-111",
        recipientId: "id-vc-identity",
      });
    });

    it("shows fetching state with elapsed time", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      expect(screen.getByText(/fetching/i)).toBeInTheDocument();
    });

    it("disables fetch button while fetching", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      expect(fetchBtn).toBeDisabled();
    });

    it("shows claims table on successful fetch", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            result: { type: "documentPage", ...sampleClaims },
          },
        });
      });

      expect(screen.getByTestId("claims-table")).toBeInTheDocument();
    });

    it("displays correct claim data in table", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            result: { type: "documentPage", ...sampleClaims },
          },
        });
      });

      // Check amount
      expect(screen.getByText("1000")).toBeInTheDocument();
      expect(screen.getByText("2500")).toBeInTheDocument();
      expect(screen.getByText("500")).toBeInTheDocument();

      // Check note
      expect(screen.getByText("First claim")).toBeInTheDocument();
      expect(screen.getByText("Third claim note")).toBeInTheDocument();

      // Check block heights
      expect(screen.getByText("12345")).toBeInTheDocument();
      expect(screen.getByText("12400")).toBeInTheDocument();
      expect(screen.getByText("12500")).toBeInTheDocument();
    });

    it("shows 3 rows for 3 claims", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            result: { type: "documentPage", ...sampleClaims },
          },
        });
      });

      expect(screen.getByTestId("claim-row-0")).toBeInTheDocument();
      expect(screen.getByTestId("claim-row-1")).toBeInTheDocument();
      expect(screen.getByTestId("claim-row-2")).toBeInTheDocument();
    });

    it("shows success message with count", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            result: { type: "documentPage", ...sampleClaims },
          },
        });
      });

      expect(screen.getByTestId("claims-message")).toHaveTextContent(
        "Found 3 claims",
      );
    });

    it("shows empty state when no claims found", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            result: { type: "documentPage", documents: [], hasMore: false },
          },
        });
      });

      expect(screen.getByTestId("claims-empty")).toBeInTheDocument();
      // Heading in empty state
      expect(screen.getByText("No Claims Found")).toBeInTheDocument();
    });

    it("shows error message on fetch error", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskErrorEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            message: "Network timeout fetching claims",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(screen.getByTestId("claims-message")).toHaveTextContent(
        "Network timeout fetching claims",
      );
    });
  });

  // ── Refresh ──────────────────────────────────────────────────────────

  describe("refresh", () => {
    it("calls tokenQueryClaims on Refresh click", async () => {
      const { user } = setup();

      const refreshBtn = screen.getByTestId("refresh-claims-button");
      await user.click(refreshBtn);

      expect(mockTokenQueryClaims).toHaveBeenCalledWith({
        tokenId: "token-vc-111",
        recipientId: "id-vc-identity",
      });
    });
  });

  // ── Navigation ───────────────────────────────────────────────────────

  describe("navigation", () => {
    it("navigates to tokens on back click", async () => {
      const { user } = setup();

      const backBtn = screen.getByTestId("back-to-tokens");
      await user.click(backBtn);

      expect(mockNavigate).toHaveBeenCalledWith({ to: "/tokens" });
    });

    it("navigates to claim screen on Claim Tokens click", async () => {
      const { user } = setup();

      const claimBtn = screen.getByTestId("claim-tokens-button");
      await user.click(claimBtn);

      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({
          to: "/tokens/claim",
        }),
      );
    });
  });

  // ── Edge cases ───────────────────────────────────────────────────────

  describe("edge cases", () => {
    it("handles unnamed token gracefully", () => {
      setup({ name: "" });
      expect(screen.getByText("Unnamed Token")).toBeInTheDocument();
    });

    it("handles claims with missing note", async () => {
      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      const listener = vi.mocked(events.taskResultEvent.listen).mock
        .calls[0]?.[0];
      await act(async () => {
        listener?.({
          payload: {
            taskId: "task-vc-1",
            result: {
              type: "documentPage",
              documents: [
                { amount: 100, $createdAt: 1700000000000, $createdAtBlockHeight: 1 },
              ],
              hasMore: false,
            },
          },
        });
      });

      expect(screen.getByTestId("claim-row-0")).toBeInTheDocument();
    });

    it("handles IPC error on fetch", async () => {
      mockTokenQueryClaims.mockResolvedValueOnce({
        status: "error",
        error: "Contract not loaded",
      });

      const { user } = setup();

      const fetchBtn = screen.getByTestId("fetch-claims-button");
      await user.click(fetchBtn);

      expect(screen.getByTestId("claims-message")).toHaveTextContent(
        "Contract not loaded",
      );
    });
  });
});
