import { screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ContactInfoEditorScreen } from "./ContactInfoEditorScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
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

import { events } from "@/bindings";

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  Toaster: () => null,
}));

function fireTaskResult(payload: unknown) {
  const listeners = vi.mocked(events.taskResultEvent.listen).mock.calls;
  for (const [cb] of listeners) {
    cb?.({ payload } as { payload: unknown });
  }
}

// ─── Test Fixtures ────────────────────────────────────────────────

const CONTACT_ID = "bb".repeat(32);
const IDENTITY_ID = "aa".repeat(32);

function makeContact(
  overrides: Partial<StoredContactDto> = {},
): StoredContactDto {
  return {
    ownerIdentityId: IDENTITY_ID,
    contactIdentityId: CONTACT_ID,
    username: "bob",
    displayName: "Bob Builder",
    avatarUrl: "https://example.com/bob.png",
    publicMessage: "I love building things",
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
    nickname: "Bobby",
    notes: "Good friend",
    isHidden: false,
    ...overrides,
  };
}

// ─── Helpers ──────────────────────────────────────────────────────

function resetStores() {
  useDashPayStore.getState().reset();
}

function setupWithContact(
  contact: StoredContactDto = makeContact(),
  privateInfo: ContactPrivateInfoDto | null = null,
) {
  useDashPayStore.setState({
    selectedIdentityId: IDENTITY_ID,
    contacts: [contact],
    contactsError: null,
  });

  const store = useDashPayStore.getState();
  vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(privateInfo);
  vi.spyOn(store, "saveContactPrivateInfo").mockResolvedValue();
  vi.spyOn(store, "updateContactInfo").mockResolvedValue();
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
});

// ─── Tests ────────────────────────────────────────────────────────

