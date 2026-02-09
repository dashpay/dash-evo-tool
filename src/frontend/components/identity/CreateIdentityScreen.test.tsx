import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach, beforeAll } from "vitest";
import {
  CreateIdentityScreen,
  type CreateIdentityScreenProps,
} from "./CreateIdentityScreen";
import type { WalletDto, AssetLockDto, PlatformAddressDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Polyfills for Radix in jsdom ───────────────────────────────────

beforeAll(() => {
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
});

// ─── Test fixtures ─────────────────────────────────────────────────

function makeAssetLock(
  overrides: Partial<AssetLockDto> = {},
): AssetLockDto {
  return {
    txid: "aabb001122334455667788990011223344556677889900112233445566778899",
    address: "yXj1K9pVm1RqdV9kGtPGdK2Jf5QYtEdn7V",
    amount: 100_000_000, // 1 DASH
    hasInstantLock: true,
    hasAssetLockProof: true,
    proofDetails: null,
    proofHex: "deadbeef",
    ...overrides,
  };
}

function makePlatformAddress(
  overrides: Partial<PlatformAddressDto> = {},
): PlatformAddressDto {
  return {
    address: "yd5KMT7jNPknNPqCjFqGiScQ9pVL5TQvvA",
    balance: 500_000_000, // 5 DASH
    ...overrides,
  };
}

function makeWallet(overrides: Partial<WalletDto> = {}): WalletDto {
  return {
    seedHash: "ab12cd34ef56789012345678901234567890abcdef1234567890abcdef123456",
    usesPassword: false,
    alias: "My Wallet",
    isMain: true,
    confirmedBalance: 5_000_000_000,
    unconfirmedBalance: 0,
    totalBalance: 5_000_000_000,
    addresses: [],
    transactions: [],
    unusedAssetLocks: [makeAssetLock()],
    platformAddresses: [makePlatformAddress()],
    identityIndexes: [0, 1],
    passwordHint: null,
    ...overrides,
  };
}

function makeWallet2(): WalletDto {
  return makeWallet({
    seedHash: "bb22dd44ff66789012345678901234567890abcdef1234567890abcdef654321",
    alias: "Backup Wallet",
    isMain: false,
    confirmedBalance: 2_000_000_000,
    unconfirmedBalance: 0,
    totalBalance: 2_000_000_000,
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [],
  });
}

/** Wallet with no asset locks, so default funding = walletBalance */
function makeWalletNoLocks(): WalletDto {
  return makeWallet({
    unusedAssetLocks: [],
    platformAddresses: [],
  });
}

const defaultProps: CreateIdentityScreenProps = {
  wallets: [makeWallet()],
  status: { type: "form" },
  onSubmit: vi.fn(),
  onDismissError: vi.fn(),
  onBack: vi.fn(),
  onBackToIdentities: vi.fn(),
  onRegisterDpns: vi.fn(),
  onCopy: vi.fn(),
  qrReceiveAddress: null,
  qrFundsReceived: false,
};

function setup(props: Partial<CreateIdentityScreenProps> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <CreateIdentityScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ──────────────────────────────────────────────────────────

describe("CreateIdentityScreen — header", () => {
  it("renders title", () => {
    setup();
    expect(screen.getByText("Create New Identity")).toBeInTheDocument();
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

  it("toggles advanced options on click", async () => {
    const { user } = setup();
    const toggleBtn = screen.getByRole("button", {
      name: /show advanced/i,
    });
    await user.click(toggleBtn);
    expect(
      screen.getByRole("button", { name: /hide advanced/i }),
    ).toBeInTheDocument();
  });
});

// ─── No wallets state ────────────────────────────────────────────────

describe("CreateIdentityScreen — no wallets", () => {
  it("shows no wallets message when wallets array is empty", () => {
    setup({ wallets: [] });
    expect(
      screen.getByText(/you need at least one hd wallet/i),
    ).toBeInTheDocument();
  });

  it("has navigable Go Back button when no wallets", () => {
    setup({ wallets: [] });
    // Two "go back" buttons: header + body. Both should work.
    const buttons = screen.getAllByRole("button", { name: /go back/i });
    expect(buttons.length).toBeGreaterThanOrEqual(1);
  });
});

// ─── Wallet selection (single wallet) ────────────────────────────────

describe("CreateIdentityScreen — single wallet", () => {
  it("does not show wallet selector when only one wallet", () => {
    setup();
    expect(screen.queryByText("1. Select Wallet")).not.toBeInTheDocument();
  });
});

// ─── Wallet selection (multiple wallets) ─────────────────────────────

describe("CreateIdentityScreen — multiple wallets", () => {
  const twoWallets = [makeWallet(), makeWallet2()];

  it("shows wallet selector when multiple wallets", () => {
    setup({ wallets: twoWallets });
    expect(screen.getByText("1. Select Wallet")).toBeInTheDocument();
  });

  it("shows wallet selector combobox", () => {
    setup({ wallets: twoWallets });
    expect(
      screen.getByRole("combobox", { name: /select wallet/i }),
    ).toBeInTheDocument();
  });
});

// ─── Alias input ─────────────────────────────────────────────────────

describe("CreateIdentityScreen — alias", () => {
  it("renders alias input", () => {
    setup();
    expect(
      screen.getByRole("textbox", { name: /identity alias/i }),
    ).toBeInTheDocument();
  });

  it("allows typing an alias", async () => {
    const { user } = setup();
    const input = screen.getByRole("textbox", { name: /identity alias/i });
    await user.type(input, "My Test Identity");
    expect(input).toHaveValue("My Test Identity");
  });
});

// ─── Funding method selection ────────────────────────────────────────

describe("CreateIdentityScreen — funding method", () => {
  it("renders funding method selector", () => {
    setup();
    expect(
      screen.getByRole("combobox", { name: /funding method/i }),
    ).toBeInTheDocument();
  });
});

// ─── Asset lock funding ──────────────────────────────────────────────

describe("CreateIdentityScreen — asset lock funding", () => {
  it("shows asset lock list with txid and amount", () => {
    setup();
    const lockEl = screen.getByTestId("asset-lock-aabb0011");
    expect(lockEl).toBeInTheDocument();
    expect(lockEl).toHaveTextContent("1.00000000 DASH");
  });

  it("shows InstantLock badge for locks with instant lock", () => {
    setup();
    expect(screen.getByText("InstantLock")).toBeInTheDocument();
  });

  it("selects asset lock on click", async () => {
    const { user } = setup();
    const lockEl = screen.getByTestId("asset-lock-aabb0011");
    await user.click(lockEl);
    expect(screen.getByText("Selected")).toBeInTheDocument();
  });

  it("falls back to wallet balance when all locks lack proofs", () => {
    const wallet = makeWallet({
      unusedAssetLocks: [makeAssetLock({ hasAssetLockProof: false })],
      platformAddresses: [],
    });
    setup({ wallets: [wallet] });
    // Asset lock method unavailable — falls back to wallet balance
    // The amount input for wallet balance funding should be visible
    expect(screen.getByText("Amount (DASH):")).toBeInTheDocument();
  });
});

// ─── Wallet balance funding (wallet without asset locks defaults to walletBalance) ──

describe("CreateIdentityScreen — wallet balance funding", () => {
  it("shows wallet balance display when no asset locks", () => {
    setup({ wallets: [makeWalletNoLocks()] });
    // The "Wallet Balance:" label appears in the funding detail panel
    expect(screen.getByText("50.00000000 DASH")).toBeInTheDocument();
  });

  it("shows amount input for wallet balance funding", () => {
    setup({ wallets: [makeWalletNoLocks()] });
    expect(screen.getByText("Amount (DASH):")).toBeInTheDocument();
  });

  it("shows Max button for wallet balance", () => {
    setup({ wallets: [makeWalletNoLocks()] });
    expect(
      screen.getByRole("button", { name: /set maximum amount/i }),
    ).toBeInTheDocument();
  });
});

// ─── QR code funding (switch via Select) ─────────────────────────────

describe("CreateIdentityScreen — QR code funding", () => {
  it("shows QR code funding with address and amount", async () => {
    const { user } = setup({
      wallets: [makeWalletNoLocks()],
      qrReceiveAddress: "yTestAddress123456789",
    });
    // Switch to QR code method
    const selector = screen.getByRole("combobox", {
      name: /funding method/i,
    });
    await user.click(selector);
    const option = screen.getByRole("option", {
      name: /address with qr code/i,
    });
    await user.click(option);
    // Enter amount
    const amountInput = screen.getByPlaceholderText(
      "Enter amount (e.g., 0.5)",
    );
    await user.type(amountInput, "1.0");
    expect(screen.getByTestId("qr-code-placeholder")).toBeInTheDocument();
  });

  it("shows funds received message", async () => {
    const { user } = setup({
      wallets: [makeWalletNoLocks()],
      qrReceiveAddress: "yTestAddr",
      qrFundsReceived: true,
    });
    const selector = screen.getByRole("combobox", {
      name: /funding method/i,
    });
    await user.click(selector);
    const option = screen.getByRole("option", {
      name: /address with qr code/i,
    });
    await user.click(option);
    expect(screen.getByText(/funds received/i)).toBeInTheDocument();
  });

  it("calls onCopy when copy button clicked", async () => {
    const { user, props } = setup({
      wallets: [makeWalletNoLocks()],
      qrReceiveAddress: "yTestAddr",
    });
    const selector = screen.getByRole("combobox", {
      name: /funding method/i,
    });
    await user.click(selector);
    const option = screen.getByRole("option", {
      name: /address with qr code/i,
    });
    await user.click(option);
    const amountInput = screen.getByPlaceholderText(
      "Enter amount (e.g., 0.5)",
    );
    await user.type(amountInput, "2.0");
    await user.click(
      screen.getByRole("button", { name: /copy payment uri/i }),
    );
    expect(props.onCopy).toHaveBeenCalledOnce();
    expect(props.onCopy).toHaveBeenCalledWith(
      "yTestAddr?amount=2.00000000",
    );
  });
});

// ─── Platform address funding ────────────────────────────────────────

describe("CreateIdentityScreen — platform address funding", () => {
  it("shows platform address selector after switching funding method", async () => {
    const { user } = setup();
    const selector = screen.getByRole("combobox", {
      name: /funding method/i,
    });
    await user.click(selector);
    const option = screen.getByRole("option", { name: /platform address/i });
    await user.click(option);
    expect(
      screen.getByRole("combobox", { name: /platform address/i }),
    ).toBeInTheDocument();
  });
});

// ─── Advanced options ────────────────────────────────────────────────

describe("CreateIdentityScreen — advanced options", () => {
  async function setupAdvanced() {
    const result = setup();
    await result.user.click(
      screen.getByRole("button", { name: /show advanced/i }),
    );
    return result;
  }

  it("shows identity index selector in advanced mode", async () => {
    await setupAdvanced();
    expect(screen.getByText("Identity Index")).toBeInTheDocument();
  });

  it("shows master key type selector in advanced mode", async () => {
    await setupAdvanced();
    expect(screen.getByText("Master Key Type")).toBeInTheDocument();
  });

  it("shows key configuration in advanced mode", async () => {
    await setupAdvanced();
    expect(screen.getByText("Key Configuration")).toBeInTheDocument();
  });

  it("shows default key list by default", async () => {
    await setupAdvanced();
    expect(screen.getByText("Default keys:")).toBeInTheDocument();
    expect(
      screen.getByText(/Authentication — Critical/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Transfer — Critical/)).toBeInTheDocument();
  });

  it("shows identity index combobox", async () => {
    await setupAdvanced();
    expect(
      screen.getByRole("combobox", { name: /identity index/i }),
    ).toBeInTheDocument();
  });
});

// ─── Advanced key editor ─────────────────────────────────────────────

describe("CreateIdentityScreen — key editor", () => {
  async function setupKeyEditor() {
    const result = setup();
    await result.user.click(
      screen.getByRole("button", { name: /show advanced/i }),
    );
    const modeSelector = screen.getByRole("combobox", {
      name: /key mode/i,
    });
    await result.user.click(modeSelector);
    const advancedOption = screen.getByRole("option", {
      name: /advanced/i,
    });
    await result.user.click(advancedOption);
    return result;
  }

  it("shows key rows in advanced mode", async () => {
    await setupKeyEditor();
    expect(screen.getByTestId("key-row-0")).toBeInTheDocument();
    expect(screen.getByTestId("key-row-1")).toBeInTheDocument();
    expect(screen.getByTestId("key-row-2")).toBeInTheDocument();
    expect(screen.getByTestId("key-row-3")).toBeInTheDocument();
    expect(screen.getByTestId("key-row-4")).toBeInTheDocument();
  });

  it("allows adding a new key", async () => {
    const { user } = await setupKeyEditor();
    await user.click(screen.getByRole("button", { name: /add key/i }));
    expect(screen.getByTestId("key-row-5")).toBeInTheDocument();
  });

  it("allows removing a key", async () => {
    const { user } = await setupKeyEditor();
    const removeButtons = screen.getAllByRole("button", {
      name: /remove key/i,
    });
    await user.click(removeButtons[0]);
    expect(screen.queryByTestId("key-row-4")).not.toBeInTheDocument();
  });
});

// ─── Status displays ─────────────────────────────────────────────────

describe("CreateIdentityScreen — status displays", () => {
  it("shows error message with dismiss button", () => {
    setup({ status: { type: "error", message: "Registration failed" } });
    expect(screen.getByText("Registration failed")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /dismiss/i }),
    ).toBeInTheDocument();
  });

  it("calls onDismissError when dismiss clicked", async () => {
    const { user, props } = setup({
      status: { type: "error", message: "Registration failed" },
    });
    await user.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(props.onDismissError).toHaveBeenCalledOnce();
  });

  it("shows waiting for funds state", () => {
    setup({ status: { type: "waitingForFunds" } });
    expect(screen.getByText(/waiting for funds/i)).toBeInTheDocument();
  });

  it("shows waiting for asset lock state", () => {
    setup({
      status: { type: "waitingForAssetLock", startedAt: Date.now() },
    });
    expect(
      screen.getByText(/waiting for core chain to produce proof/i),
    ).toBeInTheDocument();
  });

  it("shows waiting for platform state", () => {
    setup({
      status: { type: "waitingForPlatform", startedAt: Date.now() },
    });
    expect(
      screen.getByText(/waiting for platform acknowledgement/i),
    ).toBeInTheDocument();
  });
});

// ─── Success screen ──────────────────────────────────────────────────

describe("CreateIdentityScreen — success screen", () => {
  it("shows success message", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByText("Identity Registered Successfully!"),
    ).toBeInTheDocument();
  });

  it("shows Back to Identities button", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByRole("button", { name: /back to identities/i }),
    ).toBeInTheDocument();
  });

  it("shows Register DPNS Name button", () => {
    setup({ status: { type: "success" } });
    expect(
      screen.getByRole("button", { name: /register dpns name/i }),
    ).toBeInTheDocument();
  });

  it("calls onBackToIdentities when Back to Identities clicked", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /back to identities/i }),
    );
    expect(props.onBackToIdentities).toHaveBeenCalledOnce();
  });

  it("calls onRegisterDpns when Register DPNS clicked", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /register dpns name/i }),
    );
    expect(props.onRegisterDpns).toHaveBeenCalledOnce();
  });
});

