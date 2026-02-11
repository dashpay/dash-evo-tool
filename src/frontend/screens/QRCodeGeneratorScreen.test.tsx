import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { QRCodeGeneratorScreen } from "./QRCodeGeneratorScreen";
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

// ─── Mock qrcode.react ──────────────────────────────────────────

vi.mock("qrcode.react", () => ({
  QRCodeSVG: (props: { value: string }) => (
    <svg data-testid="qr-svg" data-value={props.value} />
  ),
}));

// ─── Mock navigation ─────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: () => ({
    location: { pathname: "/dashpay/qr-generator" },
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
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Alice",
    balance: 5000000000,
    keys: [
      makeKey({ keyId: 0, securityLevel: "CRITICAL" }),
      makeKey({ keyId: 1, securityLevel: "HIGH" }),
    ],
    dpnsNames: [],
    associatedWalletHashes: ["wallet-hash-1"],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...overrides,
  };
}

const QR_SUCCESS = {
  status: "ok" as const,
  data: {
    qrString: "dash:?di=abc&dapk=xyz",
    identityId: "abc",
    accountReference: 0,
    expiresAt: Math.floor(Date.now() / 1000) + 86400,
  },
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
    seedHash: "wallet-hash-1",
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
  return renderWithProviders(<QRCodeGeneratorScreen />);
}

// ─── Tests ───────────────────────────────────────────────────────

