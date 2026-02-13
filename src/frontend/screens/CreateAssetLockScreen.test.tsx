import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CreateAssetLockScreen } from "./CreateAssetLockScreen";

// ─── Mocks ──────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => ({}),
}));

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

vi.mock("@/hooks/useUtxoMonitor", () => ({
  useUtxoMonitor: () => ({ fundsReceived: false, balance: 0, polling: false }),
}));

vi.mock("qrcode.react", () => ({
  QRCodeSVG: (props: { value: string }) => (
    <svg data-testid="qr-svg" data-value={props.value} />
  ),
}));

import { commands } from "@/bindings";

let walletStoreState: Record<string, unknown> = {};
vi.mock("@/stores/walletStore", () => ({
  useWalletStore: (selector?: (s: Record<string, unknown>) => unknown) => selector ? selector(walletStoreState) : walletStoreState,
}));

// ─── Helpers ────────────────────────────────────────────────────────

function makeHdWallet(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: "seed123",
    alias: "Test Wallet",
    confirmedBalance: 1000000000,
    unconfirmedBalance: 0,
    usesPassword: false,
    passwordHint: null,
    addresses: [],
    platformAddresses: [],
    unusedAssetLocks: [],
    accounts: [],
    transactions: [],
    identityIndexes: [],
    ...overrides,
  };
}

function setupWallet(overrides: Record<string, unknown> = {}) {
  const wallet = makeHdWallet(overrides);
  walletStoreState = {
    hdWallets: [wallet],
    selectedWallet: { type: "hd", seedHash: wallet.seedHash },
  };
  return wallet;
}

// ─── Tests ──────────────────────────────────────────────────────────

