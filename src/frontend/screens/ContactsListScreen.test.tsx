import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ContactsListScreen } from "./ContactsListScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import { renderWithProviders } from "@/test/router-utils";
import type { StoredContactDto, StoredContactRequestDto } from "@/bindings";

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
  useRouterState: () => ({ location: { pathname: "/dashpay/contacts" } }),
}));

// ─── Test Fixtures ───────────────────────────────────────────────

function makeContact(
  overrides: Partial<StoredContactDto> = {},
): StoredContactDto {
  return {
    ownerIdentityId: "aa".repeat(32),
    contactIdentityId: "bb".repeat(32),
    username: "bob",
    displayName: "Bob Smith",
    avatarUrl: "https://example.com/bob.png",
    publicMessage: "Dash enthusiast and developer",
    contactStatus: "accepted",
    createdAt: Math.floor(Date.now() / 1000),
    updatedAt: Math.floor(Date.now() / 1000),
    lastSeen: null,
    ...overrides,
  };
}

function makeRequest(
  overrides: Partial<StoredContactRequestDto> = {},
): StoredContactRequestDto {
  return {
    id: 1,
    fromIdentityId: "cc".repeat(32),
    toIdentityId: "aa".repeat(32),
    toUsername: "alice",
    accountLabel: "Personal",
    requestType: "incoming",
    status: "pending",
    createdAt: Math.floor(Date.now() / 1000),
    respondedAt: null,
    expiresAt: null,
    ...overrides,
  };
}

// ─── Helpers ─────────────────────────────────────────────────────

function resetStore() {
  useDashPayStore.getState().reset();
}

