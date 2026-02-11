import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { SendPaymentScreen } from "./SendPaymentScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { renderWithProviders } from "@/test/router-utils";
import type { QualifiedIdentityDto, WalletDto, StoredContactDto } from "@/bindings";

// ─── Mock Tauri bindings ──────────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

// ─── Mock sonner toast ────────────────────────────────────────────

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

// ─── Mock navigation ─────────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: () => ({ location: { pathname: "/dashpay/send-payment/contact-abc" } }),
}));

// ─── Test Fixtures ───────────────────────────────────────────────

const CONTACT_ID = "bb".repeat(32);

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Alice",
    balance: 5000000000,
    keys: [],
    dpnsNames: [{ name: "alice.dash", contestingId: null, acquiredAt: 0 }],
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

function makeWallet(overrides: Partial<WalletDto> = {}): WalletDto {
  return {
    seedHash: "wallet-hash-1",
    alias: "Test Wallet",
    confirmedBalance: 500000000, // 5 DASH
    unconfirmedBalance: 0,
    totalBalance: 500000000,
    usesPassword: false,
    passwordHint: null,
    isMain: true,
    addresses: [],
    platformAddresses: [],
    unusedAssetLocks: [],
    transactions: [],
    identityIndexes: [],
    ...overrides,
  };
}

function makeContact(overrides: Partial<StoredContactDto> = {}): StoredContactDto {
  return {
    ownerIdentityId: "aa".repeat(32),
    contactIdentityId: CONTACT_ID,
    username: "bob.dash",
    displayName: "Bob",
    avatarUrl: null,
    publicMessage: null,
    contactStatus: "accepted",
    createdAt: Date.now(),
    updatedAt: Date.now(),
    lastSeen: null,
    ...overrides,
  };
}

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
    selectedWallet: null,
    loading: false,
    error: null,
    refreshing: false,
    refreshMode: "coreOnly",
  });
  useDashPayStore.getState().reset();
}

function setupWithIdentity(
  identity = makeIdentity(),
  wallet = makeWallet(),
  contact = makeContact(),
) {
  useIdentityStore.setState({ identities: [identity], loading: false });
  useWalletStore.setState({ hdWallets: [wallet], loading: false });
  useDashPayStore.setState({
    selectedIdentityId: identity.id,
    contacts: [contact],
    paymentsError: null,
  });
}

function renderScreen(contactId = CONTACT_ID) {
  return renderWithProviders(<SendPaymentScreen contactId={contactId} />);
}

// ─── Tests ───────────────────────────────────────────────────────

