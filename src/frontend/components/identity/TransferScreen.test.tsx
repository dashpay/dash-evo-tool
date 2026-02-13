import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  TransferScreen,
  type TransferStatus,
} from "./TransferScreen";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";
import type { IdentityOption } from "@/components/shared/IdentitySelector";
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

const knownIdentities: IdentityOption[] = [
  { id: "bbccddee11223344556677889900aabb", displayName: "Bob" },
  { id: "ccddee1122334455667788990011aabb", displayName: "Charlie" },
];

const defaultProps = {
  identity: makeIdentity(),
  status: { type: "form" } as TransferStatus,
  knownIdentities,
  onSubmitToIdentity: vi.fn(),
  onSubmitToAddress: vi.fn(),
  onDismissError: vi.fn(),
  onBack: vi.fn(),
  onViewKey: vi.fn(),
  onAddKey: vi.fn(),
  estimatedFeeIdentity: null,
  estimatedFeeAddress: null,
};

function setup(props: Partial<Parameters<typeof TransferScreen>[0]> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <TransferScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("TransferScreen — header", () => {
  it("renders title", () => {
    setup();
    expect(screen.getByText("Transfer Funds")).toBeInTheDocument();
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

describe("TransferScreen — form fields", () => {
  it("renders step 1 amount section", () => {
    setup();
    expect(
      screen.getByText("1. Input the amount to transfer"),
    ).toBeInTheDocument();
  });

  it("renders step 2 destination type section", () => {
    setup();
    expect(
      screen.getByText("2. Select transfer destination type"),
    ).toBeInTheDocument();
  });

  it("renders step 3 identity input by default", () => {
    setup();
    expect(
      screen.getByText("3. ID of the identity to transfer to"),
    ).toBeInTheDocument();
  });

  it("renders identity alias", () => {
    setup();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("renders available balance", () => {
    setup();
    expect(screen.getByText(/10\.00000000 DASH/)).toBeInTheDocument();
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

  it("renders destination type buttons", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /^identity$/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /platform address/i }),
    ).toBeInTheDocument();
  });

  it("renders transfer button", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /transfer/i }),
    ).toBeInTheDocument();
  });
});

// ─── Destination type switching ─────────────────────────────────────

describe("TransferScreen — destination type", () => {
  it("defaults to Identity destination", () => {
    setup();
    const identityBtn = screen.getByRole("button", { name: /^identity$/i });
    expect(identityBtn).toHaveAttribute("aria-pressed", "true");
  });

  it("switches to Platform Address when clicked", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    expect(
      screen.getByText("3. Platform address to transfer to"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Platform Address:")).toBeInTheDocument();
  });

  it("switches back to Identity when clicked", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    await user.click(
      screen.getByRole("button", { name: /^identity$/i }),
    );
    expect(
      screen.getByText("3. ID of the identity to transfer to"),
    ).toBeInTheDocument();
  });

  it("shows platform address input with correct placeholder", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    expect(
      screen.getByPlaceholderText(/evo1/),
    ).toBeInTheDocument();
  });
});

// ─── Advanced options ───────────────────────────────────────────────

describe("TransferScreen — advanced options", () => {
  it("does not show key selector by default", () => {
    setup();
    expect(
      screen.queryByText("4. Select the key to sign the transaction with"),
    ).not.toBeInTheDocument();
  });

  it("shows key selector when advanced toggled", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /show advanced/i }),
    );
    expect(
      screen.getByText("4. Select the key to sign the transaction with"),
    ).toBeInTheDocument();
  });
});

// ─── Max button ─────────────────────────────────────────────────────

describe("TransferScreen — Max button", () => {
  it("fills amount with max value when clicked", async () => {
    const { user } = setup();
    await user.click(
      screen.getByRole("button", { name: /set maximum/i }),
    );
    // 1T - 20M = 999.98B credits → 999,980,000 duffs = 9.99980000 DASH
    const input = screen.getByPlaceholderText("0.00000000");
    expect(input).toHaveValue("9.99980000");
  });
});

