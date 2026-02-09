import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SingleKeySendScreen } from "./SingleKeySendScreen";

// ─── Mocks ──────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => ({}),
}));

const mockCoreSendSingleKeyWalletPayment = vi.fn();
const mockWalletNotifyUnlocked = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    coreSendSingleKeyWalletPayment: (...args: unknown[]) =>
      mockCoreSendSingleKeyWalletPayment(...args),
    walletNotifyUnlocked: (...args: unknown[]) =>
      mockWalletNotifyUnlocked(...args),
  },
  events: {
    taskResultEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    taskErrorEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
  },
}));

let walletStoreState: Record<string, unknown> = {};
vi.mock("@/stores/walletStore", () => ({
  useWalletStore: () => walletStoreState,
}));

// ─── Helpers ────────────────────────────────────────────────────────

function makeSingleKeyWallet(overrides: Record<string, unknown> = {}) {
  return {
    keyHash: "abc123",
    alias: "Test Key",
    address: "yTestAddress123456789012345678",
    totalBalance: 500000000,
    unconfirmedBalance: 0,
    usesPassword: false,
    utxos: [],
    ...overrides,
  };
}

function setupWallet(overrides: Record<string, unknown> = {}) {
  const wallet = makeSingleKeyWallet(overrides);
  walletStoreState = {
    singleKeyWallets: [wallet],
    selectedWallet: { type: "singleKey", keyHash: wallet.keyHash },
  };
  return wallet;
}

// ─── Tests ──────────────────────────────────────────────────────────

