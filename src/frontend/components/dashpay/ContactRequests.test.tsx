import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ContactRequests } from "./ContactRequests";
import { useDashPayStore } from "@/stores/dashpayStore";
import { renderWithProviders } from "@/test/router-utils";
import type { StoredContactRequestDto } from "@/bindings";

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

// ─── Test Fixtures ───────────────────────────────────────────────

function makeIncomingRequest(
  overrides: Partial<StoredContactRequestDto> = {},
): StoredContactRequestDto {
  return {
    id: 1,
    fromIdentityId: "cc".repeat(32),
    toIdentityId: "aa".repeat(32),
    toUsername: "alice",
    fromUsername: "SenderUser",
    accountLabel: "Personal",
    requestType: "incoming",
    status: "pending",
    createdAt: Math.floor(Date.now() / 1000) - 3600, // 1 hour ago
    respondedAt: null,
    expiresAt: null,
    platformDocumentId: "ff".repeat(32),
    ...overrides,
  };
}

function makeOutgoingRequest(
  overrides: Partial<StoredContactRequestDto> = {},
): StoredContactRequestDto {
  return {
    id: 10,
    fromIdentityId: "aa".repeat(32),
    toIdentityId: "dd".repeat(32),
    toUsername: "dave",
    fromUsername: null,
    accountLabel: null,
    requestType: "outgoing",
    status: "pending",
    createdAt: Math.floor(Date.now() / 1000) - 86400, // 1 day ago
    respondedAt: null,
    expiresAt: null,
    platformDocumentId: "ee".repeat(32),
    ...overrides,
  };
}

// ─── Helpers ─────────────────────────────────────────────────────

function resetStore() {
  useDashPayStore.getState().reset();
}

