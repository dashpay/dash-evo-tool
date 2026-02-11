import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ContactProfileViewer } from "./ContactProfileViewer";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { renderWithProviders } from "@/test/router-utils";
import type {
  StoredContactDto,
  ContactPrivateInfoDto,
} from "@/bindings";

// ─── Mock Tauri bindings ──────────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

// ─── Test Fixtures ────────────────────────────────────────────────

const CONTACT_ID = "bb".repeat(32);
const IDENTITY_ID = "aa".repeat(32);

function makeContact(
  overrides: Partial<StoredContactDto> = {},
): StoredContactDto {
  return {
    ownerIdentityId: IDENTITY_ID,
    contactIdentityId: CONTACT_ID,
    username: "carol",
    displayName: "Carol Crypto",
    avatarUrl: "https://example.com/carol.png",
    publicMessage: "Crypto enthusiast and builder",
    contactStatus: "accepted",
    createdAt: 1707500000,
    updatedAt: 1707500000,
    lastSeen: null,
    ...overrides,
  };
}

function makePrivateInfo(
  overrides: Partial<ContactPrivateInfoDto> = {},
): ContactPrivateInfoDto {
  return {
    nickname: "",
    notes: "",
    isHidden: false,
    ...overrides,
  };
}

// ─── Helpers ──────────────────────────────────────────────────────

function resetStores() {
  useDashPayStore.getState().reset();
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
}

function setupWithProfile(
  contact: StoredContactDto | null = makeContact(),
  privateInfo: ContactPrivateInfoDto | null = null,
) {
  useDashPayStore.setState({
    selectedIdentityId: IDENTITY_ID,
    contacts: contact ? [contact] : [],
    contactsError: null,
  });

  const store = useDashPayStore.getState();
  vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(privateInfo);
  vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
  vi.spyOn(store, "saveContactPrivateInfo").mockResolvedValue();
  vi.spyOn(store, "updateContactInfo").mockResolvedValue();
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
});

// ─── Tests ────────────────────────────────────────────────────────

