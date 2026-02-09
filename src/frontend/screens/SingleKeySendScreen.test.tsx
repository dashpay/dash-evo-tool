import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  SingleKeySendScreen,
  parseMinRelayFeeError,
  estimateP2pkhTxSize,
  estimateFee,
} from "./SingleKeySendScreen";

// ─── Mocks ──────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => ({}),
}));

const mockCoreSendSingleKeyWalletPayment = vi.fn();
const mockWalletNotifyUnlocked = vi.fn();

// Capture event listeners so we can simulate backend events in tests
type EventCallback = (event: { payload: Record<string, unknown> }) => void;
let taskResultListeners: EventCallback[] = [];
let taskErrorListeners: EventCallback[] = [];

vi.mock("@/bindings", () => ({
  commands: {
    coreSendSingleKeyWalletPayment: (...args: unknown[]) =>
      mockCoreSendSingleKeyWalletPayment(...args),
    walletNotifyUnlocked: (...args: unknown[]) =>
      mockWalletNotifyUnlocked(...args),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockImplementation((cb: EventCallback) => {
        taskResultListeners.push(cb);
        return Promise.resolve(() => {
          taskResultListeners = taskResultListeners.filter((l) => l !== cb);
        });
      }),
    },
    taskErrorEvent: {
      listen: vi.fn().mockImplementation((cb: EventCallback) => {
        taskErrorListeners.push(cb);
        return Promise.resolve(() => {
          taskErrorListeners = taskErrorListeners.filter((l) => l !== cb);
        });
      }),
    },
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
    taskResultListeners = [];
    taskErrorListeners = [];
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

  describe("fee confirmation dialog", () => {
    async function sendAndGetSending(user: ReturnType<typeof userEvent.setup>) {
      mockCoreSendSingleKeyWalletPayment.mockResolvedValue({
        status: "ok",
        data: { taskId: "task-fee-test" },
      });
      await user.type(
        screen.getByLabelText("Destination address"),
        "XtAG1kz2TzYNNbCm5sJGYR5NjBhHrGzm1L",
      );
      await user.type(screen.getByPlaceholderText("Enter amount"), "1.0");
      await user.click(screen.getByText("Send").closest("button")!);
      await waitFor(() => {
        expect(screen.getByText("Sending...")).toBeInTheDocument();
      });
    }

    it("shows fee dialog on min relay fee error", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await sendAndGetSending(user);

      // Simulate a min relay fee error event
      for (const listener of taskErrorListeners) {
        listener({
          payload: {
            taskId: "task-fee-test",
            message: "min relay fee not met, 226 < 1000",
          },
        });
      }

      await waitFor(() => {
        expect(screen.getByText("Fee Confirmation Required")).toBeInTheDocument();
      });
    });

    it("re-sends with override fee on confirm", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await sendAndGetSending(user);

      // Trigger min relay fee error
      for (const listener of taskErrorListeners) {
        listener({
          payload: {
            taskId: "task-fee-test",
            message: "min relay fee not met, 226 < 1000",
          },
        });
      }

      await waitFor(() => {
        expect(screen.getByText("Fee Confirmation Required")).toBeInTheDocument();
      });

      // Reset mock for the retry call
      mockCoreSendSingleKeyWalletPayment.mockResolvedValue({
        status: "ok",
        data: { taskId: "task-fee-retry" },
      });

      // Click "Confirm & Send"
      await user.click(screen.getByText("Confirm & Send"));

      await waitFor(() => {
        expect(mockCoreSendSingleKeyWalletPayment).toHaveBeenLastCalledWith(
          expect.objectContaining({ overrideFee: 1000 }),
        );
      });
    });

    it("resets to idle on cancel", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await sendAndGetSending(user);

      // Trigger min relay fee error
      for (const listener of taskErrorListeners) {
        listener({
          payload: {
            taskId: "task-fee-test",
            message: "min relay fee not met, 226 < 1000",
          },
        });
      }

      await waitFor(() => {
        expect(screen.getByText("Fee Confirmation Required")).toBeInTheDocument();
      });

      // Click "Cancel"
      await user.click(screen.getByText("Cancel"));

      // Should be back to the form (idle state)
      await waitFor(() => {
        expect(screen.queryByText("Fee Confirmation Required")).not.toBeInTheDocument();
        expect(screen.queryByText("Sending...")).not.toBeInTheDocument();
        expect(screen.getByLabelText("Destination address")).toBeInTheDocument();
      });
    });

    it("shows normal error for non-fee errors", async () => {
      const user = userEvent.setup();
      setupWallet();
      render(<SingleKeySendScreen />);
      await sendAndGetSending(user);

      // Simulate a normal (non-fee) error
      for (const listener of taskErrorListeners) {
        listener({
          payload: {
            taskId: "task-fee-test",
            message: "Connection timeout",
          },
        });
      }

      await waitFor(() => {
        expect(screen.getByText("Connection timeout")).toBeInTheDocument();
        expect(screen.queryByText("Fee Confirmation Required")).not.toBeInTheDocument();
      });
    });
  });
});