// ─── Create Identity button ──────────────────────────────────────────

describe("CreateIdentityScreen — create button", () => {
  it("shows Create Identity button in form state", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /create identity/i }),
    ).toBeInTheDocument();
  });

  it("disables Create Identity when no asset lock selected", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /create identity/i }),
    ).toBeDisabled();
  });

  it("enables Create Identity when asset lock is selected", async () => {
    const { user } = setup();
    const lockEl = screen.getByTestId("asset-lock-aabb0011");
    await user.click(lockEl);
    expect(
      screen.getByRole("button", { name: /create identity/i }),
    ).toBeEnabled();
  });

  it("calls onSubmit with correct params when asset lock selected", async () => {
    const { user, props } = setup();
    await user.click(screen.getByTestId("asset-lock-aabb0011"));
    await user.click(
      screen.getByRole("button", { name: /create identity/i }),
    );
    expect(props.onSubmit).toHaveBeenCalledOnce();
    const callArgs = (props.onSubmit as ReturnType<typeof vi.fn>).mock
      .calls[0][0];
    expect(callArgs.walletSeedHash).toBe(makeWallet().seedHash);
    expect(callArgs.identityIndex).toBe(2); // next available (0,1 used)
    expect(callArgs.useDefaultKeys).toBe(true);
    expect(callArgs.fundingMethod.method).toBe("useAssetLock");
  });

  it("hides Create Identity button when not in form state", () => {
    setup({
      status: { type: "waitingForPlatform", startedAt: Date.now() },
    });
    expect(
      screen.queryByRole("button", { name: /create identity/i }),
    ).not.toBeInTheDocument();
  });
});

