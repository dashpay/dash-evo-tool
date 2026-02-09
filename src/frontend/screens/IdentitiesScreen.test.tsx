import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { IdentitiesScreen } from "./IdentitiesScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { renderWithProviders } from "@/test/router-utils";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Mock Tauri bindings ──────────────────────────────────────────

const { mockCommands, mockEvents } = vi.hoisted(() => {
  const mockCommands: Record<string, ReturnType<typeof vi.fn>> = {
    identityListLocal: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    identityLoadOrder: vi.fn().mockResolvedValue({ status: "ok", data: [] }),
    identitySetAlias: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    identitySaveOrder: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    identityDelete: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    identityRefresh: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t1" } }),
    identityGetById: vi.fn().mockResolvedValue({ status: "ok", data: null }),
    identityWithdraw: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t2" } }),
    identityTransfer: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t3" } }),
    identityTransferToAddresses: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t4" } }),
    identityAddKey: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t5" } }),
    identityDisableKeys: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t6" } }),
    identityReplaceKey: vi.fn().mockResolvedValue({ status: "ok", data: { taskId: "t7" } }),
    walletListAll: vi.fn().mockResolvedValue({ status: "ok", data: { hdWallets: [], singleKeyWallets: [], selected: null } }),
  };
  const mockEvents = {
    taskResultEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    taskErrorEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
    walletUpdatedEvent: { listen: vi.fn().mockResolvedValue(() => {}) },
  };
  return { mockCommands, mockEvents };
});

vi.mock("@/bindings", () => ({
  commands: new Proxy({} as Record<string, unknown>, {
    get: (_target, prop: string) => {
      if (prop in mockCommands) {
        return mockCommands[prop];
      }
      return vi.fn().mockResolvedValue({ status: "error", error: "not mocked" });
    },
  }),
  events: mockEvents,
}));

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
  mockCommands.identityListLocal.mockResolvedValue({
    status: "ok",
    data: identities,
  });
  mockCommands.identityGetById.mockImplementation(async (id: string) => {
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
  mockCommands.identityListLocal.mockResolvedValue({
    status: "ok",
    data: [],
  });
  mockCommands.identityLoadOrder.mockResolvedValue({
    status: "ok",
    data: [],
  });
  mockCommands.walletListAll.mockResolvedValue({
    status: "ok",
    data: { hdWallets: [], singleKeyWallets: [], selected: null },
  });
});

// ─── Tests ────────────────────────────────────────────────────────

describe("IdentitiesScreen", () => {
  describe("loading state", () => {
    it("shows a loading spinner while identities are loading", () => {
      mockCommands.identityListLocal.mockReturnValue(new Promise(() => {}));
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
        expect(mockCommands.identityRefresh).toHaveBeenCalledWith({
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
        expect(mockCommands.identitySetAlias).toHaveBeenCalledWith({
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
        expect(mockCommands.identityDelete).toHaveBeenCalledWith({
          identityId: identity.id,
        });
      });
    });
  });

  describe("reordering", () => {
    it("moves an identity up when up arrow is clicked", async () => {
      const user = userEvent.setup();
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

      // Find the "Move up" button on the second identity card
      const moveUpButtons = screen.getAllByLabelText("Move up");
      // The second one corresponds to the second identity
      await user.click(moveUpButtons[1]);

      await waitFor(() => {
        expect(mockCommands.identitySaveOrder).toHaveBeenCalled();
      });
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

  describe("error handling", () => {
    it("shows toast when store has an error", async () => {
      setupMocksWithIdentities([]);
      renderWithProviders(<IdentitiesScreen />);

      // Simulate error
      useIdentityStore.setState({ error: "Something went wrong" });

      await waitFor(() => {
        expect(mockToast.error).toHaveBeenCalledWith("Something went wrong");
      });
    });
  });

  describe("event subscription", () => {
    it("subscribes to task result events on mount", async () => {
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(mockEvents.taskResultEvent.listen).toHaveBeenCalled();
      });
    });

    it("subscribes to task error events on mount", async () => {
      renderWithProviders(<IdentitiesScreen />);
      await waitFor(() => {
        expect(mockEvents.taskErrorEvent.listen).toHaveBeenCalled();
      });
    });
  });

  describe("placeholder actions", () => {
    it("shows toast for Top Up (not yet implemented)", async () => {
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
        expect(mockToast.info).toHaveBeenCalledWith(
          expect.stringContaining("Top Up"),
        );
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

      mockCommands.walletListAll.mockResolvedValue({
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

      // The detail panel should show the wallet name
      await waitFor(() => {
        expect(screen.getByText("My Wallet")).toBeInTheDocument();
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
        expect(mockCommands.identityRefresh).toHaveBeenCalledTimes(2);
      });
    });
  });
});