function setupWithContacts(contacts: StoredContactDto[] = [makeContact()]) {
  useDashPayStore.setState({
    selectedIdentityId: "aa".repeat(32),
    contacts,
    contactsLoading: false,
    contactsError: null,
    contactSearchQuery: "",
    contactFilter: "all",
    contactSortField: "name",
    contactSortOrder: "ascending",
    showHidden: false,
    incomingRequests: [],
    outgoingRequests: [],
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
});

// ─── Tests ───────────────────────────────────────────────────────

describe("ContactsListScreen", () => {
  describe("no identity selected", () => {
    it("shows empty state when no identity is selected", () => {
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
    });

    it("shows description about selecting an identity", () => {
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByText(/Select an identity from the sidebar/),
      ).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner while contacts are loading", () => {
      useDashPayStore.setState({
        selectedIdentityId: "aa".repeat(32),
        contactsLoading: true,
      });
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Loading contacts...")).toBeInTheDocument();
    });
  });

  describe("empty contacts state", () => {
    it("shows empty state when no contacts exist", () => {
      setupWithContacts([]);
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("No Contacts")).toBeInTheDocument();
    });

    it("shows description about adding contacts", () => {
      setupWithContacts([]);
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByText(/haven't added any contacts/),
      ).toBeInTheDocument();
    });

    it('shows "Add Contact" button in empty state', () => {
      setupWithContacts([]);
      renderWithProviders(<ContactsListScreen />);
      // In empty state the EmptyState component renders an "Add Contact" action button
      const buttons = screen.getAllByRole("button", { name: /Add Contact/ });
      expect(buttons.length).toBeGreaterThanOrEqual(1);
    });
  });

  describe("header", () => {
    it("renders Contacts heading", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("heading", { name: "Contacts" }),
      ).toBeInTheDocument();
    });

    it("renders Refresh button", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: /Refresh/ }),
      ).toBeInTheDocument();
    });

    it("renders Add Contact button in header", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      const buttons = screen.getAllByRole("button", { name: /Add Contact/ });
      expect(buttons.length).toBeGreaterThanOrEqual(1);
    });

    it("shows spinner on Refresh button when refreshing", () => {
      setupWithContacts();
      useDashPayStore.setState({ contactsRefreshing: true });
      renderWithProviders(<ContactsListScreen />);
      const refreshBtn = screen.getByRole("button", { name: /Refresh/ });
      expect(refreshBtn).toBeDisabled();
    });
  });

  describe("tabs", () => {
    it("renders My Contacts tab", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("My Contacts")).toBeInTheDocument();
    });

    it("renders Requests tab", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Requests")).toBeInTheDocument();
    });

    it("shows pending request count badge", () => {
      setupWithContacts();
      useDashPayStore.setState({
        incomingRequests: [makeRequest(), makeRequest({ id: 2 })],
      });
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("2")).toBeInTheDocument();
    });
  });

  describe("contact card rendering", () => {
    it("renders contact card with display name", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Bob Smith")).toBeInTheDocument();
    });

    it("renders contact username", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("@bob")).toBeInTheDocument();
    });

    it("renders contact bio/public message", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByText("Dash enthusiast and developer"),
      ).toBeInTheDocument();
    });

    it("renders avatar image when URL is provided", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      const img = screen.getByAltText("Bob Smith's avatar");
      expect(img).toBeInTheDocument();
      expect(img).toHaveAttribute("src", "https://example.com/bob.png");
    });

    it("renders default avatar when no URL", () => {
      setupWithContacts([makeContact({ avatarUrl: null })]);
      renderWithProviders(<ContactsListScreen />);
      expect(screen.queryByRole("img")).not.toBeInTheDocument();
    });

    it("renders View Profile button per contact", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: "View Profile" }),
      ).toBeInTheDocument();
    });

    it("renders Hide button per contact", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: /Hide/ }),
      ).toBeInTheDocument();
    });

    it("renders Pay button per contact", () => {
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: "Pay" }),
      ).toBeInTheDocument();
    });

    it("shows [Hidden] prefix for hidden contacts", () => {
      setupWithContacts([
        makeContact({ contactStatus: "hidden" }),
      ]);
      useDashPayStore.setState({ showHidden: true });
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("[Hidden]", { exact: false })).toBeInTheDocument();
    });

    it("renders Unhide button for hidden contacts", () => {
      setupWithContacts([
        makeContact({ contactStatus: "hidden" }),
      ]);
      useDashPayStore.setState({ showHidden: true });
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: /Unhide/ }),
      ).toBeInTheDocument();
    });

    it("uses truncated ID as fallback display name", () => {
      setupWithContacts([
        makeContact({ displayName: null, username: null }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByText(/Contact \(/, { exact: false }),
      ).toBeInTheDocument();
    });
  });

  describe("multiple contacts", () => {
    it("renders all contact cards", () => {
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Bob" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Charlie" }),
        makeContact({ contactIdentityId: "dd".repeat(32), displayName: "Diana" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Bob")).toBeInTheDocument();
      expect(screen.getByText("Charlie")).toBeInTheDocument();
      expect(screen.getByText("Diana")).toBeInTheDocument();
    });

    it("renders contacts in sorted order (name ascending)", () => {
      setupWithContacts([
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Charlie" }),
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice" }),
        makeContact({ contactIdentityId: "dd".repeat(32), displayName: "Bob" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      const cards = screen.getAllByTestId("contact-card");
      expect(within(cards[0]!).getByText("Alice")).toBeInTheDocument();
      expect(within(cards[1]!).getByText("Bob")).toBeInTheDocument();
      expect(within(cards[2]!).getByText("Charlie")).toBeInTheDocument();
    });
  });

  describe("search", () => {
    it("renders search input", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByPlaceholderText("Search contacts..."),
      ).toBeInTheDocument();
    });

    it("filters contacts by display name", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      await user.type(
        screen.getByPlaceholderText("Search contacts..."),
        "Alice",
      );
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });

    it("filters contacts by username", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), username: "alice123", displayName: "Alice" }),
        makeContact({ contactIdentityId: "cc".repeat(32), username: "bob456", displayName: "Bob" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      await user.type(
        screen.getByPlaceholderText("Search contacts..."),
        "alice123",
      );
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });

    it("filters contacts by public message", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", publicMessage: "Love crypto" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", publicMessage: "Hello world" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      await user.type(
        screen.getByPlaceholderText("Search contacts..."),
        "crypto",
      );
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });

    it("search is case insensitive", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      await user.type(
        screen.getByPlaceholderText("Search contacts..."),
        "ALICE",
      );
      expect(screen.getByText("Alice")).toBeInTheDocument();
    });

    it("shows No Matches when search has no results", async () => {
      const user = userEvent.setup();
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      await user.type(
        screen.getByPlaceholderText("Search contacts..."),
        "zzzznonexistent",
      );
      expect(screen.getByText("No Matches")).toBeInTheDocument();
    });

    it("shows Clear button when search has text", async () => {
      const user = userEvent.setup();
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      await user.type(
        screen.getByPlaceholderText("Search contacts..."),
        "test",
      );
      expect(
        screen.getByRole("button", { name: "Clear search" }),
      ).toBeInTheDocument();
    });

    it("clears search when Clear button is clicked", async () => {
      const user = userEvent.setup();
      setupWithContacts([makeContact()]);
      renderWithProviders(<ContactsListScreen />);
      const input = screen.getByPlaceholderText("Search contacts...");
      await user.type(input, "test");
      await user.click(screen.getByRole("button", { name: "Clear search" }));
      expect(input).toHaveValue("");
    });
  });

  describe("filter", () => {
    it("renders filter dropdown", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: /Filter: All/ }),
      ).toBeInTheDocument();
    });

    it("filters to contacts with usernames", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", username: "alice" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", username: null }),
      ]);
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Filter: All/ }));
      await user.click(screen.getByText("With usernames"));

      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });

    it("filters to contacts without usernames", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", username: "alice" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", username: null }),
      ]);
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Filter: All/ }));
      await user.click(screen.getByText("No usernames"));

      expect(screen.queryByText("Alice")).not.toBeInTheDocument();
      expect(screen.getByText("Bob")).toBeInTheDocument();
    });

    it("filters to contacts with bio", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", publicMessage: "A bio" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", publicMessage: null }),
      ]);
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Filter: All/ }));
      await user.click(screen.getByText("With bio"));

      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });

    it("filters to hidden contacts only", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", contactStatus: "accepted" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", contactStatus: "hidden" }),
      ]);
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Filter: All/ }));
      await user.click(screen.getByText("Hidden"));

      expect(screen.queryByText("Alice")).not.toBeInTheDocument();
      expect(screen.getByText("Bob")).toBeInTheDocument();
    });

    it("filters to visible contacts only", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", contactStatus: "accepted" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", contactStatus: "hidden" }),
      ]);
      useDashPayStore.setState({ showHidden: true });
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Filter: All/ }));
      await user.click(screen.getByText("Visible"));

      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });
  });

  describe("sort", () => {
    it("renders sort dropdown", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("button", { name: /Sort: Name/ }),
      ).toBeInTheDocument();
    });

    it("sorts by username", async () => {
      const user = userEvent.setup();
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", username: "zeta" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", username: "alpha" }),
      ]);
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Sort: Name/ }));
      await user.click(screen.getByText("Username"));

      const cards = screen.getAllByTestId("contact-card");
      expect(within(cards[0]!).getByText("Bob")).toBeInTheDocument();
      expect(within(cards[1]!).getByText("Alice")).toBeInTheDocument();
    });

    it("sorts by date (newest first)", async () => {
      const user = userEvent.setup();
      const now = Math.floor(Date.now() / 1000);
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Old", createdAt: now - 86400 }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "New", createdAt: now }),
      ]);
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Sort: Name/ }));
      await user.click(screen.getByText("Date Added"));

      const cards = screen.getAllByTestId("contact-card");
      expect(within(cards[0]!).getByText("New")).toBeInTheDocument();
      expect(within(cards[1]!).getByText("Old")).toBeInTheDocument();
    });
  });

  describe("show hidden toggle", () => {
    it("renders Show hidden checkbox", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Show hidden")).toBeInTheDocument();
    });

    it("hidden contacts not shown by default", () => {
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", contactStatus: "accepted" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", contactStatus: "hidden" }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.queryByText("Bob")).not.toBeInTheDocument();
    });

    it("shows hidden contacts when toggle is on", () => {
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32), displayName: "Alice", contactStatus: "accepted" }),
        makeContact({ contactIdentityId: "cc".repeat(32), displayName: "Bob", contactStatus: "hidden" }),
      ]);
      useDashPayStore.setState({ showHidden: true });
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.getByText("Bob")).toBeInTheDocument();
    });
  });

  describe("error state", () => {
    it("shows error banner when contactsError is set", () => {
      setupWithContacts();
      useDashPayStore.setState({
        contactsError: "Failed to load contacts",
      });
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByText("Failed to load contacts"),
      ).toBeInTheDocument();
    });
  });

  describe("requests tab", () => {
    it("switches to requests tab when clicked", async () => {
      const user = userEvent.setup();
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      await user.click(screen.getByText("Requests"));
      expect(screen.getByText("No Contact Requests")).toBeInTheDocument();
    });

    it("shows incoming requests", async () => {
      const user = userEvent.setup();
      setupWithContacts();
      useDashPayStore.setState({
        incomingRequests: [makeRequest()],
      });
      renderWithProviders(<ContactsListScreen />);
      await user.click(screen.getByText("Requests"));
      expect(screen.getByText("Incoming (1)")).toBeInTheDocument();
    });

    it("shows outgoing requests", async () => {
      const user = userEvent.setup();
      setupWithContacts();
      useDashPayStore.setState({
        outgoingRequests: [
          makeRequest({
            id: 2,
            requestType: "outgoing",
            fromIdentityId: "aa".repeat(32),
            toIdentityId: "dd".repeat(32),
            toUsername: "dave",
          }),
        ],
      });
      renderWithProviders(<ContactsListScreen />);
      await user.click(screen.getByText("Requests"));
      expect(screen.getByText("Outgoing (1)")).toBeInTheDocument();
    });

    it("shows request account label", async () => {
      const user = userEvent.setup();
      setupWithContacts();
      useDashPayStore.setState({
        incomingRequests: [makeRequest({ accountLabel: "Business" })],
      });
      renderWithProviders(<ContactsListScreen />);
      await user.click(screen.getByText("Requests"));
      expect(screen.getByText("Account: Business")).toBeInTheDocument();
    });

    it("shows 'Cannot be cancelled' note for outgoing requests", async () => {
      const user = userEvent.setup();
      setupWithContacts();
      useDashPayStore.setState({
        outgoingRequests: [
          makeRequest({ id: 2, requestType: "outgoing" }),
        ],
      });
      renderWithProviders(<ContactsListScreen />);
      await user.click(screen.getByText("Requests"));
      expect(
        screen.getByText("Cannot be cancelled"),
      ).toBeInTheDocument();
    });

    it("shows loading state for requests", async () => {
      const user = userEvent.setup();
      setupWithContacts();
      useDashPayStore.setState({ requestsLoading: true });
      renderWithProviders(<ContactsListScreen />);
      await user.click(screen.getByText("Requests"));
      expect(screen.getByText("Loading requests...")).toBeInTheDocument();
    });
  });

  describe("hide/unhide action", () => {
    it("calls setContactHidden when Hide button is clicked", async () => {
      const user = userEvent.setup();
      const setContactHidden = vi.fn();
      setupWithContacts([makeContact()]);
      useDashPayStore.setState({ setContactHidden });
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Hide/ }));
      expect(setContactHidden).toHaveBeenCalledWith(
        "bb".repeat(32),
        true,
      );
    });

    it("calls setContactHidden with false when Unhide is clicked", async () => {
      const user = userEvent.setup();
      const setContactHidden = vi.fn();
      setupWithContacts([makeContact({ contactStatus: "hidden" })]);
      useDashPayStore.setState({ setContactHidden, showHidden: true });
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Unhide/ }));
      expect(setContactHidden).toHaveBeenCalledWith(
        "bb".repeat(32),
        false,
      );
    });
  });

  describe("refresh action", () => {
    it("calls refreshContacts when Refresh is clicked", async () => {
      const user = userEvent.setup();
      const refreshContacts = vi.fn();
      setupWithContacts();
      useDashPayStore.setState({ refreshContacts });
      renderWithProviders(<ContactsListScreen />);

      await user.click(screen.getByRole("button", { name: /Refresh/ }));
      expect(refreshContacts).toHaveBeenCalled();
    });
  });

  describe("accessibility", () => {
    it("has accessible contact list role", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("list", { name: "Contact list" }),
      ).toBeInTheDocument();
    });

    it("has accessible search input", () => {
      setupWithContacts();
      renderWithProviders(<ContactsListScreen />);
      expect(
        screen.getByRole("textbox", { name: "Search contacts" }),
      ).toBeInTheDocument();
    });

    it("contact cards are list items", () => {
      setupWithContacts([
        makeContact({ contactIdentityId: "bb".repeat(32) }),
        makeContact({ contactIdentityId: "cc".repeat(32) }),
      ]);
      renderWithProviders(<ContactsListScreen />);
      const items = screen.getAllByRole("listitem");
      expect(items).toHaveLength(2);
    });

    it("error banner has alert role", () => {
      setupWithContacts();
      useDashPayStore.setState({ contactsError: "Error" });
      renderWithProviders(<ContactsListScreen />);
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });
  });
});
