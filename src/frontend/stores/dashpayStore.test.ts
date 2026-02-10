/**
 * dashpayStore tests — using centralized mock IPC + fixture factories.
 *
 * Pattern:
 * 1. createMockBindings() provides defaults for all commands + events
 * 2. Override specific commands needed by the store under test
 * 3. Use fixture factories for realistic test data
 */

import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { createMockBindings, mockBindingsModule } from "@/test/mock-ipc";
import {
  createMockStoredProfile,
  createMockStoredContact,
  createMockContactRequest,
  createMockStoredPayment,
  createMockContactPrivateInfo,
} from "@/test/fixtures";
import type { TaskResultEvent } from "../bindings";

// ─── Mock bindings (centralized) ────────────────────────────────────

vi.mock("../bindings", () => {
  const initial = createMockBindings();
  return mockBindingsModule(initial);
});

import { commands, events } from "../bindings";
import { useDashPayStore } from "./dashpayStore";

// ─── Helpers ────────────────────────────────────────────────────────

function resetStore() {
  useDashPayStore.getState().reset();
}

const IDENTITY_ID = "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis";
const CONTACT_ID = "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12";

// ─── Tests ──────────────────────────────────────────────────────────

describe("dashpayStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  describe("initial state", () => {
    it("starts with empty state and no identity selected", () => {
      const state = useDashPayStore.getState();
      expect(state.selectedIdentityId).toBeNull();
      expect(state.profile).toBeNull();
      expect(state.profileLoading).toBe(false);
      expect(state.profileSaving).toBe(false);
      expect(state.profileError).toBeNull();
      expect(state.contacts).toEqual([]);
      expect(state.contactsLoading).toBe(false);
      expect(state.contactsRefreshing).toBe(false);
      expect(state.contactsError).toBeNull();
      expect(state.contactSearchQuery).toBe("");
      expect(state.contactFilter).toBe("all");
      expect(state.contactSortField).toBe("name");
      expect(state.contactSortOrder).toBe("ascending");
      expect(state.showHidden).toBe(false);
      expect(state.incomingRequests).toEqual([]);
      expect(state.outgoingRequests).toEqual([]);
      expect(state.requestsLoading).toBe(false);
      expect(state.requestsRefreshing).toBe(false);
      expect(state.requestsError).toBeNull();
      expect(state.acceptingIds.size).toBe(0);
      expect(state.rejectingIds.size).toBe(0);
      expect(state.payments).toEqual([]);
      expect(state.paymentsLoading).toBe(false);
      expect(state.paymentsRefreshing).toBe(false);
      expect(state.paymentsError).toBeNull();
      expect(state.searchResults).toEqual([]);
      expect(state.searchLoading).toBe(false);
      expect(state.searchError).toBeNull();
    });
  });

  describe("selectIdentity", () => {
    it("sets the selected identity ID", () => {
      useDashPayStore.getState().selectIdentity(IDENTITY_ID);
      expect(useDashPayStore.getState().selectedIdentityId).toBe(IDENTITY_ID);
    });

    it("clears data when switching identities", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        profile: createMockStoredProfile(),
        contacts: [createMockStoredContact()],
        payments: [createMockStoredPayment()],
      });

      useDashPayStore.getState().selectIdentity("other-identity");

      const state = useDashPayStore.getState();
      expect(state.selectedIdentityId).toBe("other-identity");
      expect(state.profile).toBeNull();
      expect(state.contacts).toEqual([]);
      expect(state.payments).toEqual([]);
    });

    it("does not clear data when selecting the same identity", () => {
      const profile = createMockStoredProfile();
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        profile,
        contacts: [createMockStoredContact()],
      });

      useDashPayStore.getState().selectIdentity(IDENTITY_ID);

      expect(useDashPayStore.getState().profile).toBe(profile);
      expect(useDashPayStore.getState().contacts).toHaveLength(1);
    });

    it("deselects when passing null", () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      useDashPayStore.getState().selectIdentity(null);
      expect(useDashPayStore.getState().selectedIdentityId).toBeNull();
    });
  });

  // ── Profile tests ───────────────────────────────────────────────

  describe("loadProfile", () => {
    it("loads profile from local DB", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      const profile = createMockStoredProfile();
      (commands.dashpayDbLoadProfile as Mock).mockResolvedValue({
        status: "ok",
        data: profile,
      });

      await useDashPayStore.getState().loadProfile();

      const state = useDashPayStore.getState();
      expect(state.profile).toEqual(profile);
      expect(state.profileLoading).toBe(false);
      expect(state.profileError).toBeNull();
      expect(commands.dashpayDbLoadProfile).toHaveBeenCalledWith(IDENTITY_ID);
    });

    it("sets profileLoading during load", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      let resolve: (v: unknown) => void;
      (commands.dashpayDbLoadProfile as Mock).mockReturnValue(
        new Promise((r) => {
          resolve = r;
        }),
      );

      const promise = useDashPayStore.getState().loadProfile();
      expect(useDashPayStore.getState().profileLoading).toBe(true);

      resolve!({ status: "ok", data: null });
      await promise;
      expect(useDashPayStore.getState().profileLoading).toBe(false);
    });

    it("handles null profile (no profile exists)", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadProfile as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useDashPayStore.getState().loadProfile();

      expect(useDashPayStore.getState().profile).toBeNull();
      expect(useDashPayStore.getState().profileLoading).toBe(false);
    });

    it("sets error on failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadProfile as Mock).mockResolvedValue({
        status: "error",
        error: "Database read failed",
      });

      await useDashPayStore.getState().loadProfile();

      expect(useDashPayStore.getState().profileError).toBe(
        "Database read failed",
      );
      expect(useDashPayStore.getState().profileLoading).toBe(false);
    });

    it("sets error on exception", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadProfile as Mock).mockRejectedValue(
        new Error("IPC error"),
      );

      await useDashPayStore.getState().loadProfile();

      expect(useDashPayStore.getState().profileError).toBe("IPC error");
    });

    it("does nothing without a selected identity", async () => {
      await useDashPayStore.getState().loadProfile();
      expect(commands.dashpayDbLoadProfile).not.toHaveBeenCalled();
    });
  });

  describe("updateProfile", () => {
    it("dispatches update profile task", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayUpdateProfile as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().updateProfile({
        displayName: "New Name",
        bio: "New bio",
        avatarUrl: null,
      });

      expect(commands.dashpayUpdateProfile).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        displayName: "New Name",
        bio: "New bio",
        avatarUrl: null,
      });
      // profileSaving stays true until task result event
      expect(useDashPayStore.getState().profileSaving).toBe(true);
    });

    it("sets error on dispatch failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayUpdateProfile as Mock).mockResolvedValue({
        status: "error",
        error: "Insufficient balance",
      });

      await useDashPayStore.getState().updateProfile({
        displayName: "Test",
        bio: null,
        avatarUrl: null,
      });

      expect(useDashPayStore.getState().profileError).toBe(
        "Insufficient balance",
      );
      expect(useDashPayStore.getState().profileSaving).toBe(false);
    });

    it("sets error on exception", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayUpdateProfile as Mock).mockRejectedValue(
        new Error("Network down"),
      );

      await useDashPayStore.getState().updateProfile({
        displayName: "Test",
        bio: null,
        avatarUrl: null,
      });

      expect(useDashPayStore.getState().profileError).toBe("Network down");
      expect(useDashPayStore.getState().profileSaving).toBe(false);
    });

    it("does nothing without a selected identity", async () => {
      await useDashPayStore.getState().updateProfile({
        displayName: "Test",
        bio: null,
        avatarUrl: null,
      });
      expect(commands.dashpayUpdateProfile).not.toHaveBeenCalled();
    });
  });

  // ── Contacts tests ──────────────────────────────────────────────

  describe("loadContacts", () => {
    it("loads contacts from local DB", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      const contacts = [
        createMockStoredContact(),
        createMockStoredContact({ contactIdentityId: "other-id" }),
      ];
      (commands.dashpayDbLoadContacts as Mock).mockResolvedValue({
        status: "ok",
        data: contacts,
      });

      await useDashPayStore.getState().loadContacts();

      expect(useDashPayStore.getState().contacts).toEqual(contacts);
      expect(useDashPayStore.getState().contactsLoading).toBe(false);
    });

    it("sets error on failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadContacts as Mock).mockResolvedValue({
        status: "error",
        error: "DB error",
      });

      await useDashPayStore.getState().loadContacts();

      expect(useDashPayStore.getState().contactsError).toBe("DB error");
      expect(useDashPayStore.getState().contactsLoading).toBe(false);
    });

    it("does nothing without a selected identity", async () => {
      await useDashPayStore.getState().loadContacts();
      expect(commands.dashpayDbLoadContacts).not.toHaveBeenCalled();
    });
  });

  describe("refreshContacts", () => {
    it("dispatches Platform refresh for contacts", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayLoadContacts as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().refreshContacts();

      expect(commands.dashpayLoadContacts).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
      });
      expect(useDashPayStore.getState().contactsRefreshing).toBe(true);
    });

    it("clears refreshing on dispatch error", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayLoadContacts as Mock).mockResolvedValue({
        status: "error",
        error: "Platform unavailable",
      });

      await useDashPayStore.getState().refreshContacts();

      expect(useDashPayStore.getState().contactsRefreshing).toBe(false);
      expect(useDashPayStore.getState().contactsError).toBe(
        "Platform unavailable",
      );
    });
  });

  describe("fetchContactProfile", () => {
    it("dispatches fetch contact profile task", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayFetchContactProfile as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().fetchContactProfile(CONTACT_ID);

      expect(commands.dashpayFetchContactProfile).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        contactId: CONTACT_ID,
      });
    });

    it("sets error on failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayFetchContactProfile as Mock).mockResolvedValue({
        status: "error",
        error: "Contact not found",
      });

      await useDashPayStore.getState().fetchContactProfile(CONTACT_ID);

      expect(useDashPayStore.getState().contactsError).toBe(
        "Contact not found",
      );
    });
  });

  describe("contact filter/sort state", () => {
    it("sets search query", () => {
      useDashPayStore.getState().setContactSearchQuery("alice");
      expect(useDashPayStore.getState().contactSearchQuery).toBe("alice");
    });

    it("sets filter", () => {
      useDashPayStore.getState().setContactFilter("withUsernames");
      expect(useDashPayStore.getState().contactFilter).toBe("withUsernames");
    });

    it("sets sort field", () => {
      useDashPayStore.getState().setContactSortField("username");
      expect(useDashPayStore.getState().contactSortField).toBe("username");
    });

    it("toggles sort order", () => {
      expect(useDashPayStore.getState().contactSortOrder).toBe("ascending");
      useDashPayStore.getState().toggleContactSortOrder();
      expect(useDashPayStore.getState().contactSortOrder).toBe("descending");
      useDashPayStore.getState().toggleContactSortOrder();
      expect(useDashPayStore.getState().contactSortOrder).toBe("ascending");
    });

    it("toggles show hidden", () => {
      expect(useDashPayStore.getState().showHidden).toBe(false);
      useDashPayStore.getState().toggleShowHidden();
      expect(useDashPayStore.getState().showHidden).toBe(true);
      useDashPayStore.getState().toggleShowHidden();
      expect(useDashPayStore.getState().showHidden).toBe(false);
    });
  });

  describe("setContactHidden", () => {
    it("hides a contact and updates the local list", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [
          createMockStoredContact({
            contactIdentityId: CONTACT_ID,
            contactStatus: "accepted",
          }),
        ],
      });
      (commands.dashpayDbSetContactHidden as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useDashPayStore.getState().setContactHidden(CONTACT_ID, true);

      expect(commands.dashpayDbSetContactHidden).toHaveBeenCalledWith(
        IDENTITY_ID,
        CONTACT_ID,
        true,
      );
      expect(useDashPayStore.getState().contacts[0].contactStatus).toBe(
        "hidden",
      );
    });

    it("unhides a contact", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        contacts: [
          createMockStoredContact({
            contactIdentityId: CONTACT_ID,
            contactStatus: "hidden",
          }),
        ],
      });
      (commands.dashpayDbSetContactHidden as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useDashPayStore.getState().setContactHidden(CONTACT_ID, false);

      expect(useDashPayStore.getState().contacts[0].contactStatus).toBe(
        "accepted",
      );
    });

    it("sets error on failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbSetContactHidden as Mock).mockResolvedValue({
        status: "error",
        error: "DB write failed",
      });

      await useDashPayStore.getState().setContactHidden(CONTACT_ID, true);

      expect(useDashPayStore.getState().contactsError).toBe("DB write failed");
    });
  });

  // ── Contact requests tests ──────────────────────────────────────

  describe("loadContactRequests", () => {
    it("loads incoming and outgoing requests from local DB", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      const incoming = [
        createMockContactRequest({ requestType: "incoming", id: 1 }),
      ];
      const outgoing = [
        createMockContactRequest({ requestType: "outgoing", id: 2 }),
      ];
      (commands.dashpayDbLoadPendingRequests as Mock)
        .mockResolvedValueOnce({ status: "ok", data: incoming })
        .mockResolvedValueOnce({ status: "ok", data: outgoing });

      await useDashPayStore.getState().loadContactRequests();

      expect(useDashPayStore.getState().incomingRequests).toEqual(incoming);
      expect(useDashPayStore.getState().outgoingRequests).toEqual(outgoing);
      expect(useDashPayStore.getState().requestsLoading).toBe(false);
      expect(commands.dashpayDbLoadPendingRequests).toHaveBeenCalledWith(
        IDENTITY_ID,
        "incoming",
      );
      expect(commands.dashpayDbLoadPendingRequests).toHaveBeenCalledWith(
        IDENTITY_ID,
        "outgoing",
      );
    });

    it("sets error when incoming request load fails", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadPendingRequests as Mock)
        .mockResolvedValueOnce({
          status: "error",
          error: "Incoming failed",
        })
        .mockResolvedValueOnce({ status: "ok", data: [] });

      await useDashPayStore.getState().loadContactRequests();

      expect(useDashPayStore.getState().requestsError).toBe("Incoming failed");
      expect(useDashPayStore.getState().incomingRequests).toEqual([]);
      expect(useDashPayStore.getState().outgoingRequests).toEqual([]);
    });

    it("does nothing without a selected identity", async () => {
      await useDashPayStore.getState().loadContactRequests();
      expect(commands.dashpayDbLoadPendingRequests).not.toHaveBeenCalled();
    });
  });

  describe("refreshContactRequests", () => {
    it("dispatches Platform refresh for requests", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayLoadContactRequests as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().refreshContactRequests();

      expect(commands.dashpayLoadContactRequests).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
      });
      expect(useDashPayStore.getState().requestsRefreshing).toBe(true);
    });
  });

  describe("sendContactRequest", () => {
    it("dispatches send contact request task", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpaySendContactRequest as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().sendContactRequest({
        signingKeyId: 0,
        toUsername: "bob.dash",
        accountLabel: "Account 0",
      });

      expect(commands.dashpaySendContactRequest).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        signingKeyId: 0,
        toUsername: "bob.dash",
        accountLabel: "Account 0",
      });
    });

    it("sets error on dispatch failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpaySendContactRequest as Mock).mockResolvedValue({
        status: "error",
        error: "Missing encryption key",
      });

      await useDashPayStore.getState().sendContactRequest({
        signingKeyId: 0,
        toUsername: "bob.dash",
        accountLabel: null,
      });

      expect(useDashPayStore.getState().requestsError).toBe(
        "Missing encryption key",
      );
    });
  });

  describe("acceptContactRequest", () => {
    it("dispatches accept and tracks accepting state", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayAcceptContactRequest as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().acceptContactRequest("1");

      expect(commands.dashpayAcceptContactRequest).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        requestId: "1",
      });
      expect(useDashPayStore.getState().acceptingIds.has(1)).toBe(true);
    });

    it("clears accepting state on error", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayAcceptContactRequest as Mock).mockResolvedValue({
        status: "error",
        error: "Accept failed",
      });

      await useDashPayStore.getState().acceptContactRequest("1");

      expect(useDashPayStore.getState().acceptingIds.size).toBe(0);
      expect(useDashPayStore.getState().requestsError).toBe("Accept failed");
    });

    it("clears accepting state on exception", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayAcceptContactRequest as Mock).mockRejectedValue(
        new Error("Network error"),
      );

      await useDashPayStore.getState().acceptContactRequest("1");

      expect(useDashPayStore.getState().acceptingIds.size).toBe(0);
      expect(useDashPayStore.getState().requestsError).toBe("Network error");
    });
  });

  describe("rejectContactRequest", () => {
    it("dispatches reject and tracks rejecting state", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayRejectContactRequest as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().rejectContactRequest("2");

      expect(commands.dashpayRejectContactRequest).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        requestId: "2",
      });
      expect(useDashPayStore.getState().rejectingIds.has(2)).toBe(true);
    });

    it("clears rejecting state on error", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayRejectContactRequest as Mock).mockResolvedValue({
        status: "error",
        error: "Reject failed",
      });

      await useDashPayStore.getState().rejectContactRequest("2");

      expect(useDashPayStore.getState().rejectingIds.size).toBe(0);
      expect(useDashPayStore.getState().requestsError).toBe("Reject failed");
    });
  });

  // ── Payments tests ──────────────────────────────────────────────

  describe("loadPayments", () => {
    it("loads payments from local DB with default limit", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      const payments = [createMockStoredPayment()];
      (commands.dashpayDbLoadPayments as Mock).mockResolvedValue({
        status: "ok",
        data: payments,
      });

      await useDashPayStore.getState().loadPayments();

      expect(useDashPayStore.getState().payments).toEqual(payments);
      expect(useDashPayStore.getState().paymentsLoading).toBe(false);
      expect(commands.dashpayDbLoadPayments).toHaveBeenCalledWith(
        IDENTITY_ID,
        50,
      );
    });

    it("loads payments with custom limit", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadPayments as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });

      await useDashPayStore.getState().loadPayments(100);

      expect(commands.dashpayDbLoadPayments).toHaveBeenCalledWith(
        IDENTITY_ID,
        100,
      );
    });

    it("sets error on failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadPayments as Mock).mockResolvedValue({
        status: "error",
        error: "DB error",
      });

      await useDashPayStore.getState().loadPayments();

      expect(useDashPayStore.getState().paymentsError).toBe("DB error");
    });

    it("does nothing without a selected identity", async () => {
      await useDashPayStore.getState().loadPayments();
      expect(commands.dashpayDbLoadPayments).not.toHaveBeenCalled();
    });
  });

  describe("refreshPayments", () => {
    it("dispatches Platform refresh for payment history", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayLoadPaymentHistory as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().refreshPayments();

      expect(commands.dashpayLoadPaymentHistory).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
      });
      expect(useDashPayStore.getState().paymentsRefreshing).toBe(true);
    });

    it("clears refreshing on dispatch error", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayLoadPaymentHistory as Mock).mockResolvedValue({
        status: "error",
        error: "Platform error",
      });

      await useDashPayStore.getState().refreshPayments();

      expect(useDashPayStore.getState().paymentsRefreshing).toBe(false);
      expect(useDashPayStore.getState().paymentsError).toBe("Platform error");
    });
  });

  describe("sendPayment", () => {
    it("dispatches send payment task", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpaySendPaymentToContact as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().sendPayment({
        contactId: CONTACT_ID,
        amountDash: 0.5,
        memo: "Thanks!",
      });

      expect(commands.dashpaySendPaymentToContact).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        contactId: CONTACT_ID,
        amountDash: 0.5,
        memo: "Thanks!",
      });
    });

    it("sets error on dispatch failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpaySendPaymentToContact as Mock).mockResolvedValue({
        status: "error",
        error: "Insufficient funds",
      });

      await useDashPayStore.getState().sendPayment({
        contactId: CONTACT_ID,
        amountDash: 100,
        memo: null,
      });

      expect(useDashPayStore.getState().paymentsError).toBe(
        "Insufficient funds",
      );
    });
  });

  // ── Contact private info tests ──────────────────────────────────

  describe("loadContactPrivateInfo", () => {
    it("returns private info for a contact", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      const info = createMockContactPrivateInfo();
      (commands.dashpayDbLoadContactPrivateInfo as Mock).mockResolvedValue({
        status: "ok",
        data: info,
      });

      const result = await useDashPayStore
        .getState()
        .loadContactPrivateInfo(CONTACT_ID);

      expect(result).toEqual(info);
      expect(commands.dashpayDbLoadContactPrivateInfo).toHaveBeenCalledWith(
        IDENTITY_ID,
        CONTACT_ID,
      );
    });

    it("returns null on error", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbLoadContactPrivateInfo as Mock).mockResolvedValue({
        status: "error",
        error: "Not found",
      });

      const result = await useDashPayStore
        .getState()
        .loadContactPrivateInfo(CONTACT_ID);

      expect(result).toBeNull();
    });

    it("returns null without selected identity", async () => {
      const result = await useDashPayStore
        .getState()
        .loadContactPrivateInfo(CONTACT_ID);

      expect(result).toBeNull();
      expect(commands.dashpayDbLoadContactPrivateInfo).not.toHaveBeenCalled();
    });
  });

  describe("saveContactPrivateInfo", () => {
    it("saves private info to local DB", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbSaveContactPrivateInfo as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useDashPayStore.getState().saveContactPrivateInfo({
        contactId: CONTACT_ID,
        nickname: "Bobby",
        notes: "Great guy",
        isHidden: false,
      });

      expect(commands.dashpayDbSaveContactPrivateInfo).toHaveBeenCalledWith(
        IDENTITY_ID,
        CONTACT_ID,
        "Bobby",
        "Great guy",
        false,
      );
    });

    it("sets error on failure", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayDbSaveContactPrivateInfo as Mock).mockResolvedValue({
        status: "error",
        error: "Write failed",
      });

      await useDashPayStore.getState().saveContactPrivateInfo({
        contactId: CONTACT_ID,
        nickname: "Bobby",
        notes: "",
        isHidden: false,
      });

      expect(useDashPayStore.getState().contactsError).toBe("Write failed");
    });
  });

  describe("updateContactInfo", () => {
    it("dispatches update contact info task", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });
      (commands.dashpayUpdateContactInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().updateContactInfo({
        contactId: CONTACT_ID,
        nickname: "Bobby",
        note: "Updated note",
        isHidden: false,
        acceptedAccounts: [0, 1],
      });

      expect(commands.dashpayUpdateContactInfo).toHaveBeenCalledWith({
        identityId: IDENTITY_ID,
        contactId: CONTACT_ID,
        nickname: "Bobby",
        note: "Updated note",
        isHidden: false,
        acceptedAccounts: [0, 1],
      });
    });
  });

  // ── Profile search tests ────────────────────────────────────────

  describe("searchProfiles", () => {
    it("dispatches search profiles task", async () => {
      (commands.dashpaySearchProfiles as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useDashPayStore.getState().searchProfiles("alice");

      expect(commands.dashpaySearchProfiles).toHaveBeenCalledWith({
        searchQuery: "alice",
      });
      expect(useDashPayStore.getState().searchLoading).toBe(true);
    });

    it("sets error on dispatch failure", async () => {
      (commands.dashpaySearchProfiles as Mock).mockResolvedValue({
        status: "error",
        error: "Search failed",
      });

      await useDashPayStore.getState().searchProfiles("bob");

      expect(useDashPayStore.getState().searchError).toBe("Search failed");
      expect(useDashPayStore.getState().searchLoading).toBe(false);
    });
  });

  describe("clearSearchResults", () => {
    it("clears search results and error", () => {
      useDashPayStore.setState({
        searchResults: [createMockStoredContact()],
        searchError: "old error",
        searchLoading: true,
      });

      useDashPayStore.getState().clearSearchResults();

      expect(useDashPayStore.getState().searchResults).toEqual([]);
      expect(useDashPayStore.getState().searchError).toBeNull();
      expect(useDashPayStore.getState().searchLoading).toBe(false);
    });
  });

  // ── Event subscription tests ────────────────────────────────────

  describe("subscribeToUpdates", () => {
    it("subscribes to task result and error events", async () => {
      const unlistenResult = vi.fn();
      const unlistenError = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(
        unlistenResult,
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unlistenError);

      const unlisten = await useDashPayStore
        .getState()
        .subscribeToUpdates();

      expect(events.taskResultEvent.listen).toHaveBeenCalledWith(
        expect.any(Function),
      );
      expect(events.taskErrorEvent.listen).toHaveBeenCalledWith(
        expect.any(Function),
      );
      expect(typeof unlisten).toBe("function");
    });

    it("calls both unsubscribe functions when unsubscribed", async () => {
      const unlistenResult = vi.fn();
      const unlistenError = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(
        unlistenResult,
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unlistenError);

      const unlisten = await useDashPayStore
        .getState()
        .subscribeToUpdates();
      unlisten();

      expect(unlistenResult).toHaveBeenCalled();
      expect(unlistenError).toHaveBeenCalled();
    });

    it("clears async flags on DashPay task result", async () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        profileSaving: true,
        contactsRefreshing: true,
        requestsRefreshing: true,
        paymentsRefreshing: true,
        searchLoading: true,
        acceptingIds: new Set([1]),
        rejectingIds: new Set([2]),
      });

      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});

      // Mock the DB reload calls
      (commands.dashpayDbLoadProfile as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });
      (commands.dashpayDbLoadContacts as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });
      (commands.dashpayDbLoadPendingRequests as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });
      (commands.dashpayDbLoadPayments as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });

      await useDashPayStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "DashPay",
          payload: {},
        },
      });

      const state = useDashPayStore.getState();
      expect(state.profileSaving).toBe(false);
      expect(state.contactsRefreshing).toBe(false);
      expect(state.requestsRefreshing).toBe(false);
      expect(state.paymentsRefreshing).toBe(false);
      expect(state.searchLoading).toBe(false);
      expect(state.acceptingIds.size).toBe(0);
      expect(state.rejectingIds.size).toBe(0);
    });

    it("reloads local data on DashPay task result", async () => {
      useDashPayStore.setState({ selectedIdentityId: IDENTITY_ID });

      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});

      (commands.dashpayDbLoadProfile as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });
      (commands.dashpayDbLoadContacts as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });
      (commands.dashpayDbLoadPendingRequests as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });
      (commands.dashpayDbLoadPayments as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });

      await useDashPayStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "DashPay",
          payload: {},
        },
      });

      await vi.waitFor(() => {
        expect(commands.dashpayDbLoadProfile).toHaveBeenCalled();
        expect(commands.dashpayDbLoadContacts).toHaveBeenCalled();
        expect(commands.dashpayDbLoadPendingRequests).toHaveBeenCalled();
        expect(commands.dashpayDbLoadPayments).toHaveBeenCalled();
      });
    });

    it("ignores non-DashPay result types", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});

      await useDashPayStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "Identity",
          payload: {},
        },
      });

      expect(commands.dashpayDbLoadProfile).not.toHaveBeenCalled();
    });

    it("sets error on task error event", async () => {
      useDashPayStore.setState({
        profileSaving: true,
        contactsRefreshing: true,
        acceptingIds: new Set([1]),
      });

      let errorCallback: (event: {
        payload: { taskId: string; message: string };
      }) => void;
      (events.taskResultEvent.listen as Mock).mockResolvedValue(() => {});
      (events.taskErrorEvent.listen as Mock).mockImplementation(
        (cb: typeof errorCallback) => {
          errorCallback = cb;
          return Promise.resolve(() => {});
        },
      );

      await useDashPayStore.getState().subscribeToUpdates();
      errorCallback!({
        payload: { taskId: "t1", message: "Task failed" },
      });

      const state = useDashPayStore.getState();
      expect(state.profileSaving).toBe(false);
      expect(state.contactsRefreshing).toBe(false);
      expect(state.acceptingIds.size).toBe(0);
      expect(state.profileError).toBe("Task failed");
    });
  });

  // ── Utility tests ───────────────────────────────────────────────

  describe("clearErrors", () => {
    it("clears all error states", () => {
      useDashPayStore.setState({
        profileError: "profile err",
        contactsError: "contacts err",
        requestsError: "requests err",
        paymentsError: "payments err",
        searchError: "search err",
      });

      useDashPayStore.getState().clearErrors();

      const state = useDashPayStore.getState();
      expect(state.profileError).toBeNull();
      expect(state.contactsError).toBeNull();
      expect(state.requestsError).toBeNull();
      expect(state.paymentsError).toBeNull();
      expect(state.searchError).toBeNull();
    });
  });

  describe("reset", () => {
    it("resets store to initial state", () => {
      useDashPayStore.setState({
        selectedIdentityId: IDENTITY_ID,
        profile: createMockStoredProfile(),
        contacts: [createMockStoredContact()],
        payments: [createMockStoredPayment()],
        profileSaving: true,
        contactsRefreshing: true,
        showHidden: true,
        contactFilter: "withUsernames",
      });

      useDashPayStore.getState().reset();

      const state = useDashPayStore.getState();
      expect(state.selectedIdentityId).toBeNull();
      expect(state.profile).toBeNull();
      expect(state.contacts).toEqual([]);
      expect(state.payments).toEqual([]);
      expect(state.profileSaving).toBe(false);
      expect(state.contactsRefreshing).toBe(false);
      expect(state.showHidden).toBe(false);
      expect(state.contactFilter).toBe("all");
    });
  });
});
