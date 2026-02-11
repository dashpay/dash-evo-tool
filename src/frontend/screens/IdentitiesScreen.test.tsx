import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { IdentitiesScreen } from "./IdentitiesScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { renderWithProviders } from "@/test/router-utils";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Mock Tauri bindings ──────────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

// Mock sonner toast
const { mockToast } = vi.hoisted(() => ({
  mockToast: {
    error: vi.fn(),
    success: vi.fn(),
    info: vi.fn(),
  },
}));

vi.mock("sonner", () => ({
  toast: mockToast,
  Toaster: () => null,
}));

const { mockToastError } = vi.hoisted(() => ({
  mockToastError: vi.fn(),
}));
vi.mock("@/lib/toastError", () => ({
  toastError: mockToastError,
}));

// ─── Test Fixtures ────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    purpose: "AUTHENTICATION",
    securityLevel: "HIGH",
    keyType: "ECDSA_SECP256K1",
    data: "aabb".repeat(16),
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
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Test Identity",
    balance: 1000000000,
    keys: [makeKey()],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: null,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

// ─── Helpers ──────────────────────────────────────────────────────

function setupMocksWithIdentities(identities: QualifiedIdentityDto[]) {
  vi.mocked(commands.identityListLocal).mockResolvedValue({
    status: "ok",
    data: identities,
  });
  vi.mocked(commands.identityGetById).mockImplementation(async (id: string) => {
    const found = identities.find((i) => i.id === id);
    return found
      ? { status: "ok", data: found }
      : { status: "ok", data: null };
  });
}

function resetStores() {
  useIdentityStore.setState({
    identities: [],
    selectedIdentityId: null,
    loading: false,
    refreshingIds: new Set(),
    refreshingAll: false,
    error: null,
    sortColumn: "alias",
    sortOrder: "ascending",
    useCustomOrder: true,
  });
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
  resetStores();
  vi.mocked(commands.identityListLocal).mockResolvedValue({
    status: "ok",
    data: [],
  });
  vi.mocked(commands.identityLoadOrder).mockResolvedValue({
    status: "ok",
    data: [],
  });
  vi.mocked(commands.walletListAll).mockResolvedValue({
    status: "ok",
    data: { hdWallets: [], singleKeyWallets: [], selected: null },
  });
});

// ─── Tests ────────────────────────────────────────────────────────

