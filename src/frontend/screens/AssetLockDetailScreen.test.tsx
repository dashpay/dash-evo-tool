import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AssetLockDetailScreen } from "./AssetLockDetailScreen";

// ─── Mocks ──────────────────────────────────────────────────────────

const mockNavigate = vi.fn();
let mockParams: Record<string, string> = { txid: "abc123txid456" };
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useParams: () => mockParams,
}));

const mockWalletNotifyUnlocked = vi.fn();
const mockWalletGetPrivateKey = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    walletNotifyUnlocked: (...args: unknown[]) =>
      mockWalletNotifyUnlocked(...args),
    walletGetPrivateKey: (...args: unknown[]) =>
      mockWalletGetPrivateKey(...args),
  },
}));

let walletStoreState: Record<string, unknown> = {};
vi.mock("@/stores/walletStore", () => ({
  useWalletStore: () => walletStoreState,
}));

// ─── Helpers ────────────────────────────────────────────────────────

function makeAssetLock(overrides: Record<string, unknown> = {}) {
  return {
    txid: "abc123txid456",
    address: "yAssetLockAddress123456789",
    amount: 50000000000,
    hasInstantLock: true,
    hasAssetLockProof: true,
    ...overrides,
  };
}

function makeHdWallet(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: "seed123",
    alias: "Test Wallet",
    confirmedBalance: 1000000000,
    unconfirmedBalance: 0,
    usesPassword: false,
    passwordHint: null,
    addresses: [
      {
        address: "yAssetLockAddress123456789",
        derivationPath: "m/44'/5'/0'/0/0",
        balance: 0,
        totalReceived: 50000000,
        accountIndex: 0,
        addressIndex: 0,
      },
    ],
    platformAddresses: [],
    unusedAssetLocks: [makeAssetLock()],
    accounts: [],
    transactions: [],
    ...overrides,
  };
}

function setupWallet(
  walletOverrides: Record<string, unknown> = {},
  assetLockOverrides: Record<string, unknown> = {},
) {
  const lock = makeAssetLock(assetLockOverrides);
  const wallet = makeHdWallet({
    unusedAssetLocks: [lock],
    ...walletOverrides,
  });
  walletStoreState = {
    hdWallets: [wallet],
    selectedWallet: { type: "hd", seedHash: wallet.seedHash },
  };
  return { wallet, lock };
}

// ─── Tests ──────────────────────────────────────────────────────────

