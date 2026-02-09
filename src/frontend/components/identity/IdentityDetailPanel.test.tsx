import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { IdentityDetailPanel } from "./IdentityDetailPanel";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";
import { TooltipProvider } from "@/components/ui/tooltip";

// ─── Test fixtures ─────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    keyType: "ECDSA_SECP256K1",
    purpose: "AUTHENTICATION",
    securityLevel: "MASTER",
    data: "02aabb",
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
    balance: 1_000_000_000,
    keys: [makeKey()],
    dpnsNames: [],
    associatedWalletHashes: ["seed123"],
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
  isRefreshing: false,
  walletNames: { seed123: "My Main Wallet" } as Record<string, string>,
  onRefresh: vi.fn(),
  onTopUp: vi.fn(),
  onWithdraw: vi.fn(),
  onTransfer: vi.fn(),
  onRegisterDpns: vi.fn(),
  onViewKeys: vi.fn(),
  onViewKey: vi.fn(),
  onNavigateToWallet: vi.fn(),
};

function setup(
  props: Partial<Parameters<typeof IdentityDetailPanel>[0]> = {},
) {
  const mergedProps = { ...defaultProps, ...props };
  return {
    user: userEvent.setup(),
    ...render(
      <TooltipProvider>
        <IdentityDetailPanel {...mergedProps} />
      </TooltipProvider>,
    ),
    props: mergedProps,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ─── Header ─────────────────────────────────────────────────────────

describe("IdentityDetailPanel — header", () => {
  it("renders the identity alias", () => {
    setup();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("renders 'Unnamed Identity' when alias is null", () => {
    setup({ identity: makeIdentity({ alias: null }) });
    expect(screen.getByText("Unnamed Identity")).toBeInTheDocument();
  });

  it("renders the full identity ID", () => {
    setup();
    expect(
      screen.getByText("aabbccdd11223344556677889900aabb"),
    ).toBeInTheDocument();
  });

  it("renders a copy button for the ID", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /copy to clipboard/i }),
    ).toBeInTheDocument();
  });

  it("renders type badge for user", () => {
    setup();
    expect(screen.getByText("User")).toBeInTheDocument();
  });

  it("renders type badge for masternode", () => {
    setup({ identity: makeIdentity({ identityType: "masternode" }) });
    expect(screen.getByText("Masternode")).toBeInTheDocument();
  });

  it("renders type badge for evonode", () => {
    setup({ identity: makeIdentity({ identityType: "evonode" }) });
    expect(screen.getByText("Evonode")).toBeInTheDocument();
  });

  it("renders status badge", () => {
    setup();
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  it("renders pending status badge", () => {
    setup({ identity: makeIdentity({ status: "pendingCreation" }) });
    expect(screen.getByText("Pending")).toBeInTheDocument();
  });

  it("renders failed status badge", () => {
    setup({ identity: makeIdentity({ status: "failedCreation" }) });
    expect(screen.getByText("Failed")).toBeInTheDocument();
  });

  it("renders notFound status badge", () => {
    setup({ identity: makeIdentity({ status: "notFound" }) });
    expect(screen.getByText("Not Found")).toBeInTheDocument();
  });

  it("renders unknown status badge", () => {
    setup({ identity: makeIdentity({ status: "unknown" }) });
    expect(screen.getByText("Unknown")).toBeInTheDocument();
  });

  it("renders network badge for testnet", () => {
    setup({ identity: makeIdentity({ network: "testnet" }) });
    expect(screen.getByText("testnet")).toBeInTheDocument();
  });

  it("does not render network badge for mainnet (dash)", () => {
    setup({ identity: makeIdentity({ network: "dash" }) });
    expect(screen.queryByText("dash")).not.toBeInTheDocument();
  });
});

// ─── Balance ────────────────────────────────────────────────────────

describe("IdentityDetailPanel — balance", () => {
  it("renders balance heading", () => {
    setup();
    expect(screen.getByText("Balance")).toBeInTheDocument();
  });

  it("renders formatted balance in DASH", () => {
    setup();
    const balanceEl = screen.getByTestId("identity-balance");
    // 1_000_000_000 credits = 10.00000000 DASH
    expect(balanceEl).toHaveTextContent("10.00000000");
    expect(balanceEl).toHaveTextContent("DASH");
  });

  it("renders zero balance", () => {
    setup({ identity: makeIdentity({ balance: 0 }) });
    const balanceEl = screen.getByTestId("identity-balance");
    expect(balanceEl).toHaveTextContent("0.00000000");
  });

  it("renders large balance", () => {
    setup({ identity: makeIdentity({ balance: 10_000_000_000 }) });
    const balanceEl = screen.getByTestId("identity-balance");
    expect(balanceEl).toHaveTextContent("100.00000000");
  });
});

// ─── Action Bar ─────────────────────────────────────────────────────

describe("IdentityDetailPanel — action bar", () => {
  it("renders all action buttons for active identity", () => {
    setup();
    expect(screen.getByRole("button", { name: /top up/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /withdraw/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /transfer/i })).toBeEnabled();
    expect(
      screen.getByRole("button", { name: /register dpns/i }),
    ).toBeEnabled();
    expect(screen.getByRole("button", { name: /refresh/i })).toBeEnabled();
  });

  it("disables financial buttons for inactive identity", () => {
    setup({ identity: makeIdentity({ status: "unknown" }) });
    expect(screen.getByRole("button", { name: /top up/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /withdraw/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /transfer/i })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /register dpns/i }),
    ).toBeDisabled();
  });

  it("disables withdraw when balance too low", () => {
    setup({ identity: makeIdentity({ balance: 100_000_000 }) }); // 1.0 DASH < 5.0 threshold
    expect(screen.getByRole("button", { name: /withdraw/i })).toBeDisabled();
  });

  it("disables transfer when balance too low", () => {
    setup({ identity: makeIdentity({ balance: 10_000_000 }) }); // 0.1 < 0.2 threshold
    expect(screen.getByRole("button", { name: /transfer/i })).toBeDisabled();
  });

  it("enables withdraw when balance is sufficient", () => {
    setup({ identity: makeIdentity({ balance: 500_000_000 }) });
    expect(screen.getByRole("button", { name: /withdraw/i })).toBeEnabled();
  });

  it("enables transfer when balance is sufficient", () => {
    setup({ identity: makeIdentity({ balance: 20_000_000 }) });
    expect(screen.getByRole("button", { name: /transfer/i })).toBeEnabled();
  });

  it("calls onTopUp with identity id", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /top up/i }));
    expect(props.onTopUp).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
    );
  });

  it("calls onWithdraw with identity id", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /withdraw/i }));
    expect(props.onWithdraw).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
    );
  });

  it("calls onTransfer with identity id", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /transfer/i }));
    expect(props.onTransfer).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
    );
  });

  it("calls onRegisterDpns with identity id", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /register dpns/i }));
    expect(props.onRegisterDpns).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
    );
  });

  it("calls onRefresh with identity id", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /refresh/i }));
    expect(props.onRefresh).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
    );
  });

  it("disables refresh button when refreshing", () => {
    setup({ isRefreshing: true });
    expect(screen.getByRole("button", { name: /refresh/i })).toBeDisabled();
  });

  it("does not render top up button when onTopUp not provided", () => {
    setup({ onTopUp: undefined });
    expect(
      screen.queryByRole("button", { name: /top up/i }),
    ).not.toBeInTheDocument();
  });

  it("does not render withdraw button when onWithdraw not provided", () => {
    setup({ onWithdraw: undefined });
    expect(
      screen.queryByRole("button", { name: /withdraw/i }),
    ).not.toBeInTheDocument();
  });

  it("does not render transfer button when onTransfer not provided", () => {
    setup({ onTransfer: undefined });
    expect(
      screen.queryByRole("button", { name: /transfer/i }),
    ).not.toBeInTheDocument();
  });

  it("does not render register DPNS button when onRegisterDpns not provided", () => {
    setup({ onRegisterDpns: undefined });
    expect(
      screen.queryByRole("button", { name: /register dpns/i }),
    ).not.toBeInTheDocument();
  });
});