describe("IdentitiesScreen", () => {
  describe("loading state", () => {
    it("shows a loading spinner while identities are loading", () => {
      vi.mocked(commands.identityListLocal).mockReturnValue(new Promise(() => {}));
      useIdentityStore.setState({ loading: true });
      renderWithProviders(<IdentitiesScreen />);
      expect(screen.getByText("Loading identities...")).toBeInTheDocument();
    });
  });

  describe("empty state", () => {
    it("shows empty state when no identities exist", async () => {
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(
          screen.getByText("No Identities Loaded"),
        ).toBeInTheDocument();
      });
    });

    it('shows "Select an identity" message when no identity is selected', async () => {
      const identity = makeIdentity();
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(
          screen.getByText("Select an identity to view details"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("identity list", () => {
    it("loads and displays identities on mount", async () => {
      const identity = makeIdentity({ alias: "My Identity" });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(screen.getByText("My Identity")).toBeInTheDocument();
      });
    });

    it("displays multiple identities", async () => {
      const id1 = makeIdentity({
        id: "aa".repeat(32),
        alias: "Identity One",
      });
      const id2 = makeIdentity({
        id: "bb".repeat(32),
        alias: "Identity Two",
      });
      setupMocksWithIdentities([id1, id2]);
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(screen.getByText("Identity One")).toBeInTheDocument();
        expect(screen.getByText("Identity Two")).toBeInTheDocument();
      });
    });

    it("shows identity count badge", async () => {
      const id1 = makeIdentity({ id: "aa".repeat(32), alias: "ID 1" });
      const id2 = makeIdentity({ id: "bb".repeat(32), alias: "ID 2" });
      setupMocksWithIdentities([id1, id2]);
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(screen.getByText("2")).toBeInTheDocument();
      });
    });
  });

  describe("identity selection", () => {
    it("selects an identity and shows detail panel", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "Click Me",
        balance: 500000000,
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("Click Me")).toBeInTheDocument();
      });

      // Click the identity card
      await user.click(screen.getByText("Click Me"));

      // Should no longer show the placeholder
      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });
    });
  });

  describe("identity refresh", () => {
    it("dispatches refresh when refresh button is clicked", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({ alias: "RefreshMe" });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("RefreshMe")).toBeInTheDocument();
      });

      // Open the context menu (more button) on the identity card
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click Refresh in the dropdown
      const refreshItem = await screen.findByRole("menuitem", {
        name: /refresh/i,
      });
      await user.click(refreshItem);

      await waitFor(() => {
        expect(vi.mocked(commands.identityRefresh)).toHaveBeenCalledWith({
          identityId: identity.id,
        });
      });
    });
  });

  describe("alias editing", () => {
    it("allows alias editing via context menu", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({ alias: "OldAlias" });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("OldAlias")).toBeInTheDocument();
      });

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click "Update Alias"
      const renameItem = await screen.findByRole("menuitem", {
        name: /update alias/i,
      });
      await user.click(renameItem);

      // Should show an inline input
      const input = await screen.findByLabelText("Identity alias");
      await user.clear(input);
      await user.type(input, "NewAlias{Enter}");

      await waitFor(() => {
        expect(vi.mocked(commands.identitySetAlias)).toHaveBeenCalledWith({
          identityId: identity.id,
          alias: "NewAlias",
        });
      });
    });
  });

  describe("identity removal", () => {
    it("shows confirmation dialog before removal", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({ alias: "DeleteMe" });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("DeleteMe")).toBeInTheDocument();
      });

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click Remove
      const removeItem = await screen.findByRole("menuitem", {
        name: /remove/i,
      });
      await user.click(removeItem);

      // Confirmation dialog should appear
      await waitFor(() => {
        expect(screen.getByText("Confirm Removal")).toBeInTheDocument();
      });
    });

    it("removes identity after confirmation", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({ alias: "DeleteMe" });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("DeleteMe")).toBeInTheDocument();
      });

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click Remove
      const removeItem = await screen.findByRole("menuitem", {
        name: /remove/i,
      });
      await user.click(removeItem);

      // Confirm
      const confirmButton = await screen.findByRole("button", {
        name: /remove/i,
      });
      await user.click(confirmButton);

      await waitFor(() => {
        expect(vi.mocked(commands.identityDelete)).toHaveBeenCalledWith({
          identityId: identity.id,
        });
      });
    });
  });

  describe("reordering", () => {
    it("renders drag handles for identity reordering", async () => {
      const id1 = makeIdentity({
        id: "aa".repeat(32),
        alias: "First",
      });
      const id2 = makeIdentity({
        id: "bb".repeat(32),
        alias: "Second",
      });
      setupMocksWithIdentities([id1, id2]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("First")).toBeInTheDocument();
        expect(screen.getByText("Second")).toBeInTheDocument();
      });

      // Verify drag handles are rendered for reordering
      const handles = screen.getAllByLabelText("Drag to reorder");
      expect(handles).toHaveLength(2);
    });
  });

  describe("sub-view navigation", () => {
    it("navigates to key management when View Keys is clicked", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "KeysIdentity",
        keys: [makeKey({ keyId: 1, purpose: "AUTHENTICATION" })],
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("KeysIdentity")).toBeInTheDocument();
      });

      // Select the identity first
      await user.click(screen.getByText("KeysIdentity"));

      // Wait for detail panel to appear (no placeholder)
      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click View Keys
      const viewKeysItem = await screen.findByRole("menuitem", {
        name: /view keys/i,
      });
      await user.click(viewKeysItem);

      // Should now show the key management screen (heading is "Identity Keys")
      await waitFor(() => {
        expect(screen.getByText("Identity Keys")).toBeInTheDocument();
      });
    });

    it("navigates to withdraw screen via context menu", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "WithdrawIdentity",
        balance: 1000000000,
        keys: [
          makeKey({
            keyId: 5,
            purpose: "TRANSFER",
            securityLevel: "HIGH",
            hasPrivateKey: true,
          }),
        ],
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("WithdrawIdentity")).toBeInTheDocument();
      });

      // Select the identity
      await user.click(screen.getByText("WithdrawIdentity"));

      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click Withdraw
      const withdrawItem = await screen.findByRole("menuitem", {
        name: /withdraw/i,
      });
      await user.click(withdrawItem);

      // Should show the withdraw screen (heading is "Withdraw Funds")
      await waitFor(() => {
        expect(screen.getByText("Withdraw Funds")).toBeInTheDocument();
      });
    });

    it("navigates to transfer screen via context menu", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "TransferIdentity",
        balance: 1000000000,
        keys: [
          makeKey({
            keyId: 5,
            purpose: "TRANSFER",
            securityLevel: "HIGH",
            hasPrivateKey: true,
          }),
        ],
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("TransferIdentity")).toBeInTheDocument();
      });

      // Select the identity
      await user.click(screen.getByText("TransferIdentity"));

      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click Transfer
      const transferItem = await screen.findByRole("menuitem", {
        name: /transfer/i,
      });
      await user.click(transferItem);

      // Should show the transfer screen (heading is "Transfer Funds")
      await waitFor(() => {
        expect(screen.getByText("Transfer Funds")).toBeInTheDocument();
      });
    });

    it("resets sub-view when selecting a different identity", async () => {
      const user = userEvent.setup();
      const id1 = makeIdentity({
        id: "aa".repeat(32),
        alias: "First",
        keys: [makeKey()],
      });
      const id2 = makeIdentity({
        id: "bb".repeat(32),
        alias: "Second",
        keys: [makeKey()],
      });
      setupMocksWithIdentities([id1, id2]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("First")).toBeInTheDocument();
      });

      // Select first identity
      await user.click(screen.getByText("First"));

      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });

      // Navigate to keys via the first identity's context menu
      // There are multiple "Identity actions" buttons (one per card), get the first
      const actionsButtons = screen.getAllByLabelText("Identity actions");
      await user.click(actionsButtons[0]);
      const viewKeysItem = await screen.findByRole("menuitem", {
        name: /view keys/i,
      });
      await user.click(viewKeysItem);

      await waitFor(() => {
        expect(screen.getByText("Identity Keys")).toBeInTheDocument();
      });

      // Select second identity — should reset to detail view
      await user.click(screen.getByText("Second"));

      await waitFor(() => {
        expect(screen.queryByText("Identity Keys")).not.toBeInTheDocument();
      });
    });
  });

  describe("direct key viewing from detail panel", () => {
    it("opens key info when clicking a key in detail panel and Back returns to detail", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "KeyDetail",
        keys: [
          makeKey({ keyId: 0, purpose: "AUTHENTICATION", securityLevel: "MASTER" }),
          makeKey({ keyId: 1, purpose: "TRANSFER", securityLevel: "HIGH" }),
        ],
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("KeyDetail")).toBeInTheDocument();
      });

      // Select the identity
      await user.click(screen.getByText("KeyDetail"));

      // Wait for detail panel keys section
      await waitFor(() => {
        expect(screen.getByText("Keys")).toBeInTheDocument();
      });

      // Click a key item in the detail panel (key #1 — T — High)
      const keyButton = screen.getByRole("button", {
        name: /Key 1: TRANSFER HIGH/i,
      });
      await user.click(keyButton);

      // Should show key info screen
      await waitFor(() => {
        expect(screen.getByText("Key Info")).toBeInTheDocument();
        expect(screen.getByText(/Key #1/)).toBeInTheDocument();
      });

      // Click back — should return to detail panel (not key management)
      const backButton = screen.getByRole("button", {
        name: /back to identity/i,
      });
      await user.click(backButton);

      // Should be back on detail panel — Keys section visible, no "Identity Keys" heading
      await waitFor(() => {
        expect(screen.getByText("Keys")).toBeInTheDocument();
        expect(screen.queryByText("Identity Keys")).not.toBeInTheDocument();
      });
    });

    it("opens key info from key management and Back returns to key management", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "KeyMgmt",
        keys: [
          makeKey({ keyId: 0, purpose: "AUTHENTICATION", securityLevel: "MASTER" }),
        ],
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("KeyMgmt")).toBeInTheDocument();
      });

      // Select the identity
      await user.click(screen.getByText("KeyMgmt"));

      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });

      // Navigate to key management via context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);
      const viewKeysItem = await screen.findByRole("menuitem", {
        name: /view keys/i,
      });
      await user.click(viewKeysItem);

      await waitFor(() => {
        expect(screen.getByText("Identity Keys")).toBeInTheDocument();
      });

      // Click a key row in key management screen
      const viewButton = screen.getByRole("button", { name: /view/i });
      await user.click(viewButton);

      // Should show key info
      await waitFor(() => {
        expect(screen.getByText("Key Info")).toBeInTheDocument();
      });

      // Click back — should return to key management (not detail)
      const backButton = screen.getByRole("button", {
        name: /back to keys/i,
      });
      await user.click(backButton);

      // Should be back on key management screen
      await waitFor(() => {
        expect(screen.getByText("Identity Keys")).toBeInTheDocument();
      });
    });
  });

  describe("error handling", () => {
    it("shows toast when store has an error", async () => {
      setupMocksWithIdentities([]);
      renderWithProviders(<IdentitiesScreen />);

      // Simulate error
      useIdentityStore.setState({ error: "Something went wrong" });

      await waitFor(() => {
        expect(mockToastError).toHaveBeenCalledWith("Something went wrong");
      });
    });
  });

  describe("event subscription", () => {
    it("subscribes to task result events on mount", async () => {
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(vi.mocked(events.taskResultEvent.listen)).toHaveBeenCalled();
      });
    });

    it("subscribes to task error events on mount", async () => {
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(vi.mocked(events.taskErrorEvent.listen)).toHaveBeenCalled();
      });
    });
  });

  describe("top-up navigation", () => {
    it("navigates to Top Up screen when Top Up clicked", async () => {
      const user = userEvent.setup();
      const identity = makeIdentity({
        alias: "TopUpMe",
        balance: 1000000000,
      });
      setupMocksWithIdentities([identity]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("TopUpMe")).toBeInTheDocument();
      });

      // Select the identity
      await user.click(screen.getByText("TopUpMe"));

      // Open context menu
      const actionsButton = screen.getByLabelText("Identity actions");
      await user.click(actionsButton);

      // Click Top Up
      const topUpItem = await screen.findByRole("menuitem", {
        name: /top up/i,
      });
      await user.click(topUpItem);

      await waitFor(() => {
        expect(
          screen.getByRole("heading", { name: /Top Up Identity/i }),
        ).toBeInTheDocument();
      });
    });
  });

  describe("withdraw flow", () => {
    it("shows withdraw screen for pre-selected identity", async () => {
      const identity = makeIdentity({
        alias: "WithdrawTest",
        balance: 2000000000,
        keys: [
          makeKey({
            keyId: 5,
            purpose: "TRANSFER",
            securityLevel: "HIGH",
            hasPrivateKey: true,
          }),
        ],
      });
      setupMocksWithIdentities([identity]);

      // Pre-select the identity and set subview via store + re-render
      useIdentityStore.setState({
        identities: [identity],
        selectedIdentityId: identity.id,
      });

      renderWithProviders(<IdentitiesScreen />);

      // The identity should be selected with the detail panel visible
      await waitFor(() => {
        expect(
          screen.queryByText("Select an identity to view details"),
        ).not.toBeInTheDocument();
      });

      // The withdraw screen heading should appear after navigating
      // via context menu
      const user = userEvent.setup();
      const actionsButton = await screen.findByLabelText("Identity actions");
      await user.click(actionsButton);

      const withdrawItem = await screen.findByRole("menuitem", {
        name: /withdraw/i,
      });
      await user.click(withdrawItem);

      await waitFor(() => {
        expect(screen.getByText("Withdraw Funds")).toBeInTheDocument();
      });
    });
  });

  describe("wallet name resolution", () => {
    it("loads wallet names for identity detail panel", async () => {
      const user = userEvent.setup();
      const walletSeedHash = "cc".repeat(32);
      const identity = makeIdentity({
        alias: "WalletLinked",
        associatedWalletHashes: [walletSeedHash],
      });
      setupMocksWithIdentities([identity]);

      vi.mocked(commands.walletListAll).mockResolvedValue({
        status: "ok",
        data: {
          hdWallets: [
            {
              seedHash: walletSeedHash,
              alias: "My Wallet",
              usesPassword: false,
              isMain: true,
              totalBalance: 0,
              confirmedBalance: 0,
              unconfirmedBalance: 0,
              utxoCount: 0,
              addresses: [],
              transactions: [],
              unusedAssetLocks: [],
              platformAddresses: [],
              identityIndexes: [],
              passwordHint: null,
            },
          ],
          singleKeyWallets: [],
          selected: null,
        },
      });

      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("WalletLinked")).toBeInTheDocument();
      });

      // Select the identity to see the detail panel
      await user.click(screen.getByText("WalletLinked"));

      // The wallet name should appear (in list card and/or detail panel)
      await waitFor(() => {
        expect(screen.getAllByText("My Wallet").length).toBeGreaterThanOrEqual(1);
      });
    });
  });

  describe("refresh all", () => {
    it("dispatches refresh for all identities", async () => {
      const user = userEvent.setup();
      const id1 = makeIdentity({ id: "aa".repeat(32), alias: "ID 1" });
      const id2 = makeIdentity({ id: "bb".repeat(32), alias: "ID 2" });
      setupMocksWithIdentities([id1, id2]);
      renderWithProviders(<IdentitiesScreen />);

      await waitFor(() => {
        expect(screen.getByText("ID 1")).toBeInTheDocument();
      });

      // Click refresh all button
      const refreshAllButton = screen.getByLabelText(
        "Refresh all identities",
      );
      await user.click(refreshAllButton);

      await waitFor(() => {
        expect(vi.mocked(commands.identityRefresh)).toHaveBeenCalledTimes(2);
      });
    });
  });
});
