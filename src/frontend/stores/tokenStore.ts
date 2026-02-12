import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  IdentityTokenBalanceDto,
  IdentityTokenIdentifierDto,
  JsonValue,
  TaskResultEvent,
} from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

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

  /** Save a token locally (synchronous DB insert). */
  saveTokenLocally: (input: {
    tokenId: string;
    contractId: string;
    tokenPosition: number;
    tokenName: string;
  }) => Promise<void>;

  /** Remove a token from local DB. */
  removeToken: (tokenId: string) => Promise<void>;

  /** Load the saved custom token ordering. */
  loadTokenOrder: () => Promise<IdentityTokenIdentifierDto[]>;

  /** Save custom token ordering. */
  saveTokenOrder: (tokenIds: IdentityTokenIdentifierDto[]) => Promise<void>;

  /** Reorder tokens by moving a token from one position to another (drag-and-drop). */
  reorderTokens: (activeTokenId: string, overTokenId: string) => Promise<void>;

  /** Sort by column (toggles direction if same column). */
  setSortColumn: (column: TokenSortColumn) => void;

  /** Subscribe to task result events for token updates. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Reset all state (used on network switch). */
  resetState: () => void;

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

// ─── Task timeout manager ────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

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
      timeouts.start("loadBalances", () => {
        set({ loading: false, fetching: false, searching: false, refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
      });
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
        timeouts.start("search", () => {
          set({ loading: false, fetching: false, searching: false, refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
        });
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
        timeouts.start("searchNext", () => {
          set({ loading: false, fetching: false, searching: false, refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
        });
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
        timeouts.start("fetchByContract", () => {
          set({ loading: false, fetching: false, searching: false, refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
        });
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
        timeouts.start("fetchByToken", () => {
          set({ loading: false, fetching: false, searching: false, refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
        });
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

  saveTokenLocally: async (input) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.tokenSaveLocally(input);
      if (result.status === "ok") {
        set({ fetching: false });
      } else {
        set({ error: result.error, fetching: false });
      }
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        fetching: false,
      });
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

  reorderTokens: async (activeTokenId, overTokenId) => {
    if (activeTokenId === overTokenId) return;
    const { tokens } = get();

    // Group by tokenId to get unique token ordering
    const tokenIds: string[] = [];
    for (const t of tokens) {
      if (!tokenIds.includes(t.tokenId)) tokenIds.push(t.tokenId);
    }

    const oldIndex = tokenIds.indexOf(activeTokenId);
    const newIndex = tokenIds.indexOf(overTokenId);
    if (oldIndex < 0 || newIndex < 0) return;

    // Reorder the tokenId list
    const reorderedIds = [...tokenIds];
    const [moved] = reorderedIds.splice(oldIndex, 1);
    reorderedIds.splice(newIndex, 0, moved ?? "");

    // Rebuild token entries array in new group order
    const byTokenId = new Map<string, TokenEntry[]>();
    for (const t of tokens) {
      const arr = byTokenId.get(t.tokenId) || [];
      arr.push(t);
      byTokenId.set(t.tokenId, arr);
    }
    const reordered: TokenEntry[] = [];
    for (const id of reorderedIds) {
      const entries = byTokenId.get(id);
      if (entries) reordered.push(...entries);
    }

    set({ tokens: reordered });

    // Persist — build IdentityTokenIdentifierDto list
    const dtos: IdentityTokenIdentifierDto[] = reordered.map((t) => ({
      identityId: t.identityId,
      tokenId: t.tokenId,
    }));
    try {
      await commands.tokenSaveOrder({ tokenIds: dtos });
    } catch {
      // Best effort
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
        const { result } = event.payload;

        // Handle token search results (transient — not in DB)
        if (result.type === "tokenSearchResults") {
          timeouts.clearAll();
          const items: TokenSearchResult[] = (result.results ?? []).map(
            (r: { contractId: string; description: string }) => ({
              contractId: r.contractId,
              description: r.description,
            }),
          );
          set({
            searchResults: items,
            searchHasMore: result.hasMore ?? false,
            searching: false,
          });
          return;
        }

        // Handle token not found
        if (result.type === "tokenNotFound") {
          timeouts.clearAll();
          set({
            fetching: false,
            error: "Token not found on Platform.",
          });
          return;
        }

        // Token pricing and reward estimates are handled by screen-level
        // listeners, not the store. Skip them here.
        if (
          result.type === "tokenPricing" ||
          result.type === "tokenRewardEstimate"
        ) {
          return;
        }

        // Token balances loaded — read from DB and populate the store
        if (result.type === "tokenBalancesLoaded") {
          timeouts.clearAll();
          set({ fetching: false, loading: false, refreshing: false });
          // Read the actual balance data from the local DB
          commands.tokenGetMyBalances().then((res) => {
            if (res.status === "ok") {
              const { sortColumn, sortOrder } = get();
              const entries: TokenEntry[] = res.data.map(
                (b: IdentityTokenBalanceDto) => ({
                  identityId: b.identityId,
                  tokenId: b.tokenId,
                  contractId: b.dataContractId,
                  tokenPosition: b.tokenPosition,
                  name: b.tokenAlias || null,
                  ownerAlias: null,
                  balance: b.balance,
                  decimals: b.decimals,
                }),
              );
              set({ tokens: sortTokens(entries, sortColumn, sortOrder) });
            }
          });
          return;
        }

        if (result.type !== "tokenCompleted") return;

        timeouts.clearAll();

        // Mutation completed (mint, burn, transfer, etc.) — refresh balances
        set({ fetching: false, refreshing: false });
        get().loadMyTokenBalances();
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "token") return;

        timeouts.clearAll();

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

  resetState: () => {
    timeouts.clearAll();
    set({
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
    });
  },

  clearError: () => {
    set({ error: null });
  },
}));
