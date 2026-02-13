import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { WithdrawScreen, type WithdrawStatus } from "./WithdrawScreen";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 1,
    keyType: "ECDSA_SECP256K1",
    purpose: "TRANSFER",
    securityLevel: "CRITICAL",
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: true,
    ...overrides,
  };
}

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aabbccdd11223344556677889900aabb",
    identityType: "user",
    alias: "Alice",
    balance: 1_000_000_000_000, // 10 DASH in credits
    keys: [makeKey()],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

const defaultProps = {
  identity: makeIdentity(),
  status: { type: "form" } as WithdrawStatus,
  onSubmit: vi.fn(),
  onDismissError: vi.fn(),
  onBack: vi.fn(),
  onViewKey: vi.fn(),
  onAddKey: vi.fn(),
  estimatedFee: null,
};

function setup(props: Partial<Parameters<typeof WithdrawScreen>[0]> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <WithdrawScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("WithdrawScreen — header", () => {
  it("renders title", () => {
    setup();
    expect(screen.getByText("Withdraw Funds")).toBeInTheDocument();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /go back/i }));
    expect(props.onBack).toHaveBeenCalledOnce();
  });

  it("shows advanced options toggle", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /show advanced/i }),
    ).toBeInTheDocument();
  });
});

// ─── Form fields ────────────────────────────────────────────────────

describe("WithdrawScreen — form fields", () => {
  it("renders step 1 amount section", () => {
    setup();
    expect(
      screen.getByText("1. Amount to withdraw (DASH)"),
    ).toBeInTheDocument();
  });

  it("renders step 2 address section", () => {
    setup();
    expect(
      screen.getByText("2. Dash address to withdraw to"),
    ).toBeInTheDocument();
  });

  it("renders identity alias in from label", () => {
    setup();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("renders truncated ID when no alias", () => {
    setup({ identity: makeIdentity({ alias: null }) });
    expect(screen.getByText(/aabbccdd…/)).toBeInTheDocument();
  });

  it("renders available balance", () => {
    setup();
    expect(screen.getByText(/10\.00000000 DASH/)).toBeInTheDocument();
  });

  it("renders identity type badge", () => {
    setup();
    expect(screen.getByText("User")).toBeInTheDocument();
  });

  it("renders amount input", () => {
    setup();
    expect(screen.getByText("Amount:")).toBeInTheDocument();
  });

  it("renders Max button", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /set maximum/i }),
    ).toBeInTheDocument();
  });

  it("renders address input", () => {
    setup();
    expect(screen.getByLabelText("Address:")).toBeInTheDocument();
  });

  it("renders withdraw button", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeInTheDocument();
  });
});

// ─── Advanced options ───────────────────────────────────────────────

describe("WithdrawScreen — advanced options", () => {
  it("does not show key selector by default", () => {
    setup();
    expect(
      screen.queryByText("3. Select the key to sign with"),
    ).not.toBeInTheDocument();
  });

  it("shows key selector when advanced toggled", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /show advanced/i }),
    );
    expect(
      screen.getByText("3. Select the key to sign with"),
    ).toBeInTheDocument();
  });

  it("hides key selector when advanced toggled off", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /show advanced/i }),
    );
    expect(
      screen.getByText("3. Select the key to sign with"),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: /hide advanced/i }),
    );
    expect(
      screen.queryByText("3. Select the key to sign with"),
    ).not.toBeInTheDocument();
  });
});

// ─── Max button ─────────────────────────────────────────────────────

describe("WithdrawScreen — Max button", () => {
  it("fills amount with max value when clicked", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /set maximum/i }),
    );
    // 1T - 500M = 999.5B credits → 999,500,000 duffs = 9.99500000 DASH
    const input = screen.getByPlaceholderText("0.00000000");
    expect(input).toHaveValue("9.99500000");
  });
});

// ─── No keys available ──────────────────────────────────────────────

describe("WithdrawScreen — no withdrawal keys", () => {
  it("shows no keys message", () => {
    setup({ identity: makeIdentity({ keys: [] }) });
    expect(
      screen.getByText(/do not have any withdrawal keys/i),
    ).toBeInTheDocument();
  });

  it("shows Add key button", () => {
    setup({ identity: makeIdentity({ keys: [] }) });
    expect(
      screen.getByRole("button", { name: /add key/i }),
    ).toBeInTheDocument();
  });

  it("calls onAddKey when Add key clicked", async () => {
    const { user, props } = setup({
      identity: makeIdentity({ keys: [] }),
    });
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(props.onAddKey).toHaveBeenCalledOnce();
  });

  it("shows Check Owner Key when owner key exists without private key", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ purpose: "OWNER", hasPrivateKey: false })],
      }),
    });
    expect(
      screen.getByRole("button", { name: /check owner key/i }),
    ).toBeInTheDocument();
  });

  it("calls onViewKey when Check Owner Key clicked", async () => {
    const { user, props } = setup({
      identity: makeIdentity({
        keys: [
          makeKey({ keyId: 5, purpose: "OWNER", hasPrivateKey: false }),
        ],
      }),
    });
    await user.click(
      screen.getByRole("button", { name: /check owner key/i }),
    );
    expect(props.onViewKey).toHaveBeenCalledWith(5);
  });

  it("shows identity type in no keys message", () => {
    setup({
      identity: makeIdentity({
        identityType: "masternode",
        keys: [],
      }),
    });
    expect(screen.getByText(/masternode/i)).toBeInTheDocument();
  });
});

