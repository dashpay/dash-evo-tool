import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { PaymentHistoryScreen } from "./PaymentHistoryScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import { renderWithProviders } from "@/test/router-utils";
import type { StoredPaymentDto, StoredContactDto } from "@/bindings";

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

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useRouterState: () => ({ location: { pathname: "/dashpay/payments" } }),
}));

// ─── Test Fixtures ───────────────────────────────────────────────

const IDENTITY_A = "aa".repeat(32);
const IDENTITY_B = "bb".repeat(32);
const IDENTITY_C = "cc".repeat(32);

function makePayment(
  overrides: Partial<StoredPaymentDto> = {},
): StoredPaymentDto {
  return {
    id: 1,
    txId: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
    fromIdentityId: IDENTITY_B,
    toIdentityId: IDENTITY_A,
    amount: 150000000, // 1.5 DASH
    memo: null,
    paymentType: "standard",
    status: "confirmed",
    createdAt: Math.floor(Date.now() / 1000) - 3600, // 1 hour ago
    confirmedAt: Math.floor(Date.now() / 1000) - 3500,
    ...overrides,
  };
}

function makeContact(
  overrides: Partial<StoredContactDto> = {},
): StoredContactDto {
  return {
    ownerIdentityId: IDENTITY_A,
    contactIdentityId: IDENTITY_B,
    username: "bob",
    displayName: "Bob Smith",
    avatarUrl: null,
    publicMessage: null,
    contactStatus: "accepted",
    createdAt: Math.floor(Date.now() / 1000),
    updatedAt: Math.floor(Date.now() / 1000),
    lastSeen: null,
    ...overrides,
  };
}

// ─── Helpers ─────────────────────────────────────────────────────

function resetStore() {
  useDashPayStore.getState().reset();
}

function setupWithPayments(
  payments: StoredPaymentDto[] = [],
  contacts: StoredContactDto[] = [],
) {
  useDashPayStore.setState({
    selectedIdentityId: IDENTITY_A,
    payments,
    paymentsLoading: false,
    paymentsRefreshing: false,
    paymentsError: null,
    contacts,
    // Override loadPayments to prevent useEffect from triggering loading state
    loadPayments: vi.fn(),
  });
}

// ─── Tests ───────────────────────────────────────────────────────