// ─── parseMinRelayFeeError unit tests ───────────────────────────────

describe("parseMinRelayFeeError", () => {
  it("parses standard format: min relay fee not met, 226 < 1000", () => {
    expect(parseMinRelayFeeError("min relay fee not met, 226 < 1000")).toBe(1000);
  });

  it("parses with extra text", () => {
    expect(
      parseMinRelayFeeError("Error: min relay fee not met, 150 < 500 (code -26)"),
    ).toBe(500);
  });

  it("returns null for non-fee errors", () => {
    expect(parseMinRelayFeeError("Insufficient funds")).toBe(null);
  });

  it("returns null when no < symbol", () => {
    expect(parseMinRelayFeeError("min relay fee issue")).toBe(null);
  });

  it("returns null for empty string", () => {
    expect(parseMinRelayFeeError("")).toBe(null);
  });

  it("parses large fee values", () => {
    expect(parseMinRelayFeeError("min relay fee not met, 5000 < 50000")).toBe(50000);
  });
});

// ─── estimateP2pkhTxSize unit tests ─────────────────────────────────

describe("estimateP2pkhTxSize", () => {
  it("calculates 1-input, 2-output transaction", () => {
    // 8 (overhead) + 1 (varint in) + 1 (varint out) + 148*1 + 34*2 = 226
    expect(estimateP2pkhTxSize(1, 2)).toBe(226);
  });

  it("calculates 2-input, 2-output transaction", () => {
    // 8 + 1 + 1 + 148*2 + 34*2 = 374
    expect(estimateP2pkhTxSize(2, 2)).toBe(374);
  });

  it("calculates 10-input, 3-output transaction", () => {
    // 8 + 1 + 1 + 148*10 + 34*3 = 1592
    expect(estimateP2pkhTxSize(10, 3)).toBe(1592);
  });

  it("uses 3-byte varint for >252 inputs", () => {
    // 8 + 3 (varint for 253) + 1 + 148*253 + 34*2 = 37524
    expect(estimateP2pkhTxSize(253, 2)).toBe(37524);
  });
});

// ─── estimateFee unit tests ──────────────────────────────────────────

describe("estimateFee", () => {
  it("returns null for empty utxos", () => {
    expect(estimateFee([], 100000, 1)).toBe(null);
  });

  it("estimates minimum tx when send amount is 0", () => {
    const utxos = [{ amount: 1000000 }];
    const result = estimateFee(utxos, 0, 1);
    expect(result).not.toBe(null);
    // 1 input, 2 outputs (1 recipient + 1 change) → 226 bytes × 1 duff/byte = 226
    expect(result!.inputCount).toBe(1);
    expect(result!.txSize).toBe(226);
    expect(result!.fee).toBe(226);
  });

  it("selects minimum UTXOs needed (greedy highest-first)", () => {
    const utxos = [
      { amount: 100000 },
      { amount: 500000 },
      { amount: 200000 },
    ];
    // Sending 400000 duffs — the 500000 UTXO alone should cover it + fee
    const result = estimateFee(utxos, 400000, 1);
    expect(result).not.toBe(null);
    expect(result!.inputCount).toBe(1); // 500000 > 400000 + 226
  });

  it("uses multiple UTXOs when largest is insufficient", () => {
    const utxos = [
      { amount: 100000 },
      { amount: 200000 },
      { amount: 150000 },
    ];
    // Sending 300000 — need 200000 + 150000 (sorted: 200000, 150000, 100000)
    const result = estimateFee(utxos, 300000, 1);
    expect(result).not.toBe(null);
    expect(result!.inputCount).toBeGreaterThanOrEqual(2);
  });

  it("accounts for multiple recipients in output count", () => {
    const utxos = [{ amount: 10000000 }];
    const result1 = estimateFee(utxos, 1000000, 1); // 2 outputs
    const result3 = estimateFee(utxos, 1000000, 3); // 4 outputs
    expect(result3!.txSize).toBeGreaterThan(result1!.txSize);
  });

  it("returns all-UTXO estimate when funds insufficient", () => {
    const utxos = [{ amount: 100 }, { amount: 200 }];
    // Sending way more than available
    const result = estimateFee(utxos, 999999999, 1);
    expect(result).not.toBe(null);
    expect(result!.inputCount).toBe(2); // used all UTXOs
  });
});