// ─── Submit with wallet balance ──────────────────────────────────────

describe("CreateIdentityScreen — submit with wallet balance", () => {
  it("calls onSubmit with wallet balance funding method", async () => {
    const { user, props } = setup({ wallets: [makeWalletNoLocks()] });
    const amountInput = screen.getByPlaceholderText(
      "Enter amount (e.g., 0.1234)",
    );
    await user.type(amountInput, "1.5");
    await user.click(
      screen.getByRole("button", { name: /create identity/i }),
    );
    expect(props.onSubmit).toHaveBeenCalledOnce();
    const callArgs = (props.onSubmit as ReturnType<typeof vi.fn>).mock
      .calls[0][0];
    expect(callArgs.fundingMethod.method).toBe("fundWithWallet");
    expect(callArgs.fundingMethod.amountDuffs).toBe(150_000_000);
  });

  it("disables create button when amount exceeds balance", async () => {
    const { user } = setup({ wallets: [makeWalletNoLocks()] });
    const amountInput = screen.getByPlaceholderText(
      "Enter amount (e.g., 0.1234)",
    );
    await user.type(amountInput, "999.0");
    expect(
      screen.getByRole("button", { name: /create identity/i }),
    ).toBeDisabled();
  });
});

// ─── Alias in submit ─────────────────────────────────────────────────