// ─── No keys available ──────────────────────────────────────────────

describe("TransferScreen — no transfer keys", () => {
  it("shows no keys message", () => {
    setup({ identity: makeIdentity({ keys: [] }) });
    expect(
      screen.getByText(/do not have any transfer keys/i),
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

  it("shows Check Transfer Key when transfer key exists without private key", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ purpose: "TRANSFER", hasPrivateKey: false })],
      }),
    });
    expect(
      screen.getByRole("button", { name: /check transfer key/i }),
    ).toBeInTheDocument();
  });

  it("calls onViewKey when Check Transfer Key clicked", async () => {
    const { user, props } = setup({
      identity: makeIdentity({
        keys: [
          makeKey({ keyId: 5, purpose: "TRANSFER", hasPrivateKey: false }),
        ],
      }),
    });
    await user.click(
      screen.getByRole("button", { name: /check transfer key/i }),
    );
    expect(props.onViewKey).toHaveBeenCalledWith(5);
  });

  it("shows identity type in no keys message", () => {
    setup({
      identity: makeIdentity({
        identityType: "evonode",
        keys: [],
      }),
    });
    expect(screen.getByText(/evonode/i)).toBeInTheDocument();
  });
});

// ─── Transfer button readiness ──────────────────────────────────────

describe("TransferScreen — transfer button state", () => {
  it("is disabled when no amount entered", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /transfer/i }),
    ).toBeDisabled();
  });

  it("is disabled when no destination entered", async () => {
    const { user } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    expect(
      screen.getByRole("button", { name: /transfer/i }),
    ).toBeDisabled();
  });

  it("is disabled during sending", () => {
    setup({
      status: { type: "sending", startedAt: Date.now() },
    });
    expect(
      screen.getByRole("button", { name: /transfer/i }),
    ).toBeDisabled();
  });
});

// ─── Identity transfer confirmation ─────────────────────────────────

describe("TransferScreen — identity transfer confirmation", () => {
  it("shows confirmation dialog when transfer clicked", async () => {
    const { user } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    // Type in the text input for identity ID (via "Other" mode)
    const identityInput = screen.getByPlaceholderText("Enter identity ID");
    await user.type(identityInput, "deadbeef1234");
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    expect(screen.getByText("Confirm Transfer")).toBeInTheDocument();
  });

  it("calls onSubmitToIdentity when confirmed", async () => {
    const { user, props } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "2.0");
    const identityInput = screen.getByPlaceholderText("Enter identity ID");
    await user.type(identityInput, "deadbeef1234");
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    await user.click(screen.getByRole("button", { name: /confirm/i }));
    expect(props.onSubmitToIdentity).toHaveBeenCalledWith({
      fromIdentityId: "aabbccdd11223344556677889900aabb",
      toIdentityId: "deadbeef1234",
      credits: 200_000_000_000,
      keyId: 1,
    });
  });

  it("does not call submit when canceled", async () => {
    const { user, props } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    const identityInput = screen.getByPlaceholderText("Enter identity ID");
    await user.type(identityInput, "deadbeef");
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    await user.click(screen.getByRole("button", { name: /cancel/i }));
    expect(props.onSubmitToIdentity).not.toHaveBeenCalled();
  });
});

// ─── Platform address transfer ──────────────────────────────────────

describe("TransferScreen — platform address transfer", () => {
  it("shows confirmation with platform address title", async () => {
    const { user } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    const addrInput = screen.getByLabelText("Platform Address:");
    await user.type(addrInput, "tevo1abc123");
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    expect(
      screen.getByText("Confirm Transfer to Platform Address"),
    ).toBeInTheDocument();
  });

  it("calls onSubmitToAddress when confirmed", async () => {
    const { user, props } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "3.0");
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    const addrInput = screen.getByLabelText("Platform Address:");
    await user.type(addrInput, "tevo1test");
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    await user.click(screen.getByRole("button", { name: /confirm/i }));
    expect(props.onSubmitToAddress).toHaveBeenCalledWith({
      identityId: "aabbccdd11223344556677889900aabb",
      address: "tevo1test",
      credits: 300_000_000_000,
      keyId: 1,
    });
  });

  it("shows address in confirmation message", async () => {
    const { user } = setup();
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    await user.type(
      screen.getByLabelText("Platform Address:"),
      "evo1myaddr",
    );
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    expect(screen.getByText(/evo1myaddr/)).toBeInTheDocument();
  });
});

