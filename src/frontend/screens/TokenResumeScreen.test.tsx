import { render, screen, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { TokenResumeScreen } from "./TokenResumeScreen";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let currentSearch: Record<string, string> = {
  tokenId: "token-resume-222",
  contractId: "contract-resume-222",
  tokenPosition: "0",
  name: "ResumeToken",
  balance: "5000000",
  decimals: "8",
  identityId: "id-resume-identity",
};

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: (opts: { select: (s: unknown) => unknown }) =>
    opts.select({ location: { search: currentSearch } }),
}));

const mockTokenResume = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-resume-1" },
});

let mockTaskResultListener: ((event: { payload: unknown }) => void) | null =
  null;
let mockTaskErrorListener: ((event: { payload: unknown }) => void) | null =
  null;

vi.mock("@/bindings", () => ({
  commands: {
    tokenResume: (...args: unknown[]) => mockTokenResume(...args),
    walletNotifyUnlocked: vi.fn().mockResolvedValue({ status: "ok" }),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockImplementation((cb) => {
        mockTaskResultListener = cb;
        return Promise.resolve(() => {
          mockTaskResultListener = null;
        });
      }),
    },
    taskErrorEvent: {
      listen: vi.fn().mockImplementation((cb) => {
        mockTaskErrorListener = cb;
        return Promise.resolve(() => {
          mockTaskErrorListener = null;
        });
      }),
    },
  },
}));

const mockIdentities = [
  {
    id: "id-resume-identity",
    alias: "ResumeIdentity",
    balance: 3000000000,
    keys: [
      {
        keyId: 7,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
        keyType: "ECDSA_SECP256K1",
        data: "resume-key-data",
        isDisabled: false,
        disabledAt: null,
        hasPrivateKey: true,
      },
    ],
    dpnsNames: [],
    associatedWalletHashes: ["seed-hash-resume"],
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
        seedHash: "seed-hash-resume",
        alias: "ResumeWallet",
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
    ...render(<TokenResumeScreen />),
  };
}

// ─── Tests ───────────────────────────────────────────────────────────────────

describe("TokenResumeScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockTaskResultListener = null;
    mockTaskErrorListener = null;
    currentSearch = {
      tokenId: "token-resume-222",
      contractId: "contract-resume-222",
      tokenPosition: "0",
      name: "ResumeToken",
      balance: "5000000",
      decimals: "8",
      identityId: "id-resume-identity",
    };
  });

  // ── Rendering ──────────────────────────────────────────────────────────

  describe("rendering", () => {
    it("renders with Resume action button", () => {
      setup();
      expect(
        screen.getByRole("button", { name: /resume/i }),
      ).toBeInTheDocument();
    });

    it("displays token name from search params", () => {
      setup();
      expect(screen.getByText("ResumeToken")).toBeInTheDocument();
    });

    it("shows signing identity selector", () => {
      setup();
      expect(screen.getByText(/signing identity/i)).toBeInTheDocument();
    });

    it("does not show amount input (resume has no amount)", () => {
      setup();
      expect(screen.queryByText(/amount/i)).not.toBeInTheDocument();
    });

    it("does not show recipient input (resume has no recipient)", () => {
      setup();
      expect(screen.queryByText(/recipient/i)).not.toBeInTheDocument();
    });

    it("resume button is enabled by default (no additional input needed)", () => {
      setup();
      const button = screen.getByRole("button", { name: /resume/i });
      expect(button).toBeEnabled();
    });
  });

  // ── Submit ─────────────────────────────────────────────────────────────

  describe("submit", () => {
    it("calls tokenResume with correct params after confirmation", async () => {
      const { user } = setup();

      const resumeButton = screen.getByRole("button", { name: /resume/i });
      await user.click(resumeButton);

      // Confirm in dialog
      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /resume/i,
      });
      await user.click(confirmButton);

      expect(mockTokenResume).toHaveBeenCalledWith(
        expect.objectContaining({
          operation: expect.objectContaining({
            contractId: "contract-resume-222",
            tokenPosition: 0,
          }),
        }),
      );
    });

    it("shows broadcasting state after confirming", async () => {
      const { user } = setup();

      const resumeButton = screen.getByRole("button", { name: /resume/i });
      await user.click(resumeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /resume/i,
      });
      await user.click(confirmButton);

      expect(screen.getByText("Resume...")).toBeInTheDocument();
    });
  });

  // ── Result handling ────────────────────────────────────────────────────

  describe("result handling", () => {
    it("shows success screen on task result", async () => {
      const { user } = setup();

      const resumeButton = screen.getByRole("button", { name: /resume/i });
      await user.click(resumeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /resume/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskResultListener?.({
          payload: {
            taskId: "task-resume-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(screen.getByText("Resume Successful")).toBeInTheDocument();
    });

    it("shows success message about resumed transfers", async () => {
      const { user } = setup();

      const resumeButton = screen.getByRole("button", { name: /resume/i });
      await user.click(resumeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /resume/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskResultListener?.({
          payload: {
            taskId: "task-resume-1",
            resultType: "Token",
            payload: null,
          },
        });
      });

      expect(
        screen.getByText(/token transfers have been resumed/i),
      ).toBeInTheDocument();
    });

    it("shows error screen on task error", async () => {
      const { user } = setup();

      const resumeButton = screen.getByRole("button", { name: /resume/i });
      await user.click(resumeButton);

      const dialog = screen.getByRole("dialog");
      const confirmButton = within(dialog).getByRole("button", {
        name: /resume/i,
      });
      await user.click(confirmButton);

      await act(async () => {
        mockTaskErrorListener?.({
          payload: {
            taskId: "task-resume-1",
            message: "Resume failed: token not paused",
            details: null,
            recoverable: false,
          },
        });
      });

      expect(
        screen.getByText("Resume failed: token not paused"),
      ).toBeInTheDocument();
    });
  });

  // ── Group action ───────────────────────────────────────────────────────

  describe("group action", () => {
    it("shows Sign Resume button when group signing", () => {
      setup({ groupActionId: "group-action-resume-1" });

      expect(
        screen.getByRole("button", { name: /sign resume/i }),
      ).toBeInTheDocument();
    });

    it("passes group info in IPC call when group signing", async () => {
      const { user } = setup({ groupActionId: "group-action-resume-1" });

      const signButton = screen.getByRole("button", {
        name: /sign resume/i,
      });
      await user.click(signButton);

      // Confirm
      const confirmButton = screen.getByRole("button", {
        name: /sign resume/i,
      });
      await user.click(confirmButton);

      expect(mockTokenResume).toHaveBeenCalledWith(
        expect.objectContaining({
          groupInfo: expect.objectContaining({
            type: "other_signer",
            action_id: "group-action-resume-1",
          }),
        }),
      );
    });
  });
});