// ─── Transaction estimation display tests ────────────────────────────

describe("SingleKeySendScreen transaction estimation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    walletStoreState = { singleKeyWallets: [], selectedWallet: null };
    taskResultListeners = [];
    taskErrorListeners = [];
  });

  it("shows estimated fee when wallet has UTXOs", () => {
    setupWallet({
      utxos: [
        { txid: "aaa", vout: 0, amount: 5000000 },
        { txid: "bbb", vout: 1, amount: 3000000 },
      ],
    });
    render(<SingleKeySendScreen />);
    expect(screen.getByText(/Estimated fee:/)).toBeInTheDocument();
    expect(screen.getByText(/Transaction details:/)).toBeInTheDocument();
  });

  it("does not show estimation when wallet has no UTXOs", () => {
    setupWallet({ utxos: [] });
    render(<SingleKeySendScreen />);
    expect(screen.queryByText(/Estimated fee:/)).not.toBeInTheDocument();
  });

  it("shows input count and byte size", () => {
    setupWallet({
      utxos: [{ txid: "aaa", vout: 0, amount: 5000000 }],
    });
    render(<SingleKeySendScreen />);
    expect(screen.getByText(/1 input, ~226 bytes/)).toBeInTheDocument();
  });

  it("shows fee in both duffs and DASH", () => {
    setupWallet({
      utxos: [{ txid: "aaa", vout: 0, amount: 5000000 }],
    });
    render(<SingleKeySendScreen />);
    // Fee display shows "226 (0.00000226 DASH)"
    expect(screen.getByText(/226 \(0\.00000226 DASH\)/)).toBeInTheDocument();
  });

  it("updates estimation as amount changes", async () => {
    const user = userEvent.setup();
    setupWallet({
      utxos: [
        { txid: "aaa", vout: 0, amount: 100000 },
        { txid: "bbb", vout: 1, amount: 500000 },
        { txid: "ccc", vout: 2, amount: 300000 },
      ],
    });
    render(<SingleKeySendScreen />);

    // Initially shows 1-input estimate (no amount entered)
    expect(screen.getByText(/1 input/)).toBeInTheDocument();

    // Type an amount that requires multiple UTXOs
    // 0.004 DASH = 400000 duffs. Largest UTXO is 500000.
    // 500000 >= 400000 + 226 (fee) = 400226, so 1 UTXO suffices? Yes.
    // 0.005 DASH = 500000 duffs. 500000 >= 500226? No.
    // Need 500000 + 300000 = 800000. 2-input fee = 374. 800000 >= 500374? Yes.
    await user.type(screen.getByPlaceholderText("Enter amount"), "0.005");

    await waitFor(() => {
      expect(screen.getByText(/2 inputs/)).toBeInTheDocument();
    });
  });

  it("shows large input warning when >100 UTXOs needed", () => {
    // Create 105 small UTXOs
    const utxos = Array.from({ length: 105 }, (_, i) => ({
      txid: `tx${i}`,
      vout: 0,
      amount: 1000, // 1000 duffs each
    }));
    setupWallet({
      totalBalance: 105000,
      utxos,
    });
    render(<SingleKeySendScreen />);

    // The estimation with 0 send amount should show 1 input (no warning)
    expect(screen.queryByText(/Large number of inputs/)).not.toBeInTheDocument();
  });

  it("shows large input warning for high-value sends requiring many UTXOs", async () => {
    const user = userEvent.setup();
    // Create 105 small UTXOs
    const utxos = Array.from({ length: 105 }, (_, i) => ({
      txid: `tx${i}`,
      vout: 0,
      amount: 1000,
    }));
    setupWallet({
      totalBalance: 105000,
      utxos,
    });
    render(<SingleKeySendScreen />);

    // Enter amount requiring all UTXOs
    await user.type(screen.getByPlaceholderText("Enter amount"), "0.001");

    await waitFor(() => {
      expect(screen.getByText(/Large number of inputs/)).toBeInTheDocument();
    });
  });

  it("shows estimation in advanced mode too", async () => {
    const user = userEvent.setup();
    setupWallet({
      utxos: [{ txid: "aaa", vout: 0, amount: 5000000 }],
    });
    render(<SingleKeySendScreen />);
    await user.click(screen.getByText("Advanced Options").closest("label")!);
    expect(screen.getByText(/Estimated fee:/)).toBeInTheDocument();
  });
});