// ─── Withdraw button readiness ──────────────────────────────────────

describe("WithdrawScreen — withdraw button state", () => {
  it("is disabled when no amount entered", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeDisabled();
  });

  it("is disabled when no address entered (for non-owner key)", async () => {
    const { user } = setup();
    const amountInput = screen.getByPlaceholderText("0.00000000");
    await user.type(amountInput, "1.0");
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeDisabled();
  });

  it("is enabled when amount and address are provided", async () => {
    const { user } = setup();
    const amountInput = screen.getByPlaceholderText("0.00000000");
    await user.type(amountInput, "1.0");
    const addressInput = screen.getByLabelText("Address:");
    await user.type(addressInput, "yWLKdRzBCPjmTMVBkyPrNbjuq6ks1VDMqG");
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeEnabled();
  });

  it("is disabled during sending", () => {
    setup({
      status: { type: "sending", startedAt: Date.now() },
    });
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeDisabled();
  });
});

// ─── Confirmation dialog ────────────────────────────────────────────

describe("WithdrawScreen — confirmation dialog", () => {
  it("shows confirmation when withdraw clicked", async () => {
    const { user } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.type(
      screen.getByLabelText("Address:"),
      "yWLKdRzBCPjmTMVBkyPrNbjuq6ks1VDMqG",
    );
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    expect(screen.getByText("Confirm Withdrawal")).toBeInTheDocument();
  });

  it("shows amount and address in confirmation", async () => {
    const { user } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.type(
      screen.getByLabelText("Address:"),
      "yTestAddress123",
    );
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    expect(
      screen.getByText(/1\.00000000 DASH/),
    ).toBeInTheDocument();
    expect(screen.getByText(/yTestAddress123/)).toBeInTheDocument();
  });

  it("calls onSubmit when confirmed", async () => {
    const { user, props } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.type(
      screen.getByLabelText("Address:"),
      "yTestAddr",
    );
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    await user.click(screen.getByRole("button", { name: /confirm/i }));
    expect(props.onSubmit).toHaveBeenCalledWith({
      identityId: "aabbccdd11223344556677889900aabb",
      toAddress: "yTestAddr",
      credits: 100_000_000_000, // 1.0 DASH
      keyId: 1,
    });
  });

  it("does not call onSubmit when canceled", async () => {
    const { user, props } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.type(screen.getByLabelText("Address:"), "yAddr");
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    await user.click(screen.getByRole("button", { name: /cancel/i }));
    expect(props.onSubmit).not.toHaveBeenCalled();
  });
});

// ─── Owner key behavior ─────────────────────────────────────────────

describe("WithdrawScreen — owner key", () => {
  it("allows empty address with owner key selected", async () => {
    const identity = makeIdentity({
      keys: [makeKey({ purpose: "OWNER", keyId: 2 })],
    });
    const { user } = setup({ identity });
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    // No address entered — should still be enabled with OWNER key
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeEnabled();
  });

  it("shows payout address hint for owner key", () => {
    const identity = makeIdentity({
      keys: [makeKey({ purpose: "OWNER", keyId: 2 })],
    });
    setup({ identity });
    expect(
      screen.getByText(/masternode payout address/i),
    ).toBeInTheDocument();
  });

  it("sends null address when owner key with no address", async () => {
    const identity = makeIdentity({
      keys: [makeKey({ purpose: "OWNER", keyId: 2 })],
    });
    const { user, props } = setup({ identity });
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    await user.click(screen.getByRole("button", { name: /confirm/i }));
    expect(props.onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ toAddress: null }),
    );
  });
});

// ─── Fee estimation ─────────────────────────────────────────────────

describe("WithdrawScreen — fee estimation", () => {
  it("does not show fee when not provided", () => {
    setup();
    expect(screen.queryByText(/estimated fee/i)).not.toBeInTheDocument();
  });

  it("shows estimated fee when provided", () => {
    setup({ estimatedFee: 50_000_000 });
    expect(screen.getByText(/estimated fee/i)).toBeInTheDocument();
    expect(screen.getByText(/0\.00050000 DASH/)).toBeInTheDocument();
  });
});

// ─── Error state ────────────────────────────────────────────────────