describe("CreateAssetLockScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    walletStoreState = { hdWallets: [], selectedWallet: null };
  });

  describe("no wallet selected", () => {
    it("shows no wallet message", () => {
      render(<CreateAssetLockScreen />);
      expect(screen.getByText("No HD wallet selected")).toBeInTheDocument();
    });

    it("shows back button", async () => {
      render(<CreateAssetLockScreen />);
      await userEvent.click(screen.getByText("Back to Wallets"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });
  });

  describe("initial render", () => {
    it("shows create asset lock heading", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByText("Create Asset Lock")).toBeInTheDocument();
    });

    it("shows wallet info", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByText("Test Wallet")).toBeInTheDocument();
    });

    it("shows balance", () => {
      setupWallet({ confirmedBalance: 1000000000 });
      render(<CreateAssetLockScreen />);
      expect(screen.getByText(/10\.00000000 DASH/)).toBeInTheDocument();
    });

    it("shows back button in header", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByLabelText("Back to wallets")).toBeInTheDocument();
    });
  });

  describe("purpose selection step", () => {
    it("shows registration and top up options", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByText("Registration")).toBeInTheDocument();
      expect(screen.getByText("Top Up")).toBeInTheDocument();
    });

    it("shows purpose descriptions", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByText(/new identity on the Dash Platform/)).toBeInTheDocument();
      expect(screen.getByText(/Add credits to an existing identity/)).toBeInTheDocument();
    });

    it("shows question prompt", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByText("What is this asset lock for?")).toBeInTheDocument();
    });
  });

  describe("registration flow", () => {
    it("moves to configure step on registration click", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByText("Amount (DASH)")).toBeInTheDocument();
    });

    it("shows change button", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByText("Change")).toBeInTheDocument();
    });

    it("goes back to purpose on change click", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Change"));
      expect(screen.getByText("What is this asset lock for?")).toBeInTheDocument();
    });

    it("shows generate funding address button", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByText("Generate Funding Address")).toBeInTheDocument();
    });

    it("shows cancel button", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByText("Cancel")).toBeInTheDocument();
    });

    it("default amount is 0.5", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByDisplayValue("0.5")).toBeInTheDocument();
    });

    it("shows credits conversion", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByText(/50,000,000,000 credits/)).toBeInTheDocument();
    });
  });

  describe("top-up flow", () => {
    it("shows identity selector on top-up", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Top Up"));
      expect(screen.getByText("Select Identity to Top Up")).toBeInTheDocument();
    });

    it("shows no identities message when empty", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Top Up"));
      expect(screen.getByText(/No identities found/)).toBeInTheDocument();
    });
  });

  describe("advanced options", () => {
    it("shows advanced options checkbox on configure step", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      expect(screen.getByText("Advanced Options")).toBeInTheDocument();
    });

    it("shows identity index selector when enabled", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Advanced Options").closest("label")!);
      expect(screen.getByText("Identity Index")).toBeInTheDocument();
    });
  });

  describe("funding step", () => {
    async function generateAddressAndEmitResult(user: ReturnType<typeof userEvent.setup>) {
      vi.mocked(commands.walletGenerateReceiveAddress).mockResolvedValue({
        status: "ok", data: { address: "yRealDashAddress123" },
      });
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Generate Funding Address"));

      await waitFor(() => {
        expect(commands.walletGenerateReceiveAddress).toHaveBeenCalled();
      });
    }

    it("generates funding address on button click", async () => {
      const user = userEvent.setup();
      setupWallet();
      await generateAddressAndEmitResult(user);

      await waitFor(() => {
        expect(screen.getByText("Fund This Address")).toBeInTheDocument();
      });
    });

    it("shows the generated address", async () => {
      const user = userEvent.setup();
      setupWallet();
      await generateAddressAndEmitResult(user);

      await waitFor(() => {
        expect(screen.getByText("yRealDashAddress123")).toBeInTheDocument();
      });
    });

    it("shows waiting for funds indicator", async () => {
      const user = userEvent.setup();
      setupWallet();
      await generateAddressAndEmitResult(user);

      await waitFor(() => {
        expect(screen.getByText("Waiting for funds...")).toBeInTheDocument();
      });
    });

    it("shows manual create button", async () => {
      const user = userEvent.setup();
      setupWallet();
      await generateAddressAndEmitResult(user);

      await waitFor(() => {
        expect(screen.getByText("Create Asset Lock Now")).toBeInTheDocument();
      });
    });

    it("shows error when address generation fails", async () => {
      const user = userEvent.setup();
      setupWallet();
      vi.mocked(commands.walletGenerateReceiveAddress).mockResolvedValue({
        status: "error", error: "Failed to generate address",
      });
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Generate Funding Address"));

      await waitFor(() => {
        expect(screen.getByText("Failed to generate address")).toBeInTheDocument();
      });
    });
  });

  describe("asset lock creation", () => {
    async function goToFundingStep(user: ReturnType<typeof userEvent.setup>) {
      vi.mocked(commands.walletGenerateReceiveAddress).mockResolvedValue({
        status: "ok", data: { address: "yRealDashAddress123" },
      });
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Generate Funding Address"));
      await waitFor(() => {
        expect(commands.walletGenerateReceiveAddress).toHaveBeenCalled();
      });
      await waitFor(() => {
        expect(screen.getByText("Create Asset Lock Now")).toBeInTheDocument();
      });
    }

    it("dispatches registration asset lock", async () => {
      const user = userEvent.setup();
      setupWallet();
      vi.mocked(commands.coreCreateRegistrationAssetLock).mockResolvedValue({
        status: "ok", data: { taskId: "task-lock-123" },
      });
      await goToFundingStep(user);
      await user.click(screen.getByText("Create Asset Lock Now"));

      await waitFor(() => {
        expect(commands.coreCreateRegistrationAssetLock).toHaveBeenCalledWith({
          walletSeedHash: "seed123",
          amountCredits: 50000000000,
          identityIndex: 0,
        });
      });
    });

    it("shows creating state", async () => {
      const user = userEvent.setup();
      setupWallet();
      vi.mocked(commands.coreCreateRegistrationAssetLock).mockResolvedValue({
        status: "ok", data: { taskId: "task-lock-123" },
      });
      await goToFundingStep(user);
      await user.click(screen.getByText("Create Asset Lock Now"));

      await waitFor(() => {
        expect(screen.getByText("Creating Asset Lock")).toBeInTheDocument();
      });
    });

    it("shows error when creation fails", async () => {
      const user = userEvent.setup();
      setupWallet();
      vi.mocked(commands.coreCreateRegistrationAssetLock).mockResolvedValue({
        status: "error", error: "Creation failed",
      });
      await goToFundingStep(user);
      await user.click(screen.getByText("Create Asset Lock Now"));

      await waitFor(() => {
        expect(screen.getByText("Creation failed")).toBeInTheDocument();
      });
    });
  });

  describe("wallet unlock", () => {
    it("shows unlock gate when wallet has password", () => {
      setupWallet({ usesPassword: true });
      render(<CreateAssetLockScreen />);
      expect(screen.getByText("Wallet is locked. Please unlock to continue.")).toBeInTheDocument();
    });

    it("hides form when locked", () => {
      setupWallet({ usesPassword: true });
      render(<CreateAssetLockScreen />);
      expect(screen.queryByText("Registration")).not.toBeInTheDocument();
    });
  });

  describe("navigation", () => {
    it("navigates back on cancel", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Cancel"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });

    it("navigates back via header button", async () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      await userEvent.click(screen.getByLabelText("Back to wallets"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });
  });

  describe("accessibility", () => {
    it("has proper heading", () => {
      setupWallet();
      render(<CreateAssetLockScreen />);
      expect(screen.getByRole("heading", { level: 1, name: "Create Asset Lock" })).toBeInTheDocument();
    });

    it("has error role on error banner", async () => {
      const user = userEvent.setup();
      setupWallet();
      vi.mocked(commands.walletGenerateReceiveAddress).mockResolvedValue({
        status: "error", error: "Some error",
      });
      render(<CreateAssetLockScreen />);
      await user.click(screen.getByText("Registration"));
      await user.click(screen.getByText("Generate Funding Address"));

      await waitFor(() => {
        expect(screen.getByRole("alert")).toBeInTheDocument();
      });
    });
  });
});