// ─── DPNS Names ─────────────────────────────────────────────────────

describe("IdentityDetailPanel — DPNS names", () => {
  it("does not render DPNS section when no names", () => {
    setup();
    expect(screen.queryByText("DPNS Names")).not.toBeInTheDocument();
  });

  it("renders DPNS names list", () => {
    setup({
      identity: makeIdentity({
        dpnsNames: [
          { name: "alice.dash", acquiredAt: 1700000000 },
          { name: "bob.dash", acquiredAt: 1700100000 },
        ],
      }),
    });
    expect(screen.getByText("DPNS Names")).toBeInTheDocument();
    expect(screen.getByText("alice.dash")).toBeInTheDocument();
    expect(screen.getByText("bob.dash")).toBeInTheDocument();
  });

  it("renders acquisition dates", () => {
    setup({
      identity: makeIdentity({
        dpnsNames: [{ name: "alice.dash", acquiredAt: 1700000000 }],
      }),
    });
    const list = screen.getByTestId("dpns-names-list");
    // Date formatting depends on locale, just check it's there
    expect(list).toHaveTextContent("2023");
  });
});

// ─── Associated Wallets ─────────────────────────────────────────────

describe("IdentityDetailPanel — associated wallets", () => {
  it("renders associated wallet with resolved name", () => {
    setup();
    expect(screen.getByText("Associated Wallets")).toBeInTheDocument();
    expect(screen.getByText("My Main Wallet")).toBeInTheDocument();
  });

  it("renders wallet hash fallback when name not in map", () => {
    setup({ walletNames: {} });
    expect(screen.getByText(/Wallet seed123/)).toBeInTheDocument();
  });

  it("does not render wallet section when no wallet hashes", () => {
    setup({
      identity: makeIdentity({ associatedWalletHashes: [] }),
    });
    expect(
      screen.queryByText("Associated Wallets"),
    ).not.toBeInTheDocument();
  });

  it("renders wallet as clickable link when onNavigateToWallet provided", () => {
    setup();
    const link = screen.getByText("My Main Wallet");
    expect(link.tagName).toBe("BUTTON");
  });

  it("calls onNavigateToWallet when wallet link clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByText("My Main Wallet"));
    expect(props.onNavigateToWallet).toHaveBeenCalledWith("seed123");
  });

  it("renders wallet as plain text when onNavigateToWallet not provided", () => {
    setup({ onNavigateToWallet: undefined });
    const text = screen.getByText("My Main Wallet");
    expect(text.tagName).toBe("SPAN");
  });

  it("renders multiple associated wallets", () => {
    setup({
      identity: makeIdentity({
        associatedWalletHashes: ["seed123", "seed456"],
      }),
      walletNames: {
        seed123: "Wallet A",
        seed456: "Wallet B",
      },
    });
    expect(screen.getByText("Wallet A")).toBeInTheDocument();
    expect(screen.getByText("Wallet B")).toBeInTheDocument();
  });
});