describe("WithdrawScreen — error state", () => {
  it("shows error message", () => {
    setup({
      status: { type: "error", message: "Insufficient funds for operation" },
    });
    // InlineError translates raw messages via translateError()
    expect(
      screen.getByText("Insufficient funds for this operation."),
    ).toBeInTheDocument();
  });

  it("shows dismiss button", () => {
    setup({
      status: { type: "error", message: "Some error" },
    });
    expect(
      screen.getByRole("button", { name: /dismiss/i }),
    ).toBeInTheDocument();
  });

  it("calls onDismissError when dismiss clicked", async () => {
    const { user, props } = setup({
      status: { type: "error", message: "Some error" },
    });
    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(props.onDismissError).toHaveBeenCalledOnce();
  });
});

// ─── Sending state ──────────────────────────────────────────────────

describe("WithdrawScreen — sending state", () => {
  it("shows sending indicator", () => {
    setup({
      status: { type: "sending", startedAt: Date.now() },
    });
    expect(screen.getByText(/withdrawing…/i)).toBeInTheDocument();
  });

  it("disables form controls during sending", () => {
    setup({
      status: { type: "sending", startedAt: Date.now() },
    });
    expect(screen.getByPlaceholderText("0.00000000")).toBeDisabled();
    expect(screen.getByLabelText("Address:")).toBeDisabled();
  });
});

// ─── Success state ──────────────────────────────────────────────────

describe("WithdrawScreen — success state", () => {
  it("shows success message", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByText("Withdrawal Successful!"),
    ).toBeInTheDocument();
  });

  it("shows note about Core chain delay", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByText(/may take a few minutes/i),
    ).toBeInTheDocument();
  });

  it("shows back button", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByRole("button", { name: /back to identities/i }),
    ).toBeInTheDocument();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /back to identities/i }),
    );
    expect(props.onBack).toHaveBeenCalledOnce();
  });
});

// ─── Multiple keys ──────────────────────────────────────────────────

describe("WithdrawScreen — multiple keys", () => {
  it("auto-selects first transfer key", async () => {
    const identity = makeIdentity({
      keys: [
        makeKey({ keyId: 3, purpose: "TRANSFER" }),
        makeKey({ keyId: 7, purpose: "TRANSFER" }),
      ],
    });
    const { user, props } = setup({ identity });
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.type(screen.getByLabelText("Address:"), "yAddr");
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    await user.click(screen.getByRole("button", { name: /confirm/i }));
    expect(props.onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({ keyId: 3 }),
    );
  });

  it("excludes keys without private keys", () => {
    const identity = makeIdentity({
      keys: [
        makeKey({ keyId: 1, purpose: "TRANSFER", hasPrivateKey: false }),
        makeKey({ keyId: 2, purpose: "TRANSFER", hasPrivateKey: true }),
      ],
    });
    setup({ identity });
    // Should not show the "no keys" message since key 2 is available
    expect(
      screen.queryByText(/do not have any withdrawal keys/i),
    ).not.toBeInTheDocument();
  });

  it("excludes disabled keys", () => {
    const identity = makeIdentity({
      keys: [
        makeKey({ keyId: 1, purpose: "TRANSFER", isDisabled: true }),
      ],
    });
    setup({ identity });
    expect(
      screen.getByText(/do not have any withdrawal keys/i),
    ).toBeInTheDocument();
  });
});

// ─── Wallet unlock gate ──────────────────────────────────────────────

describe("WithdrawScreen — wallet unlock gate", () => {
  it("shows wallet locked warning when walletLocked is true", () => {
    setup({ walletLocked: true });
    expect(screen.getByTestId("wallet-locked-gate")).toBeInTheDocument();
    expect(
      screen.getByText(/wallet is locked/i),
    ).toBeInTheDocument();
  });

  it("shows unlock button when onRequestUnlock is provided", () => {
    setup({ walletLocked: true, onRequestUnlock: vi.fn() });
    expect(
      screen.getByRole("button", { name: /unlock wallet/i }),
    ).toBeInTheDocument();
  });

  it("calls onRequestUnlock when unlock button clicked", async () => {
    const onRequestUnlock = vi.fn();
    const { user } = setup({ walletLocked: true, onRequestUnlock });
    await user.click(
      screen.getByRole("button", { name: /unlock wallet/i }),
    );
    expect(onRequestUnlock).toHaveBeenCalledOnce();
  });

  it("does not show wallet locked warning when walletLocked is false", () => {
    setup({ walletLocked: false });
    expect(screen.queryByTestId("wallet-locked-gate")).not.toBeInTheDocument();
  });

  it("disables withdraw button when wallet is locked", () => {
    setup({ walletLocked: true });
    expect(
      screen.getByRole("button", { name: /withdraw/i }),
    ).toBeDisabled();
  });

  it("disables amount input when wallet is locked", () => {
    setup({ walletLocked: true });
    expect(screen.getByPlaceholderText("0.00000000")).toBeDisabled();
  });
});
