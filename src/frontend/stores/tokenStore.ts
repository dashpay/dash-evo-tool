import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  IdentityTokenIdentifierDto,
  TaskResultEvent,
} from "../bindings";

// ─── Sort types ──────────────────────────────────────────────────────

export type TokenSortColumn = "ownerAlias" | "name" | "balance";

export type TokenSortOrder = "ascending" | "descending";

// ─── Token item (from query results via events) ─────────────────────

/**
 * A token entry as it appears in the "My Tokens" list.
 * The backend emits these via TaskResultEvent with resultType "Token".
 * Shape is loosely typed because the payload is JSON — screens consuming
 * the store should cast fields as needed.
 */
export interface TokenEntry {
  /** Owning identity ID (hex). */
  identityId: string;
  /** Token ID (hex). */
  tokenId: string;
  /** Contract ID (hex). */
  contractId: string;
  /** Token position within the contract. */
  tokenPosition: number;
  /** Human-readable token name (may be null). */
  name: string | null;
  /** Owner alias (identity alias, may be null). */
  ownerAlias: string | null;
  /** Balance (string representation of u128). */
  balance: string;
  /** Decimals for display formatting. */
  decimals: number;
}

// ─── Search result types ────────────────────────────────────────────

export interface TokenSearchResult {
  /** Contract ID (hex). */
  contractId: string;
  /** Description text. */
  description: string;
}

// ─── Store state ────────────────────────────────────────────────────

interface TokenState {
  /** All "My Tokens" entries. */
  tokens: TokenEntry[];

  /** Search results from keyword search. */
  searchResults: TokenSearchResult[];

  /** Current search keyword. */
  searchKeyword: string;

  /** Pagination cursor for search (hex, null = first page). */
  searchCursor: string | null;

  /** Whether search has more results. */
  searchHasMore: boolean;

  /** Whether a search query is in progress. */
  searching: boolean;

  /** Whether an initial load of "my tokens" is in progress. */
  loading: boolean;

  /** Whether a query/fetch operation is in progress. */
  fetching: boolean;

  /** Whether a refresh of token balances is in progress. */
  refreshing: boolean;

  /** Error message from the last operation. */
  error: string | null;

  /** Current sort column. */
  sortColumn: TokenSortColumn;

  /** Current sort direction. */
  sortOrder: TokenSortOrder;
}

// ─── Store actions ──────────────────────────────────────────────────

interface TokenActions {
  /** Load my token balances from the backend (async — result via event). */
  loadMyTokenBalances: () => Promise<string | null>;

  /** Search tokens by keyword (async — result via event). */
  searchByKeyword: (keyword: string) => Promise<string | null>;

  /** Search next page of results. */
  searchNextPage: () => Promise<string | null>;

  /** Clear search results and keyword. */
  clearSearch: () => void;

  /** Fetch a token by contract ID (async — result via event). */
  fetchTokenByContractId: (contractId: string) => Promise<string | null>;

  /** Fetch a token by token ID (async — result via event). */
  fetchTokenByTokenId: (tokenId: string) => Promise<string | null>;

  /** Save a token locally (async — result via event). */
  saveTokenLocally: (tokenInfoJson: unknown) => Promise<string | null>;

  /** Remove a token from local DB. */
  removeToken: (tokenId: string) => Promise<void>;

  /** Load the saved custom token ordering. */
  loadTokenOrder: () => Promise<IdentityTokenIdentifierDto[]>;

  /** Save custom token ordering. */
  saveTokenOrder: (tokenIds: IdentityTokenIdentifierDto[]) => Promise<void>;

  /** Sort by column (toggles direction if same column). */
  setSortColumn: (column: TokenSortColumn) => void;

  /** Subscribe to task result events for token updates. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Clear error state. */
  clearError: () => void;
}

// ─── Combined store type ────────────────────────────────────────────

export type TokenStore = TokenState & TokenActions;

// ─── Helpers ────────────────────────────────────────────────────────