describe("QRCodeGeneratorScreen", () => {
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
      expect(
        screen.getByText("Generate Contact QR Code"),
      ).toBeInTheDocument();
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

    it("renders configuration section", () => {
      renderScreen();
      expect(screen.getByText("Configuration")).toBeInTheDocument();
    });

    it("renders identity selector", () => {
      renderScreen();
      expect(screen.getByText("Identity:")).toBeInTheDocument();
    });

    it("renders generate button", () => {
      renderScreen();
      expect(screen.getByText("Generate QR Code")).toBeInTheDocument();
    });

    it("renders info button", () => {
      renderScreen();
      expect(
        screen.getByLabelText("QR code information"),
      ).toBeInTheDocument();
    });

    it("renders advanced options checkbox", () => {
      renderScreen();
      expect(screen.getByText("Advanced Options")).toBeInTheDocument();
    });
  });

  // ── Advanced options ──

  describe("advanced options", () => {
    beforeEach(() => setupWithIdentity());

    it("hides advanced fields by default", () => {
      renderScreen();
      expect(screen.queryByText("Account Index:")).not.toBeInTheDocument();
      expect(screen.queryByText("Validity (hours):")).not.toBeInTheDocument();
    });

    it("shows advanced fields when checkbox is checked", async () => {
      renderScreen();
      await user.click(screen.getByLabelText("Show advanced options"));
      expect(screen.getByText("Account Index:")).toBeInTheDocument();
      expect(screen.getByText("Validity (hours):")).toBeInTheDocument();
    });

    it("shows validity hint text", async () => {
      renderScreen();
      await user.click(screen.getByLabelText("Show advanced options"));
      expect(
        screen.getByText(/How long the QR code remains valid/),
      ).toBeInTheDocument();
    });
  });

  // ── QR generation ──

  describe("QR generation", () => {
    beforeEach(() => setupWithIdentity());

    it("calls generate IPC on button click", async () => {
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue(
        QR_SUCCESS,
      );

      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));

      await waitFor(() => {
        expect(commands.dashpayGenerateAutoAcceptProof).toHaveBeenCalledWith({
          identityId: "aa".repeat(32),
          accountIndex: 0,
          validityHours: 24,
        });
      });
    });

    it("shows QR code on success", async () => {
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue(
        QR_SUCCESS,
      );

      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));

      await waitFor(() => {
        expect(screen.getByTestId("qr-svg")).toBeInTheDocument();
      });
    });

    it("shows success message", async () => {
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue(
        QR_SUCCESS,
      );

      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));

      await waitFor(() => {
        expect(
          screen.getByText("QR code generated successfully."),
        ).toBeInTheDocument();
      });
    });

    it("shows error message on failure", async () => {
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue({
        status: "error",
        error: "No suitable key found",
      });

      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));

      await waitFor(() => {
        expect(screen.getByText("No suitable key found")).toBeInTheDocument();
      });
    });

    it("shows clear button after generation", async () => {
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue(
        QR_SUCCESS,
      );

      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));

      await waitFor(() => {
        expect(screen.getByText("Clear")).toBeInTheDocument();
      });
    });

    it("clears QR code on clear click", async () => {
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue(
        QR_SUCCESS,
      );

      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));

      await waitFor(() => {
        expect(screen.getByTestId("qr-svg")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Clear"));
      expect(screen.queryByTestId("qr-svg")).not.toBeInTheDocument();
    });
  });

  // ── QR code display ──

  describe("QR code display", () => {
    beforeEach(() => {
      setupWithIdentity();
      vi.mocked(commands.dashpayGenerateAutoAcceptProof).mockResolvedValue(
        QR_SUCCESS,
      );
    });

    it("renders Generated QR Code section", async () => {
      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));
      await waitFor(() => {
        expect(screen.getByText("Generated QR Code")).toBeInTheDocument();
      });
    });

    it("shows copy button", async () => {
      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));
      await waitFor(() => {
        expect(
          screen.getByText("Copy Data to Clipboard"),
        ).toBeInTheDocument();
      });
    });

    it("shows warning text", async () => {
      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));
      await waitFor(() => {
        expect(
          screen.getByText(
            /Anyone with this QR code can automatically become your contact/,
          ),
        ).toBeInTheDocument();
      });
    });

    it("shows expandable data section", async () => {
      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));
      await waitFor(() => {
        expect(screen.getByText("QR Code Data (text)")).toBeInTheDocument();
      });
    });

    it("expands data section on click", async () => {
      renderScreen();
      await user.click(screen.getByText("Generate QR Code"));
      await waitFor(() => {
        expect(screen.getByText("QR Code Data (text)")).toBeInTheDocument();
      });
      await user.click(screen.getByText("QR Code Data (text)"));
      expect(screen.getByText("dash:?di=abc&dapk=xyz")).toBeInTheDocument();
    });
  });

  // ── Wallet locked ──

  describe("wallet locked state", () => {
    it("shows wallet locked warning when wallet uses password", () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      renderScreen();
      expect(
        screen.getByText(/Wallet is locked\. Please unlock to generate QR code/),
      ).toBeInTheDocument();
    });

    it("shows unlock wallet button when wallet locked", () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      renderScreen();
      expect(screen.getByText("Unlock Wallet")).toBeInTheDocument();
    });

    it("disables generate button when wallet locked", () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      renderScreen();
      expect(screen.getByText("Generate QR Code")).toBeDisabled();
    });

    it("does not show wallet locked warning when wallet is unlocked", () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: false }));
      renderScreen();
      expect(
        screen.queryByText(/Wallet is locked/),
      ).not.toBeInTheDocument();
    });

    it("enables generate button when wallet is unlocked", () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: false }));
      renderScreen();
      expect(screen.getByText("Generate QR Code")).toBeEnabled();
    });
  });

  // ── Info dialog ──

  describe("info dialog", () => {
    beforeEach(() => setupWithIdentity());

    it("opens info dialog on icon click", async () => {
      renderScreen();
      await user.click(screen.getByLabelText("QR code information"));
      await waitFor(() => {
        expect(
          screen.getByText("About Contact QR Codes"),
        ).toBeInTheDocument();
      });
    });

    it("shows QR code info points", async () => {
      renderScreen();
      await user.click(screen.getByLabelText("QR code information"));
      await waitFor(() => {
        expect(
          screen.getByText(
            /QR codes allow instant mutual contact establishment/,
          ),
        ).toBeInTheDocument();
      });
    });
  });
});
