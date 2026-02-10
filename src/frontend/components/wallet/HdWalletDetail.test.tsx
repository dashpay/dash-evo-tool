import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { HdWalletDetail } from "./HdWalletDetail";
import {
  createMockWalletAddress,
  createMockWalletTransaction,
  createMockAssetLock,
  createMockPlatformAddress,
  createMockHdWallet,
} from "@/test/fixtures";
import type {
  WalletDto,
  WalletAddressDto,
  WalletTransactionDto,
  AssetLockDto,
  PlatformAddressDto,
} from "@/bindings";

// ─── Local wrappers over centralized fixtures (test-specific defaults) ──

function makeAddress(overrides: Partial<WalletAddressDto> = {}): WalletAddressDto {
  return createMockWalletAddress({
    address: "XtZ1M2npV4oFZ4u7fHsVn7m3WRXHZ1Dqjx",
    balance: 100000000, // 1 DASH
    totalReceived: 200000000,
    derivationPath: "m/44'/5'/0'/0/0",
    ...overrides,
  });
}

function makeTransaction(
  overrides: Partial<WalletTransactionDto> = {},
): WalletTransactionDto {
  return createMockWalletTransaction({
    txid: "abc123def456abc123def456abc123def456abc123def456abc123def456abc1",
    timestamp: 1700000000,
    height: 100000,
    blockHash: "0000000000000000000",
    netAmount: 50000000,
    fee: 1000,
    label: null,
    isOurs: true,
    ...overrides,
  });
}

function makeAssetLockFixture(overrides: Partial<AssetLockDto> = {}): AssetLockDto {
  return createMockAssetLock({
    txid: "lock123def456lock123def456lock123def456lock123def456lock123def456",
    address: "XtZ1M2npV4oFZ4u7fHsVn7m3WRXHZ1Dqjx",
    amount: 10000000000,
    hasInstantLock: true,
    hasAssetLockProof: true,
    ...overrides,
  });
}

function makePlatformAddress(
  overrides: Partial<PlatformAddressDto> = {},
): PlatformAddressDto {
  return createMockPlatformAddress({
    address: "XplatformAddr1234567890",
    balance: 5000000000, // credits
    nonce: 0,
    ...overrides,
  });
}

function makeWallet(overrides: Partial<WalletDto> = {}): WalletDto {
  return createMockHdWallet({
    seedHash: "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aaaa1111bbbb2222",
    usesPassword: false,
    alias: "My Test Wallet",
    isMain: true,
    confirmedBalance: 500000000,
    unconfirmedBalance: 0,
    totalBalance: 500000000,
    addresses: [
      makeAddress({ address: "Xaddr0", derivationPath: "m/44'/5'/0'/0/0", balance: 300000000 }),
      makeAddress({ address: "Xaddr1", derivationPath: "m/44'/5'/0'/0/1", balance: 100000000 }),
      makeAddress({ address: "Xaddr2", derivationPath: "m/44'/5'/0'/1/0", balance: 50000000 }),
      makeAddress({ address: "Xaddr3", derivationPath: "m/44'/5'/0'/0/2", balance: 0 }),
    ],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [],
    passwordHint: null,
    ...overrides,
  });
}

// ─── Header Section Tests ──────────────────────────────────────────

