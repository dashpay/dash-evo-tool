import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { DashPayScreen } from "./DashPayScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useDashPayStore } from "@/stores/dashpayStore";
import { renderWithProviders } from "@/test/router-utils";
import type { QualifiedIdentityDto } from "@/bindings";

// ─── Mock Tauri bindings ──────────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands } from "@/bindings";

// ─── Mock router ──────────────────────────────────────────────────

const mockNavigate = vi.fn();
let mockPathname = "/dashpay/profile";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  useRouterState: () => ({
    location: { pathname: mockPathname },
  }),
  Outlet: () => <div data-testid="outlet">Subscreen Content</div>,
}));

// ─── Mock sonner toast ────────────────────────────────────────────

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

// ─── Test Fixtures ────────────────────────────────────────────────

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Test Identity",
    balance: 1000000000,
    keys: [],
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
  useDashPayStore.getState().reset();
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
  mockPathname = "/dashpay/profile";
  vi.mocked(commands.identityListLocal).mockResolvedValue({
    status: "ok",
    data: [],
  });
  vi.mocked(commands.identityLoadOrder).mockResolvedValue({
    status: "ok",
    data: [],
  });
});

// ─── Tests ────────────────────────────────────────────────────────

describe("DashPayScreen", () => {
  describe("no-identities state", () => {
    it("shows no-identities empty state when no identities exist", async () => {
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByText("No Identities Loaded")).toBeInTheDocument();
      });
    });

    it("shows Load Identity action button", async () => {
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: "Load Identity" }),
        ).toBeInTheDocument();
      });
    });

    it("navigates to identities screen when Load Identity is clicked", async () => {
      const user = userEvent.setup();
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByText("No Identities Loaded")).toBeInTheDocument();
      });
      await user.click(screen.getByRole("button", { name: "Load Identity" }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/identities" });
    });

    it("does not render sidebar navigation in no-identities state", async () => {
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByText("No Identities Loaded")).toBeInTheDocument();
      });
      expect(screen.queryByText("My Profile")).not.toBeInTheDocument();
      expect(screen.queryByText("Contacts")).not.toBeInTheDocument();
    });
  });

  describe("with identities", () => {
    const identity1 = makeIdentity({ alias: "Alice", id: "aa".repeat(32) });
    const identity2 = makeIdentity({
      alias: "Bob",
      id: "bb".repeat(32),
    });

    function setupWithIdentities(identities: QualifiedIdentityDto[]) {
      // Mock the command so loadIdentities keeps the identities
      vi.mocked(commands.identityListLocal).mockResolvedValue({
        status: "ok",
        data: identities,
      });
      useIdentityStore.setState({
        identities,
        loading: false,
      });
    }

    it("renders sidebar navigation tabs", async () => {
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      expect(screen.getByText("My Profile")).toBeInTheDocument();
      expect(screen.getByText("Contacts")).toBeInTheDocument();
      expect(screen.getByText("Payment History")).toBeInTheDocument();
      expect(screen.getByText("Search Profiles")).toBeInTheDocument();
    });

    it("renders the outlet for subscreen content", () => {
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      expect(screen.getByTestId("outlet")).toBeInTheDocument();
    });

    it("highlights the active tab based on pathname", () => {
      mockPathname = "/dashpay/contacts";
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      const contactsBtn = screen.getByRole("button", { name: "Contacts" });
      expect(contactsBtn).toHaveAttribute("aria-current", "page");
    });

    it("does not highlight inactive tabs", () => {
      mockPathname = "/dashpay/profile";
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      const contactsBtn = screen.getByRole("button", { name: "Contacts" });
      expect(contactsBtn).not.toHaveAttribute("aria-current", "page");
    });

    it("navigates to contacts when Contacts tab is clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByRole("button", { name: "Contacts" })).toBeInTheDocument();
      });
      mockNavigate.mockClear();
      await user.click(screen.getByRole("button", { name: "Contacts" }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/contacts" });
    });

    it("navigates to profile when My Profile tab is clicked", async () => {
      mockPathname = "/dashpay/contacts"; // Start on contacts so profile click is meaningful
      const user = userEvent.setup();
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByRole("button", { name: "My Profile" })).toBeInTheDocument();
      });
      mockNavigate.mockClear();
      await user.click(screen.getByRole("button", { name: "My Profile" }));
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/profile" });
    });

    it("navigates to payments when Payment History tab is clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByRole("button", { name: "Payment History" })).toBeInTheDocument();
      });
      mockNavigate.mockClear();
      await user.click(
        screen.getByRole("button", { name: "Payment History" }),
      );
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/payments" });
    });

    it("navigates to search when Search Profiles tab is clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      await waitFor(() => {
        expect(screen.getByRole("button", { name: "Search Profiles" })).toBeInTheDocument();
      });
      mockNavigate.mockClear();
      await user.click(
        screen.getByRole("button", { name: "Search Profiles" }),
      );
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashpay/search" });
    });

    it("renders the identity selector", () => {
      setupWithIdentities([identity1]);
      renderWithProviders(<DashPayScreen />);
      expect(screen.getByRole("combobox")).toBeInTheDocument();
    });

    it("auto-selects first identity", () => {
      setupWithIdentities([identity1, identity2]);
      renderWithProviders(<DashPayScreen />);
      expect(useDashPayStore.getState().selectedIdentityId).toBe(identity1.id);
    });
  });

  describe("redirect behavior", () => {
    it("redirects /dashpay to /dashpay/profile", () => {
      mockPathname = "/dashpay";
      useIdentityStore.setState({ identities: [makeIdentity()], loading: false });
      renderWithProviders(<DashPayScreen />);
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/dashpay/profile",
        replace: true,
      });
    });

    it("redirects /dashpay/ to /dashpay/profile", () => {
      mockPathname = "/dashpay/";
      useIdentityStore.setState({ identities: [makeIdentity()], loading: false });
      renderWithProviders(<DashPayScreen />);
      expect(mockNavigate).toHaveBeenCalledWith({
        to: "/dashpay/profile",
        replace: true,
      });
    });
  });

  describe("sidebar layout", () => {
    it("wraps sidebar in an Island", () => {
      useIdentityStore.setState({
        identities: [makeIdentity()],
        loading: false,
      });
      const { container } = renderWithProviders(<DashPayScreen />);
      const islands = container.querySelectorAll(".island");
      expect(islands.length).toBeGreaterThanOrEqual(1);
    });

    it("has a navigation landmark with proper label", () => {
      useIdentityStore.setState({
        identities: [makeIdentity()],
        loading: false,
      });
      renderWithProviders(<DashPayScreen />);
      expect(
        screen.getByRole("navigation", { name: "DashPay sections" }),
      ).toBeInTheDocument();
    });
  });
});
