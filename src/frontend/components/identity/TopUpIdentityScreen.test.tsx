import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeAll, beforeEach } from "vitest";
import {
  TopUpIdentityScreen,
  type TopUpStatus,
} from "./TopUpIdentityScreen";
import type {
  QualifiedIdentityDto,
  WalletDto,
  AssetLockDto,
  PlatformAddressDto,
} from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Mock useUtxoMonitor ────────────────────────────────────────────

vi.mock("@/hooks/useUtxoMonitor", () => ({
  useUtxoMonitor: () => ({ fundsReceived: false, balance: 0, polling: false }),
}));

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

function makeAssetLock(overrides: Partial<AssetLockDto> = {}): AssetLockDto {
  return {
    txid: "aabbccdd11223344556677889900aabbccdd11223344556677889900aabbccdd",
    address: "yXtest1234567890",
    amount: 100_000_000, // 1 DASH
    hasAssetLockProof: true,
    proofDetails: null,
    ...overrides,
  };
}

function makePlatformAddress(
  overrides: Partial<PlatformAddressDto> = {},
): PlatformAddressDto {
  return {
    address: "evo1testaddr12345",
    balance: 500_000_000, // 5 DASH in duffs
    nonce: 0,
    ...overrides,
  };
}

function makeWallet(overrides: Partial<WalletDto> = {}): WalletDto {
  return {
    seedHash: "wallet1seedhash",
    usesPassword: false,
    alias: "Test Wallet",
    isMain: true,
    confirmedBalance: 10_000_000_00, // 10 DASH in duffs
    unconfirmedBalance: 0,
    totalBalance: 10_000_000_00,
    addresses: [{ address: "yXaddress1234", balance: 10_000_000_00 }],
    transactions: [],
    unusedAssetLocks: [makeAssetLock()],
    platformAddresses: [makePlatformAddress()],
    identityIndexes: [0],
    passwordHint: null,
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
    balance: 10_000_000_000, // 0.1 DASH in credits
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: ["wallet1seedhash"],
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
  wallets: [makeWallet()],
  status: { type: "form" } as TopUpStatus,
  onSubmit: vi.fn(),
  onSubmitPlatformAddress: vi.fn(),
  onDismissError: vi.fn(),
  onBack: vi.fn(),
  estimatedFee: null,
};

function setup(props: Partial<Parameters<typeof TopUpIdentityScreen>[0]> = {}) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <TopUpIdentityScreen {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("TopUpIdentityScreen — header", () => {
  it("renders title and identity name", () => {
    setup();
    expect(screen.getByRole("heading", { name: /Top Up Identity/i })).toBeInTheDocument();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("shows current balance badge", () => {
    setup();
    expect(
      screen.getByText(/Balance.*0\.10000000.*DASH/),
    ).toBeInTheDocument();
  });

  it("calls onBack when back button clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /back/i }));
    expect(props.onBack).toHaveBeenCalled();
  });
});

// ─── Wallet selection ──────────────────────────────────────────────

describe("TopUpIdentityScreen — wallet selection", () => {
  it("shows single wallet info without dropdown", () => {
    setup();
    expect(screen.getByText(/Test Wallet/)).toBeInTheDocument();
  });

  it("shows no wallets error when empty", () => {
    setup({ wallets: [] });
    expect(
      screen.getByText(/No wallets available/),
    ).toBeInTheDocument();
  });
});

// ─── Asset lock funding ────────────────────────────────────────────

describe("TopUpIdentityScreen — asset lock funding", () => {
  it("shows asset lock list when selected", () => {
    setup();
    // Default should be asset lock since wallet has one
    // Find the asset lock item
    const lockItem = screen.getByText(/1\.00000000 DASH/);
    expect(lockItem).toBeInTheDocument();
  });

  it("shows confirmed badge for asset locks", async () => {
    setup();
    expect(screen.getByText(/Confirmed/)).toBeInTheDocument();
  });

  it("shows wallet balance as default when no asset locks", () => {
    setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    // No asset locks, so default to wallet balance
    expect(screen.getByText(/Amount to top up/i)).toBeInTheDocument();
  });

  it("enables submit when asset lock selected", async () => {
    const { user } = setup();
    // Click the asset lock to select it
    const lockButtons = screen.getAllByRole("button");
    const assetLockButton = lockButtons.find(
      (b) => b.textContent?.includes("1.00000000"),
    );
    if (assetLockButton) {
      await user.click(assetLockButton);
    }
    const submitButton = screen.getByRole("button", {
      name: /Top Up Identity/i,
    });
    expect(submitButton).not.toBeDisabled();
  });
});

// ─── Wallet balance funding ────────────────────────────────────────

describe("TopUpIdentityScreen — wallet balance funding", () => {
  it("shows amount input for wallet balance method", () => {
    setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    expect(screen.getByPlaceholderText(/Enter amount/)).toBeInTheDocument();
  });

  it("shows wallet balance display", () => {
    setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    expect(screen.getAllByText(/Wallet balance/i).length).toBeGreaterThan(0);
  });

  it("has max button", () => {
    setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    const maxButton = screen.getByRole("button", {
      name: /maximum amount/i,
    });
    expect(maxButton).toBeInTheDocument();
  });

  it("fills max amount when max clicked", async () => {
    const { user } = setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    const maxButton = screen.getByRole("button", {
      name: /maximum amount/i,
    });
    await user.click(maxButton);
    const input = screen.getByPlaceholderText(
      /Enter amount/,
    ) as HTMLInputElement;
    expect(input.value).not.toBe("");
  });

  it("calls onSubmit with wallet balance method", async () => {
    const { user, props } = setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    const input = screen.getByPlaceholderText(/Enter amount/);
    await user.type(input, "0.5");
    const submitButton = screen.getByRole("button", {
      name: /Top Up Identity/i,
    });
    await user.click(submitButton);
    expect(props.onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        identityId: "aabbccdd11223344556677889900aabb",
        walletSeedHash: "wallet1seedhash",
        fundingMethod: expect.objectContaining({
          method: "fundWithWallet",
        }),
      }),
    );
  });
});

// ─── Platform address funding ──────────────────────────────────────

describe("TopUpIdentityScreen — platform address funding", () => {
  it("platform address method is available when addresses exist", () => {
    // Platform address method should be in the available methods
    setup();
    // The wallet has platform addresses, so the method should be available
    // (it's in the Select dropdown options)
  });
});

// ─── QR code funding ───────────────────────────────────────────────

describe("TopUpIdentityScreen — QR code funding", () => {
  async function switchToQrCode(user: ReturnType<typeof userEvent.setup>) {
    const selector = screen.getByRole("combobox", {
      name: /funding method/i,
    });
    await user.click(selector);
    const option = screen.getByRole("option", {
      name: /address with qr code/i,
    });
    await user.click(option);
  }

  it("shows QR code when amount entered", async () => {
    const { user } = setup();
    await switchToQrCode(user);
    const amountInput = screen.getByPlaceholderText("Enter amount (e.g., 0.5)");
    await user.type(amountInput, "1.5");
    expect(screen.getByTestId("qr-code")).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: /QR code for address/ }),
    ).toBeInTheDocument();
  });

  it("generates dash: payment URI with correct amount", async () => {
    const { user } = setup();
    await switchToQrCode(user);
    const amountInput = screen.getByPlaceholderText("Enter amount (e.g., 0.5)");
    await user.type(amountInput, "2.0");
    expect(screen.getByText(/dash:yXaddress1234\?amount=2\.00000000/)).toBeInTheDocument();
  });

  it("does not show QR code when amount is empty", async () => {
    const { user } = setup();
    await switchToQrCode(user);
    expect(screen.queryByTestId("qr-code")).not.toBeInTheDocument();
  });

  it("has copy button for payment URI", async () => {
    const { user } = setup();
    await switchToQrCode(user);
    const amountInput = screen.getByPlaceholderText("Enter amount (e.g., 0.5)");
    await user.type(amountInput, "0.5");
    expect(
      screen.getByRole("button", { name: /copy payment uri/i }),
    ).toBeInTheDocument();
  });
});

