import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  StoredProfileDto,
  StoredContactDto,
  StoredContactRequestDto,
  StoredPaymentDto,
  ContactPrivateInfoDto,
  TaskResultEvent,
} from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

// ─── Types ──────────────────────────────────────────────────────────

export type ContactFilter =
  | "all"
  | "withUsernames"
  | "noUsernames"
  | "withBio"
  | "recent"
  | "hidden"
  | "visible";

export type ContactSortField = "name" | "username" | "date" | "account";

export type ContactSortOrder = "ascending" | "descending";

/** A profile search result returned from the backend. */
export interface ProfileSearchResult {
  identityId: string;
  username: string;
  displayName: string | null;
  publicMessage: string | null;
  avatarUrl: string | null;
}

// ─── Store state ────────────────────────────────────────────────────

interface DashPayState {
  // ── Identity selection ──
  /** Currently selected identity ID for all DashPay operations. */
  selectedIdentityId: string | null;

  // ── Profile slice ──
  /** Profile loaded from local DB (may be stale). */
  profile: StoredProfileDto | null;
  /** Whether a profile load/update is in progress. */
  profileLoading: boolean;
  /** Whether a profile save/update is being dispatched. */
  profileSaving: boolean;
  /** Profile error message. */
  profileError: string | null;

  // ── Contacts slice ──
  /** Contacts loaded from local DB. */
  contacts: StoredContactDto[];
  /** Whether contacts are being loaded. */
  contactsLoading: boolean;
  /** Whether a Platform refresh of contacts is in progress. */
  contactsRefreshing: boolean;
  /** Contacts error message. */
  contactsError: string | null;
  /** Search query for filtering contacts. */
  contactSearchQuery: string;
  /** Active filter for contacts. */
  contactFilter: ContactFilter;
  /** Sort field for contacts. */
  contactSortField: ContactSortField;
  /** Sort order for contacts. */
  contactSortOrder: ContactSortOrder;
  /** Whether to show hidden contacts. */
  showHidden: boolean;

  // ── Contact requests slice ──
  /** Incoming contact requests. */
  incomingRequests: StoredContactRequestDto[];
  /** Outgoing contact requests. */
  outgoingRequests: StoredContactRequestDto[];
  /** Whether requests are being loaded. */
  requestsLoading: boolean;
  /** Whether a Platform refresh of requests is in progress. */
  requestsRefreshing: boolean;
  /** Requests error message. */
  requestsError: string | null;
  /** Set of request IDs currently being accepted. */
  acceptingIds: Set<number>;
  /** Set of request IDs currently being rejected. */
  rejectingIds: Set<number>;

  // ── Payments slice ──
  /** Payment history records. */
  payments: StoredPaymentDto[];
  /** Whether payments are being loaded. */
  paymentsLoading: boolean;
  /** Whether a Platform refresh of payments is in progress. */
  paymentsRefreshing: boolean;
  /** Payments error message. */
  paymentsError: string | null;

  // ── Profile search slice ──
  /** Profile search results. */
  searchResults: ProfileSearchResult[];
  /** Whether a profile search is in progress. */
  searchLoading: boolean;
  /** Profile search error message. */
  searchError: string | null;

  // ── In-flight operation tracking ──
  /** Set of operation keys currently in-flight (for targeted reload on event). */
  pendingOps: Set<string>;
}

// ─── Store actions ──────────────────────────────────────────────────

interface DashPayActions {
  // ── Identity selection ──
  /** Set the active identity for all DashPay screens. */
  selectIdentity: (identityId: string | null) => void;

  // ── Profile actions ──
  /** Load profile from local DB for the selected identity. */
  loadProfile: () => Promise<void>;
  /** Update profile on Platform (async task dispatch). */
  updateProfile: (input: {
    displayName: string | null;
    bio: string | null;
    avatarUrl: string | null;
  }) => Promise<void>;

  // ── Contacts actions ──
  /** Load contacts from local DB for the selected identity. */
  loadContacts: () => Promise<void>;
  /** Refresh contacts from Platform (async task dispatch). */
  refreshContacts: () => Promise<void>;
  /** Fetch a specific contact's profile from Platform (async task dispatch). */
  fetchContactProfile: (contactId: string) => Promise<void>;
  /** Set search query for filtering contacts. */
  setContactSearchQuery: (query: string) => void;
  /** Set filter for contacts. */
  setContactFilter: (filter: ContactFilter) => void;
  /** Set sort field for contacts. */
  setContactSortField: (field: ContactSortField) => void;
  /** Toggle sort order. */
  toggleContactSortOrder: () => void;
  /** Toggle show hidden contacts. */
  toggleShowHidden: () => void;
  /** Hide/unhide a contact. */
  setContactHidden: (contactId: string, isHidden: boolean) => Promise<void>;

