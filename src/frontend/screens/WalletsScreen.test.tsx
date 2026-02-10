import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { WalletsScreen } from "./WalletsScreen";
import { useWalletStore } from "@/stores/walletStore";
import type { WalletDto, SingleKeyWalletDto } from "@/bindings";

// ─── Centralized mock bindings ───────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

// Mock sonner toast
vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
    info: vi.fn(),
  },
  Toaster: () => null,
}));

// Mock react-router navigation
const { mockNavigate } = vi.hoisted(() => ({
  mockNavigate: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

import { commands, events } from "@/bindings";

// ─── Test Fixtures ────────────────────────────────────────────────

const makeHdWallet = (overrides: Partial<WalletDto> = {}): WalletDto => ({
  seedHash: "aa".repeat(32),
  usesPassword: false,
  alias: "Test HD Wallet",
  isMain: true,
  totalBalance: 100000000,
  confirmedBalance: 100000000,
  unconfirmedBalance: 0,
  utxoCount: 2,
  addresses: [
    {
      address: "XpYvN123abc",
      balance: 50000000,
      totalReceived: 100000000,
      derivationPath: "m/44'/5'/0'/0/0",
    },
  ],
  transactions: [],
  unusedAssetLocks: [],
  platformAddresses: [],
  identityIndexes: [],
  passwordHint: null,
  ...overrides,
});

const makeSingleKeyWallet = (
  overrides: Partial<SingleKeyWalletDto> = {},
): SingleKeyWalletDto => ({
  keyHash: "bb".repeat(32),
  usesPassword: false,
  publicKey: "cc".repeat(33),
  address: "XsingleKey456",
  alias: "Test Single Key",
  totalBalance: 50000000,
  balance: 50000000,
  unconfirmedBalance: 0,
  utxos: [],
  ...overrides,
});

// ─── Helpers ──────────────────────────────────────────────────────

function setupMocksWithWallets(opts: {
  hdWallets?: WalletDto[];
  singleKeyWallets?: SingleKeyWalletDto[];
  selected?: { type: "hd"; seedHash: string } | { type: "singleKey"; keyHash: string } | null;
  devMode?: boolean;
} = {}) {
  const hd = opts.hdWallets ?? [];
  const sk = opts.singleKeyWallets ?? [];
  const selected = opts.selected ?? null;

  vi.mocked(commands.walletListAll).mockResolvedValue({
    status: "ok",
    data: { hdWallets: hd, singleKeyWallets: sk, selected },
  });
  vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(
    opts.devMode ?? false,
  );
  // For refreshing an HD wallet, make walletGetHd return the same wallet
  if (hd.length > 0) {
    vi.mocked(commands.walletGetHd).mockImplementation(async (hash: string) => {
      const found = hd.find((x) => x.seedHash === hash);
      return found ? { status: "ok", data: found } : { status: "error", error: "not found" };
    });
  }
  if (sk.length > 0) {
    vi.mocked(commands.walletGetSingleKey).mockImplementation(async (hash: string) => {
      const found = sk.find((x) => x.keyHash === hash);
      return found ? { status: "ok", data: found } : { status: "error", error: "not found" };
    });
  }
}

function resetStore() {
  useWalletStore.setState({
    hdWallets: [],
    singleKeyWallets: [],
    selectedWallet: null,
    loading: false,
    refreshing: false,
    error: null,
    refreshMode: "coreAndPlatformAuto",
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
  // Reset to defaults
  vi.mocked(commands.walletListAll).mockResolvedValue({
    status: "ok",
    data: { hdWallets: [], singleKeyWallets: [], selected: null },
  });
  vi.mocked(commands.contextIsDeveloperMode).mockResolvedValue(false);
});

// ─── Tests ────────────────────────────────────────────────────────

describe("WalletsScreen", () => {
  describe("loading state", () => {
    it("shows a loading spinner while wallets are loading", () => {
      // Prevent the useEffect from completing by making loadWallets hang
      vi.mocked(commands.walletListAll).mockReturnValue(new Promise(() => {})); // never resolves
      useWalletStore.setState({ loading: true });
      render(<WalletsScreen />);
      expect(screen.getByText("Loading wallets...")).toBeInTheDocument();
    });
  });

  describe("empty state", () => {
    it("shows empty state when no wallets exist", async () => {
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("No Wallets Loaded")).toBeInTheDocument();
      });
    });

    it("shows 'Select a wallet' message when no wallet is selected", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({ hdWallets: [hd], selected: null });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("Select a wallet to view details")).toBeInTheDocument();
      });
    });
  });

  describe("wallet list display", () => {
    it("renders HD wallet in list after loading", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getAllByText("Test HD Wallet").length).toBeGreaterThanOrEqual(1);
      });
    });

    it("renders single-key wallet in list after loading", async () => {
      const sk = makeSingleKeyWallet();
      setupMocksWithWallets({ singleKeyWallets: [sk], selected: null });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("Test Single Key")).toBeInTheDocument();
      });
    });

    it("renders both HD and single-key wallets", async () => {
      const hd = makeHdWallet();
      const sk = makeSingleKeyWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        singleKeyWallets: [sk],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getAllByText("Test HD Wallet").length).toBeGreaterThanOrEqual(1);
        expect(screen.getByText("Test Single Key")).toBeInTheDocument();
      });
    });
  });

  describe("HD wallet detail", () => {
    it("shows HD wallet detail when HD wallet is selected", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText(/Core balance/)).toBeInTheDocument();
      });
    });

    it("shows Send button for HD wallet", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        // Find Send button in the detail panel (not in menus)
        const sendButtons = screen.getAllByRole("button", { name: /Send/ });
        expect(sendButtons.length).toBeGreaterThanOrEqual(1);
      });
    });

    it("shows Receive button for HD wallet", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        const receiveButtons = screen.getAllByRole("button", { name: /Receive/ });
        expect(receiveButtons.length).toBeGreaterThanOrEqual(1);
      });
    });

    it("shows Refresh button for HD wallet", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        const refreshButtons = screen.getAllByRole("button", { name: /Refresh/ });
        expect(refreshButtons.length).toBeGreaterThanOrEqual(1);
      });
    });

    it("shows Addresses and Asset Locks tabs", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByRole("tab", { name: "Addresses" })).toBeInTheDocument();
        expect(screen.getByRole("tab", { name: "Asset Locks" })).toBeInTheDocument();
      });
    });

    it("does not show Transactions tab when not in developer mode", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
        devMode: false,
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByRole("tab", { name: "Addresses" })).toBeInTheDocument();
      });
      expect(screen.queryByRole("tab", { name: "Transactions" })).not.toBeInTheDocument();
    });
  });

  describe("single-key wallet detail", () => {
    it("shows single-key wallet detail when selected", async () => {
      const sk = makeSingleKeyWallet();
      setupMocksWithWallets({
        singleKeyWallets: [sk],
        selected: { type: "singleKey", keyHash: sk.keyHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("XsingleKey456")).toBeInTheDocument();
      });
    });

    it("shows UTXO section heading", async () => {
      const sk = makeSingleKeyWallet();
      setupMocksWithWallets({
        singleKeyWallets: [sk],
        selected: { type: "singleKey", keyHash: sk.keyHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("UTXOs (0)")).toBeInTheDocument();
      });
    });
  });

  describe("navigation callbacks", () => {
    it("navigates to create wallet on Create Wallet click", async () => {
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("No Wallets Loaded")).toBeInTheDocument();
      });
      await user.click(screen.getByRole("button", { name: /Create Wallet/ }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets/create" });
    });

    it("navigates to import wallet on Import Wallet click", async () => {
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByText("No Wallets Loaded")).toBeInTheDocument();
      });
      await user.click(screen.getByRole("button", { name: /Import Wallet/ }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets/import" });
    });

    it("navigates to HD send screen on Send click", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getAllByRole("button", { name: /Send/ }).length).toBeGreaterThanOrEqual(1);
      });
      // Click the first Send button (in the detail panel)
      await user.click(screen.getAllByRole("button", { name: /Send/ })[0]!);
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/wallets/send/hd" });
    });
  });

  describe("receive dialog", () => {
    it("opens receive dialog on Receive click", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getAllByRole("button", { name: /Receive/ }).length).toBeGreaterThanOrEqual(1);
      });
      await user.click(screen.getAllByRole("button", { name: /Receive/ })[0]!);
      await waitFor(() => {
        expect(screen.getByText(/Receive — Test HD Wallet/)).toBeInTheDocument();
      });
    });
  });

  describe("view key flow", () => {
    it("fetches and shows private key for non-password wallet", async () => {
      const hd = makeHdWallet({ usesPassword: false });
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByRole("tab", { name: "Addresses" })).toBeInTheDocument();
      });
      const viewKeyBtn = screen.getByRole("button", { name: /View key for/ });
      await user.click(viewKeyBtn);
      await waitFor(() => {
        expect(screen.getByText("Private Key")).toBeInTheDocument();
      });
    });

    it("shows unlock dialog for password-protected wallet before viewing key", async () => {
      const hd = makeHdWallet({ usesPassword: true, passwordHint: "Hint!" });
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByRole("tab", { name: "Addresses" })).toBeInTheDocument();
      });
      const viewKeyBtn = screen.getByRole("button", { name: /View key for/ });
      await user.click(viewKeyBtn);
      await waitFor(() => {
        // The unlock dialog should appear
        expect(screen.getByText("Unlock Wallet")).toBeInTheDocument();
      });
    });
  });

  describe("refresh", () => {
    it("calls refreshSelectedWallet on Refresh click", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      const user = userEvent.setup();
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getAllByRole("button", { name: /Refresh/ }).length).toBeGreaterThanOrEqual(1);
      });
      await user.click(screen.getAllByRole("button", { name: /Refresh/ })[0]!);
      await waitFor(() => {
        expect(commands.coreRefreshWalletInfo).toHaveBeenCalled();
      });
    });
  });

  describe("layout", () => {
    it("renders split-pane layout with list and detail panels", async () => {
      const hd = makeHdWallet();
      setupMocksWithWallets({
        hdWallets: [hd],
        selected: { type: "hd", seedHash: hd.seedHash },
      });
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(screen.getByRole("region", { name: "Wallet list" })).toBeInTheDocument();
      });
    });
  });

  describe("initialization", () => {
    it("loads wallets on mount", async () => {
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(commands.walletListAll).toHaveBeenCalled();
      });
    });

    it("subscribes to wallet update events", async () => {
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(events.walletUpdatedEvent.listen).toHaveBeenCalled();
      });
    });

    it("checks developer mode on mount", async () => {
      render(<WalletsScreen />);
      await waitFor(() => {
        expect(commands.contextIsDeveloperMode).toHaveBeenCalled();
      });
    });
  });
});
