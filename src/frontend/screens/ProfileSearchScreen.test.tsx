import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ProfileSearchScreen } from "./ProfileSearchScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import type { ProfileSearchResult } from "@/stores/dashpayStore";
import { renderWithProviders } from "@/test/router-utils";

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
  useRouterState: () => ({ location: { pathname: "/dashpay/search" } }),
}));

// ─── Test Fixtures ───────────────────────────────────────────────

const IDENTITY_A = "aa".repeat(32);
const IDENTITY_B = "bb".repeat(32);
const IDENTITY_C = "cc".repeat(32);

function makeSearchResult(
  overrides: Partial<ProfileSearchResult> = {},
): ProfileSearchResult {
  return {
    identityId: IDENTITY_B,
    username: "alice.dash",
    displayName: "Alice Wonderland",
    publicMessage: "Hello from Dash Platform!",
    avatarUrl: null,
    ...overrides,
  };
}

// ─── Helpers ─────────────────────────────────────────────────────

function resetStore() {
  useDashPayStore.getState().reset();
}

function setupWithIdentity() {
  useDashPayStore.setState({ selectedIdentityId: IDENTITY_A });
}

function setupWithResults(results: ProfileSearchResult[]) {
  useDashPayStore.setState({
    selectedIdentityId: IDENTITY_A,
    searchResults: results,
    searchLoading: false,
  });
}

async function renderScreen() {
  return renderWithProviders(<ProfileSearchScreen />);
}

// ─── Tests ────────────────────────────────────────────────────────