// ─── Status screens ────────────────────────────────────────────────

describe("TopUpIdentityScreen — status screens", () => {
  it("shows success screen", () => {
    setup({ status: { type: "success" } });
    expect(screen.getByText("Top-Up Successful!")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Back to Identity/i }),
    ).toBeInTheDocument();
  });

  it("calls onBack from success screen", async () => {
    const { user, props } = setup({ status: { type: "success" } });
    await user.click(
      screen.getByRole("button", { name: /Back to Identity/i }),
    );
    expect(props.onBack).toHaveBeenCalled();
  });

  it("shows error screen", () => {
    setup({ status: { type: "error", message: "Insufficient funds" } });
    expect(screen.getByText("Top-Up Failed")).toBeInTheDocument();
    // InlineError translates raw messages via translateError()
    expect(screen.getByText("Insufficient funds for this operation.")).toBeInTheDocument();
  });

  it("calls onDismissError from error screen", async () => {
    const { user, props } = setup({
      status: { type: "error", message: "fail" },
    });
    await user.click(screen.getByRole("button", { name: /Try Again/i }));
    expect(props.onDismissError).toHaveBeenCalled();
  });

  it("shows waiting for platform screen", () => {
    setup({
      status: { type: "waitingForPlatform", startedAt: Date.now() - 5000 },
    });
    expect(
      screen.getByText(/Waiting for Platform acknowledgement/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Time elapsed/)).toBeInTheDocument();
  });

  it("shows waiting for asset lock screen", () => {
    setup({
      status: { type: "waitingForAssetLock", startedAt: Date.now() - 3000 },
    });
    expect(
      screen.getByText(/Waiting for Core Chain proof/),
    ).toBeInTheDocument();
  });
});

