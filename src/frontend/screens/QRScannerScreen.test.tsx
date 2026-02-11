import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QRScannerScreen } from "./QRScannerScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { renderWithProviders } from "@/test/router-utils";
import { commands } from "@/bindings";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

// ─── Mock Tauri bindings ──────────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

// ─── Mock sonner ─────────────────────────────────────────────────

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

// ─── Mock navigation ─────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: () => ({
    location: { pathname: "/dashpay/qr-scanner" },
  }),
}));

// ─── Fixtures ────────────────────────────────────────────────────

function makeKey(overrides: Partial<IdentityKeyDto> = {}): IdentityKeyDto {
  return {
    keyId: 0,
    keyType: "ECDSA_SECP256K1",
    purpose: "AUTHENTICATION",
    securityLevel: "HIGH",
    data: "deadbeef".repeat(8),
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: true,
    contractBounds: null,
    ...overrides,
  };
}

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "bb".repeat(32),
    identityType: "user",
    alias: "Bob",
    balance: 3000000000,
    keys: [
      makeKey({ keyId: 0, securityLevel: "CRITICAL" }),
      makeKey({ keyId: 1, securityLevel: "HIGH" }),
      makeKey({
        keyId: 2,
        purpose: "VOTING",
        securityLevel: "MEDIUM",
        keyType: "BLS12_381",
      }),
    ],
    dpnsNames: [],
    associatedWalletHashes: ["wallet-hash-2"],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

const VALID_QR_DATA = "dash:?di=5M2s8gJZ3FPCJLq5k2JxqZEt&dapk=testdata123";
const PARSED_RESULT = {
  identityId: "5M2s8gJZ3FPCJLq5k2JxqZEt",
  proofKeyHex: "aabbccdd".repeat(8),
  accountReference: 0,
  expiresAt: Math.floor(Date.now() / 1000) + 86400,
};

// ─── Helpers ─────────────────────────────────────────────────────

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
    selectedWalletHash: null,
    loading: false,
    error: null,
    refreshing: false,
    refreshMode: "coreOnly",
  });
  useDashPayStore.getState().reset();
}

function makeWallet(overrides: Record<string, unknown> = {}) {
  return {
    seedHash: "wallet-hash-2",
    usesPassword: false,
    alias: "Test Wallet",
    isMain: true,
    confirmedBalance: 500_000_000,
    unconfirmedBalance: 0,
    totalBalance: 500_000_000,
    addresses: [],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [0],
    passwordHint: null,
    ...overrides,
  };
}

function setupWithIdentity(identity = makeIdentity(), wallet = makeWallet()) {
  useIdentityStore.setState({ identities: [identity], loading: false });
  useWalletStore.setState({ hdWallets: [wallet] });
  useDashPayStore.setState({ selectedIdentityId: identity.id });
}

function renderScreen() {
  return renderWithProviders(<QRScannerScreen />);
}

// ─── Tests ───────────────────────────────────────────────────────