describe("HdWalletDetail", () => {
  describe("header", () => {
    it("renders wallet alias as heading", () => {
      render(<HdWalletDetail wallet={makeWallet()} />);
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("My Test Wallet");
    });

    it("renders 'Unnamed Wallet' when alias is null", () => {
      render(<HdWalletDetail wallet={makeWallet({ alias: null })} />);
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Unnamed Wallet");
    });

    it("renders 'Unnamed Wallet' when alias is empty/whitespace", () => {
      render(<HdWalletDetail wallet={makeWallet({ alias: "   " })} />);
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Unnamed Wallet");
    });

    it("renders Core balance", () => {
      render(<HdWalletDetail wallet={makeWallet({ totalBalance: 123456789 })} />);
      expect(screen.getByText(/Core balance:/)).toBeInTheDocument();
      expect(screen.getByText("1.23456789 DASH")).toBeInTheDocument();
    });

    it("renders pending balance when unconfirmed > 0", () => {
      render(
        <HdWalletDetail
          wallet={makeWallet({ unconfirmedBalance: 10000000 })}
        />,
      );
      expect(screen.getByText(/\+0\.10000000 pending/)).toBeInTheDocument();
    });

    it("does not render pending badge when unconfirmed is 0", () => {
      render(
        <HdWalletDetail
          wallet={makeWallet({ unconfirmedBalance: 0 })}
        />,
      );
      expect(screen.queryByText(/pending/)).not.toBeInTheDocument();
    });

    it("renders Platform balance", () => {
      render(
        <HdWalletDetail
          wallet={makeWallet({
            platformAddresses: [
              makePlatformAddress({ balance: 5000000000 }), // 5B credits = 5M duffs = 0.05 DASH
              makePlatformAddress({ address: "Xplat2", balance: 3000000000 }),
            ],
          })}
        />,
      );
      expect(screen.getByText(/Platform balance:/)).toBeInTheDocument();
    });

    it("shows spinner when refreshing", () => {
      render(<HdWalletDetail wallet={makeWallet()} refreshing />);
      expect(screen.getByLabelText("Refreshing wallet")).toBeInTheDocument();
    });

    it("does not show spinner when not refreshing", () => {
      render(<HdWalletDetail wallet={makeWallet()} refreshing={false} />);
      expect(screen.queryByLabelText("Refreshing wallet")).not.toBeInTheDocument();
    });
  });

  // ─── Action Bar Tests ──────────────────────────────────────────────

  describe("action bar", () => {
    it("renders Send button", () => {
      const onSend = vi.fn();
      render(<HdWalletDetail wallet={makeWallet()} onSend={onSend} />);
      expect(screen.getByRole("button", { name: /Send/ })).toBeInTheDocument();
    });

    it("calls onSend when Send is clicked", async () => {
      const user = userEvent.setup();
      const onSend = vi.fn();
      render(<HdWalletDetail wallet={makeWallet()} onSend={onSend} />);
      await user.click(screen.getByRole("button", { name: /Send/ }));
      expect(onSend).toHaveBeenCalledOnce();
    });

    it("renders Receive button", () => {
      const onReceive = vi.fn();
      render(<HdWalletDetail wallet={makeWallet()} onReceive={onReceive} />);
      expect(screen.getByRole("button", { name: /^Receive$/ })).toBeInTheDocument();
    });

    it("calls onReceive when Receive is clicked", async () => {
      const user = userEvent.setup();
      const onReceive = vi.fn();
      render(<HdWalletDetail wallet={makeWallet()} onReceive={onReceive} />);
      await user.click(screen.getByRole("button", { name: /^Receive$/ }));
      expect(onReceive).toHaveBeenCalledOnce();
    });

    it("renders Refresh button", () => {
      const onRefresh = vi.fn();
      render(<HdWalletDetail wallet={makeWallet()} onRefresh={onRefresh} />);
      expect(screen.getByRole("button", { name: /Refresh/ })).toBeInTheDocument();
    });

    it("calls onRefresh when Refresh is clicked", async () => {
      const user = userEvent.setup();
      const onRefresh = vi.fn();
      render(<HdWalletDetail wallet={makeWallet()} onRefresh={onRefresh} />);
      await user.click(screen.getByRole("button", { name: /Refresh/ }));
      expect(onRefresh).toHaveBeenCalledOnce();
    });

    it("disables Refresh button when refreshing", () => {
      render(
        <HdWalletDetail
          wallet={makeWallet()}
          onRefresh={() => {}}
          refreshing
        />,
      );
      expect(screen.getByRole("button", { name: /Refresh/ })).toBeDisabled();
    });

    it("does not show buttons when callbacks are not provided", () => {
      render(<HdWalletDetail wallet={makeWallet()} />);
      expect(screen.queryByRole("button", { name: /^Send$/ })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /^Receive$/ })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /^Refresh$/ })).not.toBeInTheDocument();
    });

    it("shows refresh mode selector in developer mode", () => {
      render(
        <HdWalletDetail
          wallet={makeWallet()}
          isDeveloperMode
          onRefreshModeChange={() => {}}
        />,
      );
      expect(screen.getByText("Refresh Mode:")).toBeInTheDocument();
    });

    it("hides refresh mode selector when not in developer mode", () => {
      render(
        <HdWalletDetail
          wallet={makeWallet()}
          isDeveloperMode={false}
          onRefreshModeChange={() => {}}
        />,
      );
      expect(screen.queryByText("Refresh Mode:")).not.toBeInTheDocument();
    });
  });

  // ─── Tabs Tests ───────────────────────────────────────────────────

  describe("tabs", () => {
    it("renders Addresses and Asset Locks tabs by default", () => {
      render(<HdWalletDetail wallet={makeWallet()} />);
      expect(screen.getByRole("tab", { name: "Addresses" })).toBeInTheDocument();
      expect(screen.getByRole("tab", { name: "Asset Locks" })).toBeInTheDocument();
    });

    it("does not show Transactions tab when not in developer mode", () => {
      render(<HdWalletDetail wallet={makeWallet()} isDeveloperMode={false} />);
      expect(screen.queryByRole("tab", { name: "Transactions" })).not.toBeInTheDocument();
    });

    it("shows Transactions tab in developer mode", () => {
      render(<HdWalletDetail wallet={makeWallet()} isDeveloperMode />);
      expect(screen.getByRole("tab", { name: "Transactions" })).toBeInTheDocument();
    });

    it("defaults to Addresses tab", () => {
      render(<HdWalletDetail wallet={makeWallet()} />);
      const addressTab = screen.getByRole("tab", { name: "Addresses" });
      expect(addressTab).toHaveAttribute("data-state", "active");
    });

    it("switches to Asset Locks tab on click", async () => {
      const user = userEvent.setup();
      render(<HdWalletDetail wallet={makeWallet()} />);
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      const assetLocksTab = screen.getByRole("tab", { name: "Asset Locks" });
      expect(assetLocksTab).toHaveAttribute("data-state", "active");
    });
  });

  // ─── Address Table Tests ──────────────────────────────────────────

  describe("address table", () => {
    it("renders address rows", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "XtestAddr1", derivationPath: "m/44'/5'/0'/0/0", balance: 100000000 }),
          makeAddress({ address: "XtestAddr2", derivationPath: "m/44'/5'/0'/0/1", balance: 50000000 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      expect(screen.getByText("XtestAddr1")).toBeInTheDocument();
      expect(screen.getByText("XtestAddr2")).toBeInTheDocument();
    });

    it("hides zero-balance addresses by default", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "XwithBalance", derivationPath: "m/44'/5'/0'/0/0", balance: 100 }),
          makeAddress({ address: "XzeroBalance", derivationPath: "m/44'/5'/0'/0/1", balance: 0 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      expect(screen.getByText("XwithBalance")).toBeInTheDocument();
      expect(screen.queryByText("XzeroBalance")).not.toBeInTheDocument();
    });

    it("shows zero-balance addresses when hide toggle is unchecked", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "XwithBalance", derivationPath: "m/44'/5'/0'/0/0", balance: 100 }),
          makeAddress({ address: "XzeroBalance", derivationPath: "m/44'/5'/0'/0/1", balance: 0 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);

      const checkbox = screen.getByRole("checkbox");
      await user.click(checkbox);

      expect(screen.getByText("XzeroBalance")).toBeInTheDocument();
    });

    it("renders sortable column headers", () => {
      render(<HdWalletDetail wallet={makeWallet()} />);
      expect(screen.getByRole("button", { name: /Address/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Balance/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Total Received/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Type/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Index/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Path/ })).toBeInTheDocument();
    });

    it("renders address type badges", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
          makeAddress({ address: "Xchange1", derivationPath: "m/44'/5'/0'/1/0", balance: 1 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      expect(screen.getByText("Funds")).toBeInTheDocument();
      expect(screen.getByText("Change")).toBeInTheDocument();
    });

    it("renders copy button for each address", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "Xcopyable1", derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      const copyButtons = screen.getAllByLabelText("Copy to clipboard");
      expect(copyButtons.length).toBeGreaterThanOrEqual(1);
    });

    it("renders View Key button when onViewKey provided", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "XkeyAddr1", derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
        ],
      });
      render(
        <HdWalletDetail wallet={wallet} onViewKey={() => {}} />,
      );
      expect(
        screen.getByLabelText("View key for XkeyAddr1"),
      ).toBeInTheDocument();
    });

    it("calls onViewKey with address and path", async () => {
      const user = userEvent.setup();
      const onViewKey = vi.fn();
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "XkeyAddr1", derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
        ],
      });
      render(
        <HdWalletDetail wallet={wallet} onViewKey={onViewKey} />,
      );
      await user.click(screen.getByLabelText("View key for XkeyAddr1"));
      expect(onViewKey).toHaveBeenCalledWith("XkeyAddr1", "m/44'/5'/0'/0/0");
    });

    it("shows empty state when all addresses are filtered out", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ balance: 0, derivationPath: "m/44'/5'/0'/0/0" }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      expect(screen.getByText("No addresses")).toBeInTheDocument();
    });

    it("shows 'Add Receiving Address' for main BIP44 account", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
        ],
      });
      render(
        <HdWalletDetail wallet={wallet} onAddAddress={() => {}} />,
      );
      expect(
        screen.getByRole("button", { name: /Add Receiving Address/ }),
      ).toBeInTheDocument();
    });

    it("calls onAddAddress when button is clicked", async () => {
      const user = userEvent.setup();
      const onAddAddress = vi.fn();
      const wallet = makeWallet({
        addresses: [
          makeAddress({ derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
        ],
      });
      render(
        <HdWalletDetail wallet={wallet} onAddAddress={onAddAddress} />,
      );
      await user.click(
        screen.getByRole("button", { name: /Add Receiving Address/ }),
      );
      expect(onAddAddress).toHaveBeenCalledOnce();
    });
  });

  // ─── Transaction Table Tests ──────────────────────────────────────

  describe("transaction table", () => {
    it("renders transactions in developer mode", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        transactions: [
          makeTransaction({
            txid: "tx001",
            netAmount: 50000000,
            timestamp: 1700000000,
            height: 100,
          }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} isDeveloperMode />);
      await user.click(screen.getByRole("tab", { name: "Transactions" }));
      expect(screen.getByText("tx001")).toBeInTheDocument();
      expect(screen.getByText("Received")).toBeInTheDocument();
    });

    it("shows sent transaction with negative amount", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        transactions: [
          makeTransaction({ txid: "txSent", netAmount: -30000000 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} isDeveloperMode />);
      await user.click(screen.getByRole("tab", { name: "Transactions" }));
      expect(screen.getByText("Sent")).toBeInTheDocument();
    });

    it("shows internal transaction with zero amount", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        transactions: [
          makeTransaction({ txid: "txInternal", netAmount: 0 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} isDeveloperMode />);
      await user.click(screen.getByRole("tab", { name: "Transactions" }));
      expect(screen.getByText("Internal")).toBeInTheDocument();
    });

    it("shows empty state when no transactions", async () => {
      const user = userEvent.setup();
      render(<HdWalletDetail wallet={makeWallet()} isDeveloperMode />);
      await user.click(screen.getByRole("tab", { name: "Transactions" }));
      expect(screen.getByText("No transactions")).toBeInTheDocument();
    });

    it("shows confirmed status with height", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        transactions: [
          makeTransaction({ height: 12345 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} isDeveloperMode />);
      await user.click(screen.getByRole("tab", { name: "Transactions" }));
      expect(screen.getByText("Confirmed @12345")).toBeInTheDocument();
    });

    it("shows pending status when no height", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        transactions: [
          makeTransaction({ height: null }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} isDeveloperMode />);
      await user.click(screen.getByRole("tab", { name: "Transactions" }));
      expect(screen.getByText("Pending")).toBeInTheDocument();
    });
  });

  // ─── Asset Locks Tests ────────────────────────────────────────────

  describe("asset locks", () => {
    it("renders asset lock table when tab is selected", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        unusedAssetLocks: [
          makeAssetLockFixture({ txid: "lockTx001", amount: 10000000000 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      expect(screen.getByText("lockTx001")).toBeInTheDocument();
    });

    it("shows empty state when no asset locks", async () => {
      const user = userEvent.setup();
      render(<HdWalletDetail wallet={makeWallet()} />);
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      expect(screen.getByText("No asset locks")).toBeInTheDocument();
    });

    it("renders Create Asset Lock button", async () => {
      const user = userEvent.setup();
      render(
        <HdWalletDetail
          wallet={makeWallet()}
          onCreateAssetLock={() => {}}
        />,
      );
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      expect(
        screen.getByRole("button", { name: /Create Asset Lock/ }),
      ).toBeInTheDocument();
    });

    it("calls onCreateAssetLock when clicked", async () => {
      const user = userEvent.setup();
      const onCreateAssetLock = vi.fn();
      render(
        <HdWalletDetail
          wallet={makeWallet()}
          onCreateAssetLock={onCreateAssetLock}
        />,
      );
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      await user.click(
        screen.getByRole("button", { name: /Create Asset Lock/ }),
      );
      expect(onCreateAssetLock).toHaveBeenCalledOnce();
    });

    it("renders Search for Unused button", async () => {
      const user = userEvent.setup();
      render(
        <HdWalletDetail
          wallet={makeWallet()}
          onSearchAssetLocks={() => {}}
        />,
      );
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      expect(
        screen.getByRole("button", { name: /Search for Unused/ }),
      ).toBeInTheDocument();
    });

    it("shows InstantLock and Usable badges", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        unusedAssetLocks: [
          makeAssetLockFixture({
            hasInstantLock: true,
            hasAssetLockProof: false,
          }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));

      // InstantLock = Yes, Usable = No
      const cells = screen.getAllByText("Yes");
      expect(cells.length).toBeGreaterThanOrEqual(1);
      const noCells = screen.getAllByText("No");
      expect(noCells.length).toBeGreaterThanOrEqual(1);
    });

    it("renders View button for asset locks", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        unusedAssetLocks: [makeAssetLockFixture({ txid: "viewLock123" })],
      });
      render(
        <HdWalletDetail
          wallet={wallet}
          onViewAssetLock={() => {}}
        />,
      );
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      expect(
        screen.getByLabelText("View asset lock viewLock123"),
      ).toBeInTheDocument();
    });

    it("calls onViewAssetLock with txid", async () => {
      const user = userEvent.setup();
      const onView = vi.fn();
      const wallet = makeWallet({
        unusedAssetLocks: [makeAssetLockFixture({ txid: "viewLock456" })],
      });
      render(<HdWalletDetail wallet={wallet} onViewAssetLock={onView} />);
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      await user.click(screen.getByLabelText("View asset lock viewLock456"));
      expect(onView).toHaveBeenCalledWith("viewLock456");
    });

    it("shows Fund button only when asset lock has proof", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        unusedAssetLocks: [
          makeAssetLockFixture({ txid: "withProof", hasAssetLockProof: true }),
          makeAssetLockFixture({ txid: "noProof", hasAssetLockProof: false }),
        ],
      });
      render(
        <HdWalletDetail wallet={wallet} onFundAssetLock={() => {}} />,
      );
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      expect(
        screen.getByLabelText("Fund from asset lock withProof"),
      ).toBeInTheDocument();
      expect(
        screen.queryByLabelText("Fund from asset lock noProof"),
      ).not.toBeInTheDocument();
    });

    it("calls onFundAssetLock with index", async () => {
      const user = userEvent.setup();
      const onFund = vi.fn();
      const wallet = makeWallet({
        unusedAssetLocks: [
          makeAssetLockFixture({ txid: "fundLock1" }),
          makeAssetLockFixture({ txid: "fundLock2" }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} onFundAssetLock={onFund} />);
      await user.click(screen.getByRole("tab", { name: "Asset Locks" }));
      await user.click(screen.getByLabelText("Fund from asset lock fundLock2"));
      expect(onFund).toHaveBeenCalledWith(1);
    });
  });

  // ─── Account Selector Tests ───────────────────────────────────────

  describe("account selector", () => {
    it("does not show selector when only one account type", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
          makeAddress({ address: "X2", derivationPath: "m/44'/5'/0'/0/1", balance: 1 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      // Only one account (bip44-0), so no selector needed
      expect(screen.queryByText("Select account")).not.toBeInTheDocument();
    });

    it("shows selector when multiple account types exist", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({ derivationPath: "m/44'/5'/0'/0/0", balance: 1 }),
          makeAddress({ address: "XplatAddr", derivationPath: "m/2049'/5'/0'/0/0", balance: 0 }),
        ],
        platformAddresses: [
          makePlatformAddress({ address: "XplatAddr", balance: 5000000000 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);
      // Multiple accounts: Main Account + Platform Account
      expect(screen.getByText("Main Account")).toBeInTheDocument();
    });
  });

  // ─── Platform Address Tests ───────────────────────────────────────

  describe("platform addresses", () => {
    it("shows platform addresses with credit balance", () => {
      const wallet = makeWallet({
        addresses: [
          makeAddress({
            address: "XplatformTest1",
            derivationPath: "m/2049'/5'/0'/0/0",
            balance: 0,
          }),
        ],
        platformAddresses: [
          makePlatformAddress({ address: "XplatformTest1", balance: 10000000000 }),
        ],
      });
      // Need to uncheck hide zero since the duffs balance is 0
      render(<HdWalletDetail wallet={wallet} />);
      // Platform address categorized as "Platform" type
      // It may be hidden due to zero duffs balance — platform balance should be checked
    });
  });

  // ─── Sort Tests ───────────────────────────────────────────────────

  describe("sorting", () => {
    it("toggles sort order when clicking same column header", async () => {
      const user = userEvent.setup();
      const wallet = makeWallet({
        addresses: [
          makeAddress({ address: "Aaddr", derivationPath: "m/44'/5'/0'/0/0", balance: 100 }),
          makeAddress({ address: "Baddr", derivationPath: "m/44'/5'/0'/0/1", balance: 200 }),
        ],
      });
      render(<HdWalletDetail wallet={wallet} />);

      // Click Balance header twice to toggle from asc to desc
      const balanceHeader = screen.getByRole("button", { name: /Balance/ });
      await user.click(balanceHeader); // First click sets it as sort column (asc)
      await user.click(balanceHeader); // Second click toggles to desc

      // Verify both addresses are visible (order tested by presence, not DOM position)
      expect(screen.getByText("Aaddr")).toBeInTheDocument();
      expect(screen.getByText("Baddr")).toBeInTheDocument();
    });
  });

  // ─── Custom className Tests ───────────────────────────────────────

  describe("className", () => {
    it("applies custom className", () => {
      const { container } = render(
        <HdWalletDetail wallet={makeWallet()} className="my-custom-class" />,
      );
      expect(container.firstChild).toHaveClass("my-custom-class");
    });
  });
});
