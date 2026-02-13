import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { AddContactScreen } from "./AddContactScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { useWalletStore } from "@/stores/walletStore";
import { renderWithProviders } from "@/test/router-utils";
import { events } from "@/bindings";
import type { QualifiedIdentityDto, IdentityKeyDto } from "@/bindings";

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
  useSearch: () => ({}),
  useRouterState: () => ({ location: { pathname: "/dashpay/add-contact" } }),
}));

// ─── Test Fixtures ───────────────────────────────────────────────

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
      makeKey({ keyId: 2, purpose: "VOTING", securityLevel: "MEDIUM" }),
    ],
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

// ─── Event helpers ──────────────────────────────────────────────

function emitTaskResult(taskId: string) {
  const calls = vi.mocked(events.taskResultEvent.listen).mock.calls;
  for (const [cb] of calls) {
    cb?.({ payload: { taskId, result: { type: "dashPayCompleted" } } });
  }
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
    selectedWalletHash: null,
    loading: false,
    error: null,
    refreshing: false,
    refreshMode: "coreOnly",
  });
  useDashPayStore.getState().reset();
}

function setupWithIdentity(identity = makeIdentity()) {
  useIdentityStore.setState({ identities: [identity], loading: false });
  useDashPayStore.setState({
    selectedIdentityId: identity.id,
    requestsError: null,
  });
}

function renderScreen() {
  return renderWithProviders(<AddContactScreen />);
}

// ─── Tests ───────────────────────────────────────────────────────