describe("AssetLockDetailScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    walletStoreState = { hdWallets: [], selectedWallet: null };
    mockParams = { txid: "abc123txid456" };
  });

  describe("no wallet selected", () => {
    it("shows no wallet message", () => {
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("No HD wallet selected")).toBeInTheDocument();
    });

    it("shows back button", async () => {
      render(<AssetLockDetailScreen />);
      await userEvent.click(screen.getByText("Back to Wallets"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });
  });

  describe("asset lock not found", () => {
    it("shows not found message", () => {
      const wallet = makeHdWallet({ unusedAssetLocks: [] });
      walletStoreState = {
        hdWallets: [wallet],
        selectedWallet: { type: "hd", seedHash: wallet.seedHash },
      };
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Asset lock not found")).toBeInTheDocument();
    });
  });

  describe("transaction information", () => {
    it("shows asset lock detail heading", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Asset Lock Detail")).toBeInTheDocument();
    });

    it("shows wallet name", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Test Wallet")).toBeInTheDocument();
    });

    it("shows transaction information heading", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Transaction Information")).toBeInTheDocument();
    });

    it("shows transaction ID", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("abc123txid456")).toBeInTheDocument();
    });

    it("shows address", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("yAssetLockAddress123456789")).toBeInTheDocument();
    });

    it("shows amount in DASH", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText(/0\.50000000 DASH/)).toBeInTheDocument();
    });

    it("shows amount in duffs", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText(/50,000,000 duffs/)).toBeInTheDocument();
    });

    it("shows copy buttons", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByLabelText("Copy TX ID")).toBeInTheDocument();
      expect(screen.getByLabelText("Copy address")).toBeInTheDocument();
    });
  });

  describe("proof status", () => {
    it("shows Instant Send Locked for instant lock with proof", () => {
      setupWallet({}, { hasInstantLock: true, hasAssetLockProof: true });
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Instant Send Locked")).toBeInTheDocument();
    });

    it("shows Chain Locked for non-instant lock with proof", () => {
      setupWallet({}, { hasInstantLock: false, hasAssetLockProof: true });
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Chain Locked")).toBeInTheDocument();
    });

    it("shows Waiting for Lock when no proof", () => {
      setupWallet({}, { hasInstantLock: false, hasAssetLockProof: false });
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Waiting for Lock")).toBeInTheDocument();
    });

    it("shows Usable Not yet badge when no proof", () => {
      setupWallet({}, { hasAssetLockProof: false, hasInstantLock: false });
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Not yet")).toBeInTheDocument();
    });
  });

  describe("private key section", () => {
    it("shows private key heading", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("Private Key")).toBeInTheDocument();
    });

    it("shows security warning", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText(/Keep your private key secure/)).toBeInTheDocument();
    });

    it("shows view private key button", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByText("View Private Key")).toBeInTheDocument();
    });

    it("fetches and shows private key on click", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockWalletGetPrivateKey.mockResolvedValue({
        status: "ok", data: "cWIFkey123456789abcdefghijklmnop",
      });
      render(<AssetLockDetailScreen />);
      await user.click(screen.getByText("View Private Key"));

      await waitFor(() => {
        expect(screen.getByText("cWIFkey123456789abcdefghijklmnop")).toBeInTheDocument();
      });
    });

    it("shows hide key button after revealing", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockWalletGetPrivateKey.mockResolvedValue({
        status: "ok", data: "cWIFkey123456789abcdefghijklmnop",
      });
      render(<AssetLockDetailScreen />);
      await user.click(screen.getByText("View Private Key"));

      await waitFor(() => {
        expect(screen.getByText("Hide Key")).toBeInTheDocument();
      });
    });

    it("hides key on hide click", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockWalletGetPrivateKey.mockResolvedValue({
        status: "ok", data: "cWIFkey123456789abcdefghijklmnop",
      });
      render(<AssetLockDetailScreen />);
      await user.click(screen.getByText("View Private Key"));
      await waitFor(() => { expect(screen.getByText("cWIFkey123456789abcdefghijklmnop")).toBeInTheDocument(); });
      await user.click(screen.getByText("Hide Key"));
      expect(screen.queryByText("cWIFkey123456789abcdefghijklmnop")).not.toBeInTheDocument();
      expect(screen.getByText("View Private Key")).toBeInTheDocument();
    });

    it("shows copy button when key is revealed", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockWalletGetPrivateKey.mockResolvedValue({
        status: "ok", data: "cWIFkey123456789abcdefghijklmnop",
      });
      render(<AssetLockDetailScreen />);
      await user.click(screen.getByText("View Private Key"));

      await waitFor(() => {
        expect(screen.getByLabelText("Copy private key")).toBeInTheDocument();
      });
    });

    it("shows error when key fetch fails", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockWalletGetPrivateKey.mockResolvedValue({
        status: "error", error: "Failed to get key",
      });
      render(<AssetLockDetailScreen />);
      await user.click(screen.getByText("View Private Key"));

      await waitFor(() => {
        expect(screen.getByText("Failed to get key")).toBeInTheDocument();
      });
    });

    it("requests unlock when wallet has password", () => {
      setupWallet({ usesPassword: true });
      render(<AssetLockDetailScreen />);
      expect(screen.getByText(/Unlock your wallet to view/)).toBeInTheDocument();
    });
  });

  describe("navigation", () => {
    it("navigates back via header button", async () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      await userEvent.click(screen.getByLabelText("Back to wallets"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets" });
    });
  });

  describe("accessibility", () => {
    it("has proper heading levels", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Asset Lock Detail");
      expect(screen.getByRole("heading", { level: 2, name: "Transaction Information" })).toBeInTheDocument();
      expect(screen.getByRole("heading", { level: 2, name: "Private Key" })).toBeInTheDocument();
    });

    it("has alert role on security warning", () => {
      setupWallet();
      render(<AssetLockDetailScreen />);
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });

    it("has aria label for private key display", async () => {
      const user = userEvent.setup();
      setupWallet();
      mockWalletGetPrivateKey.mockResolvedValue({
        status: "ok", data: "cWIFkey123456789",
      });
      render(<AssetLockDetailScreen />);
      await user.click(screen.getByText("View Private Key"));

      await waitFor(() => {
        expect(screen.getByLabelText("Private key WIF")).toBeInTheDocument();
      });
    });
  });
});
