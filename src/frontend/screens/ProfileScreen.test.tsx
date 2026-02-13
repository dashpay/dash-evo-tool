import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ProfileScreen } from "./ProfileScreen";
import { useIdentityStore } from "@/stores/identityStore";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useWalletStore } from "@/stores/walletStore";
import { renderWithProviders } from "@/test/router-utils";
import type { QualifiedIdentityDto, StoredProfileDto } from "@/bindings";

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

// ─── Test Fixtures ────────────────────────────────────────────────

function makeIdentity(
  overrides: Partial<QualifiedIdentityDto> = {},
): QualifiedIdentityDto {
  return {
    id: "aa".repeat(32),
    identityType: "user",
    alias: "Alice",
    balance: 5000000000, // 50 DASH in credits
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

function makeProfile(
  overrides: Partial<StoredProfileDto> = {},
): StoredProfileDto {
  return {
    identityId: "aa".repeat(32),
    displayName: "Alice",
    bio: "Dash developer and enthusiast",
    avatarUrl: "https://example.com/alice-avatar.png",
    publicMessage: null,
    createdAt: 1707500000,
    updatedAt: 1707500000,
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

function setupWithIdentityAndProfile(
  identity = makeIdentity(),
  profile: StoredProfileDto | null = makeProfile(),
) {
  useIdentityStore.setState({ identities: [identity], loading: false });
  useDashPayStore.setState({
    selectedIdentityId: identity.id,
    profile,
    profileLoading: false,
    profileSaving: false,
    profileError: null,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
});

// ─── Tests ────────────────────────────────────────────────────────

describe("ProfileScreen", () => {
  describe("no identity selected", () => {
    it("shows empty state when no identity is selected", () => {
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
    });

    it("shows description about selecting an identity", () => {
      renderWithProviders(<ProfileScreen />);
      expect(
        screen.getByText(/Select an identity from the sidebar/),
      ).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner while profile is loading", () => {
      useDashPayStore.setState({
        selectedIdentityId: "aa".repeat(32),
        profileLoading: true,
      });
      useIdentityStore.setState({
        identities: [makeIdentity()],
        loading: false,
      });
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("Loading profile...")).toBeInTheDocument();
    });
  });

  describe("no profile state", () => {
    it("shows empty state when identity has no profile", () => {
      setupWithIdentityAndProfile(makeIdentity(), null);
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("No DashPay Profile")).toBeInTheDocument();
    });

    it('shows "Create Profile" button when no profile exists', () => {
      setupWithIdentityAndProfile(makeIdentity(), null);
      renderWithProviders(<ProfileScreen />);
      expect(
        screen.getByRole("button", { name: "Create Profile" }),
      ).toBeInTheDocument();
    });

    it("shows empty state when all profile fields are null", () => {
      setupWithIdentityAndProfile(
        makeIdentity(),
        makeProfile({
          displayName: null,
          bio: null,
          avatarUrl: null,
        }),
      );
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("No DashPay Profile")).toBeInTheDocument();
    });

    it("enters edit mode when Create Profile is clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile(makeIdentity(), null);
      renderWithProviders(<ProfileScreen />);
      await user.click(
        screen.getByRole("button", { name: "Create Profile" }),
      );
      expect(screen.getByText("Create Profile")).toBeInTheDocument();
      expect(screen.getByLabelText(/Display Name/)).toBeInTheDocument();
    });
  });

  describe("view mode — profile exists", () => {
    it("renders profile display name", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("Alice")).toBeInTheDocument();
    });

    it("renders heading", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("My DashPay Profile")).toBeInTheDocument();
    });

    it("renders Edit Profile button", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      expect(
        screen.getByRole("button", { name: /Edit Profile/ }),
      ).toBeInTheDocument();
    });

    it("renders DPNS name", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("@alice.dash")).toBeInTheDocument();
    });

    it("renders identity ID", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("aa".repeat(32))).toBeInTheDocument();
    });

    it("renders bio text", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      expect(
        screen.getByText("Dash developer and enthusiast"),
      ).toBeInTheDocument();
    });

    it("shows 'No bio set' when bio is empty", () => {
      setupWithIdentityAndProfile(makeIdentity(), makeProfile({ bio: null }));
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("No bio set")).toBeInTheDocument();
    });

    it("renders balance badge", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      // balance: 5,000,000,000 credits / 100B CREDITS_PER_DASH = 0.05 DASH
      expect(screen.getByText(/0\.05000000 DASH/)).toBeInTheDocument();
    });

    it("renders avatar image when URL is provided", () => {
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      const img = screen.getByAltText("Alice's avatar");
      expect(img).toBeInTheDocument();
      expect(img).toHaveAttribute(
        "src",
        "https://example.com/alice-avatar.png",
      );
    });

    it("renders User icon when no avatar URL", () => {
      setupWithIdentityAndProfile(
        makeIdentity(),
        makeProfile({ avatarUrl: null }),
      );
      renderWithProviders(<ProfileScreen />);
      expect(screen.queryByRole("img")).not.toBeInTheDocument();
    });

    it("renders profile error from store", () => {
      setupWithIdentityAndProfile();
      useDashPayStore.setState({ profileError: "Failed to load profile" });
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText("Failed to load profile")).toBeInTheDocument();
    });

    it("enters edit mode when Edit Profile is clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      await user.click(
        screen.getByRole("button", { name: /Edit Profile/ }),
      );
      expect(screen.getByText("Edit Profile")).toBeInTheDocument();
      expect(screen.getByLabelText(/Display Name/)).toBeInTheDocument();
    });
  });

  describe("edit mode", () => {
    function enterEditMode(
      profile: StoredProfileDto | null = makeProfile(),
    ) {
      setupWithIdentityAndProfile(makeIdentity(), profile);
      renderWithProviders(<ProfileScreen />);
      const btn = profile
        ? screen.getByRole("button", { name: /Edit Profile/ })
        : screen.getByRole("button", { name: "Create Profile" });
      return userEvent.setup().click(btn);
    }

    it("shows Edit Profile heading for existing profile", async () => {
      await enterEditMode();
      expect(screen.getByText("Edit Profile")).toBeInTheDocument();
    });

    it("shows Create Profile heading for new profile", async () => {
      await enterEditMode(null);
      expect(screen.getByText("Create Profile")).toBeInTheDocument();
    });

    it("populates fields from existing profile", async () => {
      await enterEditMode();
      expect(screen.getByLabelText(/Display Name/)).toHaveValue("Alice");
      expect(screen.getByLabelText(/Bio/)).toHaveValue(
        "Dash developer and enthusiast",
      );
      expect(screen.getByLabelText(/Avatar URL/)).toHaveValue(
        "https://example.com/alice-avatar.png",
      );
    });

    it("shows empty fields when creating new profile", async () => {
      await enterEditMode(null);
      expect(screen.getByLabelText(/Display Name/)).toHaveValue("");
      expect(screen.getByLabelText(/Bio/)).toHaveValue("");
      expect(screen.getByLabelText(/Avatar URL/)).toHaveValue("");
    });

    it("shows character counter for display name", async () => {
      await enterEditMode(null);
      expect(screen.getByText("0/25")).toBeInTheDocument();
    });

    it("shows character counter for bio", async () => {
      await enterEditMode(null);
      expect(screen.getByText("0/140")).toBeInTheDocument();
    });

    it("updates display name character counter on input", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      const input = screen.getByLabelText(/Display Name/);
      await user.type(input, "Test");
      expect(screen.getByText("4/25")).toBeInTheDocument();
    });

    it("shows avatar URL counter only when field has content", async () => {
      await enterEditMode(null);
      // No counter when empty
      expect(screen.queryByText(/\/500/)).not.toBeInTheDocument();
    });

    it("shows avatar URL counter when field has content", async () => {
      await enterEditMode();
      // Counter should be visible for existing URL
      expect(
        screen.getByText(
          `${"https://example.com/alice-avatar.png".length}/500`,
        ),
      ).toBeInTheDocument();
    });

    it("shows validation error for empty display name", async () => {
      await enterEditMode(null);
      // Error appears both inline and in summary
      const errors = screen.getAllByText("Display name is required");
      expect(errors.length).toBeGreaterThanOrEqual(1);
    });

    it("clears validation error when valid name is entered", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      expect(screen.getAllByText("Display name is required").length).toBeGreaterThanOrEqual(1);
      await user.type(screen.getByLabelText(/Display Name/), "Alice");
      expect(
        screen.queryByText("Display name is required"),
      ).not.toBeInTheDocument();
    });

    it("shows error for display name exceeding 25 chars", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      await user.type(
        screen.getByLabelText(/Display Name/),
        "a".repeat(26),
      );
      const errors = screen.getAllByText(/26 characters/);
      expect(errors.length).toBeGreaterThanOrEqual(1);
    });

    it("shows error for bio exceeding 140 chars", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      await user.type(screen.getByLabelText(/Display Name/), "Test");
      await user.type(
        screen.getByLabelText(/Bio/),
        "a".repeat(141),
      );
      const errors = screen.getAllByText(/141 characters/);
      expect(errors.length).toBeGreaterThanOrEqual(1);
    });

    it("shows error for invalid avatar URL", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      await user.type(screen.getByLabelText(/Display Name/), "Test");
      await user.type(
        screen.getByLabelText(/Avatar URL/),
        "not-a-url",
      );
      const errors = screen.getAllByText(/Must start with http/);
      expect(errors.length).toBeGreaterThanOrEqual(1);
    });

    it("accepts valid avatar URL", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      await user.type(screen.getByLabelText(/Display Name/), "Test");
      await user.type(
        screen.getByLabelText(/Avatar URL/),
        "https://example.com/avatar.png",
      );
      expect(
        screen.queryByText(/Must start with http/),
      ).not.toBeInTheDocument();
    });

    it("shows fee estimation", async () => {
      await enterEditMode();
      expect(screen.getByText(/Estimated fee/)).toBeInTheDocument();
    });

    it("shows identity balance", async () => {
      await enterEditMode();
      expect(screen.getByText(/Identity balance/)).toBeInTheDocument();
    });

    it("disables Save button when validation errors exist", async () => {
      await enterEditMode(null);
      const saveBtn = screen.getByRole("button", { name: /Save Profile/ });
      expect(saveBtn).toBeDisabled();
    });

    it("enables Save button when form is valid", async () => {
      const user = userEvent.setup();
      await enterEditMode(null);
      await user.type(screen.getByLabelText(/Display Name/), "Alice");
      const saveBtn = screen.getByRole("button", { name: /Save Profile/ });
      expect(saveBtn).toBeEnabled();
    });

    it("renders Cancel button", async () => {
      await enterEditMode();
      expect(
        screen.getByRole("button", { name: "Cancel" }),
      ).toBeInTheDocument();
    });

    it("cancels without dialog when no changes made", async () => {
      const user = userEvent.setup();
      await enterEditMode();
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      // Should be back in view mode
      expect(screen.getByText("My DashPay Profile")).toBeInTheDocument();
    });

    it("shows discard dialog when canceling with unsaved changes", async () => {
      const user = userEvent.setup();
      await enterEditMode();
      await user.clear(screen.getByLabelText(/Display Name/));
      await user.type(screen.getByLabelText(/Display Name/), "Changed");
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      expect(screen.getByText("Discard Changes?")).toBeInTheDocument();
    });

    it("discards changes when confirmed", async () => {
      const user = userEvent.setup();
      await enterEditMode();
      await user.clear(screen.getByLabelText(/Display Name/));
      await user.type(screen.getByLabelText(/Display Name/), "Changed");
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      await user.click(screen.getByRole("button", { name: "Discard" }));
      // Should be back in view mode
      expect(screen.getByText("My DashPay Profile")).toBeInTheDocument();
    });

    it("keeps editing when discard is cancelled", async () => {
      const user = userEvent.setup();
      await enterEditMode();
      await user.clear(screen.getByLabelText(/Display Name/));
      await user.type(screen.getByLabelText(/Display Name/), "Changed");
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      await user.click(screen.getByRole("button", { name: "Keep Editing" }));
      // Should still be in edit mode
      expect(screen.getByLabelText(/Display Name/)).toBeInTheDocument();
    });

    it("calls updateProfile when Save is clicked", async () => {
      const user = userEvent.setup();
      const updateProfile = vi.fn();
      await enterEditMode(null);
      useDashPayStore.setState({ updateProfile });
      await user.type(screen.getByLabelText(/Display Name/), "NewName");
      await user.click(
        screen.getByRole("button", { name: /Save Profile/ }),
      );
      expect(updateProfile).toHaveBeenCalledWith({
        displayName: "NewName",
        bio: null,
        avatarUrl: null,
      });
    });

    it("shows saving state in view mode", () => {
      setupWithIdentityAndProfile();
      useDashPayStore.setState({ profileSaving: true });
      renderWithProviders(<ProfileScreen />);
      expect(screen.getByText(/Saving profile/)).toBeInTheDocument();
    });

    it("shows guidelines sheet when info button is clicked", async () => {
      const user = userEvent.setup();
      await enterEditMode();
      const infoBtn = screen.getByLabelText("Profile guidelines");
      await user.click(infoBtn);
      expect(screen.getByText("Profile Guidelines")).toBeInTheDocument();
    });

    it("shows avatar guidelines sheet when avatar info button is clicked", async () => {
      const user = userEvent.setup();
      await enterEditMode();
      const infoBtn = screen.getByLabelText("Avatar guidelines");
      await user.click(infoBtn);
      expect(
        screen.getByText("Avatar Image Guidelines"),
      ).toBeInTheDocument();
    });
  });

  describe("success screen", () => {
    it("shows success screen after save completes for new profile", async () => {
      const user = userEvent.setup();
      // Mock updateProfile to simulate profileSaving transitions
      const mockUpdate = vi.fn().mockImplementation(async () => {
        // Simulate: store sets profileSaving=true then later false
        useDashPayStore.setState({ profileSaving: true });
        // After a tick, complete
        await Promise.resolve();
        useDashPayStore.setState({ profileSaving: false, profileError: null });
      });
      setupWithIdentityAndProfile(makeIdentity(), null);
      useDashPayStore.setState({ updateProfile: mockUpdate });
      renderWithProviders(<ProfileScreen />);

      // Enter edit mode
      await user.click(
        screen.getByRole("button", { name: "Create Profile" }),
      );
      await user.type(screen.getByLabelText(/Display Name/), "Alice");

      // Click save
      await user.click(
        screen.getByRole("button", { name: /Save Profile/ }),
      );

      // The success screen should show after save completes
      await waitFor(() => {
        expect(
          screen.getByText(/Created Successfully/),
        ).toBeInTheDocument();
      });
    });

    it("shows View Profile button on success screen", async () => {
      const user = userEvent.setup();
      const mockUpdate = vi.fn().mockImplementation(async () => {
        useDashPayStore.setState({ profileSaving: true });
        await Promise.resolve();
        useDashPayStore.setState({ profileSaving: false, profileError: null });
      });
      setupWithIdentityAndProfile(makeIdentity(), makeProfile());
      useDashPayStore.setState({ updateProfile: mockUpdate });
      renderWithProviders(<ProfileScreen />);

      // Enter edit mode and change something
      await user.click(
        screen.getByRole("button", { name: /Edit Profile/ }),
      );
      await user.clear(screen.getByLabelText(/Display Name/));
      await user.type(screen.getByLabelText(/Display Name/), "Updated");

      // Save
      await user.click(
        screen.getByRole("button", { name: /Save Profile/ }),
      );

      // Success screen should have View Profile button
      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: "View Profile" }),
        ).toBeInTheDocument();
      });
    });

    it("returns to view mode when View Profile is clicked", async () => {
      const user = userEvent.setup();
      const mockUpdate = vi.fn().mockImplementation(async () => {
        useDashPayStore.setState({ profileSaving: true });
        await Promise.resolve();
        useDashPayStore.setState({
          profileSaving: false,
          profileError: null,
          profile: makeProfile(),
        });
      });
      setupWithIdentityAndProfile(makeIdentity(), null);
      useDashPayStore.setState({ updateProfile: mockUpdate });
      renderWithProviders(<ProfileScreen />);

      // Create profile flow
      await user.click(
        screen.getByRole("button", { name: "Create Profile" }),
      );
      await user.type(screen.getByLabelText(/Display Name/), "Alice");
      await user.click(
        screen.getByRole("button", { name: /Save Profile/ }),
      );

      // Wait for success screen
      await waitFor(() => {
        expect(
          screen.getByRole("button", { name: "View Profile" }),
        ).toBeInTheDocument();
      });

      // Click View Profile
      await user.click(
        screen.getByRole("button", { name: "View Profile" }),
      );

      // Should be back in view mode
      await waitFor(() => {
        expect(screen.getByText("My DashPay Profile")).toBeInTheDocument();
      });
    });
  });

  describe("identity change", () => {
    it("resets to view mode when identity changes", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);

      // Enter edit mode
      await user.click(
        screen.getByRole("button", { name: /Edit Profile/ }),
      );
      expect(screen.getByLabelText(/Display Name/)).toBeInTheDocument();

      // Change identity
      useDashPayStore.setState({
        selectedIdentityId: "bb".repeat(32),
        profile: null,
      });
      useIdentityStore.setState({
        identities: [
          makeIdentity({ id: "bb".repeat(32), alias: "Bob" }),
        ],
      });

      renderWithProviders(<ProfileScreen />);

      // Should be reset - no longer in edit mode
      await waitFor(() => {
        expect(
          screen.queryByLabelText(/Display Name/),
        ).not.toBeInTheDocument();
      });
    });
  });

  describe("avatar dialog", () => {
    it("opens avatar dialog when avatar is clicked", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      const avatarBtn = screen.getByLabelText("View avatar");
      await user.click(avatarBtn);
      expect(screen.getByText("Profile Avatar")).toBeInTheDocument();
    });

    it("shows avatar URL in dialog", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      await user.click(screen.getByLabelText("View avatar"));
      expect(
        screen.getByText("https://example.com/alice-avatar.png"),
      ).toBeInTheDocument();
    });

    it("shows copy URL button in dialog", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile();
      renderWithProviders(<ProfileScreen />);
      await user.click(screen.getByLabelText("View avatar"));
      expect(
        screen.getByRole("button", { name: /Copy URL/ }),
      ).toBeInTheDocument();
    });

    it("disables avatar button when no URL", () => {
      setupWithIdentityAndProfile(
        makeIdentity(),
        makeProfile({ avatarUrl: null }),
      );
      renderWithProviders(<ProfileScreen />);
      const avatarBtn = screen.getByLabelText("No avatar set");
      expect(avatarBtn).toBeDisabled();
    });
  });

  describe("validation edge cases", () => {
    it("trims display name for validation", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile(makeIdentity(), null);
      renderWithProviders(<ProfileScreen />);
      await user.click(
        screen.getByRole("button", { name: "Create Profile" }),
      );
      // Type only spaces
      await user.type(screen.getByLabelText(/Display Name/), "   ");
      // Error appears both inline and in summary
      const errors = screen.getAllByText("Display name is required");
      expect(errors.length).toBeGreaterThanOrEqual(1);
    });

    it("shows error for avatar URL exceeding 500 chars", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile(makeIdentity(), null);
      renderWithProviders(<ProfileScreen />);
      await user.click(
        screen.getByRole("button", { name: "Create Profile" }),
      );
      await user.type(screen.getByLabelText(/Display Name/), "Test");
      await user.type(
        screen.getByLabelText(/Avatar URL/),
        "https://example.com/" + "a".repeat(490),
      );
      const errors = screen.getAllByText(/characters, must be 500 or less/);
      expect(errors.length).toBeGreaterThanOrEqual(1);
    });

    it("allows http:// URLs", async () => {
      const user = userEvent.setup();
      setupWithIdentityAndProfile(makeIdentity(), null);
      renderWithProviders(<ProfileScreen />);
      await user.click(
        screen.getByRole("button", { name: "Create Profile" }),
      );
      await user.type(screen.getByLabelText(/Display Name/), "Test");
      await user.type(
        screen.getByLabelText(/Avatar URL/),
        "http://example.com/avatar.png",
      );
      expect(
        screen.queryByText(/Must start with http/),
      ).not.toBeInTheDocument();
    });
  });
});