describe("SendPaymentScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStores();
  });

  // ── Empty / No identity state ──

  describe("when no identity is selected", () => {
    it("shows empty state message", () => {
      renderScreen();
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
      expect(
        screen.getByText("Select an identity from the sidebar to send a payment."),
      ).toBeInTheDocument();
    });

    it("does not show the form", () => {
      renderScreen();
      expect(screen.queryByText("Send Payment")).not.toBeInTheDocument();
    });
  });

  // ── Header ──

  describe("header", () => {
    it("renders Send Payment heading", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Send Payment");
    });

    it("renders back button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: "Go back" })).toBeInTheDocument();
    });

    it("navigates back on back button click", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      await user.click(screen.getByRole("button", { name: "Go back" }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/contacts" });
    });

    it("renders info button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: "Payment guidelines" })).toBeInTheDocument();
    });
  });

  // ── From Identity display ──

  describe("from identity display", () => {
    it("shows identity alias", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("Alice")).toBeInTheDocument();
    });

    it("shows DPNS name when no alias", () => {
      setupWithIdentity(makeIdentity({ alias: null }));
      renderScreen();
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    it("shows truncated ID when no alias or DPNS name", () => {
      setupWithIdentity(makeIdentity({ alias: null, dpnsNames: [] }));
      renderScreen();
      // Should show truncated hex ID
      const idText = screen.getByText(/^aaaaaaaa\.\.\./);
      expect(idText).toBeInTheDocument();
    });
  });

  // ── Wallet balance display ──

  describe("wallet balance", () => {
    it("shows wallet name and balance", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("Test Wallet")).toBeInTheDocument();
      expect(screen.getByText("5.00000000 DASH")).toBeInTheDocument();
    });

    it("shows no wallet warning when no associated wallet", () => {
      setupWithIdentity(makeIdentity({ associatedWalletHashes: [] }));
      renderScreen();
      expect(
        screen.getByText("No wallet associated with this identity"),
      ).toBeInTheDocument();
    });

    it("shows wallet locked warning when password required", () => {
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      renderScreen();
      expect(screen.getByText("Wallet is locked.")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /Unlock Wallet/i })).toBeInTheDocument();
    });
  });

  // ── To Contact display ──

  describe("to contact display", () => {
    it("shows contact display name", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("Bob")).toBeInTheDocument();
    });

    it("shows contact username alongside display name", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("@bob.dash")).toBeInTheDocument();
    });

    it("shows truncated contact ID when contact not found", () => {
      setupWithIdentity(makeIdentity(), makeWallet(), makeContact());
      useDashPayStore.setState({ contacts: [] });
      renderScreen();
      // Should show truncated ID
      const el = screen.getByText(/^bbbbbbbb\.\.\.bbbbbbbb$/);
      expect(el).toBeInTheDocument();
    });

    it("shows username when no display name", () => {
      setupWithIdentity(
        makeIdentity(),
        makeWallet(),
        makeContact({ displayName: null }),
      );
      renderScreen();
      expect(screen.getByText("bob.dash")).toBeInTheDocument();
    });
  });

  // ── Amount input ──

  describe("amount input", () => {
    it("renders amount input with label", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("Amount")).toBeInTheDocument();
      expect(
        screen.getByPlaceholderText("Enter amount in Dash"),
      ).toBeInTheDocument();
    });

    it("renders Max button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: "Set maximum amount" })).toBeInTheDocument();
    });

    it("fills max amount when Max button clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      await user.click(screen.getByRole("button", { name: "Set maximum amount" }));
      const input = screen.getByPlaceholderText("Enter amount in Dash") as HTMLInputElement;
      expect(input.value).toBe("5.00000000");
    });

    it("accepts valid DASH amount", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.5");
      expect(input).toHaveValue("1.5");
    });
  });

  // ── Memo field ──

  describe("memo field", () => {
    it("renders memo textarea", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("Memo")).toBeInTheDocument();
      expect(
        screen.getByPlaceholderText("Add a note to this payment (optional)"),
      ).toBeInTheDocument();
    });

    it("shows character counter", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("0/100")).toBeInTheDocument();
    });

    it("updates character counter as user types", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      const textarea = screen.getByPlaceholderText("Add a note to this payment (optional)");
      await user.type(textarea, "Hello");
      expect(screen.getByText("5/100")).toBeInTheDocument();
    });

    it("shows error when memo exceeds max length", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      const textarea = screen.getByPlaceholderText("Add a note to this payment (optional)");
      const longText = "a".repeat(101);
      await user.type(textarea, longText);
      expect(screen.getByText("Memo must be 100 characters or less")).toBeInTheDocument();
    });
  });

  // ── Send button ──

  describe("send button", () => {
    it("renders Send Payment button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: /Send Payment/i })).toBeInTheDocument();
    });

    it("disables send button when amount is empty", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: /Send Payment/i })).toBeDisabled();
    });

    it("enables send button when valid amount entered", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.0");
      expect(screen.getByRole("button", { name: /Send Payment/i })).toBeEnabled();
    });

    it("disables send button when no wallet associated", async () => {
      const user = userEvent.setup();
      setupWithIdentity(makeIdentity({ associatedWalletHashes: [] }));
      renderScreen();
      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.0");
      expect(screen.getByRole("button", { name: /Send Payment/i })).toBeDisabled();
    });
  });

  // ── Cancel button ──

  describe("cancel button", () => {
    it("renders Cancel button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    });

    it("navigates back on cancel click", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/contacts" });
    });
  });

  // ── Send flow ──

  describe("send flow", () => {
    it("dispatches sendPayment with correct arguments", async () => {
      const user = userEvent.setup();
      const sendPaymentMock = vi.fn().mockResolvedValue(undefined);
      setupWithIdentity();
      useDashPayStore.setState({ sendPayment: sendPaymentMock });
      renderScreen();

      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "2.5");

      const memo = screen.getByPlaceholderText("Add a note to this payment (optional)");
      await user.type(memo, "Thanks!");

      await user.click(screen.getByRole("button", { name: /Send Payment/i }));

      expect(sendPaymentMock).toHaveBeenCalledWith({
        contactId: CONTACT_ID,
        amountDash: 2.5,
        memo: "Thanks!",
      });
    });

    it("sends null memo when memo is empty", async () => {
      const user = userEvent.setup();
      const sendPaymentMock = vi.fn().mockResolvedValue(undefined);
      setupWithIdentity();
      useDashPayStore.setState({ sendPayment: sendPaymentMock });
      renderScreen();

      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.0");

      await user.click(screen.getByRole("button", { name: /Send Payment/i }));

      expect(sendPaymentMock).toHaveBeenCalledWith({
        contactId: CONTACT_ID,
        amountDash: 1,
        memo: null,
      });
    });

    it("shows sending state with spinner", async () => {
      const user = userEvent.setup();
      let resolvePayment: () => void;
      const sendPaymentMock = vi.fn(
        () => new Promise<void>((resolve) => { resolvePayment = resolve; }),
      );
      setupWithIdentity();
      useDashPayStore.setState({ sendPayment: sendPaymentMock });
      renderScreen();

      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.0");
      await user.click(screen.getByRole("button", { name: /Send Payment/i }));

      expect(screen.getByText("Sending...")).toBeInTheDocument();

      // Clean up
      resolvePayment!();
    });

    it("shows success screen after payment sent", async () => {
      const user = userEvent.setup();
      const sendPaymentMock = vi.fn().mockResolvedValue(undefined);
      setupWithIdentity();
      useDashPayStore.setState({ sendPayment: sendPaymentMock });
      renderScreen();

      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.5");
      await user.click(screen.getByRole("button", { name: /Send Payment/i }));

      await waitFor(() => {
        expect(screen.getByText("Payment Sent!")).toBeInTheDocument();
      });
      expect(screen.getByText(/1\.50000000 DASH sent to Bob/)).toBeInTheDocument();
    });

    it("shows error when sendPayment throws", async () => {
      const user = userEvent.setup();
      const sendPaymentMock = vi.fn().mockRejectedValue(new Error("Network error"));
      setupWithIdentity();
      useDashPayStore.setState({ sendPayment: sendPaymentMock });
      renderScreen();

      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "1.0");
      await user.click(screen.getByRole("button", { name: /Send Payment/i }));

      await waitFor(() => {
        expect(screen.getByText("Network error")).toBeInTheDocument();
      });
    });
  });

  // ── Success screen ──

  describe("success screen", () => {
    async function navigateToSuccess() {
      const user = userEvent.setup();
      const sendPaymentMock = vi.fn().mockResolvedValue(undefined);
      setupWithIdentity();
      useDashPayStore.setState({ sendPayment: sendPaymentMock });
      renderScreen();

      const input = screen.getByPlaceholderText("Enter amount in Dash");
      await user.type(input, "3.0");
      await user.click(screen.getByRole("button", { name: /Send Payment/i }));

      await waitFor(() => {
        expect(screen.getByText("Payment Sent!")).toBeInTheDocument();
      });

      return user;
    }

    it("shows Back to DashPay button", async () => {
      await navigateToSuccess();
      expect(screen.getByRole("button", { name: "Back to DashPay" })).toBeInTheDocument();
    });

    it("shows Send Another Payment button", async () => {
      await navigateToSuccess();
      expect(
        screen.getByRole("button", { name: "Send Another Payment" }),
      ).toBeInTheDocument();
    });

    it("navigates back on Back to DashPay click", async () => {
      const user = await navigateToSuccess();
      await user.click(screen.getByRole("button", { name: "Back to DashPay" }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/contacts" });
    });

    it("resets form on Send Another Payment click", async () => {
      const user = await navigateToSuccess();
      await user.click(
        screen.getByRole("button", { name: "Send Another Payment" }),
      );
      // Should be back to form state
      expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Send Payment");
      expect(screen.getByPlaceholderText("Enter amount in Dash")).toHaveValue("");
    });
  });

  // ── Info popup ──

  describe("info popup", () => {
    it("opens payment guidelines dialog", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      await user.click(screen.getByRole("button", { name: "Payment guidelines" }));
      expect(screen.getByText("Payment Guidelines")).toBeInTheDocument();
    });

    it("displays all payment guidelines", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      await user.click(screen.getByRole("button", { name: "Payment guidelines" }));
      expect(
        screen.getByText("Payments are sent on the Dash Platform using DashPay."),
      ).toBeInTheDocument();
      expect(
        screen.getByText("Payments cannot be reversed once sent."),
      ).toBeInTheDocument();
    });
  });

  // ── Wallet unlock dialog ──

  describe("wallet unlock dialog", () => {
    it("opens wallet unlock dialog on Unlock Wallet click", async () => {
      const user = userEvent.setup();
      setupWithIdentity(makeIdentity(), makeWallet({ usesPassword: true }));
      renderScreen();
      await user.click(screen.getByRole("button", { name: /Unlock Wallet/i }));
      expect(screen.getByText("Unlock Wallet", { selector: "h2" })).toBeInTheDocument();
    });
  });

  // ── Accessibility ──

  describe("accessibility", () => {
    it("has accessible labels for all form fields", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("From")).toBeInTheDocument();
      expect(screen.getByText("Wallet Balance")).toBeInTheDocument();
      expect(screen.getByText("To")).toBeInTheDocument();
      expect(screen.getByText("Amount")).toBeInTheDocument();
      expect(screen.getByText("Memo")).toBeInTheDocument();
    });

    it("has accessible name for back button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("button", { name: "Go back" })).toBeInTheDocument();
    });

    it("has accessible error messages with role alert", async () => {
      const user = userEvent.setup();
      setupWithIdentity();
      renderScreen();
      const textarea = screen.getByPlaceholderText("Add a note to this payment (optional)");
      await user.type(textarea, "a".repeat(101));
      const alert = screen.getByRole("alert");
      expect(alert).toHaveTextContent("Memo must be 100 characters or less");
    });
  });
});