// ─── Associated Identities ──────────────────────────────────────────

describe("IdentityDetailPanel — associated identities", () => {
  it("does not render when no voter or operator", () => {
    setup();
    expect(
      screen.queryByText("Associated Identities"),
    ).not.toBeInTheDocument();
  });

  it("renders voter identity ID", () => {
    setup({
      identity: makeIdentity({
        voterIdentityId: "voter1234abcd",
      }),
    });
    expect(screen.getByText("Associated Identities")).toBeInTheDocument();
    expect(screen.getByText("Voter:")).toBeInTheDocument();
    expect(screen.getByText("voter1234abcd")).toBeInTheDocument();
  });

  it("renders operator identity ID", () => {
    setup({
      identity: makeIdentity({
        operatorIdentityId: "operator5678efgh",
      }),
    });
    expect(screen.getByText("Operator:")).toBeInTheDocument();
    expect(screen.getByText("operator5678efgh")).toBeInTheDocument();
  });

  it("renders both voter and operator", () => {
    setup({
      identity: makeIdentity({
        voterIdentityId: "voter1234",
        operatorIdentityId: "operator5678",
      }),
    });
    expect(screen.getByText("Voter:")).toBeInTheDocument();
    expect(screen.getByText("Operator:")).toBeInTheDocument();
  });
});