  // ── Contact request actions ──
  /** Load contact requests from local DB. */
  loadContactRequests: () => Promise<void>;
  /** Refresh contact requests from Platform. */
  refreshContactRequests: () => Promise<void>;
  /** Send a new contact request (async task dispatch). Returns taskId or null on IPC error. */
  sendContactRequest: (input: {
    signingKeyId: number;
    toUsername: string;
    accountLabel: string | null;
  }) => Promise<string | null>;
  /** Accept an incoming contact request (async task dispatch). */
  acceptContactRequest: (dbRowId: number, platformDocId: string) => Promise<void>;
  /** Reject an incoming contact request (async task dispatch). */
  rejectContactRequest: (dbRowId: number, platformDocId: string) => Promise<void>;

  // ── Payment actions ──
  /** Load payment history from local DB. */
  loadPayments: (limit?: number) => Promise<void>;
  /** Refresh payment history from Platform. */
  refreshPayments: () => Promise<void>;
  /** Send a payment to a contact (async task dispatch). Returns taskId or null on IPC error. */
  sendPayment: (input: {
    contactId: string;
    amountDash: number;
    memo: string | null;
  }) => Promise<string | null>;

  // ── Contact info actions ──
  /** Load private info for a specific contact. */
  loadContactPrivateInfo: (
    contactId: string,
  ) => Promise<ContactPrivateInfoDto | null>;
  /** Save private info for a specific contact. */
  saveContactPrivateInfo: (input: {
    contactId: string;
    nickname: string;
    notes: string;
    isHidden: boolean;
  }) => Promise<void>;
  /** Update contact info on Platform (async task dispatch). Returns taskId or null on IPC error. */
  updateContactInfo: (input: {
    contactId: string;
    nickname: string | null;
    note: string | null;
    isHidden: boolean;
    acceptedAccounts: number[];
  }) => Promise<string | null>;

  // ── Search actions ──
  /** Search for profiles by username prefix (async task dispatch). */
  searchProfiles: (query: string) => Promise<void>;
  /** Clear search results. */
  clearSearchResults: () => void;

  // ── Event subscription ──
  /** Subscribe to task result/error events. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  // ── Utility ──
  /** Clear all error states. */
  clearErrors: () => void;
  /** Reset store to initial state. */
  reset: () => void;

  /** Reset all state (used on network switch). */
  resetState: () => void;
}

export type DashPayStore = DashPayState & DashPayActions;

// ─── Initial state ──────────────────────────────────────────────────

const initialState: DashPayState = {
  selectedIdentityId: null,
  profile: null,
  profileLoading: false,
  profileSaving: false,
  profileError: null,
  contacts: [],
  contactsLoading: false,
  contactsRefreshing: false,
  contactsError: null,
  contactSearchQuery: "",
  contactFilter: "all",
  contactSortField: "name",
  contactSortOrder: "ascending",
  showHidden: false,
  incomingRequests: [],
  outgoingRequests: [],
  requestsLoading: false,
  requestsRefreshing: false,
  requestsError: null,
  acceptingIds: new Set(),
  rejectingIds: new Set(),
  payments: [],
  paymentsLoading: false,
  paymentsRefreshing: false,
  paymentsError: null,
  searchResults: [],
  searchLoading: false,
  searchError: null,
  pendingOps: new Set(),
};

// ─── Helpers ─────────────────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

/** Remove an op from pendingOps and set an error on a specific slice. */
function failOp(
  set: (fn: (s: DashPayState) => Partial<DashPayState>) => void,
  opKey: string,
  errorKey: keyof Pick<DashPayState, "contactsError" | "requestsError" | "paymentsError">,
  message: string,
): void {
  set((s) => {
    const next = new Set(s.pendingOps);
    next.delete(opKey);
    return { [errorKey]: message, pendingOps: next };
  });
}

// ─── Store ──────────────────────────────────────────────────────────