describe("PaymentHistoryScreen", () => {
  beforeEach(() => {
    resetStore();
    vi.clearAllMocks();
  });

  // ── No identity selected ──

  describe("when no identity is selected", () => {
    it("shows empty state asking to select an identity", () => {
      useDashPayStore.setState({ selectedIdentityId: null });
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
      expect(
        screen.getByText(
          "Please select an identity from the sidebar to view payment history.",
        ),
      ).toBeInTheDocument();
    });
  });

  // ── Loading state ──

  describe("when loading", () => {
    it("shows loading spinner when payments are loading and empty", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        payments: [],
        paymentsLoading: true,
      });
      renderWithProviders(<PaymentHistoryScreen />);

      expect(
        screen.getByText("Loading payment history…"),
      ).toBeInTheDocument();
    });

    it("shows payment list instead of spinner when loading with existing data", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        payments: [makePayment()],
        paymentsLoading: true,
      });
      renderWithProviders(<PaymentHistoryScreen />);

      expect(
        screen.queryByText("Loading payment history…"),
      ).not.toBeInTheDocument();
      expect(screen.getByRole("list")).toBeInTheDocument();
    });
  });

  // ── Empty state ──

  describe("when no payments exist", () => {
    it("shows empty state message", () => {
      setupWithPayments([]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("No Payment History")).toBeInTheDocument();
      expect(
        screen.getByText("No payments have been made with this identity."),
      ).toBeInTheDocument();
    });
  });

  // ── Header ──

  describe("header", () => {
    it("renders title and refresh button", () => {
      setupWithPayments([]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("Payment History")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /refresh/i }),
      ).toBeInTheDocument();
    });

    it("shows refreshing state on refresh button", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        payments: [],
        paymentsRefreshing: true,
      });
      renderWithProviders(<PaymentHistoryScreen />);

      const btn = screen.getByRole("button", {
        name: /refresh payment history/i,
      });
      expect(btn).toBeDisabled();
      expect(screen.getByText("Refreshing…")).toBeInTheDocument();
    });

    it("calls refreshPayments when refresh button clicked", async () => {
      const user = userEvent.setup();
      const refreshSpy = vi.fn();
      setupWithPayments([]);
      useDashPayStore.setState({ refreshPayments: refreshSpy });

      renderWithProviders(<PaymentHistoryScreen />);

      await user.click(
        screen.getByRole("button", { name: /refresh payment history/i }),
      );
      expect(refreshSpy).toHaveBeenCalled();
    });
  });

  // ── Error display ──

  describe("error display", () => {
    it("shows error banner when paymentsError is set", () => {
      setupWithPayments([]);
      useDashPayStore.setState({
        paymentsError: "Failed to load payments",
      });
      renderWithProviders(<PaymentHistoryScreen />);

      expect(
        screen.getByText("Failed to load payments"),
      ).toBeInTheDocument();
    });

    it("does not show error banner when no error", () => {
      setupWithPayments([makePayment()]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(
        screen.queryByText("Failed to load payments"),
      ).not.toBeInTheDocument();
    });
  });

  // ── Payment card rendering ──

  describe("payment card rendering", () => {
    it("renders incoming payment with green amount and down arrow", () => {
      const payment = makePayment({
        fromIdentityId: IDENTITY_B,
        toIdentityId: IDENTITY_A,
        amount: 150000000, // 1.5 DASH
      });
      setupWithPayments([payment], [makeContact()]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("Bob Smith")).toBeInTheDocument();
      expect(screen.getByText("+1.50000000 Dash")).toBeInTheDocument();
      expect(screen.getByLabelText("Incoming")).toBeInTheDocument();
    });

    it("renders outgoing payment with red amount and up arrow", () => {
      const payment = makePayment({
        fromIdentityId: IDENTITY_A,
        toIdentityId: IDENTITY_B,
        amount: 250000000, // 2.5 DASH
      });
      setupWithPayments([payment], [makeContact()]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("Bob Smith")).toBeInTheDocument();
      expect(screen.getByText("-2.50000000 Dash")).toBeInTheDocument();
      expect(screen.getByLabelText("Outgoing")).toBeInTheDocument();
    });

    it("displays memo in italics when present", () => {
      const payment = makePayment({ memo: "Thanks for the coffee" });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      const memoElement = screen.getByText(/Thanks for the coffee/);
      expect(memoElement).toBeInTheDocument();
      expect(memoElement).toHaveClass("italic");
    });

    it("does not show memo section when memo is null", () => {
      const payment = makePayment({ memo: null });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      // No italic text with quotes
      const items = screen.getAllByRole("listitem");
      const firstItem = items[0];
      expect(
        within(firstItem).queryByText(/\u201c/), // left double quotation mark
      ).not.toBeInTheDocument();
    });

    it("shows truncated transaction ID with copy button", () => {
      const payment = makePayment({
        txId: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/abcdef12…34567890/)).toBeInTheDocument();
    });

    it("shows timestamp for confirmed payment", () => {
      const payment = makePayment({
        confirmedAt: Math.floor(Date.now() / 1000) - 7200, // 2 hours ago
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/2h ago/)).toBeInTheDocument();
    });

    it("shows 'Pending' when no confirmedAt or createdAt", () => {
      const payment = makePayment({
        confirmedAt: null,
        createdAt: 0,
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/Pending/)).toBeInTheDocument();
    });

    it("shows status badge for non-confirmed payments", () => {
      const payment = makePayment({ status: "pending" });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("pending")).toBeInTheDocument();
    });

    it("shows green confirmed badge for confirmed payments", () => {
      const payment = makePayment({ status: "confirmed" });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      const items = screen.getAllByRole("listitem");
      const badge = within(items[0]).getByText("confirmed");
      expect(badge).toBeInTheDocument();
      expect(badge.className).toContain("text-green-600");
    });
  });

  // ── Contact name resolution ──

  describe("contact name resolution", () => {
    it("uses displayName from contacts", () => {
      const payment = makePayment({
        fromIdentityId: IDENTITY_B,
        toIdentityId: IDENTITY_A,
      });
      const contact = makeContact({
        contactIdentityId: IDENTITY_B,
        displayName: "Bob Smith",
        username: "bob",
      });
      setupWithPayments([payment], [contact]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("Bob Smith")).toBeInTheDocument();
    });

    it("uses username when displayName is empty", () => {
      const payment = makePayment({
        fromIdentityId: IDENTITY_B,
        toIdentityId: IDENTITY_A,
      });
      const contact = makeContact({
        contactIdentityId: IDENTITY_B,
        displayName: "",
        username: "bobdash",
      });
      setupWithPayments([payment], [contact]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText("bobdash")).toBeInTheDocument();
    });

    it("shows truncated ID when contact not found", () => {
      const payment = makePayment({
        fromIdentityId: IDENTITY_C,
        toIdentityId: IDENTITY_A,
      });
      setupWithPayments([payment], []); // no contacts
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/Unknown \(cccccccc\)/)).toBeInTheDocument();
    });
  });

  // ── Multiple payments ──

  describe("multiple payments", () => {
    it("renders all payment cards", () => {
      const payments = [
        makePayment({ id: 1, amount: 100000000 }),
        makePayment({
          id: 2,
          amount: 200000000,
          fromIdentityId: IDENTITY_A,
          toIdentityId: IDENTITY_B,
        }),
        makePayment({ id: 3, amount: 300000000, memo: "Dinner" }),
      ];
      setupWithPayments(payments);
      renderWithProviders(<PaymentHistoryScreen />);

      const items = screen.getAllByRole("listitem");
      expect(items).toHaveLength(3);
    });
  });

  // ── Copy transaction ID ──

  describe("copy transaction ID", () => {
    it("copies full txId to clipboard on click", async () => {
      const user = userEvent.setup();
      const writeTextSpy = vi.fn().mockResolvedValue(undefined);
      Object.defineProperty(navigator, "clipboard", {
        value: { writeText: writeTextSpy },
        writable: true,
        configurable: true,
      });

      const txId =
        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
      const payment = makePayment({ txId });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      const copyButton = screen.getByLabelText(/Copy transaction ID/);
      await user.click(copyButton);

      expect(writeTextSpy).toHaveBeenCalledWith(txId);
    });
  });

  // ── Data loading ──

  describe("data loading", () => {
    it("calls loadPayments on mount with identity selected", () => {
      const loadSpy = vi.fn();
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        payments: [],
        paymentsLoading: false,
        loadPayments: loadSpy,
      });
      renderWithProviders(<PaymentHistoryScreen />);

      expect(loadSpy).toHaveBeenCalledWith(100);
    });

    it("does not call loadPayments when no identity selected", () => {
      const loadSpy = vi.fn();
      useDashPayStore.setState({
        selectedIdentityId: null,
        loadPayments: loadSpy,
      });
      renderWithProviders(<PaymentHistoryScreen />);

      expect(loadSpy).not.toHaveBeenCalled();
    });
  });

  // ── Timestamp formatting ──

  describe("timestamp formatting", () => {
    it("shows 'just now' for very recent timestamps", () => {
      const payment = makePayment({
        confirmedAt: Math.floor(Date.now() / 1000) - 30, // 30 seconds ago
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/just now/)).toBeInTheDocument();
    });

    it("shows minutes for recent timestamps", () => {
      const payment = makePayment({
        confirmedAt: Math.floor(Date.now() / 1000) - 300, // 5 minutes ago
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/5m ago/)).toBeInTheDocument();
    });

    it("shows days for older timestamps", () => {
      const payment = makePayment({
        confirmedAt: Math.floor(Date.now() / 1000) - 172800, // 2 days ago
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/2d ago/)).toBeInTheDocument();
    });

    it("falls back to createdAt when confirmedAt is null", () => {
      const payment = makePayment({
        confirmedAt: null,
        createdAt: Math.floor(Date.now() / 1000) - 3600, // 1 hour ago
      });
      setupWithPayments([payment]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByText(/1h ago/)).toBeInTheDocument();
    });
  });

  // ── Accessibility ──

  describe("accessibility", () => {
    it("has accessible list role", () => {
      setupWithPayments([makePayment()]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getByRole("list")).toBeInTheDocument();
    });

    it("has accessible listitem roles for payment cards", () => {
      setupWithPayments([makePayment(), makePayment({ id: 2 })]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(screen.getAllByRole("listitem")).toHaveLength(2);
    });

    it("has aria-label on refresh button", () => {
      setupWithPayments([]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(
        screen.getByLabelText("Refresh payment history from Platform"),
      ).toBeInTheDocument();
    });

    it("has aria-label on copy transaction ID button", () => {
      setupWithPayments([makePayment()]);
      renderWithProviders(<PaymentHistoryScreen />);

      expect(
        screen.getByLabelText(/Copy transaction ID/),
      ).toBeInTheDocument();
    });
  });
});
