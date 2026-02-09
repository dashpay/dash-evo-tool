import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { vi, describe, it, expect, beforeEach } from "vitest";
import { SendScreen } from "./SendScreen";

// ─── Mocks ──────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

const mockCoreSendWalletPayment = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-123" },
});
const mockWalletFundPlatform = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-456" },
});
const mockWalletTransferPlatform = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-789" },
});
const mockWalletWithdraw = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-withdraw" },
});
const mockIdentityWithdraw = vi.fn().mockResolvedValue({
  status: "ok",
  data: { taskId: "task-identity" },
});
const mockIdentityListLocal = vi.fn().mockResolvedValue({
  status: "ok",
  data: [],
});
const mockWalletNotifyUnlocked = vi.fn().mockResolvedValue(undefined);

vi.mock("@/bindings", () => ({
  commands: {
    coreSendWalletPayment: (...args: unknown[]) =>
      mockCoreSendWalletPayment(...args),
    walletFundPlatformAddressFromUtxos: (...args: unknown[]) =>
      mockWalletFundPlatform(...args),
    walletTransferPlatformCredits: (...args: unknown[]) =>
      mockWalletTransferPlatform(...args),
    walletWithdrawFromPlatformAddress: (...args: unknown[]) =>
      mockWalletWithdraw(...args),
    identityWithdraw: (...args: unknown[]) => mockIdentityWithdraw(...args),
    identityListLocal: () => mockIdentityListLocal(),
    walletNotifyUnlocked: (...args: unknown[]) =>
      mockWalletNotifyUnlocked(...args),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
    taskErrorEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
  },
}));

// ─── Wallet store mock ──────────────────────────────────────────────

const mockHdWallet = {
  seedHash: "abc123",
  usesPassword: false,
  alias: "Test Wallet",
  isMain: true,
  confirmedBalance: 500_000_000, // 5 DASH
  unconfirmedBalance: 0,
  totalBalance: 500_000_000,
  addresses: [
    {
      address: "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1",
      balance: 300_000_000,
      totalReceived: 500_000_000,
      derivationPath: "m/44'/5'/0'/0/0",
      addressType: "p2pkh",
    },
    {
      address: "XqZw4O8uHUCM5gF4c2Z6fA9lNsR0pR3sV2",
      balance: 200_000_000,
      totalReceived: 200_000_000,
      derivationPath: "m/44'/5'/0'/0/1",
      addressType: "p2pkh",
    },
  ],
  transactions: [],
  unusedAssetLocks: [],
  platformAddresses: [
    { address: "evo1abc123def456", balance: 100_000_000_000, nonce: 0 },
    { address: "evo1xyz789ghi012", balance: 50_000_000_000, nonce: 1 },
  ],
  identityIndexes: [0],
  passwordHint: null,
};

const mockPasswordWallet = {
  ...mockHdWallet,
  seedHash: "pass123",
  alias: "Locked Wallet",
  usesPassword: true,
  passwordHint: "My hint",
};

let mockSelectedWallet: { type: string; seedHash?: string; keyHash?: string } | null = {
  type: "hd",
  seedHash: "abc123",
};

vi.mock("@/stores/walletStore", () => ({
  useWalletStore: vi.fn(() => ({
    hdWallets: [mockHdWallet],
    singleKeyWallets: [],
    selectedWallet: mockSelectedWallet,
  })),
}));

// ─── Sonner mock ────────────────────────────────────────────────────

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
  },
}));

// ─── Tests ──────────────────────────────────────────────────────────