describe("ProfileSearchScreen", () => {
  beforeEach(() => {
    resetStore();
    mockNavigate.mockClear();
  });

  // ── No identity selected ─────────────────────────────────────

  describe("when no identity selected", () => {
    it("shows empty state asking to select identity", async () => {
      await renderScreen();
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
      expect(
        screen.getByText(/select an identity/i),
      ).toBeInTheDocument();
    });

    it("does not show search input", async () => {
      await renderScreen();
      expect(
        screen.queryByPlaceholderText("Enter DPNS username..."),
      ).not.toBeInTheDocument();
    });
  });

  // ── Header ───────────────────────────────────────────────────

  describe("header", () => {
    it("shows search heading", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(
        screen.getByText("Search Public Profiles"),
      ).toBeInTheDocument();
    });

    it("shows info button", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(
        screen.getByRole("button", { name: /about profile search/i }),
      ).toBeInTheDocument();
    });
  });

  // ── Search input ─────────────────────────────────────────────

  describe("search input", () => {
    it("renders search input and button", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(
        screen.getByPlaceholderText("Enter DPNS username..."),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /^search$/i }),
      ).toBeInTheDocument();
    });

    it("shows tip text about prefix search", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(screen.getByText(/tip: search by dpns username prefix/i)).toBeInTheDocument();
    });

    it("disables search button when input is empty", async () => {
      setupWithIdentity();
      await renderScreen();
      const searchButton = screen.getByRole("button", { name: /^search$/i });
      expect(searchButton).toBeDisabled();
    });

    it("enables search button when input has text", async () => {
      setupWithIdentity();
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "alice");
      const searchButton = screen.getByRole("button", { name: /^search$/i });
      expect(searchButton).toBeEnabled();
    });

    it("calls searchProfiles on button click", async () => {
      setupWithIdentity();
      const searchSpy = vi.fn();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      useDashPayStore.setState({ searchProfiles: searchSpy } as any);
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "alice");
      await user.click(screen.getByRole("button", { name: /^search$/i }));
      expect(searchSpy).toHaveBeenCalledWith("alice");
    });

    it("calls searchProfiles on Enter key", async () => {
      setupWithIdentity();
      const searchSpy = vi.fn();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      useDashPayStore.setState({ searchProfiles: searchSpy } as any);
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "bob{Enter}");
      expect(searchSpy).toHaveBeenCalledWith("bob");
    });

    it("does not call searchProfiles when input is whitespace only", async () => {
      setupWithIdentity();
      const searchSpy = vi.fn();
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      useDashPayStore.setState({ searchProfiles: searchSpy } as any);
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "   {Enter}");
      expect(searchSpy).not.toHaveBeenCalled();
    });
  });

  // ── Clear button ─────────────────────────────────────────────

  describe("clear button", () => {
    it("shows clear button after typing", async () => {
      setupWithIdentity();
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "test");
      expect(
        screen.getByRole("button", { name: /clear/i }),
      ).toBeInTheDocument();
    });

    it("clears input and results on click", async () => {
      const clearSpy = vi.fn();
      setupWithResults([makeSearchResult()]);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      useDashPayStore.setState({ clearSearchResults: clearSpy } as any);
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "alice");
      await user.click(screen.getByRole("button", { name: /clear/i }));
      expect(clearSpy).toHaveBeenCalled();
      expect(input).toHaveValue("");
    });
  });

  // ── Loading state ────────────────────────────────────────────

  describe("loading state", () => {
    it("shows loading spinner when searching", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        searchLoading: true,
      });
      await renderScreen();
      expect(screen.getByText("Searching...")).toBeInTheDocument();
    });

    it("does not show results while loading", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        searchLoading: true,
        searchResults: [makeSearchResult()],
      });
      await renderScreen();
      expect(screen.queryByText("Search Results")).not.toBeInTheDocument();
    });
  });

  // ── Error display ────────────────────────────────────────────

  describe("error display", () => {
    it("shows error message", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        searchError: "Network error occurred",
      });
      await renderScreen();
      expect(
        screen.getByText("Network error occurred"),
      ).toBeInTheDocument();
    });

    it("uses alert role for error", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        searchError: "Something went wrong",
      });
      await renderScreen();
      expect(screen.getByRole("alert")).toBeInTheDocument();
    });
  });

  // ── Search results ───────────────────────────────────────────

  describe("search results", () => {
    it("shows results count badge", async () => {
      setupWithResults([makeSearchResult(), makeSearchResult({ identityId: IDENTITY_C, username: "bob.dash" })]);
      await renderScreen();
      expect(screen.getByText("Search Results")).toBeInTheDocument();
      expect(screen.getByText("2")).toBeInTheDocument();
    });

    it("renders username prominently", async () => {
      setupWithResults([makeSearchResult({ username: "alice.dash" })]);
      await renderScreen();
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
    });

    it("renders display name", async () => {
      setupWithResults([makeSearchResult({ displayName: "Alice Wonderland" })]);
      await renderScreen();
      expect(screen.getByText("Alice Wonderland")).toBeInTheDocument();
    });

    it("renders public message preview", async () => {
      setupWithResults([makeSearchResult({ publicMessage: "Hello from Dash!" })]);
      await renderScreen();
      expect(screen.getByText("Hello from Dash!")).toBeInTheDocument();
    });

    it("truncates long public messages", async () => {
      const longMessage = "A".repeat(80);
      setupWithResults([makeSearchResult({ publicMessage: longMessage })]);
      await renderScreen();
      expect(screen.getByText(`${"A".repeat(60)}...`)).toBeInTheDocument();
    });

    it("renders identity ID", async () => {
      setupWithResults([makeSearchResult({ identityId: IDENTITY_B })]);
      await renderScreen();
      expect(screen.getByText(`ID: ${IDENTITY_B}`)).toBeInTheDocument();
    });

    it("does not render display name when null", async () => {
      setupWithResults([makeSearchResult({ displayName: null })]);
      await renderScreen();
      expect(screen.queryByText("Alice Wonderland")).not.toBeInTheDocument();
    });

    it("does not render public message when null", async () => {
      setupWithResults([makeSearchResult({ publicMessage: null })]);
      await renderScreen();
      expect(screen.queryByText(/hello from/i)).not.toBeInTheDocument();
    });

    it("renders multiple results", async () => {
      setupWithResults([
        makeSearchResult({ identityId: IDENTITY_B, username: "alice.dash" }),
        makeSearchResult({ identityId: IDENTITY_C, username: "bob.dash", displayName: "Bob" }),
      ]);
      await renderScreen();
      expect(screen.getByText("alice.dash")).toBeInTheDocument();
      expect(screen.getByText("bob.dash")).toBeInTheDocument();
      expect(screen.getByText("Bob")).toBeInTheDocument();
    });
  });

  // ── Action buttons ───────────────────────────────────────────

  describe("action buttons", () => {
    it("shows View Profile button for each result", async () => {
      setupWithResults([makeSearchResult()]);
      await renderScreen();
      expect(
        screen.getByRole("button", { name: /view profile/i }),
      ).toBeInTheDocument();
    });

    it("shows Add Contact button for each result", async () => {
      setupWithResults([makeSearchResult()]);
      await renderScreen();
      expect(
        screen.getByRole("button", { name: /add contact/i }),
      ).toBeInTheDocument();
    });

    it("navigates to profile on View Profile click", async () => {
      setupWithResults([makeSearchResult({ identityId: IDENTITY_B })]);
      await renderScreen();
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: /view profile/i }));
      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({
          to: "/dashpay/contact-profile/$contactId",
          params: { contactId: IDENTITY_B },
        }),
      );
    });

    it("navigates to add contact on Add Contact click", async () => {
      setupWithResults([makeSearchResult({ identityId: IDENTITY_B })]);
      await renderScreen();
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: /add contact/i }));
      expect(mockNavigate).toHaveBeenCalledWith(
        expect.objectContaining({
          to: "/dashpay/add-contact",
        }),
      );
    });
  });

  // ── No results state ─────────────────────────────────────────

  describe("no results state", () => {
    it("shows no results message after search with no results", async () => {
      setupWithIdentity();
      await renderScreen();
      const user = userEvent.setup();
      const input = screen.getByPlaceholderText("Enter DPNS username...");
      await user.type(input, "nonexistent{Enter}");
      // Simulate search completing with no results
      useDashPayStore.setState({ searchResults: [], searchLoading: false });
      // Re-render to pick up state change
      await renderScreen();
      expect(screen.getByText("No Users Found")).toBeInTheDocument();
    });

    it("does not show no results before search", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(screen.queryByText("No Users Found")).not.toBeInTheDocument();
    });
  });

  // ── Info popup ───────────────────────────────────────────────

  describe("info popup", () => {
    it("opens info dialog on info button click", async () => {
      setupWithIdentity();
      await renderScreen();
      const user = userEvent.setup();
      await user.click(
        screen.getByRole("button", { name: /about profile search/i }),
      );
      expect(
        screen.getByText("About Profile Search"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/search for users by their dpns username/i),
      ).toBeInTheDocument();
    });

    it("shows all info text paragraphs", async () => {
      setupWithIdentity();
      await renderScreen();
      const user = userEvent.setup();
      await user.click(
        screen.getByRole("button", { name: /about profile search/i }),
      );
      expect(
        screen.getByText("Usernames are unique, verified identifiers on Dash Platform."),
      ).toBeInTheDocument();
      expect(
        screen.getByText("Add contacts directly from search results."),
      ).toBeInTheDocument();
    });
  });

  // ── Accessibility ────────────────────────────────────────────

  describe("accessibility", () => {
    it("search input has accessible label", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(
        screen.getByRole("textbox", { name: /dpns username search/i }),
      ).toBeInTheDocument();
    });

    it("info button has accessible label", async () => {
      setupWithIdentity();
      await renderScreen();
      expect(
        screen.getByRole("button", { name: /about profile search/i }),
      ).toBeInTheDocument();
    });

    it("error display uses alert role", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_A,
        searchError: "Error occurred",
      });
      await renderScreen();
      const alert = screen.getByRole("alert");
      expect(alert).toHaveTextContent("Error occurred");
    });
  });
});