/** Sort tokens by a column. Returns a new sorted array. */
function sortTokens(
  tokens: TokenEntry[],
  column: TokenSortColumn,
  order: TokenSortOrder,
): TokenEntry[] {
  const sorted = [...tokens];
  const dir = order === "ascending" ? 1 : -1;

  sorted.sort((a, b) => {
    switch (column) {
      case "ownerAlias":
        return dir * (a.ownerAlias ?? "").localeCompare(b.ownerAlias ?? "");
      case "name":
        return dir * (a.name ?? "").localeCompare(b.name ?? "");
      case "balance": {
        // Compare as BigInt for u128 values
        const aBal = BigInt(a.balance || "0");
        const bBal = BigInt(b.balance || "0");
        if (aBal < bBal) return -dir;
        if (aBal > bBal) return dir;
        return 0;
      }
      default:
        return 0;
    }
  });

  return sorted;
}

/**
 * Extract token entries from a TaskResultEvent payload.
 * The backend sends token balance results as an array of token info objects.
 */
function extractTokenEntries(payload: unknown): TokenEntry[] | null {
  if (!payload || typeof payload !== "object") return null;

  // Payload could be an array of token entries or an object with a tokens field
  if (Array.isArray(payload)) {
    return payload.map(normalizeTokenEntry).filter(Boolean) as TokenEntry[];
  }

  const p = payload as Record<string, unknown>;
  if (Array.isArray(p.tokens)) {
    return (p.tokens as unknown[])
      .map(normalizeTokenEntry)
      .filter(Boolean) as TokenEntry[];
  }

  // Single token result
  const entry = normalizeTokenEntry(payload);
  if (entry) return [entry];

  return null;
}

/** Normalize a raw token payload object into a TokenEntry. */
function normalizeTokenEntry(raw: unknown): TokenEntry | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;

  // Must have at least tokenId or token_id
  const tokenId = (r.tokenId ?? r.token_id) as string | undefined;
  if (!tokenId) return null;

  return {
    identityId: ((r.identityId ?? r.identity_id) as string) || "",
    tokenId,
    contractId: ((r.contractId ?? r.contract_id) as string) || "",
    tokenPosition: ((r.tokenPosition ?? r.token_position) as number) || 0,
    name: ((r.name ?? r.token_name) as string) || null,
    ownerAlias: ((r.ownerAlias ?? r.owner_alias) as string) || null,
    balance: String(r.balance ?? "0"),
    decimals: ((r.decimals) as number) ?? 8,
  };
}

/** Extract search results from a TaskResultEvent payload. */
function extractSearchResults(payload: unknown): TokenSearchResult[] | null {
  if (!payload || typeof payload !== "object") return null;

  const p = payload as Record<string, unknown>;
  if (Array.isArray(p.results)) {
    return (p.results as unknown[]).map((r) => {
      const item = r as Record<string, unknown>;
      return {
        contractId: (item.contractId ?? item.contract_id ?? "") as string,
        description: (item.description ?? "") as string,
      };
    });
  }

  return null;
}

// ─── Store implementation ───────────────────────────────────────────