describe("CreateIdentityScreen — alias in submit", () => {
  it("includes alias in submit params", async () => {
    const { user, props } = setup();
    const aliasInput = screen.getByRole("textbox", {
      name: /identity alias/i,
    });
    await user.type(aliasInput, "My New Identity");
    await user.click(screen.getByTestId("asset-lock-aabb0011"));
    await user.click(
      screen.getByRole("button", { name: /create identity/i }),
    );
    const callArgs = (props.onSubmit as ReturnType<typeof vi.fn>).mock
      .calls[0][0];
    expect(callArgs.alias).toBe("My New Identity");
  });
});

// ─── Disabled state ──────────────────────────────────────────────────

describe("CreateIdentityScreen — disabled during processing", () => {
  it("disables inputs during waitingForPlatform", () => {
    setup({
      status: { type: "waitingForPlatform", startedAt: Date.now() },
    });
    const aliasInput = screen.getByRole("textbox", {
      name: /identity alias/i,
    });
    expect(aliasInput).toBeDisabled();
  });

  it("disables funding method selector during waitingForAssetLock", () => {
    setup({
      status: { type: "waitingForAssetLock", startedAt: Date.now() },
    });
    const fmSelector = screen.getByRole("combobox", {
      name: /funding method/i,
    });
    expect(fmSelector).toBeDisabled();
  });
});