describe("SingleKeySendScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    walletStoreState = { singleKeyWallets: [], selectedWallet: null };
  });

  describe("no wallet selected", () => {
    it("shows no wallet message", () => {
      render(<SingleKeySendScreen />);
      expect(screen.getByText("No single-key wallet selected")).toBeInTheDocument();
    });

    it("shows back button", async () => {
      render(<SingleKeySendScreen />);
      await userEvent.click(screen.getByText("Back to Wallets"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });
  });

  describe("initial render with wallet", () => {
    it("shows send dash heading", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Send Dash")).toBeInTheDocument();
    });

    it("shows wallet alias", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Test Key")).toBeInTheDocument();
    });

    it("shows wallet address", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText(/yTestAddress/)).toBeInTheDocument();
    });

    it("shows balance", () => {
      setupWallet({ totalBalance: 500000000 });
      render(<SingleKeySendScreen />);
      expect(screen.getByText(/5\.00000000 DASH/)).toBeInTheDocument();
    });

    it("shows back button in header", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByLabelText("Back to wallets")).toBeInTheDocument();
    });

    it("shows advanced options checkbox", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Advanced Options")).toBeInTheDocument();
    });

    it("shows destination address input", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByLabelText("Destination address")).toBeInTheDocument();
    });

    it("shows amount input", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Amount")).toBeInTheDocument();
    });

    it("shows subtract fee checkbox", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Subtract fee from amount")).toBeInTheDocument();
    });

    it("shows cancel and send buttons", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Cancel")).toBeInTheDocument();
      expect(screen.getByText("Send")).toBeInTheDocument();
    });

    it("send button is disabled initially", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      const sendBtn = screen.getByText("Send").closest("button");
      expect(sendBtn).toBeDisabled();
    });
  });

  describe("wallet unlock", () => {
    it("shows unlock gate when wallet has password", () => {
      setupWallet({ usesPassword: true });
      render(<SingleKeySendScreen />);
      expect(screen.getByText("Wallet is locked. Please unlock to continue.")).toBeInTheDocument();
    });

    it("does not show form when locked", () => {
      setupWallet({ usesPassword: true });
      render(<SingleKeySendScreen />);
      expect(screen.queryByLabelText("Destination address")).not.toBeInTheDocument();
    });

    it("auto-unlocks when wallet has no password", () => {
      setupWallet({ usesPassword: false });
      render(<SingleKeySendScreen />);
      expect(screen.getByLabelText("Destination address")).toBeInTheDocument();
    });
  });

  describe("address validation", () => {
    it("shows error for platform addresses", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "evo1abc123");
      expect(screen.getByText(/Platform addresses are not supported/)).toBeInTheDocument();
    });

    it("shows error for invalid address format", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "invalidaddress");
      expect(screen.getByText("Invalid Dash address format")).toBeInTheDocument();
    });

    it("accepts valid dash address", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      expect(screen.queryByText("Invalid Dash address format")).not.toBeInTheDocument();
    });
  });

  describe("send flow", () => {
    it("dispatches coreSendSingleKeyWalletPayment", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockCoreSendSingleKeyWalletPayment.mockResolvedValue({
        status: "ok", data: { taskId: "task123" },
      });

      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Send").closest("button")!);

      await waitFor(() => {
        expect(mockCoreSendSingleKeyWalletPayment).toHaveBeenCalledWith({
          keyHash: "abc123",
          recipients: [{ address: "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L", amount: 100000000 }],
          subtractFeeFromAmount: false,
          memo: null,
          overrideFee: null,
        });
      });
    });

    it("shows sending state", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockCoreSendSingleKeyWalletPayment.mockResolvedValue({
        status: "ok", data: { taskId: "task123" },
      });

      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Send").closest("button")!);

      await waitFor(() => {
        expect(screen.getByText("Sending...")).toBeInTheDocument();
      });
    });

    it("shows error when send fails", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockCoreSendSingleKeyWalletPayment.mockResolvedValue({
        status: "error", error: "Insufficient funds",
      });

      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Send").closest("button")!);

      await waitFor(() => {
        expect(screen.getByText("Insufficient funds")).toBeInTheDocument();
      });
    });

    it("passes subtract fee flag", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockCoreSendSingleKeyWalletPayment.mockResolvedValue({
        status: "ok", data: { taskId: "task123" },
      });

      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Subtract fee from amount").closest("label")!);
      await user.click(screen.getByText("Send").closest("button")!);

      await waitFor(() => {
        expect(mockCoreSendSingleKeyWalletPayment).toHaveBeenCalledWith(
          expect.objectContaining({ subtractFeeFromAmount: true }),
        );
      });
    });

    it("shows insufficient balance error", async () => {
      const user = userEvent.setup();
      setupWallet({ totalBalance: 100 });
      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Send").closest("button")!);

      await waitFor(() => {
        expect(screen.getByText(/Insufficient balance/)).toBeInTheDocument();
      });
    });
  });

  describe("advanced mode", () => {
    it("shows recipients section in advanced mode", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.click(screen.getByText("Advanced Options").closest("label")!);
      expect(screen.getByText("Recipients")).toBeInTheDocument();
    });

    it("can add recipients", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.click(screen.getByText("Advanced Options").closest("label")!);
      await user.click(screen.getByText("Add Recipient"));
      expect(screen.getAllByPlaceholderText("Enter Dash address (e.g., y...)").length).toBe(2);
    });

    it("can remove recipients when multiple exist", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.click(screen.getByText("Advanced Options").closest("label")!);
      await user.click(screen.getByText("Add Recipient"));
      const removeButtons = screen.getAllByLabelText(/Remove recipient/);
      expect(removeButtons.length).toBe(2);
      await user.click(removeButtons[0]);
      expect(screen.getAllByPlaceholderText("Enter Dash address (e.g., y...)").length).toBe(1);
    });

    it("shows memo field", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await user.click(screen.getByText("Advanced Options").closest("label")!);
      expect(screen.getByLabelText("Transaction memo")).toBeInTheDocument();
    });
  });

  describe("navigation", () => {
    it("navigates back on cancel", async () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      await userEvent.click(screen.getByText("Cancel"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });

    it("navigates back via header button", async () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      await userEvent.click(screen.getByLabelText("Back to wallets"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });
  });

  describe("error handling", () => {
    it("dismisses error on click", async () => {
      const user = userEvent.setup();
      setupWallet({ totalBalance: 100 });
      render(<SingleKeySendScreen />);
      await user.type(screen.getByLabelText("Destination address"), "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L");
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Send").closest("button")!);

      await waitFor(() => { expect(screen.getByText(/Insufficient balance/)).toBeInTheDocument(); });
      await user.click(screen.getByLabelText("Dismiss error"));
      expect(screen.queryByText(/Insufficient balance/)).not.toBeInTheDocument();
    });
  });

  describe("accessibility", () => {
    it("has proper heading levels", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Send Dash");
    });

    it("has aria labels on inputs", () => {
      setupWallet();
      render(<SingleKeySendScreen />);
      expect(screen.getByLabelText("Destination address")).toBeInTheDocument();
      expect(screen.getByLabelText("Back to wallets")).toBeInTheDocument();
    });
  });
});