describe("QRScannerScreen", () => {
  const user = userEvent.setup();

  beforeEach(() => {
    vi.clearAllMocks();
    resetStores();
  });

  // ── Empty state ──

  describe("when no identities are loaded", () => {
    it("shows empty state", () => {
      renderScreen();
      expect(screen.getByText("No Identities Loaded")).toBeInTheDocument();
    });

    it("shows load identity button", () => {
      renderScreen();
      expect(screen.getByText("Load Identity")).toBeInTheDocument();
    });

    it("navigates to identities on action click", async () => {
      renderScreen();
      await user.click(screen.getByText("Load Identity"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
    });
  });

  // ── Basic rendering ──

  describe("when identity is loaded", () => {
    beforeEach(() => setupWithIdentity());

    it("renders heading", () => {
      renderScreen();
      expect(screen.getByText("Scan Contact QR Code")).toBeInTheDocument();
    });

    it("renders back button", () => {
      renderScreen();
      expect(screen.getByText("Back")).toBeInTheDocument();
    });

    it("back button navigates to contacts", async () => {
      renderScreen();
      await user.click(screen.getByText("Back"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/contacts" });
    });

    it("renders step 1 identity section", () => {
      renderScreen();
      expect(
        screen.getByText("1. Select Your Identity"),
      ).toBeInTheDocument();
    });

    it("renders step 2 QR data section", () => {
      renderScreen();
      expect(screen.getByText("2. Enter QR Code Data")).toBeInTheDocument();
    });

    it("renders QR data textarea", () => {
      renderScreen();
      expect(screen.getByLabelText("QR code data")).toBeInTheDocument();
    });

    it("renders parse button", () => {
      renderScreen();
      expect(screen.getByText("Parse QR Code")).toBeInTheDocument();
    });

    it("parse button is disabled when textarea empty", () => {
      renderScreen();
      expect(screen.getByText("Parse QR Code")).toBeDisabled();
    });

    it("renders about section", () => {
      renderScreen();
      expect(
        screen.getByText("About QR Code Scanning"),
      ).toBeInTheDocument();
    });

    it("shows scanning info bullets", () => {
      renderScreen();
      expect(
        screen.getByText(
          /QR codes enable instant mutual contact establishment/,
        ),
      ).toBeInTheDocument();
    });
  });

  // ── QR parsing ──

  describe("QR parsing", () => {
    beforeEach(() => setupWithIdentity());

    it("enables parse button when text is entered", async () => {
      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), "test");
      expect(screen.getByText("Parse QR Code")).toBeEnabled();
    });

    it("calls parse IPC on button click", async () => {
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(commands.dashpayParseAutoAcceptProof).toHaveBeenCalledWith({
          qrData: VALID_QR_DATA,
        });
      });
    });

    it("shows parsed details on success", async () => {
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText("3. QR Code Details"),
        ).toBeInTheDocument();
      });
    });

    it("shows contact identity ID", async () => {
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText(PARSED_RESULT.identityId),
        ).toBeInTheDocument();
      });
    });

    it("shows Add Contact button after parsing", async () => {
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });
    });

    it("shows error on parse failure", async () => {
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "error",
        error: "Invalid QR code format",
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), "invalid");
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText("Invalid QR code format"),
        ).toBeInTheDocument();
      });
    });

    it("clears on clear button click", async () => {
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText("3. QR Code Details"),
        ).toBeInTheDocument();
      });

      await user.click(screen.getByText("Clear"));
      expect(
        screen.queryByText("3. QR Code Details"),
      ).not.toBeInTheDocument();
    });
  });

  // ── Add contact ──

  describe("adding contact", () => {
    beforeEach(() => {
      setupWithIdentity();
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });
    });

    it("sends contact request on Add Contact click", async () => {
      vi.mocked(
        commands.dashpaySendContactRequestWithProof,
      ).mockResolvedValue({
        status: "ok",
        data: { taskId: "123" },
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Add Contact"));

      await waitFor(() => {
        expect(
          commands.dashpaySendContactRequestWithProof,
        ).toHaveBeenCalledWith(
          expect.objectContaining({
            identityId: "bb".repeat(32),
            toIdentityId: PARSED_RESULT.identityId,
            proofKeyHex: PARSED_RESULT.proofKeyHex,
            proofAccountReference: PARSED_RESULT.accountReference,
            proofExpiresAt: PARSED_RESULT.expiresAt,
          }),
        );
      });
    });

    it("shows success screen on success", async () => {
      vi.mocked(
        commands.dashpaySendContactRequestWithProof,
      ).mockResolvedValue({
        status: "ok",
        data: { taskId: "123" },
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Add Contact"));

      await waitFor(() => {
        expect(screen.getByText("Contact Added!")).toBeInTheDocument();
      });
    });

    it("shows scan another and back buttons on success", async () => {
      vi.mocked(
        commands.dashpaySendContactRequestWithProof,
      ).mockResolvedValue({
        status: "ok",
        data: { taskId: "123" },
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Add Contact"));

      await waitFor(() => {
        expect(screen.getByText("Scan Another")).toBeInTheDocument();
        expect(screen.getByText("Back to Contacts")).toBeInTheDocument();
      });
    });

    it("shows error on send failure", async () => {
      vi.mocked(
        commands.dashpaySendContactRequestWithProof,
      ).mockImplementation(() =>
        Promise.resolve({
          status: "error" as const,
          error: "Network error",
        }),
      );

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Add Contact"));

      await waitFor(() => {
        expect(screen.getByText("Network error")).toBeInTheDocument();
      });
    });

    it("shows retry button on error", async () => {
      vi.mocked(
        commands.dashpaySendContactRequestWithProof,
      ).mockImplementation(() =>
        Promise.resolve({
          status: "error" as const,
          error: "Network error",
        }),
      );

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Add Contact"));

      await waitFor(() => {
        expect(screen.getByText("Retry")).toBeInTheDocument();
      });
    });
  });

  // ── No signing key ──

  describe("identity without signing key", () => {
    it("shows warning when no ECDSA AUTH key", () => {
      const identity = makeIdentity({
        keys: [
          makeKey({
            keyId: 0,
            purpose: "VOTING",
            keyType: "BLS12_381",
            securityLevel: "HIGH",
          }),
        ],
      });
      setupWithIdentity(identity);
      renderScreen();
      expect(
        screen.getByText(/No suitable ECDSA AUTHENTICATION key found/),
      ).toBeInTheDocument();
    });

    it("disables Add Contact when no signing key", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({
            keyId: 0,
            purpose: "VOTING",
            keyType: "BLS12_381",
            securityLevel: "HIGH",
          }),
        ],
      });
      setupWithIdentity(identity);

      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeDisabled();
      });
    });
  });

  // ── Expired QR code ──

  describe("expired QR code", () => {
    it("shows expired warning for expired QR", async () => {
      const expiredResult = {
        ...PARSED_RESULT,
        expiresAt: Math.floor(Date.now() / 1000) - 3600,
      };
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: expiredResult,
      });
      setupWithIdentity();

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("EXPIRED")).toBeInTheDocument();
      });
    });

    it("shows cannot use expired QR message", async () => {
      const expiredResult = {
        ...PARSED_RESULT,
        expiresAt: Math.floor(Date.now() / 1000) - 3600,
      };
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: expiredResult,
      });
      setupWithIdentity();

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText(
            /This QR code has expired and cannot be used/,
          ),
        ).toBeInTheDocument();
      });
    });
  });

  // ── Wallet locked ──

  describe("wallet locked state", () => {
    it("shows wallet locked warning instead of Add Contact when wallet uses password", async () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText(/Wallet is locked\. Please unlock to add contact/),
        ).toBeInTheDocument();
      });
      expect(screen.queryByText("Add Contact")).not.toBeInTheDocument();
    });

    it("shows unlock wallet button when wallet locked", async () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Unlock Wallet")).toBeInTheDocument();
      });
    });

    it("shows Add Contact button when wallet is unlocked", async () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: false }));
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });

      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Add Contact")).toBeInTheDocument();
      });
      expect(screen.queryByText(/Wallet is locked/)).not.toBeInTheDocument();
    });
  });

  // ── Info text ──

  describe("info messages", () => {
    beforeEach(() => {
      setupWithIdentity();
      vi.mocked(commands.dashpayParseAutoAcceptProof).mockResolvedValue({
        status: "ok",
        data: PARSED_RESULT,
      });
    });

    it("shows auto-accept info after parsing", async () => {
      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText(
            /This will send a contact request that will be automatically accepted/,
          ),
        ).toBeInTheDocument();
      });
    });

    it("shows instant contact info after parsing", async () => {
      renderScreen();
      await user.type(screen.getByLabelText("QR code data"), VALID_QR_DATA);
      await user.click(screen.getByText("Parse QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText(
            /Both you and the contact will become mutual contacts instantly/,
          ),
        ).toBeInTheDocument();
      });
    });
  });
});