describe("ContactInfoEditorScreen", () => {
  describe("no identity selected", () => {
    it("shows empty state when no identity is selected", () => {
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
    });

    it("shows description about selecting an identity", () => {
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );
      expect(
        screen.getByText(/Select an identity.*to edit contact details/),
      ).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner while data is loading", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [],
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockReturnValue(
        new Promise(() => {}),
      );

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );
      expect(
        screen.getByText("Loading contact info..."),
      ).toBeInTheDocument();
    });
  });

  describe("header and layout", () => {
    it("renders header with title and back button", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText("Edit Private Contact Details"),
        ).toBeInTheDocument();
      });
      expect(screen.getByText("Back")).toBeInTheDocument();
    });

    it("renders info tooltip button", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("About private contact information"),
        ).toBeInTheDocument();
      });
    });

    it("shows contact identifier with display name", async () => {
      setupWithContact(makeContact({ displayName: "Bob Builder" }));
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Bob Builder")).toBeInTheDocument();
      });
      expect(screen.getByText("Contact:")).toBeInTheDocument();
    });

    it("shows contact username when no display name", async () => {
      setupWithContact(
        makeContact({ displayName: "", username: "alice" }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("@alice")).toBeInTheDocument();
      });
    });

    it("shows contact ID when no display name or username", async () => {
      setupWithContact(
        makeContact({ displayName: "", username: "" }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        // Contact ID appears twice: once as the display name, once as the monospace ID
        const elements = screen.getAllByText(CONTACT_ID);
        expect(elements.length).toBeGreaterThanOrEqual(2);
      });
    });

    it("displays contact ID in monospace", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        const idElements = screen.getAllByText(CONTACT_ID);
        const monoElement = idElements.find(
          (el) => el.className.includes("font-mono"),
        );
        expect(monoElement).toBeDefined();
      });
    });
  });

  describe("form fields", () => {
    it("renders nickname field", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByLabelText("Private Nickname")).toBeInTheDocument();
      });
    });

    it("renders note field", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByLabelText("Private Note")).toBeInTheDocument();
      });
    });

    it("renders hidden checkbox", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Hide this contact from my list"),
        ).toBeInTheDocument();
      });
    });

    it("renders account indices field with Parse button", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });
      expect(screen.getByText("Parse")).toBeInTheDocument();
    });

    it("shows nickname description text", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText(/custom name that ONLY YOU will see/),
        ).toBeInTheDocument();
      });
    });

    it("shows note description text", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText(/notes about this contact.*only visible to you/),
        ).toBeInTheDocument();
      });
    });

    it("shows account indices description text", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText(/which account indices this contact can pay to/),
        ).toBeInTheDocument();
      });
    });
  });

  describe("loading existing data", () => {
    it("pre-fills form with existing private info", async () => {
      setupWithContact(
        makeContact(),
        makePrivateInfo({ nickname: "Bobby", notes: "Good friend", isHidden: false }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByLabelText("Private Nickname")).toHaveValue("Bobby");
      });
      expect(screen.getByLabelText("Private Note")).toHaveValue("Good friend");
    });

    it("pre-fills hidden checkbox when contact is hidden", async () => {
      setupWithContact(
        makeContact(),
        makePrivateInfo({ isHidden: true }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        const checkbox = screen.getByRole("checkbox", {
          name: "Hide this contact from my list",
        });
        expect(checkbox).toHaveAttribute("data-state", "checked");
      });
    });

    it("shows empty fields when no private info exists", async () => {
      setupWithContact(makeContact(), null);
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByLabelText("Private Nickname")).toHaveValue("");
      });
      expect(screen.getByLabelText("Private Note")).toHaveValue("");
    });
  });

  describe("hidden checkbox behavior", () => {
    it("shows warning text when hidden is checked", async () => {
      setupWithContact(
        makeContact(),
        makePrivateInfo({ isHidden: true }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText(
            /Hidden contacts won't appear.*can still send you payments/,
          ),
        ).toBeInTheDocument();
      });
    });

    it("shows default text when not hidden", async () => {
      setupWithContact(makeContact(), makePrivateInfo({ isHidden: false }));
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText("Contact will appear in your contact list"),
        ).toBeInTheDocument();
      });
    });

    it("toggles warning text when checkbox changes", async () => {
      const user = userEvent.setup();
      setupWithContact(makeContact(), makePrivateInfo({ isHidden: false }));
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText("Contact will appear in your contact list"),
        ).toBeInTheDocument();
      });

      const checkbox = screen.getByRole("checkbox", {
        name: "Hide this contact from my list",
      });
      await user.click(checkbox);

      expect(
        screen.getByText(
          /Hidden contacts won't appear.*can still send you payments/,
        ),
      ).toBeInTheDocument();
    });
  });

  describe("account indices parsing", () => {
    it("parses comma-separated account indices", async () => {
      const user = userEvent.setup();
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });

      const input = screen.getByLabelText("Accepted Account Indices");
      await user.type(input, "2, 0, 1");
      await user.click(screen.getByText("Parse"));

      // Should be sorted and deduplicated
      expect(input).toHaveValue("0, 1, 2");
      expect(screen.getByText("Accepted accounts: [0, 1, 2]")).toBeInTheDocument();
    });

    it("shows default text when no accounts specified", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText("All accounts accepted (default)"),
        ).toBeInTheDocument();
      });
    });

    it("ignores invalid account input", async () => {
      const user = userEvent.setup();
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });

      const input = screen.getByLabelText("Accepted Account Indices");
      await user.type(input, "abc, 1, xyz");
      await user.click(screen.getByText("Parse"));

      expect(input).toHaveValue("1");
      expect(screen.getByText("Accepted accounts: [1]")).toBeInTheDocument();
    });

    it("removes duplicate account indices", async () => {
      const user = userEvent.setup();
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });

      const input = screen.getByLabelText("Accepted Account Indices");
      await user.type(input, "1, 2, 1, 3, 2");
      await user.click(screen.getByText("Parse"));

      expect(input).toHaveValue("1, 2, 3");
    });

    it("ignores negative numbers", async () => {
      const user = userEvent.setup();
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });

      const input = screen.getByLabelText("Accepted Account Indices");
      await user.type(input, "-1, 0, 3");
      await user.click(screen.getByText("Parse"));

      expect(input).toHaveValue("0, 3");
    });
  });

  describe("save flow", () => {
    it("renders Save Changes and Cancel buttons", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });
      expect(screen.getByText("Cancel")).toBeInTheDocument();
    });

    it("calls saveContactPrivateInfo and updateContactInfo on save", async () => {
      const user = userEvent.setup();
      setupWithContact(
        makeContact(),
        makePrivateInfo({ nickname: "", notes: "", isHidden: false }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByLabelText("Private Nickname")).toBeInTheDocument();
      });

      // Fill in form
      await user.type(screen.getByLabelText("Private Nickname"), "My Friend");
      await user.type(screen.getByLabelText("Private Note"), "Met at conference");

      // Click save
      await user.click(screen.getByText("Save Changes"));

      const store = useDashPayStore.getState();
      await waitFor(() => {
        expect(store.saveContactPrivateInfo).toHaveBeenCalledWith({
          contactId: CONTACT_ID,
          nickname: "My Friend",
          notes: "Met at conference",
          isHidden: false,
        });
      });

      expect(store.updateContactInfo).toHaveBeenCalledWith({
        contactId: CONTACT_ID,
        nickname: "My Friend",
        note: "Met at conference",
        isHidden: false,
        acceptedAccounts: [],
      });
    });

    it("sends empty strings as null for nickname/note on Platform update", async () => {
      const user = userEvent.setup();
      setupWithContact(
        makeContact(),
        makePrivateInfo({ nickname: "", notes: "", isHidden: false }),
      );
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });

      // Save with empty fields
      await user.click(screen.getByText("Save Changes"));

      const store = useDashPayStore.getState();
      await waitFor(() => {
        expect(store.updateContactInfo).toHaveBeenCalledWith(
          expect.objectContaining({
            nickname: null,
            note: null,
          }),
        );
      });
    });

    it("shows success message after save", async () => {
      const user = userEvent.setup();
      setupWithContact(makeContact(), makePrivateInfo());

      // Mock updateContactInfo to return a task ID
      const store = useDashPayStore.getState();
      vi.spyOn(store, "updateContactInfo").mockResolvedValue("task-save-1");

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Save Changes"));

      // Simulate task completion event
      await act(async () => {
        fireTaskResult({
          taskId: "task-save-1",
          result: { type: "dashPayCompleted" },
        });
      });

      await waitFor(() => {
        expect(
          screen.getByText("Contact information updated successfully"),
        ).toBeInTheDocument();
      });
    });

    it("shows saving spinner during save", async () => {
      const user = userEvent.setup();
      let resolveSave: () => void;
      const savePromise = new Promise<void>((resolve) => {
        resolveSave = resolve;
      });

      setupWithContact(makeContact(), makePrivateInfo());
      const store = useDashPayStore.getState();
      vi.spyOn(store, "saveContactPrivateInfo").mockReturnValue(savePromise);

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Save Changes"));

      expect(screen.getByText("Saving...")).toBeInTheDocument();

      // Resolve to clean up
      resolveSave!();
      await waitFor(() => {
        expect(screen.queryByText("Saving...")).not.toBeInTheDocument();
      });
    });

    it("shows error message when save fails", async () => {
      const user = userEvent.setup();
      setupWithContact(makeContact(), makePrivateInfo());
      const store = useDashPayStore.getState();
      vi.spyOn(store, "saveContactPrivateInfo").mockRejectedValue(
        new Error("Network error"),
      );

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Save Changes"));

      await waitFor(() => {
        expect(screen.getByText("Network error")).toBeInTheDocument();
      });
    });

    it("includes accepted accounts in Platform update", async () => {
      const user = userEvent.setup();
      setupWithContact(makeContact(), makePrivateInfo());
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });

      // Parse some accounts
      const input = screen.getByLabelText("Accepted Account Indices");
      await user.type(input, "0, 2");
      await user.click(screen.getByText("Parse"));

      // Save
      await user.click(screen.getByText("Save Changes"));

      const store = useDashPayStore.getState();
      await waitFor(() => {
        expect(store.updateContactInfo).toHaveBeenCalledWith(
          expect.objectContaining({
            acceptedAccounts: [0, 2],
          }),
        );
      });
    });

    it("disables form fields while saving", async () => {
      const user = userEvent.setup();
      let resolveSave: () => void;
      const savePromise = new Promise<void>((resolve) => {
        resolveSave = resolve;
      });

      setupWithContact(makeContact(), makePrivateInfo());
      const store = useDashPayStore.getState();
      vi.spyOn(store, "saveContactPrivateInfo").mockReturnValue(savePromise);

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Save Changes"));

      expect(screen.getByLabelText("Private Nickname")).toBeDisabled();
      expect(screen.getByLabelText("Private Note")).toBeDisabled();
      expect(screen.getByLabelText("Accepted Account Indices")).toBeDisabled();

      // Resolve to clean up
      resolveSave!();
      await waitFor(() => {
        expect(screen.getByLabelText("Private Nickname")).not.toBeDisabled();
      });
    });
  });

  describe("store error display", () => {
    it("shows contactsError from store", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [makeContact()],
        contactsError: "Failed to update contact",
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(null);

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByText("Failed to update contact"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("info popup", () => {
    it("opens info dialog when info button is clicked", async () => {
      const user = userEvent.setup();
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(
          screen.getByLabelText("About private contact information"),
        ).toBeInTheDocument();
      });

      await user.click(
        screen.getByLabelText("About private contact information"),
      );

      expect(
        screen.getByText("About Private Contact Information"),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/NEVER shared with the contact/),
      ).toBeInTheDocument();
    });
  });

  describe("accessibility", () => {
    it("has proper labels for all form fields", async () => {
      setupWithContact();
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByLabelText("Private Nickname")).toBeInTheDocument();
        expect(screen.getByLabelText("Private Note")).toBeInTheDocument();
        expect(
          screen.getByLabelText("Hide this contact from my list"),
        ).toBeInTheDocument();
        expect(
          screen.getByLabelText("Accepted Account Indices"),
        ).toBeInTheDocument();
      });
    });

    it("has role=status on message display", async () => {
      const user = userEvent.setup();
      setupWithContact(makeContact(), makePrivateInfo());
      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByText("Save Changes")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Save Changes"));

      await waitFor(() => {
        const statusElement = screen.getByRole("status");
        expect(statusElement).toBeInTheDocument();
      });
    });

    it("has role=alert on store error", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [makeContact()],
        contactsError: "Something went wrong",
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(null);

      renderWithProviders(
        <ContactInfoEditorScreen contactId={CONTACT_ID} />,
      );

      await waitFor(() => {
        expect(screen.getByRole("alert")).toBeInTheDocument();
      });
    });
  });
});