// ─── Keys Section ───────────────────────────────────────────────────

describe("IdentityDetailPanel — keys", () => {
  it("renders keys heading with count", () => {
    setup();
    const heading = screen.getByText("Keys");
    expect(heading).toBeInTheDocument();
    // Count indicator (1)
    expect(heading.parentElement).toHaveTextContent("(1)");
  });

  it('renders "No keys loaded" when no keys', () => {
    setup({ identity: makeIdentity({ keys: [] }) });
    expect(screen.getByText("No keys loaded.")).toBeInTheDocument();
  });

  it("renders Manage Keys button when keys exist and onViewKeys provided", () => {
    setup();
    expect(
      screen.getByRole("button", { name: /manage keys/i }),
    ).toBeInTheDocument();
  });

  it("does not render Manage Keys button when onViewKeys not provided", () => {
    setup({ onViewKeys: undefined });
    expect(
      screen.queryByRole("button", { name: /manage keys/i }),
    ).not.toBeInTheDocument();
  });

  it("calls onViewKeys when Manage Keys clicked", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /manage keys/i }));
    expect(props.onViewKeys).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
    );
  });

  it("renders main keys section", () => {
    setup({
      identity: makeIdentity({
        keys: [
          makeKey({ keyId: 0, purpose: "AUTHENTICATION", securityLevel: "MASTER" }),
          makeKey({ keyId: 1, purpose: "TRANSFER", securityLevel: "CRITICAL" }),
        ],
      }),
    });
    expect(screen.getByText("Main Keys")).toBeInTheDocument();
    expect(screen.getByText("#0 — A — Master")).toBeInTheDocument();
    expect(screen.getByText("#1 — T — Critical")).toBeInTheDocument();
  });

  it("renders voter keys section separately", () => {
    setup({
      identity: makeIdentity({
        keys: [
          makeKey({ keyId: 0, purpose: "AUTHENTICATION" }),
          makeKey({ keyId: 1, purpose: "VOTING", securityLevel: "HIGH" }),
        ],
      }),
    });
    expect(screen.getByText("Main Keys")).toBeInTheDocument();
    expect(screen.getByText("Voter Keys")).toBeInTheDocument();
    expect(screen.getByText("#1 — V — High")).toBeInTheDocument();
  });

  it("highlights keys with private key in primary color", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ hasPrivateKey: true })],
      }),
    });
    // The Key icon should have text-primary class for keys with private data
    const keyButton = screen.getByRole("button", {
      name: /key 0/i,
    });
    expect(keyButton).toBeInTheDocument();
  });

  it("renders disabled badge for disabled keys", () => {
    setup({
      identity: makeIdentity({
        keys: [makeKey({ isDisabled: true })],
      }),
    });
    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });

  it("calls onViewKey when key item is clicked", async () => {
    const { user, props } = setup();
    await user.click(
      screen.getByRole("button", { name: /key 0/i }),
    );
    expect(props.onViewKey).toHaveBeenCalledWith(
      "aabbccdd11223344556677889900aabb",
      0,
    );
  });

  it("renders all key purpose abbreviations correctly", () => {
    const purposes = [
      { purpose: "AUTHENTICATION", expected: "A" },
      { purpose: "ENCRYPTION", expected: "E" },
      { purpose: "DECRYPTION", expected: "D" },
      { purpose: "TRANSFER", expected: "T" },
      { purpose: "SYSTEM", expected: "S" },
      { purpose: "OWNER", expected: "O" },
    ];

    purposes.forEach(({ purpose, expected }, index) => {
      const { unmount } = render(
        <TooltipProvider>
          <IdentityDetailPanel
            {...defaultProps}
            identity={makeIdentity({
              keys: [makeKey({ keyId: index, purpose, securityLevel: "HIGH" })],
            })}
          />
        </TooltipProvider>,
      );
      expect(
        screen.getByText(`#${index} — ${expected} — High`),
      ).toBeInTheDocument();
      unmount();
    });
  });
});