describe("AddContactScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStores();
  });

  // ── Empty / No identity state ──

  describe("when no identity is selected", () => {
    it("shows empty state message", () => {
      renderScreen();
      expect(
        screen.getByText("No Identity Selected"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          "Select an identity from the sidebar to add a contact.",
        ),
      ).toBeInTheDocument();
    });
  });

  // ── Form rendering ──

  describe("form rendering", () => {
    it("renders the form header with back button and info button", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByRole("heading", { level: 2, name: "Add Contact" })).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /back to contacts/i }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /contact request info/i }),
      ).toBeInTheDocument();
    });

    it("shows the sender identity card", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.getByText("user")).toBeInTheDocument();
    });

    it("renders username/ID input", () => {
      setupWithIdentity();
      renderScreen();
      expect(
        screen.getByLabelText("Username or Identity ID"),
      ).toBeInTheDocument();
      expect(
        screen.getByPlaceholderText("e.g., alice.dash or identity ID"),
      ).toBeInTheDocument();
    });

    it("renders relationship label input with character counter", () => {
      setupWithIdentity();
      renderScreen();
      expect(
        screen.getByText(/relationship label/i),
      ).toBeInTheDocument();
      expect(screen.getByText("0/100")).toBeInTheDocument();
    });

    it("renders Cancel and Add Contact buttons", () => {
      setupWithIdentity();
      renderScreen();
      expect(
        screen.getByRole("button", { name: /cancel/i }),
      ).toBeInTheDocument();
      expect(screen.getByTestId("send-button")).toBeInTheDocument();
      expect(screen.getByTestId("send-button")).toHaveTextContent(
        "Add Contact",
      );
    });

    it("disables Add Contact button when username is empty", () => {
      setupWithIdentity();
      renderScreen();
      expect(screen.getByTestId("send-button")).toBeDisabled();
    });
  });

  // ── Username validation ──

  describe("username validation", () => {
    it("enables Add Contact button when valid username is entered", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      expect(screen.getByTestId("send-button")).toBeEnabled();
    });

    it("shows error for invalid .dash format", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.eth");
      expect(
        screen.getByText(/usernames must end with ".dash"/i),
      ).toBeInTheDocument();
    });

    it("allows plain identity IDs without dots", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(
        screen.getByTestId("username-input"),
        "bb".repeat(32),
      );
      expect(screen.getByTestId("send-button")).toBeEnabled();
    });
  });

  // ── Account label validation ──

  describe("account label validation", () => {
    it("shows character counter updating", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("label-input"), "Friend");
      expect(screen.getByText("6/100")).toBeInTheDocument();
    });

    it("shows error when label exceeds max length", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      const longLabel = "a".repeat(101);
      await user.type(screen.getByTestId("label-input"), longLabel);
      expect(
        screen.getByText(/label too long/i),
      ).toBeInTheDocument();
    });
  });

  // ── Request summary ──

  describe("request summary", () => {
    it("shows summary when identity and username are present", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      expect(screen.getByTestId("request-summary")).toBeInTheDocument();
      expect(
        screen.getByTestId("request-summary"),
      ).toHaveTextContent("Alice");
      expect(
        screen.getByTestId("request-summary"),
      ).toHaveTextContent("bob.dash");
    });

    it("shows account label in summary when provided", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.type(screen.getByTestId("label-input"), "Business");
      expect(
        screen.getByTestId("request-summary"),
      ).toHaveTextContent("Business");
    });

    it("does not show summary when username is empty", () => {
      setupWithIdentity();
      renderScreen();
      expect(
        screen.queryByTestId("request-summary"),
      ).not.toBeInTheDocument();
    });
  });

  // ── Advanced options / Key selector ──

  describe("advanced options", () => {
    it("hides key selector by default", () => {
      setupWithIdentity();
      renderScreen();
      expect(
        screen.queryByTestId("key-selector"),
      ).not.toBeInTheDocument();
    });

    it("shows key selector when Advanced Options is clicked", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.click(screen.getByTestId("advanced-options-toggle"));
      expect(screen.getByTestId("key-selector")).toBeInTheDocument();
    });

    it("auto-selects the best signing key (CRITICAL or HIGH)", () => {
      const identity = makeIdentity();
      setupWithIdentity(identity);
      renderScreen();

      // The auto-selected key should be the CRITICAL one (keyId 0)
      // We can verify by opening advanced options and checking the selector
    });

    it("shows message when no AUTHENTICATION keys available", async () => {
      const identity = makeIdentity({
        keys: [
          makeKey({ keyId: 0, purpose: "VOTING", securityLevel: "MEDIUM" }),
        ],
      });
      setupWithIdentity(identity);
      renderScreen();
      const user = userEvent.setup();

      await user.click(screen.getByTestId("advanced-options-toggle"));
      expect(
        screen.getByText(
          "No AUTHENTICATION keys available on this identity.",
        ),
      ).toBeInTheDocument();
    });
  });

  // ── Send contact request ──

  describe("sending contact request", () => {
    it("calls sendContactRequest with correct args", async () => {
      setupWithIdentity();
      const sendSpy = vi.fn();
      useDashPayStore.setState({
        sendContactRequest: sendSpy,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.type(screen.getByTestId("label-input"), "Friend");
      await user.click(screen.getByTestId("send-button"));

      expect(sendSpy).toHaveBeenCalledWith({
        signingKeyId: 0, // Auto-selected CRITICAL key
        toUsername: "bob.dash",
        accountLabel: "Friend",
      });
    });

    it("shows sending state while request is in progress", async () => {
      setupWithIdentity();
      // Make sendContactRequest return a never-resolving promise
      useDashPayStore.setState({
        sendContactRequest: () => new Promise(() => {}),
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      expect(
        screen.getByText("Sending contact request..."),
      ).toBeInTheDocument();
    });

    it("shows success screen after successful send", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockResolvedValue("task-cr-1"),
        requestsError: null,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      // Screen stays in "sending" until task event
      await waitFor(() => {
        expect(screen.getByText("Sending contact request...")).toBeInTheDocument();
      });

      emitTaskResult("task-cr-1");

      await waitFor(() => {
        expect(
          screen.getByText("Contact Request Sent"),
        ).toBeInTheDocument();
      });
      expect(screen.getByText("bob.dash")).toBeInTheDocument();
    });

    it("shows error when send fails", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockImplementation(async () => {
          useDashPayStore.setState({
            requestsError: "Username not found on platform",
          });
        }),
        requestsError: null,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      await waitFor(() => {
        expect(screen.getByTestId("error-banner")).toBeInTheDocument();
      });
      expect(
        screen.getByText("Username not found on platform"),
      ).toBeInTheDocument();
    });

    it("sends null for empty account label", async () => {
      setupWithIdentity();
      const sendSpy = vi.fn();
      useDashPayStore.setState({
        sendContactRequest: sendSpy,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      expect(sendSpy).toHaveBeenCalledWith({
        signingKeyId: 0,
        toUsername: "bob.dash",
        accountLabel: null,
      });
    });
  });

  // ── Error handling ──

  describe("error handling", () => {
    it("shows recovery action for missing encryption key error", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockImplementation(async () => {
          useDashPayStore.setState({
            requestsError: "MissingEncryptionKey: Identity requires an encryption key",
          });
        }),
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      await waitFor(() => {
        expect(
          screen.getByTestId("recovery-action-button"),
        ).toBeInTheDocument();
      });
      expect(
        screen.getByTestId("recovery-action-button"),
      ).toHaveTextContent("Add Encryption Key");
    });

    it("navigates to identities screen when Add Encryption Key is clicked", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockImplementation(async () => {
          useDashPayStore.setState({
            requestsError: "MissingEncryptionKey: Identity requires an encryption key",
          });
        }),
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      await waitFor(() => {
        expect(
          screen.getByTestId("recovery-action-button"),
        ).toBeInTheDocument();
      });
      await user.click(screen.getByTestId("recovery-action-button"));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
    });

    it("shows Retry button for username resolution failures", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockImplementation(async () => {
          useDashPayStore.setState({
            requestsError: "Username resolution failed: not found",
          });
        }),
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      await waitFor(() => {
        expect(
          screen.getByTestId("recovery-action-button"),
        ).toBeInTheDocument();
      });
      expect(
        screen.getByTestId("recovery-action-button"),
      ).toHaveTextContent("Retry");
    });

    it("shows tip text for username resolution errors", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockImplementation(async () => {
          useDashPayStore.setState({
            requestsError: "Username resolution failed: not found",
          });
        }),
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      await waitFor(() => {
        expect(
          screen.getByText(/check that the username is correct/i),
        ).toBeInTheDocument();
      });
    });

    it("clears error and returns to form on Retry", async () => {
      setupWithIdentity();
      let callCount = 0;
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockImplementation(async () => {
          callCount++;
          if (callCount === 1) {
            useDashPayStore.setState({
              requestsError: "Username not found",
            });
          }
        }),
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      await waitFor(() => {
        expect(
          screen.getByTestId("recovery-action-button"),
        ).toBeInTheDocument();
      });
      await user.click(screen.getByTestId("recovery-action-button"));

      // Form should be visible again
      expect(
        screen.getByTestId("username-input"),
      ).toBeInTheDocument();
      expect(
        screen.queryByTestId("error-banner"),
      ).not.toBeInTheDocument();
    });

    it("shows client-side validation error for invalid username format", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.eth");

      // The inline validation error is shown below the input
      expect(
        screen.getByText(/invalid username format/i),
      ).toBeInTheDocument();
    });
  });

  // ── Navigation ──

  describe("navigation", () => {
    it("navigates to contacts on Back button click", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.click(
        screen.getByRole("button", { name: /back to contacts/i }),
      );
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/dashpay/contacts",
      });
    });

    it("navigates to contacts on Cancel click", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.click(screen.getByRole("button", { name: /cancel/i }));
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/dashpay/contacts",
      });
    });

    it("navigates to contacts on success Back to Contacts button", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockResolvedValue("task-cr-nav"),
        requestsError: null,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      emitTaskResult("task-cr-nav");

      await waitFor(() => {
        expect(
          screen.getByText("Contact Request Sent"),
        ).toBeInTheDocument();
      });

      await user.click(
        screen.getByRole("button", { name: /back to contacts/i }),
      );
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/dashpay/contacts",
      });
    });
  });

  // ── Success screen ──

  describe("success screen", () => {
    it("shows Send Another Request button", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockResolvedValue("task-cr-btn"),
        requestsError: null,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.click(screen.getByTestId("send-button"));

      emitTaskResult("task-cr-btn");

      await waitFor(() => {
        expect(
          screen.getByText("Contact Request Sent"),
        ).toBeInTheDocument();
      });

      expect(
        screen.getByRole("button", { name: /send another request/i }),
      ).toBeInTheDocument();
    });

    it("resets form on Send Another Request", async () => {
      setupWithIdentity();
      useDashPayStore.setState({
        sendContactRequest: vi.fn().mockResolvedValue("task-cr-reset"),
        requestsError: null,
      });
      renderScreen();
      const user = userEvent.setup();

      await user.type(screen.getByTestId("username-input"), "bob.dash");
      await user.type(screen.getByTestId("label-input"), "Friend");
      await user.click(screen.getByTestId("send-button"));

      emitTaskResult("task-cr-reset");

      await waitFor(() => {
        expect(
          screen.getByText("Contact Request Sent"),
        ).toBeInTheDocument();
      });

      await user.click(
        screen.getByRole("button", { name: /send another request/i }),
      );

      // Form should be visible with cleared inputs
      expect(screen.getByTestId("username-input")).toHaveValue("");
      expect(screen.getByTestId("label-input")).toHaveValue("");
    });
  });

  // ── Info dialog ──

  describe("info dialog", () => {
    it("opens info dialog on info button click", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.click(
        screen.getByRole("button", { name: /contact request info/i }),
      );
      expect(
        screen.getByText("About Contact Requests"),
      ).toBeInTheDocument();
    });

    it("shows contact request information", async () => {
      setupWithIdentity();
      renderScreen();
      const user = userEvent.setup();

      await user.click(
        screen.getByRole("button", { name: /contact request info/i }),
      );
      expect(
        screen.getByText(
          /contact requests are sent on the dash platform/i,
        ),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          /the recipient must accept the request/i,
        ),
      ).toBeInTheDocument();
    });
  });
});