describe("ContactProfileViewer", () => {
  describe("no identity selected", () => {
    it("shows empty state when no identity is selected", () => {
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner while profile is loading", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [],
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockReturnValue(
        new Promise(() => {}),
      );
      vi.spyOn(store, "fetchContactProfile").mockReturnValue(
        new Promise(() => {}),
      );

      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);
      expect(
        screen.getByText("Loading public profile..."),
      ).toBeInTheDocument();
    });
  });

  describe("no profile found", () => {
    it("shows no profile message when contact has no profile", async () => {
      setupWithProfile(null);
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("No profile found")).toBeInTheDocument();
      });
    });

    it("shows explanation text", async () => {
      setupWithProfile(null);
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText(
            "This contact has not created a public profile yet.",
          ),
        ).toBeInTheDocument();
      });
    });

    it("shows Retry button", async () => {
      setupWithProfile(null);
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Retry")).toBeInTheDocument();
      });
    });

    it("shows Pay button even with no profile", async () => {
      setupWithProfile(null);
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Pay")).toBeInTheDocument();
      });
    });
  });

  describe("profile display", () => {
    it("displays contact display name", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Carol Crypto")).toBeInTheDocument();
      });
    });

    it("shows 'No display name set' when name is empty", async () => {
      setupWithProfile(makeContact({ displayName: null }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("No display name set"),
        ).toBeInTheDocument();
      });
    });

    it("displays identity ID", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText(`Identity: ${CONTACT_ID}`),
        ).toBeInTheDocument();
      });
    });

    it("shows public message label", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Public Message:")).toBeInTheDocument();
      });
    });

    it("displays public message content", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("Crypto enthusiast and builder"),
        ).toBeInTheDocument();
      });
    });

    it("shows 'No public message' when message is empty", async () => {
      setupWithProfile(makeContact({ publicMessage: null }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("No public message")).toBeInTheDocument();
      });
    });

    it("shows avatar image when URL is present", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        const avatar = screen.getByAltText("Carol Crypto avatar");
        expect(avatar).toBeInTheDocument();
        expect(avatar).toHaveAttribute(
          "src",
          "https://example.com/carol.png",
        );
      });
    });

    it("shows 'No avatar' label when no avatar URL", async () => {
      setupWithProfile(makeContact({ avatarUrl: null }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("No avatar")).toBeInTheDocument();
      });
    });

    it("shows avatar verification section when avatar URL exists", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("Avatar Verification"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("action buttons", () => {
    it("shows Refresh button", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Refresh")).toBeInTheDocument();
      });
    });

    it("shows Pay button", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Pay")).toBeInTheDocument();
      });
    });

    it("calls fetchContactProfile on Refresh click", async () => {
      const user = userEvent.setup();
      setupWithProfile();
      const store = useDashPayStore.getState();

      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Refresh")).toBeInTheDocument();
      });

      // Clear the initial call count
      vi.mocked(store.fetchContactProfile).mockClear();

      await user.click(screen.getByText("Refresh"));

      await waitFor(() => {
        expect(store.fetchContactProfile).toHaveBeenCalledWith(CONTACT_ID);
      });
    });
  });

  describe("header", () => {
    it("shows Public Profile heading", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Public Profile")).toBeInTheDocument();
      });
    });

    it("shows Back button", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Back")).toBeInTheDocument();
      });
    });

    it("shows info icon for public profile help", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByLabelText("About public profiles"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("private info section", () => {
    it("shows Private Contact Information heading", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("Private Contact Information"),
        ).toBeInTheDocument();
      });
    });

    it("shows nickname as Not set when empty", async () => {
      setupWithProfile(makeContact(), makePrivateInfo({ nickname: "" }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Not set")).toBeInTheDocument();
      });
    });

    it("shows notes as No notes when empty", async () => {
      setupWithProfile(makeContact(), makePrivateInfo({ notes: "" }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("No notes")).toBeInTheDocument();
      });
    });

    it("shows hidden status as No by default", async () => {
      setupWithProfile(makeContact(), makePrivateInfo({ isHidden: false }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        const hiddenField = screen.getByText("Hidden:");
        expect(hiddenField.parentElement).toHaveTextContent("No");
      });
    });

    it("shows hidden status as Yes when hidden", async () => {
      setupWithProfile(makeContact(), makePrivateInfo({ isHidden: true }));
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        const hiddenField = screen.getByText("Hidden:");
        expect(hiddenField.parentElement).toHaveTextContent("Yes");
      });
    });

    it("shows Edit button for private info", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });
    });

    it("enters edit mode on Edit click", async () => {
      const user = userEvent.setup();
      setupWithProfile(
        makeContact(),
        makePrivateInfo({ nickname: "MyFriend" }),
      );
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));

      expect(screen.getByLabelText("Nickname")).toBeInTheDocument();
      expect(screen.getByLabelText("Notes")).toBeInTheDocument();
      expect(
        screen.getByText("Hide this contact from the main list"),
      ).toBeInTheDocument();
    });

    it("populates edit fields with current values", async () => {
      const user = userEvent.setup();
      setupWithProfile(
        makeContact(),
        makePrivateInfo({ nickname: "MyFriend", notes: "Met at conference" }),
      );
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));

      expect(screen.getByLabelText("Nickname")).toHaveValue("MyFriend");
      expect(screen.getByLabelText("Notes")).toHaveValue(
        "Met at conference",
      );
    });

    it("cancels edit on Cancel click", async () => {
      const user = userEvent.setup();
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));
      expect(screen.getByLabelText("Nickname")).toBeInTheDocument();

      await user.click(screen.getByText("Cancel"));
      expect(screen.queryByLabelText("Nickname")).not.toBeInTheDocument();
    });

    it("saves private info on Save click", async () => {
      const user = userEvent.setup();
      setupWithProfile(makeContact(), makePrivateInfo());
      const store = useDashPayStore.getState();

      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));

      const nicknameInput = screen.getByLabelText("Nickname");
      await user.type(nicknameInput, "Crypto Carol");

      await user.click(screen.getByText("Save"));

      await waitFor(() => {
        expect(store.saveContactPrivateInfo).toHaveBeenCalledWith(
          expect.objectContaining({
            contactId: CONTACT_ID,
            nickname: "Crypto Carol",
          }),
        );
      });
    });

    it("shows info popup button for private info", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByLabelText("About private contact information"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("message display", () => {
    it("shows success message after refresh", async () => {
      const user = userEvent.setup();
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Refresh")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Refresh"));

      await waitFor(() => {
        expect(screen.getByText("Profile refreshed")).toBeInTheDocument();
      });
    });
  });

  describe("accessibility", () => {
    it("has proper alt text for avatar image", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByAltText("Carol Crypto avatar"),
        ).toBeInTheDocument();
      });
    });

    it("has aria-label on info buttons", async () => {
      setupWithProfile();
      renderWithProviders(<ContactProfileViewer contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByLabelText("About public profiles"),
        ).toBeInTheDocument();
        expect(
          screen.getByLabelText("About private contact information"),
        ).toBeInTheDocument();
      });
    });
  });
});