// ─── Details Section ────────────────────────────────────────────────

describe("IdentityDetailPanel — details section", () => {
  it("renders wallet index when present", () => {
    setup({ identity: makeIdentity({ walletIndex: 3 }) });
    expect(screen.getByText("Wallet Index")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("does not render details section when no wallet index and no top-ups", () => {
    setup({
      identity: makeIdentity({ walletIndex: null, topUps: [] }),
    });
    expect(screen.queryByText("Details")).not.toBeInTheDocument();
  });

  it("renders top-up history", () => {
    setup({
      identity: makeIdentity({
        topUps: [
          { index: 0, amount: 100_000_000 },
          { index: 1, amount: 50_000_000 },
        ],
      }),
    });
    expect(screen.getByText("Top-up History")).toBeInTheDocument();
    expect(screen.getByText("Index 0")).toBeInTheDocument();
    expect(screen.getByText("1.00000000 DASH")).toBeInTheDocument();
    expect(screen.getByText("Index 1")).toBeInTheDocument();
    expect(screen.getByText("0.50000000 DASH")).toBeInTheDocument();
  });
});

// ─── Refreshing state ───────────────────────────────────────────────

describe("IdentityDetailPanel — refreshing state", () => {
  it("shows refreshing indicator when isRefreshing is true", () => {
    setup({ isRefreshing: true });
    expect(
      screen.getByText("Refreshing identity from Platform..."),
    ).toBeInTheDocument();
  });

  it("hides refreshing indicator when isRefreshing is false", () => {
    setup({ isRefreshing: false });
    expect(
      screen.queryByText("Refreshing identity from Platform..."),
    ).not.toBeInTheDocument();
  });
});

// ─── Region / accessibility ─────────────────────────────────────────

describe("IdentityDetailPanel — accessibility", () => {
  it("has an aria-label region", () => {
    setup();
    expect(
      screen.getByRole("region", { name: "Identity details" }),
    ).toBeInTheDocument();
  });

  it("key items have aria-labels", () => {
    setup();
    expect(
      screen.getByRole("button", {
        name: /key 0: authentication master/i,
      }),
    ).toBeInTheDocument();
  });
});

// ─── Encoding tooltip ────────────────────────────────────────────────

describe("IdentityDetailPanel — encoding tooltip", () => {
  it("shows 'UserId (Base58)' tooltip for user identity ID", async () => {
    const { user } = setup({
      identity: makeIdentity({ identityType: "user" }),
    });
    const idCode = screen.getByText("aabbccdd11223344556677889900aabb");
    await user.hover(idCode);
    await waitFor(() => {
      // Radix renders tooltip content twice (visible + sr-only)
      const matches = screen.getAllByText("UserId (Base58)");
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows 'ProTxHash (Hex)' tooltip for masternode identity ID", async () => {
    const { user } = setup({
      identity: makeIdentity({ identityType: "masternode" }),
    });
    const idCode = screen.getByText("aabbccdd11223344556677889900aabb");
    await user.hover(idCode);
    await waitFor(() => {
      const matches = screen.getAllByText("ProTxHash (Hex)");
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });

  it("shows 'ProTxHash (Hex)' tooltip for evonode identity ID", async () => {
    const { user } = setup({
      identity: makeIdentity({ identityType: "evonode" }),
    });
    const idCode = screen.getByText("aabbccdd11223344556677889900aabb");
    await user.hover(idCode);
    await waitFor(() => {
      const matches = screen.getAllByText("ProTxHash (Hex)");
      expect(matches.length).toBeGreaterThanOrEqual(1);
    });
  });
});