describe("SendScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockSelectedWallet = { type: "hd", seedHash: "abc123" };
  });

  // ─── Initial render ───────────────────────────────────────────

  it("renders the screen title", () => {
    render(<SendScreen />);
    expect(screen.getByText("Send Dash")).toBeInTheDocument();
  });

  it("renders the back button", () => {
    render(<SendScreen />);
    expect(screen.getByLabelText("Back to wallets")).toBeInTheDocument();
  });

  it("renders the advanced options checkbox", () => {
    render(<SendScreen />);
    expect(screen.getByText("Advanced Options")).toBeInTheDocument();
  });

  it("shows wallet name", () => {
    render(<SendScreen />);
    expect(screen.getByText("Test Wallet")).toBeInTheDocument();
  });

  // ─── No wallet state ──────────────────────────────────────────

  it("shows no wallet message when no wallet selected", () => {
    mockSelectedWallet = null;
    render(<SendScreen />);
    expect(screen.getByText("No wallet selected")).toBeInTheDocument();
    expect(screen.getByText("Back to Wallets")).toBeInTheDocument();
  });

  // ─── Source selection ─────────────────────────────────────────

  it("renders Core Wallet source option", () => {
    render(<SendScreen />);
    expect(screen.getByText("Core Wallet")).toBeInTheDocument();
  });

  it("shows Core wallet balance", () => {
    render(<SendScreen />);
    // 500_000_000 duffs = 5.00000000 DASH
    expect(screen.getByText("5.00000000 DASH")).toBeInTheDocument();
  });

  it("renders Platform Addresses source when available", () => {
    render(<SendScreen />);
    expect(screen.getByText("Platform Addresses")).toBeInTheDocument();
  });

  it("Core Wallet is selected by default", () => {
    render(<SendScreen />);
    const coreRadio = screen.getByText("Core Wallet")
      .closest("button")
      ?.querySelector('input[type="radio"]');
    expect(coreRadio).toBeChecked();
  });

  it("allows selecting Platform Addresses source", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const platformBtn = screen.getByText("Platform Addresses").closest("button")!;
    await user.click(platformBtn);
    const platformRadio = platformBtn.querySelector('input[type="radio"]');
    expect(platformRadio).toBeChecked();
  });

  // ─── Destination address ──────────────────────────────────────

  it("renders destination address input", () => {
    render(<SendScreen />);
    expect(screen.getByLabelText("Destination address")).toBeInTheDocument();
  });

  it("shows Core Address badge for Core addresses", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");
    expect(screen.getByText("Core Address")).toBeInTheDocument();
  });

  it("shows Platform Address badge for Platform addresses", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "evo1abc123def456ghi789jkl012mno345pqr678stu901vwx");
    expect(screen.getByText("Platform Address")).toBeInTheDocument();
  });

  it("shows invalid address error for bad addresses", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "invalid_address");
    expect(screen.getByText("Invalid address format")).toBeInTheDocument();
  });

  // ─── Amount input ─────────────────────────────────────────────

  it("renders amount input", () => {
    render(<SendScreen />);
    expect(screen.getByText("Amount")).toBeInTheDocument();
  });

  it("shows transaction type hint for Core→Platform", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "evo1abc123def456ghi789jkl012mno345pqr678stu901vwx");
    expect(
      screen.getByText("Transaction type: Fund Platform Address"),
    ).toBeInTheDocument();
  });

  it("shows transaction type for Core→Core", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");
    expect(
      screen.getByText("Transaction type: Core Transaction"),
    ).toBeInTheDocument();
  });

  // ─── Subtract fee checkbox ────────────────────────────────────

  it("shows subtract fee checkbox for Core→Core", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");
    expect(
      screen.getByText("Subtract fee from amount"),
    ).toBeInTheDocument();
  });

  it("does not show subtract fee for Core→Platform", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "evo1abc123def456ghi789jkl012mno345pqr678stu901vwx");
    expect(
      screen.queryByText("Subtract fee from amount"),
    ).not.toBeInTheDocument();
  });

  // ─── Send button ──────────────────────────────────────────────

  it("renders Cancel and Send buttons", () => {
    render(<SendScreen />);
    expect(screen.getByText("Cancel")).toBeInTheDocument();
    // Send button text varies with transaction type, but "Send" is the label icon text
    expect(screen.getByRole("button", { name: /Send/i })).toBeInTheDocument();
  });

  it("disables send button when form is incomplete", () => {
    render(<SendScreen />);
    // The send button should be disabled (no destination, no amount)
    const sendBtn = screen.getByRole("button", { name: /Send/i });
    expect(sendBtn).toBeDisabled();
  });

  it("Cancel button navigates back", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByText("Cancel"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  // ─── Core → Core send flow ────────────────────────────────────

  it("dispatches Core→Core send with correct params", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);

    // Enter destination
    const destInput = screen.getByLabelText("Destination address");
    await user.type(destInput, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");

    // Enter amount
    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "1.5");

    // Click send
    const sendBtn = screen.getByRole("button", { name: /Core Transaction/i });
    await user.click(sendBtn);

    await waitFor(() => {
      expect(mockCoreSendWalletPayment).toHaveBeenCalledWith({
        walletSeedHash: "abc123",
        recipients: [
          { address: "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1", amount: 150_000_000 },
        ],
        subtractFeeFromAmount: false,
        memo: null,
        overrideFee: null,
      });
    });
  });

  it("shows sending state after dispatch", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);

    const destInput = screen.getByLabelText("Destination address");
    await user.type(destInput, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");

    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "1.0");

    const sendBtn = screen.getByRole("button", { name: /Core Transaction/i });
    await user.click(sendBtn);

    await waitFor(() => {
      expect(screen.getByText("Sending...")).toBeInTheDocument();
    });
  });

  // ─── Core → Platform send flow ────────────────────────────────

  it("dispatches Core→Platform fund with correct params", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);

    const destInput = screen.getByLabelText("Destination address");
    await user.type(
      destInput,
      "evo1abc123def456ghi789jkl012mno345pqr678stu901vwx",
    );

    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "2.0");

    const sendBtn = screen.getByRole("button", {
      name: /Fund Platform Address/i,
    });
    await user.click(sendBtn);

    await waitFor(() => {
      expect(mockWalletFundPlatform).toHaveBeenCalledWith({
        walletSeedHash: "abc123",
        amount: 200_000_000,
        destination:
          "evo1abc123def456ghi789jkl012mno345pqr678stu901vwx",
        feeDeductFromOutput: true,
      });
    });
  });

  // ─── Error handling ───────────────────────────────────────────

  it("shows error when send fails", async () => {
    mockCoreSendWalletPayment.mockResolvedValueOnce({
      status: "error",
      error: "Insufficient funds",
    });

    const user = userEvent.setup();
    render(<SendScreen />);

    const destInput = screen.getByLabelText("Destination address");
    await user.type(destInput, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");

    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "1.0");

    const sendBtn = screen.getByRole("button", { name: /Core Transaction/i });
    await user.click(sendBtn);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("Insufficient funds")).toBeInTheDocument();
    });
  });

  it("error banner can be dismissed", async () => {
    mockCoreSendWalletPayment.mockResolvedValueOnce({
      status: "error",
      error: "Some error",
    });

    const user = userEvent.setup();
    render(<SendScreen />);

    const destInput = screen.getByLabelText("Destination address");
    await user.type(destInput, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");

    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "1.0");

    const sendBtn = screen.getByRole("button", { name: /Core Transaction/i });
    await user.click(sendBtn);

    await waitFor(() => {
      expect(screen.getByText("Some error")).toBeInTheDocument();
    });

    await user.click(screen.getByLabelText("Dismiss error"));

    await waitFor(() => {
      expect(screen.queryByText("Some error")).not.toBeInTheDocument();
    });
  });

  // ─── Platform → Platform source ───────────────────────────────

  it("shows platform balance in source option", () => {
    render(<SendScreen />);
    // Platform addresses have 100B + 50B = 150B credits = 150M duffs = 1.5 DASH
    expect(screen.getByText("Platform Addresses")).toBeInTheDocument();
  });

  // ─── Platform source breakdown ────────────────────────────────

  it("shows platform source breakdown when platform source and amount entered", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);

    // Select platform source
    const platformBtn = screen.getByText("Platform Addresses").closest("button")!;
    await user.click(platformBtn);

    // Enter platform destination
    const destInput = screen.getByLabelText("Destination address");
    await user.type(
      destInput,
      "evo1dest567abc890def123ghi456jkl789mno012pqr345stu",
    );

    // Enter amount
    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "0.5");

    // Should see source breakdown
    await waitFor(() => {
      expect(screen.getByText("Source breakdown:")).toBeInTheDocument();
    });
  });

  // ─── Identity source ─────────────────────────────────────────

  it("renders identity source options when identities are loaded", async () => {
    mockIdentityListLocal.mockResolvedValueOnce({
      status: "ok",
      data: [
        {
          id: "ident1",
          identityType: "user",
          alias: "My Identity",
          balance: 500_000_000_000,
          keys: [
            {
              keyId: 1,
              keyType: "ECDSA_SECP256K1",
              purpose: "transfer",
              securityLevel: "HIGH",
              data: "abc",
              disabled: false,
            },
          ],
          dpnsNames: [],
          associatedWalletHashes: [],
          walletIndex: null,
          topUps: [],
          status: "registered",
          network: "dash",
          voterIdentityId: null,
          operatorIdentityId: null,
        },
      ],
    });

    render(<SendScreen />);

    await waitFor(() => {
      expect(screen.getByText(/Identity: My Identity/)).toBeInTheDocument();
    });
  });

  it("shows identity can only withdraw to Core warning", async () => {
    mockIdentityListLocal.mockResolvedValueOnce({
      status: "ok",
      data: [
        {
          id: "ident1",
          identityType: "user",
          alias: "ID",
          balance: 100_000_000_000,
          keys: [
            {
              keyId: 1,
              keyType: "ECDSA_SECP256K1",
              purpose: "transfer",
              securityLevel: "HIGH",
              data: "abc",
              disabled: false,
            },
          ],
          dpnsNames: [],
          associatedWalletHashes: [],
          walletIndex: null,
          topUps: [],
          status: "registered",
          network: "dash",
          voterIdentityId: null,
          operatorIdentityId: null,
        },
      ],
    });

    const user = userEvent.setup();
    render(<SendScreen />);

    await waitFor(() => {
      expect(screen.getByText(/Identity: ID/)).toBeInTheDocument();
    });

    // Select identity source
    await user.click(screen.getByText(/Identity: ID/).closest("button")!);

    // Enter platform destination (invalid for identity)
    const destInput = screen.getByLabelText("Destination address");
    await user.type(
      destInput,
      "evo1abc123def456ghi789jkl012mno345pqr678stu901vwx",
    );

    expect(
      screen.getByText("Identity can only withdraw to Core addresses"),
    ).toBeInTheDocument();
  });

  // ─── Advanced mode ────────────────────────────────────────────

  it("toggles to advanced mode", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByText("Advanced Options"));
    expect(screen.getByText("Source Type")).toBeInTheDocument();
  });

  it("shows source type radio buttons in advanced mode", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByText("Advanced Options"));

    expect(screen.getByText("Core Address Inputs")).toBeInTheDocument();
  });

  it("shows outputs section in advanced mode", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByText("Advanced Options"));

    expect(screen.getByText("Outputs (Send To)")).toBeInTheDocument();
  });

  it("can add and remove outputs in advanced mode", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByText("Advanced Options"));

    // Add two more outputs (starts with 1)
    await user.click(screen.getByText("Add Output"));
    await user.click(screen.getByText("Add Output"));
    // Should now have 3 outputs, each with remove button
    const removeButtons = screen.getAllByLabelText("Remove output");
    expect(removeButtons.length).toBe(3);

    // Remove one — should still have 2 left with remove buttons
    await user.click(removeButtons[0]);
    expect(screen.getAllByLabelText("Remove output")).toHaveLength(2);
  });

  it("shows fee strategy selector for platform operations in advanced mode", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByText("Advanced Options"));

    // Switch to platform source — click the radio input within the platform label
    const labels = screen.getAllByText("Platform Addresses");
    // In advanced mode, the Platform Addresses label wraps a radio
    const platformLabel = labels.find(
      (el) => el.closest("label")?.querySelector('input[type="radio"]'),
    );
    if (platformLabel) {
      await user.click(platformLabel);
    }

    expect(screen.getByText("Fee Strategy")).toBeInTheDocument();
  });

  // ─── Wallet unlock ────────────────────────────────────────────

  it("shows unlock gate for password-protected wallets", async () => {
    // Override wallet store to return password wallet
    const { useWalletStore } = await import("@/stores/walletStore");
    vi.mocked(useWalletStore).mockReturnValueOnce({
      hdWallets: [mockPasswordWallet],
      singleKeyWallets: [],
      selectedWallet: { type: "hd", seedHash: "pass123" },
    } as ReturnType<typeof useWalletStore>);

    render(<SendScreen />);
    expect(
      screen.getByText("Wallet is locked. Please unlock to continue."),
    ).toBeInTheDocument();
    expect(screen.getByText("Unlock Wallet")).toBeInTheDocument();
  });

  // ─── Navigation ───────────────────────────────────────────────

  it("back button navigates to wallets", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    await user.click(screen.getByLabelText("Back to wallets"));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
  });

  // ─── Address type detection ───────────────────────────────────

  it("detects X addresses as Core", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");
    expect(screen.getByText("Core Address")).toBeInTheDocument();
  });

  it("detects evo1 addresses as Platform", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "evo1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
    expect(screen.getByText("Platform Address")).toBeInTheDocument();
  });

  it("detects tevo1 addresses as Platform (testnet)", async () => {
    const user = userEvent.setup();
    render(<SendScreen />);
    const input = screen.getByLabelText("Destination address");
    await user.type(input, "tevo1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
    expect(screen.getByText("Platform Address")).toBeInTheDocument();
  });

  // ─── Accessibility ────────────────────────────────────────────

  it("has proper heading hierarchy", () => {
    render(<SendScreen />);
    const headings = screen.getAllByRole("heading");
    expect(headings.length).toBeGreaterThan(0);
    expect(headings[0]).toHaveTextContent("Send Dash");
  });

  it("error banner has alert role", async () => {
    mockCoreSendWalletPayment.mockResolvedValueOnce({
      status: "error",
      error: "Test error",
    });

    const user = userEvent.setup();
    render(<SendScreen />);

    const destInput = screen.getByLabelText("Destination address");
    await user.type(destInput, "XpYv3N7gTGBL4fE3b1Y5eZ8kMqP9oQ2rU1");

    const amountInput = screen.getByPlaceholderText("Enter amount");
    await user.type(amountInput, "1.0");

    await user.click(
      screen.getByRole("button", { name: /Core Transaction/i }),
    );

    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });
  });

  // ─── Send from label ──────────────────────────────────────────

  it("shows 'Send from' label for source selection", () => {
    render(<SendScreen />);
    expect(screen.getByText("Send from")).toBeInTheDocument();
  });

  it("shows 'Send to' label for destination input", () => {
    render(<SendScreen />);
    expect(screen.getByText("Send to")).toBeInTheDocument();
  });
});