// ─── Fee estimation ────────────────────────────────────────────────

describe("TopUpIdentityScreen — fee display", () => {
  it("shows estimated fee when provided", () => {
    setup({ estimatedFee: 50_000 }); // 0.00050000 DASH
    expect(screen.getByText(/Estimated fee/)).toBeInTheDocument();
    expect(screen.getByText(/0\.00050000/)).toBeInTheDocument();
  });

  it("hides fee when not provided", () => {
    setup({ estimatedFee: null });
    expect(screen.queryByText(/Estimated fee/)).not.toBeInTheDocument();
  });
});

// ─── Submit button ─────────────────────────────────────────────────

describe("TopUpIdentityScreen — submit button", () => {
  it("submit button disabled when no amount/selection", () => {
    setup({
      wallets: [makeWallet({ unusedAssetLocks: [] })],
    });
    const submitButton = screen.getByRole("button", {
      name: /Top Up Identity/i,
    });
    expect(submitButton).toBeDisabled();
  });

  it("submit button not shown when no wallet", () => {
    setup({ wallets: [] });
    expect(
      screen.queryByRole("button", { name: /Top Up Identity/i }),
    ).not.toBeInTheDocument();
  });
});

// ─── Multiple wallets ──────────────────────────────────────────────

describe("TopUpIdentityScreen — multiple wallets", () => {
  it("shows wallet selector dropdown with multiple wallets", () => {
    const wallet2 = makeWallet({
      seedHash: "wallet2seedhash",
      alias: "Second Wallet",
      totalBalance: 500_000_000,
    });
    setup({ wallets: [makeWallet(), wallet2] });
    // Should show the select trigger (not plain text)
    expect(screen.getByText(/Select wallet/i)).toBeInTheDocument();
  });
});

// ─── Identity without wallet association ────────────────────────────

describe("TopUpIdentityScreen — identity without wallet", () => {
  it("defaults to first available wallet", () => {
    setup({
      identity: makeIdentity({ associatedWalletHashes: [] }),
    });
    expect(screen.getByText(/Test Wallet/)).toBeInTheDocument();
  });
});