export const useDashPayStore = create<DashPayStore>((set, get) => ({
  ...initialState,

  // ── Identity selection ──────────────────────────────────────────

  selectIdentity: (identityId) => {
    const prev = get().selectedIdentityId;
    if (prev === identityId) return;

    // Clear data when switching identities
    set({
      selectedIdentityId: identityId,
      profile: null,
      profileError: null,
      contacts: [],
      contactsError: null,
      incomingRequests: [],
      outgoingRequests: [],
      requestsError: null,
      payments: [],
      paymentsError: null,
      searchResults: [],
      searchError: null,
    });
  },

  // ── Profile ─────────────────────────────────────────────────────

  loadProfile: async () => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set({ profileLoading: true, profileError: null });
    try {
      const result = await commands.dashpayDbLoadProfile(selectedIdentityId);
      if (result.status === "ok") {
        set({ profile: result.data, profileLoading: false });
      } else {
        set({ profileError: result.error, profileLoading: false });
      }
    } catch (e) {
      set({
        profileError: e instanceof Error ? e.message : String(e),
        profileLoading: false,
      });
    }
  },

  updateProfile: async ({ displayName, bio, avatarUrl }) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set((s) => ({
      profileSaving: true, profileError: null,
      pendingOps: new Set(s.pendingOps).add("profile"),
    }));
    try {
      const result = await commands.dashpayUpdateProfile({
        identityId: selectedIdentityId,
        displayName,
        bio,
        avatarUrl,
      });
      if (result.status === "error") {
        set({ profileError: result.error, profileSaving: false });
      } else {
        timeouts.start("profile", () => {
          set({ profileSaving: false, profileError: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // On success, profileSaving stays true — cleared by task result event
    } catch (e) {
      set({
        profileError: e instanceof Error ? e.message : String(e),
        profileSaving: false,
      });
    }
  },

  // ── Contacts ────────────────────────────────────────────────────

  loadContacts: async () => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set({ contactsLoading: true, contactsError: null });
    try {
      const result = await commands.dashpayDbLoadContacts(selectedIdentityId);
      if (result.status === "ok") {
        set({ contacts: result.data, contactsLoading: false });
      } else {
        set({ contactsError: result.error, contactsLoading: false });
      }
    } catch (e) {
      set({
        contactsError: e instanceof Error ? e.message : String(e),
        contactsLoading: false,
      });
    }
  },

  refreshContacts: async () => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set((s) => ({
      contactsRefreshing: true, contactsError: null,
      pendingOps: new Set(s.pendingOps).add("contacts"),
    }));
    try {
      const result = await commands.dashpayLoadContacts({
        identityId: selectedIdentityId,
      });
      if (result.status === "error") {
        set({ contactsError: result.error, contactsRefreshing: false });
      } else {
        timeouts.start("contacts", () => {
          set({ contactsRefreshing: false, contactsError: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // contactsRefreshing cleared by task result event
    } catch (e) {
      set({
        contactsError: e instanceof Error ? e.message : String(e),
        contactsRefreshing: false,
      });
    }
  },

  fetchContactProfile: async (contactId) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    try {
      const result = await commands.dashpayFetchContactProfile({
        identityId: selectedIdentityId,
        contactId,
      });
      if (result.status === "error") {
        set({ contactsError: result.error });
      }
    } catch (e) {
      set({ contactsError: e instanceof Error ? e.message : String(e) });
    }
  },

  setContactSearchQuery: (query) => {
    set({ contactSearchQuery: query });
  },

  setContactFilter: (filter) => {
    set({ contactFilter: filter });
  },

  setContactSortField: (field) => {
    set({ contactSortField: field });
  },

  toggleContactSortOrder: () => {
    set((state) => ({
      contactSortOrder:
        state.contactSortOrder === "ascending" ? "descending" : "ascending",
    }));
  },

  toggleShowHidden: () => {
    set((state) => ({ showHidden: !state.showHidden }));
  },

  setContactHidden: async (contactId, isHidden) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    try {
      const result = await commands.dashpayDbSetContactHidden(
        selectedIdentityId,
        contactId,
        isHidden,
      );
      if (result.status === "ok") {
        // Update the contact in the local list
        set((state) => ({
          contacts: state.contacts.map((c) =>
            c.contactIdentityId === contactId
              ? { ...c, contactStatus: isHidden ? "hidden" : "accepted" }
              : c,
          ),
        }));
      } else {
        set({ contactsError: result.error });
      }
    } catch (e) {
      set({ contactsError: e instanceof Error ? e.message : String(e) });
    }
  },

  // ── Contact requests ────────────────────────────────────────────

  loadContactRequests: async () => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set({ requestsLoading: true, requestsError: null });
    try {
      const [incomingResult, outgoingResult] = await Promise.all([
        commands.dashpayDbLoadPendingRequests(selectedIdentityId, "incoming"),
        commands.dashpayDbLoadPendingRequests(selectedIdentityId, "outgoing"),
      ]);

      const incoming =
        incomingResult.status === "ok" ? incomingResult.data : [];
      const outgoing =
        outgoingResult.status === "ok" ? outgoingResult.data : [];

      const error =
        incomingResult.status === "error"
          ? incomingResult.error
          : outgoingResult.status === "error"
            ? outgoingResult.error
            : null;

      set({
        incomingRequests: incoming,
        outgoingRequests: outgoing,
        requestsLoading: false,
        requestsError: error,
      });
    } catch (e) {
      set({
        requestsError: e instanceof Error ? e.message : String(e),
        requestsLoading: false,
      });
    }
  },

  refreshContactRequests: async () => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set((s) => ({
      requestsRefreshing: true, requestsError: null,
      pendingOps: new Set(s.pendingOps).add("requests"),
    }));
    try {
      const result = await commands.dashpayLoadContactRequests({
        identityId: selectedIdentityId,
      });
      if (result.status === "error") {
        set({ requestsError: result.error, requestsRefreshing: false });
      } else {
        timeouts.start("requests", () => {
          set({ requestsRefreshing: false, requestsError: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // requestsRefreshing cleared by task result event
    } catch (e) {
      set({
        requestsError: e instanceof Error ? e.message : String(e),
        requestsRefreshing: false,
      });
    }
  },

  sendContactRequest: async ({ signingKeyId, toUsername, accountLabel }) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return null;

    set((s) => ({
      requestsError: null,
      pendingOps: new Set(s.pendingOps).add("sendRequest"),
    }));
    try {
      const result = await commands.dashpaySendContactRequest({
        identityId: selectedIdentityId,
        signingKeyId,
        toUsername,
        accountLabel,
      });
      if (result.status === "error") {
        failOp(set, "sendRequest", "requestsError", result.error);
        return null;
      }
      timeouts.start("sendRequest", () => {
        failOp(set, "sendRequest", "requestsError", TIMEOUT_ERROR_MESSAGE);
      });
      return result.data.taskId;
    } catch (e) {
      failOp(set, "sendRequest", "requestsError", e instanceof Error ? e.message : String(e));
      return null;
    }
  },

  acceptContactRequest: async (dbRowId, platformDocId) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set((state) => ({
      acceptingIds: new Set(state.acceptingIds).add(dbRowId),
      requestsError: null,
      pendingOps: new Set(state.pendingOps).add("accept"),
    }));

    try {
      const result = await commands.dashpayAcceptContactRequest({
        identityId: selectedIdentityId,
        requestId: platformDocId,
      });
      if (result.status === "error") {
        set((state) => {
          const newIds = new Set(state.acceptingIds);
          newIds.delete(dbRowId);
          return { acceptingIds: newIds, requestsError: result.error };
        });
      } else {
        timeouts.start(`accept:${dbRowId}`, () => {
          set((s) => {
            const newIds = new Set(s.acceptingIds);
            newIds.delete(dbRowId);
            return { acceptingIds: newIds, requestsError: TIMEOUT_ERROR_MESSAGE };
          });
        });
      }
      // acceptingIds cleared by task result event
    } catch (e) {
      set((state) => {
        const newIds = new Set(state.acceptingIds);
        newIds.delete(dbRowId);
        return {
          acceptingIds: newIds,
          requestsError: e instanceof Error ? e.message : String(e),
        };
      });
    }
  },

  rejectContactRequest: async (dbRowId, platformDocId) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set((state) => ({
      rejectingIds: new Set(state.rejectingIds).add(dbRowId),
      requestsError: null,
      pendingOps: new Set(state.pendingOps).add("reject"),
    }));

    try {
      const result = await commands.dashpayRejectContactRequest({
        identityId: selectedIdentityId,
        requestId: platformDocId,
      });
      if (result.status === "error") {
        set((state) => {
          const newIds = new Set(state.rejectingIds);
          newIds.delete(dbRowId);
          return { rejectingIds: newIds, requestsError: result.error };
        });
      } else {
        timeouts.start(`reject:${dbRowId}`, () => {
          set((s) => {
            const newIds = new Set(s.rejectingIds);
            newIds.delete(dbRowId);
            return { rejectingIds: newIds, requestsError: TIMEOUT_ERROR_MESSAGE };
          });
        });
      }
      // rejectingIds cleared by task result event
    } catch (e) {
      set((state) => {
        const newIds = new Set(state.rejectingIds);
        newIds.delete(dbRowId);
        return {
          rejectingIds: newIds,
          requestsError: e instanceof Error ? e.message : String(e),
        };
      });
    }
  },

  // ── Payments ────────────────────────────────────────────────────

  loadPayments: async (limit = 50) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set({ paymentsLoading: true, paymentsError: null });
    try {
      const result = await commands.dashpayDbLoadPayments(
        selectedIdentityId,
        limit,
      );
      if (result.status === "ok") {
        set({ payments: result.data, paymentsLoading: false });
      } else {
        set({ paymentsError: result.error, paymentsLoading: false });
      }
    } catch (e) {
      set({
        paymentsError: e instanceof Error ? e.message : String(e),
        paymentsLoading: false,
      });
    }
  },

  refreshPayments: async () => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    set((s) => ({
      paymentsRefreshing: true, paymentsError: null,
      pendingOps: new Set(s.pendingOps).add("payments"),
    }));
    try {
      const result = await commands.dashpayLoadPaymentHistory({
        identityId: selectedIdentityId,
      });
      if (result.status === "error") {
        set({ paymentsError: result.error, paymentsRefreshing: false });
      } else {
        timeouts.start("payments", () => {
          set({ paymentsRefreshing: false, paymentsError: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // paymentsRefreshing cleared by task result event
    } catch (e) {
      set({
        paymentsError: e instanceof Error ? e.message : String(e),
        paymentsRefreshing: false,
      });
    }
  },

  sendPayment: async ({ contactId, amountDash, memo }) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return null;

    set((s) => ({
      paymentsError: null,
      pendingOps: new Set(s.pendingOps).add("sendPayment"),
    }));
    try {
      const result = await commands.dashpaySendPaymentToContact({
        identityId: selectedIdentityId,
        contactId,
        amountDash,
        memo,
      });
      if (result.status === "error") {
        failOp(set, "sendPayment", "paymentsError", result.error);
        return null;
      }
      timeouts.start("sendPayment", () => {
        failOp(set, "sendPayment", "paymentsError", TIMEOUT_ERROR_MESSAGE);
      });
      return result.data.taskId;
    } catch (e) {
      failOp(set, "sendPayment", "paymentsError", e instanceof Error ? e.message : String(e));
      return null;
    }
  },

  // ── Contact private info ────────────────────────────────────────

  loadContactPrivateInfo: async (contactId) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return null;

    try {
      const result = await commands.dashpayDbLoadContactPrivateInfo(
        selectedIdentityId,
        contactId,
      );
      if (result.status === "ok") {
        return result.data;
      }
      return null;
    } catch {
      return null;
    }
  },

  saveContactPrivateInfo: async ({
    contactId,
    nickname,
    notes,
    isHidden,
  }) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return;

    try {
      const result = await commands.dashpayDbSaveContactPrivateInfo(
        selectedIdentityId,
        contactId,
        nickname,
        notes,
        isHidden,
      );
      if (result.status === "error") {
        set({ contactsError: result.error });
      }
    } catch (e) {
      set({ contactsError: e instanceof Error ? e.message : String(e) });
    }
  },

  updateContactInfo: async ({
    contactId,
    nickname,
    note,
    isHidden,
    acceptedAccounts,
  }) => {
    const { selectedIdentityId } = get();
    if (!selectedIdentityId) return null;

    set((s) => ({
      contactsError: null,
      pendingOps: new Set(s.pendingOps).add("updateContactInfo"),
    }));
    try {
      const result = await commands.dashpayUpdateContactInfo({
        identityId: selectedIdentityId,
        contactId,
        nickname,
        note,
        isHidden,
        acceptedAccounts,
      });
      if (result.status === "error") {
        failOp(set, "updateContactInfo", "contactsError", result.error);
        return null;
      }
      timeouts.start("updateContactInfo", () => {
        failOp(set, "updateContactInfo", "contactsError", TIMEOUT_ERROR_MESSAGE);
      });
      return result.data.taskId;
    } catch (e) {
      failOp(set, "updateContactInfo", "contactsError", e instanceof Error ? e.message : String(e));
      return null;
    }
  },

  // ── Profile search ──────────────────────────────────────────────

  searchProfiles: async (query) => {
    set((s) => ({
      searchLoading: true, searchError: null,
      pendingOps: new Set(s.pendingOps).add("search"),
    }));
    try {
      const result = await commands.dashpaySearchProfiles({
        searchQuery: query,
      });
      if (result.status === "error") {
        set({ searchError: result.error, searchLoading: false });
      } else {
        timeouts.start("search", () => {
          set({ searchLoading: false, searchError: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // searchLoading cleared by task result event
    } catch (e) {
      set({
        searchError: e instanceof Error ? e.message : String(e),
        searchLoading: false,
      });
    }
  },

  clearSearchResults: () => {
    set({ searchResults: [], searchError: null, searchLoading: false });
  },

  // ── Event subscription ──────────────────────────────────────────

  subscribeToUpdates: async () => {
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { result } = event.payload;

        if (result.type !== "dashPayCompleted" && result.type !== "dashPayProfileSearchResults") return;

        timeouts.clearAll();

        const state = get();
        const ops = state.pendingOps;

        // Handle ProfileSearchResults payload from the backend
        if (result.type === "dashPayProfileSearchResults") {
          set({ searchResults: result.results as unknown as ProfileSearchResult[], searchLoading: false });
          set((s) => {
            const next = new Set(s.pendingOps);
            next.delete("search");
            return { pendingOps: next };
          });
          return;
        }

        // Targeted reload: only clear flags and reload data for pending ops
        if (ops.has("profile")) {
          set({ profileSaving: false });
          if (state.selectedIdentityId) state.loadProfile();
        }
        if (ops.has("contacts")) {
          set({ contactsRefreshing: false });
          if (state.selectedIdentityId) state.loadContacts();
        }
        if (ops.has("requests") || ops.has("sendRequest")) {
          set({ requestsRefreshing: false });
          if (state.selectedIdentityId) state.loadContactRequests();
        }
        if (ops.has("accept") || ops.has("reject")) {
          set({ acceptingIds: new Set(), rejectingIds: new Set() });
          if (state.selectedIdentityId) {
            state.loadContactRequests();
            state.loadContacts();
          }
        }
        if (ops.has("payments") || ops.has("sendPayment")) {
          set({ paymentsRefreshing: false });
          if (state.selectedIdentityId) state.loadPayments();
        }
        if (ops.has("updateContactInfo")) {
          if (state.selectedIdentityId) state.loadContacts();
        }

        // If no specific ops were pending, fall back to reloading everything
        if (ops.size === 0 && state.selectedIdentityId) {
          set({
            profileSaving: false,
            contactsRefreshing: false,
            requestsRefreshing: false,
            paymentsRefreshing: false,
            searchLoading: false,
            acceptingIds: new Set(),
            rejectingIds: new Set(),
          });
          state.loadProfile();
          state.loadContacts();
          state.loadContactRequests();
          state.loadPayments();
        }

        // Clear pending ops
        set({ pendingOps: new Set() });
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "dashPay") return;

        timeouts.clearAll();

        const state = get();
        const ops = state.pendingOps;
        const msg = event.payload.message;

        // Route error to the correct slice based on which operation was pending
        const updates: Partial<DashPayState> = { pendingOps: new Set() };

        if (ops.has("profile") || ops.size === 0) {
          updates.profileSaving = false;
          updates.profileError = msg;
        }
        if (ops.has("contacts")) {
          updates.contactsRefreshing = false;
          updates.contactsError = msg;
        }
        if (ops.has("requests") || ops.has("sendRequest")) {
          updates.requestsRefreshing = false;
          updates.requestsError = msg;
        }
        if (ops.has("accept") || ops.has("reject")) {
          updates.acceptingIds = new Set();
          updates.rejectingIds = new Set();
          updates.requestsError = msg;
        }
        if (ops.has("payments") || ops.has("sendPayment")) {
          updates.paymentsRefreshing = false;
          updates.paymentsError = msg;
        }
        if (ops.has("search")) {
          updates.searchLoading = false;
          updates.searchError = msg;
        }
        if (ops.has("updateContactInfo")) {
          updates.contactsError = msg;
        }

        set(updates);
      },
    );

    return () => {
      unlistenResult();
      unlistenError();
    };
  },

  // ── Utility ─────────────────────────────────────────────────────

  clearErrors: () => {
    set({
      profileError: null,
      contactsError: null,
      requestsError: null,
      paymentsError: null,
      searchError: null,
    });
  },

  reset: () => {
    set(initialState);
  },

  resetState: () => {
    timeouts.clearAll();
    set(initialState);
  },
}));