function setupWithRequests(opts: {
  incoming?: StoredContactRequestDto[];
  outgoing?: StoredContactRequestDto[];
} = {}) {
  useDashPayStore.setState({
    selectedIdentityId: "aa".repeat(32),
    incomingRequests: opts.incoming ?? [],
    outgoingRequests: opts.outgoing ?? [],
    requestsLoading: false,
    requestsError: null,
    acceptingIds: new Set(),
    rejectingIds: new Set(),
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
});

// ─── Tests ───────────────────────────────────────────────────────

describe("ContactRequests", () => {
  describe("loading state", () => {
    it("shows loading spinner when requests are loading", () => {
      useDashPayStore.setState({
        selectedIdentityId: "aa".repeat(32),
        requestsLoading: true,
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("Loading requests...")).toBeInTheDocument();
    });
  });

  describe("sub-tabs", () => {
    it("renders Incoming and Outgoing sub-tabs", () => {
      setupWithRequests();
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("Incoming")).toBeInTheDocument();
      expect(screen.getByText("Outgoing")).toBeInTheDocument();
    });

    it("shows incoming tab by default", () => {
      setupWithRequests();
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByText("No Incoming Requests"),
      ).toBeInTheDocument();
    });

    it("switches to outgoing tab when clicked", async () => {
      const user = userEvent.setup();
      setupWithRequests();
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(
        screen.getByText("No Outgoing Requests"),
      ).toBeInTheDocument();
    });

    it("shows pending count badge on incoming tab", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest(), makeIncomingRequest({ id: 2 })],
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("2")).toBeInTheDocument();
    });

    it("shows count badge on outgoing tab", () => {
      setupWithRequests({
        outgoing: [makeOutgoingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("1")).toBeInTheDocument();
    });
  });

  describe("incoming requests empty state", () => {
    it("shows empty state when no incoming requests", () => {
      setupWithRequests({ incoming: [] });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByText("No Incoming Requests"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/don't have any pending contact requests/),
      ).toBeInTheDocument();
    });
  });

  describe("outgoing requests empty state", () => {
    it("shows empty state when no outgoing requests", async () => {
      const user = userEvent.setup();
      setupWithRequests({ outgoing: [] });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(
        screen.getByText("No Outgoing Requests"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/haven't sent any contact requests/),
      ).toBeInTheDocument();
    });

    it("shows Add Contact button in outgoing empty state", async () => {
      const user = userEvent.setup();
      const onAddContact = vi.fn();
      setupWithRequests({ outgoing: [] });
      renderWithProviders(<ContactRequests onAddContact={onAddContact} />);
      await user.click(screen.getByText("Outgoing"));
      const addBtn = screen.getByRole("button", { name: /Add Contact/ });
      expect(addBtn).toBeInTheDocument();
      await user.click(addBtn);
      expect(onAddContact).toHaveBeenCalled();
    });
  });

  describe("incoming request card rendering", () => {
    it("renders incoming request card with display name", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ fromUsername: "alice" })],
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("alice")).toBeInTheDocument();
    });

    it("renders identity ID on incoming request", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByText(/ID:/, { exact: false }),
      ).toBeInTheDocument();
    });

    it("renders account label on incoming request", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ accountLabel: "Business" })],
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("Account: Business")).toBeInTheDocument();
    });

    it("renders timestamp on incoming request", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByText(/Received:/, { exact: false }),
      ).toBeInTheDocument();
    });

    it("renders Accept and Reject buttons for pending request", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByRole("button", { name: /Accept/ }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /Reject/ }),
      ).toBeInTheDocument();
    });

    it("shows Accepted badge for accepted requests", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ status: "accepted" })],
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("Accepted")).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /Accept/ }),
      ).not.toBeInTheDocument();
    });

    it("shows Rejected badge for rejected requests", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ status: "rejected" })],
      });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByText("Rejected")).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /Reject/ }),
      ).not.toBeInTheDocument();
    });

    it("falls back to truncated identity ID when no username", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ fromUsername: null })],
      });
      renderWithProviders(<ContactRequests />);
      // Should show truncated fromIdentityId as the card's display name
      const card = screen.getByTestId("incoming-request-card");
      const displayName = card.querySelector("p.text-sm.font-medium");
      expect(displayName?.textContent).toContain("...");
    });
  });

  describe("outgoing request card rendering", () => {
    it("renders outgoing request card with To: prefix", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        outgoing: [makeOutgoingRequest({ toUsername: "dave" })],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(screen.getByText("To: dave")).toBeInTheDocument();
    });

    it("shows Status on outgoing request", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        outgoing: [makeOutgoingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(
        screen.getByText(/Status:/, { exact: false }),
      ).toBeInTheDocument();
    });

    it("shows 'Cannot be cancelled' note on outgoing request", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        outgoing: [makeOutgoingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(
        screen.getByText("Cannot be cancelled once sent"),
      ).toBeInTheDocument();
    });

    it("shows Sent timestamp on outgoing request", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        outgoing: [makeOutgoingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(
        screen.getByText(/Sent:/, { exact: false }),
      ).toBeInTheDocument();
    });
  });

  describe("accept flow", () => {
    it("shows confirmation dialog when Accept is clicked", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        incoming: [makeIncomingRequest({ fromUsername: "alice" })],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByRole("button", { name: /Accept/ }));
      expect(
        screen.getByText("Accept Contact Request"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/accept the contact request from alice/),
      ).toBeInTheDocument();
    });

    it("calls acceptContactRequest when confirmed", async () => {
      const user = userEvent.setup();
      const acceptContactRequest = vi.fn();
      const platformDocId = "ab".repeat(32);
      setupWithRequests({
        incoming: [makeIncomingRequest({ id: 42, platformDocumentId: platformDocId })],
      });
      useDashPayStore.setState({ acceptContactRequest });
      renderWithProviders(<ContactRequests />);

      await user.click(screen.getByRole("button", { name: /Accept/ }));
      // Click "Accept" in the confirmation dialog
      const dialogConfirmBtn = screen.getAllByRole("button", {
        name: /Accept/,
      });
      // The last one is the dialog's confirm button
      await user.click(dialogConfirmBtn[dialogConfirmBtn.length - 1]!);

      expect(acceptContactRequest).toHaveBeenCalledWith(42, platformDocId);
    });

    it("does not call acceptContactRequest when canceled", async () => {
      const user = userEvent.setup();
      const acceptContactRequest = vi.fn();
      setupWithRequests({
        incoming: [makeIncomingRequest()],
      });
      useDashPayStore.setState({ acceptContactRequest });
      renderWithProviders(<ContactRequests />);

      await user.click(screen.getByRole("button", { name: /Accept/ }));
      await user.click(screen.getByRole("button", { name: /Cancel/ }));

      expect(acceptContactRequest).not.toHaveBeenCalled();
    });

    it("shows loading spinner when accepting", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ id: 1 })],
      });
      useDashPayStore.setState({ acceptingIds: new Set([1]) });
      renderWithProviders(<ContactRequests />);
      // Accept button should be disabled
      expect(
        screen.getByRole("button", { name: /Accept/ }),
      ).toBeDisabled();
      expect(
        screen.getByRole("button", { name: /Reject/ }),
      ).toBeDisabled();
    });
  });

  describe("reject flow", () => {
    it("shows rejection confirmation dialog when Reject is clicked", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        incoming: [makeIncomingRequest({ fromUsername: "alice" })],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByRole("button", { name: /Reject/ }));
      expect(
        screen.getByText("Reject Contact Request"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/reject the contact request from alice/),
      ).toBeInTheDocument();
    });

    it("calls rejectContactRequest when confirmed", async () => {
      const user = userEvent.setup();
      const rejectContactRequest = vi.fn();
      const platformDocId = "cd".repeat(32);
      setupWithRequests({
        incoming: [makeIncomingRequest({ id: 99, platformDocumentId: platformDocId })],
      });
      useDashPayStore.setState({ rejectContactRequest });
      renderWithProviders(<ContactRequests />);

      await user.click(screen.getByRole("button", { name: /Reject/ }));
      // Click "Reject" in the confirmation dialog
      const dialogConfirmBtns = screen.getAllByRole("button", {
        name: /Reject/,
      });
      await user.click(dialogConfirmBtns[dialogConfirmBtns.length - 1]!);

      expect(rejectContactRequest).toHaveBeenCalledWith(99, platformDocId);
    });

    it("does not call rejectContactRequest when canceled", async () => {
      const user = userEvent.setup();
      const rejectContactRequest = vi.fn();
      setupWithRequests({
        incoming: [makeIncomingRequest()],
      });
      useDashPayStore.setState({ rejectContactRequest });
      renderWithProviders(<ContactRequests />);

      await user.click(screen.getByRole("button", { name: /Reject/ }));
      await user.click(screen.getByRole("button", { name: /Cancel/ }));

      expect(rejectContactRequest).not.toHaveBeenCalled();
    });

    it("shows loading spinner when rejecting", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest({ id: 1 })],
      });
      useDashPayStore.setState({ rejectingIds: new Set([1]) });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByRole("button", { name: /Accept/ }),
      ).toBeDisabled();
      expect(
        screen.getByRole("button", { name: /Reject/ }),
      ).toBeDisabled();
    });
  });

  describe("error display", () => {
    it("shows error banner when requestsError is set", () => {
      setupWithRequests();
      useDashPayStore.setState({
        requestsError: "Failed to load requests",
      });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByText("Failed to load requests"),
      ).toBeInTheDocument();
    });

    it("error banner has alert role", () => {
      setupWithRequests();
      useDashPayStore.setState({ requestsError: "Error" });
      renderWithProviders(<ContactRequests />);
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });
  });

  describe("multiple requests", () => {
    it("renders all incoming request cards", () => {
      setupWithRequests({
        incoming: [
          makeIncomingRequest({ id: 1, fromUsername: "alice" }),
          makeIncomingRequest({ id: 2, fromUsername: "bob" }),
          makeIncomingRequest({ id: 3, fromUsername: "charlie" }),
        ],
      });
      renderWithProviders(<ContactRequests />);
      const cards = screen.getAllByTestId("incoming-request-card");
      expect(cards).toHaveLength(3);
    });

    it("renders all outgoing request cards", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        outgoing: [
          makeOutgoingRequest({ id: 10, toUsername: "dave" }),
          makeOutgoingRequest({ id: 11, toUsername: "eve" }),
        ],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      const cards = screen.getAllByTestId("outgoing-request-card");
      expect(cards).toHaveLength(2);
    });
  });

  describe("accessibility", () => {
    it("incoming requests list has proper role and label", () => {
      setupWithRequests({
        incoming: [makeIncomingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      expect(
        screen.getByRole("list", { name: "Incoming requests" }),
      ).toBeInTheDocument();
    });

    it("outgoing requests list has proper role and label", async () => {
      const user = userEvent.setup();
      setupWithRequests({
        outgoing: [makeOutgoingRequest()],
      });
      renderWithProviders(<ContactRequests />);
      await user.click(screen.getByText("Outgoing"));
      expect(
        screen.getByRole("list", { name: "Outgoing requests" }),
      ).toBeInTheDocument();
    });

    it("request cards are list items", () => {
      setupWithRequests({
        incoming: [
          makeIncomingRequest({ id: 1 }),
          makeIncomingRequest({ id: 2 }),
        ],
      });
      renderWithProviders(<ContactRequests />);
      const items = screen.getAllByRole("listitem");
      expect(items).toHaveLength(2);
    });
  });
});
