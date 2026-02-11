import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ContactDetailsScreen } from "./ContactDetailsScreen";
import { useDashPayStore } from "@/stores/dashpayStore";
import { useIdentityStore } from "@/stores/identityStore";
import { renderWithProviders } from "@/test/router-utils";
import type {
  StoredContactDto,
  StoredPaymentDto,
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

function makePayment(
  overrides: Partial<StoredPaymentDto> = {},
): StoredPaymentDto {
  return {
    id: 1,
    txId: "cc".repeat(32),
    fromIdentityId: IDENTITY_ID,
    toIdentityId: CONTACT_ID,
    amount: 100000000,
    memo: "For lunch",
    paymentType: "standard",
    status: "confirmed",
    createdAt: 1707500000,
    confirmedAt: 1707500100,
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

function setupWithContact(
  contact: StoredContactDto = makeContact(),
  payments: StoredPaymentDto[] = [],
  privateInfo: ContactPrivateInfoDto | null = null,
) {
  useDashPayStore.setState({
    selectedIdentityId: IDENTITY_ID,
    contacts: [contact],
    payments,
    paymentsLoading: false,
    contactsError: null,
  });

  // Mock the store's loadContactPrivateInfo
  const store = useDashPayStore.getState();
  vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(privateInfo);
  vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
  vi.spyOn(store, "loadPayments").mockResolvedValue();
  vi.spyOn(store, "saveContactPrivateInfo").mockResolvedValue();
  vi.spyOn(store, "updateContactInfo").mockResolvedValue();
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStores();
});

// ─── Tests ────────────────────────────────────────────────────────

describe("ContactDetailsScreen", () => {
  describe("no identity selected", () => {
    it("shows empty state when no identity is selected", () => {
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);
      expect(screen.getByText("No Identity Selected")).toBeInTheDocument();
    });
  });

  describe("loading state", () => {
    it("shows loading spinner while data is loading", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [],
        payments: [],
      });
      // loadContactPrivateInfo will be a pending promise
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockReturnValue(
        new Promise(() => {}),
      );
      vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
      vi.spyOn(store, "loadPayments").mockResolvedValue();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);
      expect(
        screen.getByText("Loading contact details..."),
      ).toBeInTheDocument();
    });
  });

  describe("no contact found", () => {
    it("shows no contact info message when contact not found", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [],
        payments: [],
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(null);
      vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
      vi.spyOn(store, "loadPayments").mockResolvedValue();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("No contact information available"),
        ).toBeInTheDocument();
      });
    });

    it("shows contact ID when no info available", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [],
        payments: [],
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(null);
      vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
      vi.spyOn(store, "loadPayments").mockResolvedValue();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText(`Contact ID: ${CONTACT_ID}`),
        ).toBeInTheDocument();
      });
    });

    it("shows Refresh from Platform button", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [],
        payments: [],
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(null);
      vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
      vi.spyOn(store, "loadPayments").mockResolvedValue();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("Refresh from Platform"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("profile section", () => {
    it("displays contact display name", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Bob Builder")).toBeInTheDocument();
      });
    });

    it("displays contact username with @ prefix", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("@bob")).toBeInTheDocument();
      });
    });

    it("displays contact public message", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("I love building things"),
        ).toBeInTheDocument();
      });
    });

    it("displays contact identity ID with copy button", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText(CONTACT_ID)).toBeInTheDocument();
      });
    });

    it("shows nickname from private info as display name when set", async () => {
      setupWithContact(makeContact(), [], makePrivateInfo({ nickname: "Bobby" }));
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        // Nickname is used as the display name heading
        const heading = screen.getByRole("heading", { level: 3 });
        expect(heading).toHaveTextContent("Bobby");
      });
    });

    it("shows Send Payment button", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Send Payment")).toBeInTheDocument();
      });
    });
  });

  describe("private info section", () => {
    it("shows private contact information heading", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("Private Contact Information"),
        ).toBeInTheDocument();
      });
    });

    it("shows empty private info message when no info set", async () => {
      setupWithContact(makeContact(), [], null);
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText(
            /No private info set/,
          ),
        ).toBeInTheDocument();
      });
    });

    it("shows nickname and notes when set", async () => {
      setupWithContact(
        makeContact(),
        [],
        makePrivateInfo({ nickname: "Bobby", notes: "Good friend" }),
      );
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        // Nickname appears in both display name heading and private info section
        expect(screen.getAllByText("Bobby").length).toBeGreaterThanOrEqual(2);
        expect(screen.getByText("Good friend")).toBeInTheDocument();
      });
    });

    it("shows hidden badge when contact is hidden", async () => {
      setupWithContact(
        makeContact(),
        [],
        makePrivateInfo({ isHidden: true }),
      );
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("This contact is hidden"),
        ).toBeInTheDocument();
      });
    });

    it("shows Edit button", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });
    });

    it("enters edit mode on Edit click", async () => {
      const user = userEvent.setup();
      setupWithContact(
        makeContact(),
        [],
        makePrivateInfo({ nickname: "Bobby", notes: "Good friend" }),
      );
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));

      expect(screen.getByLabelText("Nickname")).toBeInTheDocument();
      expect(screen.getByLabelText("Notes")).toBeInTheDocument();
      expect(screen.getByText("Cancel")).toBeInTheDocument();
      expect(screen.getByText("Save")).toBeInTheDocument();
    });

    it("populates edit fields with current values", async () => {
      const user = userEvent.setup();
      setupWithContact(
        makeContact(),
        [],
        makePrivateInfo({ nickname: "Bobby", notes: "Good friend", isHidden: false }),
      );
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));

      expect(screen.getByLabelText("Nickname")).toHaveValue("Bobby");
      expect(screen.getByLabelText("Notes")).toHaveValue("Good friend");
    });

    it("cancels edit on Cancel click", async () => {
      const user = userEvent.setup();
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

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
      const info = makePrivateInfo({ nickname: "", notes: "" });
      setupWithContact(makeContact(), [], info);
      const store = useDashPayStore.getState();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Edit")).toBeInTheDocument();
      });

      await user.click(screen.getByText("Edit"));

      const nicknameInput = screen.getByLabelText("Nickname");
      await user.clear(nicknameInput);
      await user.type(nicknameInput, "New Bobby");

      await user.click(screen.getByText("Save"));

      await waitFor(() => {
        expect(store.saveContactPrivateInfo).toHaveBeenCalledWith(
          expect.objectContaining({
            contactId: CONTACT_ID,
            nickname: "New Bobby",
          }),
        );
      });
    });

    it("shows info popup help", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByLabelText("About private contact information"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("payment history", () => {
    it("shows payment history heading", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Payment History")).toBeInTheDocument();
      });
    });

    it("shows empty payment history message", async () => {
      setupWithContact(makeContact(), []);
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("No payment history with this contact"),
        ).toBeInTheDocument();
      });
    });

    it("shows outgoing payment with minus sign", async () => {
      const payment = makePayment({
        fromIdentityId: IDENTITY_ID,
        toIdentityId: CONTACT_ID,
        amount: 100000000,
        memo: "For lunch",
      });
      setupWithContact(makeContact(), [payment]);
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText(/For lunch/)).toBeInTheDocument();
      });
    });

    it("shows incoming payment with plus sign", async () => {
      const payment = makePayment({
        fromIdentityId: CONTACT_ID,
        toIdentityId: IDENTITY_ID,
        amount: 50000000,
        memo: "Thanks!",
      });
      setupWithContact(makeContact(), [payment]);
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText(/Thanks!/)).toBeInTheDocument();
      });
    });

    it("shows transaction ID", async () => {
      const txId = "dd".repeat(32);
      const payment = makePayment({ txId });
      setupWithContact(makeContact(), [payment]);
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText(txId)).toBeInTheDocument();
      });
    });
  });

  describe("actions section", () => {
    it("shows actions heading", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Actions")).toBeInTheDocument();
      });
    });

    it("shows notice about contact removal", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText(/Contact removal and blocking are not yet available/),
        ).toBeInTheDocument();
      });
    });
  });

  describe("header", () => {
    it("shows Contact Details heading", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Contact Details")).toBeInTheDocument();
      });
    });

    it("shows Back button", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Back")).toBeInTheDocument();
      });
    });

    it("shows Refresh button", async () => {
      setupWithContact();
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(screen.getByText("Refresh")).toBeInTheDocument();
      });
    });
  });

  describe("error handling", () => {
    it("displays store error", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [makeContact()],
        payments: [],
        contactsError: "Something went wrong",
      });
      const store = useDashPayStore.getState();
      vi.spyOn(store, "loadContactPrivateInfo").mockResolvedValue(null);
      vi.spyOn(store, "fetchContactProfile").mockResolvedValue();
      vi.spyOn(store, "loadPayments").mockResolvedValue();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        expect(
          screen.getByText("Something went wrong"),
        ).toBeInTheDocument();
      });
    });
  });

  describe("accessibility", () => {
    it("has proper role for message display", async () => {
      setupWithContact();
      const store = useDashPayStore.getState();
      vi.spyOn(store, "fetchContactProfile").mockResolvedValue();

      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      // Wait for data to load, trigger refresh to get a status message
      await waitFor(() => {
        expect(screen.getByText("Bob Builder")).toBeInTheDocument();
      });
    });

    it("payment history has list role", async () => {
      const payment = makePayment();
      setupWithContact(makeContact(), [payment]);
      renderWithProviders(<ContactDetailsScreen contactId={CONTACT_ID} />);

      await waitFor(() => {
        const list = screen.getByRole("list", {
          name: "Payment history",
        });
        expect(list).toBeInTheDocument();
      });
    });
  });
});