export const useTokenStore = create<TokenStore>((set, get) => ({
  // Initial state
  tokens: [],
  searchResults: [],
  searchKeyword: "",
  searchCursor: null,
  searchHasMore: false,
  searching: false,
  loading: false,
  fetching: false,
  refreshing: false,
  error: null,
  sortColumn: "name",
  sortOrder: "ascending",

  loadMyTokenBalances: async () => {
    set({ loading: true, error: null });
    try {
      const response = await commands.tokenQueryMyBalances();
      return response.taskId;
      // loading will be cleared when the TaskResultEvent arrives
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        loading: false,
      });
      return null;
    }
  },

  searchByKeyword: async (keyword: string) => {
    set({
      searching: true,
      error: null,
      searchKeyword: keyword,
      searchResults: [],
      searchCursor: null,
      searchHasMore: false,
    });
    try {
      const result = await commands.tokenQueryDescriptionsByKeyword({
        keyword,
        startAfter: null,
      });
      if (result.status === "ok") {
        return result.data.taskId;
      }
      set({ error: result.error, searching: false });
      return null;
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        searching: false,
      });
      return null;
    }
  },

  searchNextPage: async () => {
    const { searchKeyword, searchCursor } = get();
    if (!searchKeyword) return null;

    set({ searching: true, error: null });
    try {
      const result = await commands.tokenQueryDescriptionsByKeyword({
        keyword: searchKeyword,
        startAfter: searchCursor,
      });
      if (result.status === "ok") {
        return result.data.taskId;
      }
      set({ error: result.error, searching: false });
      return null;
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        searching: false,
      });
      return null;
    }
  },

  clearSearch: () => {
    set({
      searchResults: [],
      searchKeyword: "",
      searchCursor: null,
      searchHasMore: false,
      searching: false,
    });
  },

  fetchTokenByContractId: async (contractId: string) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.tokenFetchByContractId({ contractId });
      if (result.status === "ok") {
        return result.data.taskId;
      }
      set({ error: result.error, fetching: false });
      return null;
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        fetching: false,
      });
      return null;
    }
  },

  fetchTokenByTokenId: async (tokenId: string) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.tokenFetchByTokenId({ tokenId });
      if (result.status === "ok") {
        return result.data.taskId;
      }
      set({ error: result.error, fetching: false });
      return null;
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        fetching: false,
      });
      return null;
    }
  },

  saveTokenLocally: async (tokenInfoJson: unknown) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.tokenSaveLocally({ tokenInfoJson });
      if (result.status === "ok") {
        return result.data.taskId;
      }
      set({ error: result.error, fetching: false });
      return null;
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        fetching: false,
      });
      return null;
    }
  },

  removeToken: async (tokenId: string) => {
    try {
      const result = await commands.tokenRemove({ tokenId });
      if (result.status === "ok") {
        set((state) => ({
          tokens: state.tokens.filter((t) => t.tokenId !== tokenId),
        }));
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  loadTokenOrder: async () => {
    try {
      const result = await commands.tokenLoadOrder();
      if (result.status === "ok") {
        return result.data;
      }
      set({ error: result.error });
      return [];
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
      return [];
    }
  },

  saveTokenOrder: async (tokenIds: IdentityTokenIdentifierDto[]) => {
    try {
      const result = await commands.tokenSaveOrder({ tokenIds });
      if (result.status !== "ok") {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  setSortColumn: (column: TokenSortColumn) => {
    const { sortColumn, sortOrder } = get();
    if (column === sortColumn) {
      // Toggle direction
      const newOrder =
        sortOrder === "ascending" ? "descending" : "ascending";
      set((state) => ({
        sortOrder: newOrder,
        tokens: sortTokens(state.tokens, column, newOrder),
      }));
    } else {
      // New column, default ascending
      set((state) => ({
        sortColumn: column,
        sortOrder: "ascending",
        tokens: sortTokens(state.tokens, column, "ascending"),
      }));
    }
  },

  subscribeToUpdates: async () => {
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { resultType, payload } = event.payload;

        if (resultType !== "Token") return;

        const state = get();
        const { sortColumn, sortOrder } = state;

        // Try to extract token entries from the payload
        const entries = extractTokenEntries(payload);
        if (entries !== null) {
          set({
            tokens: sortTokens(entries, sortColumn, sortOrder),
            loading: false,
            refreshing: false,
            fetching: false,
          });
          return;
        }

        // Try to extract search results
        const searchResults = extractSearchResults(payload);
        if (searchResults !== null) {
          const p = payload as Record<string, unknown>;
          const cursor = (p.nextCursor ?? p.next_cursor ?? null) as string | null;
          set((s) => ({
            searchResults:
              s.searchCursor !== null
                ? [...s.searchResults, ...searchResults]
                : searchResults,
            searchCursor: cursor,
            searchHasMore: cursor !== null,
            searching: false,
          }));
          return;
        }

        // Fallback — clear fetching states, reload
        set({ fetching: false, loading: false, refreshing: false });
        state.loadMyTokenBalances();
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; message: string } }) => {
        set({
          fetching: false,
          loading: false,
          refreshing: false,
          searching: false,
          error: event.payload.message,
        });
      },
    );

    return () => {
      unlistenResult();
      unlistenError();
    };
  },

  clearError: () => {
    set({ error: null });
  },
}));