// ─── Fee estimation ─────────────────────────────────────────────────

describe("TransferScreen — fee estimation", () => {
  it("does not show fee when not provided", () => {
    setup();
    expect(screen.queryByText(/estimated fee/i)).not.toBeInTheDocument();
  });

  it("shows identity transfer fee", () => {
    setup({ estimatedFeeIdentity: 10_000_000 });
    expect(screen.getByText(/estimated fee/i)).toBeInTheDocument();
    expect(screen.getByText(/0\.00010000 DASH/)).toBeInTheDocument();
  });

  it("shows address transfer fee when platform address selected", async () => {
    const { user } = setup({ estimatedFeeAddress: 25_000_000 });
    await user.click(
      screen.getByRole("button", { name: /platform address/i }),
    );
    expect(screen.getByText(/0\.00025000 DASH/)).toBeInTheDocument();
  });
});

// ─── Error state ────────────────────────────────────────────────────

describe("TransferScreen — error state", () => {
  it("shows error message", () => {
    setup({
      status: { type: "error", message: "Invalid identifier format" },
    });
    expect(
      screen.getByText("Invalid identifier format"),
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

describe("TransferScreen — sending state", () => {
  it("shows sending indicator", () => {
    setup({
      status: { type: "sending", startedAt: Date.now() },
    });
    expect(screen.getByText(/transferring…/i)).toBeInTheDocument();
  });

  it("disables amount input during sending", () => {
    setup({
      status: { type: "sending", startedAt: Date.now() },
    });
    expect(screen.getByPlaceholderText("0.00000000")).toBeDisabled();
  });
});

// ─── Success state ──────────────────────────────────────────────────

describe("TransferScreen — success state", () => {
  it("shows success message", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByText("Transfer Successful!"),
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

describe("TransferScreen — multiple keys", () => {
  it("auto-selects first transfer key", async () => {
    const identity = makeIdentity({
      keys: [
        makeKey({ keyId: 4, purpose: "TRANSFER" }),
        makeKey({ keyId: 9, purpose: "TRANSFER" }),
      ],
    });
    const { user, props } = setup({ identity });
    await user.type(screen.getByPlaceholderText("0.00000000"), "1.0");
    const identityInput = screen.getByPlaceholderText("Enter identity ID");
    await user.type(identityInput, "deadbeef");
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    await user.click(screen.getByRole("button", { name: /confirm/i }));
    expect(props.onSubmitToIdentity).toHaveBeenCalledWith(
      expect.objectContaining({ keyId: 4 }),
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
    expect(
      screen.queryByText(/do not have any transfer keys/i),
    ).not.toBeInTheDocument();
  });

  it("only shows TRANSFER keys (not OWNER, AUTH, etc.)", () => {
    const identity = makeIdentity({
      keys: [
        makeKey({ keyId: 1, purpose: "AUTHENTICATION", hasPrivateKey: true }),
        makeKey({ keyId: 2, purpose: "OWNER", hasPrivateKey: true }),
      ],
    });
    setup({ identity });
    expect(
      screen.getByText(/do not have any transfer keys/i),
    ).toBeInTheDocument();
  });
});

// ─── Wallet unlock gate ──────────────────────────────────────────────

describe("TransferScreen — wallet unlock gate", () => {
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

  it("disables transfer button when wallet is locked", () => {
    setup({ walletLocked: true });
    expect(
      screen.getByRole("button", { name: /transfer/i }),
    ).toBeDisabled();
  });

  it("disables amount input when wallet is locked", () => {
    setup({ walletLocked: true });
    expect(screen.getByPlaceholderText("0.00000000")).toBeDisabled();
  });
});
