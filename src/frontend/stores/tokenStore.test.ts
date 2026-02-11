/**
 * Tests for the tokenStore Zustand store.
 *
 * Uses centralized mock IPC infrastructure from `@/test/mock-ipc` and
 * centralized fixture factories from `@/test/fixtures` to avoid inline
 * mock definitions and keep test setup consistent across the codebase.
 */

import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { createMockBindings, mockBindingsModule } from "@/test/mock-ipc";
import { createMockToken, createMockTokenSearchResult } from "@/test/fixtures";
import { useTokenStore } from "./tokenStore";
import type { TokenEntry, TokenSearchResult } from "./tokenStore";
import type { TaskResultEvent } from "../bindings";

// ─── Mock bindings ──────────────────────────────────────────────────

vi.mock("../bindings", () => {
  const initial = createMockBindings();
  return mockBindingsModule(initial);
});

import { commands, events } from "../bindings";

// ─── Test fixtures ──────────────────────────────────────────────────

/** Wrapper matching the original inline defaults used throughout these tests. */
function makeToken(overrides?: Partial<TokenEntry>): TokenEntry {
  return createMockToken({
    identityId: "identity001",
    tokenId: "token001",
    contractId: "contract001",
    name: "TestToken",
    balance: "1000000000",
    ...overrides,
  });
}

/** Wrapper matching the original inline defaults used throughout these tests. */
function makeSearchResult(
  overrides?: Partial<TokenSearchResult>,
): TokenSearchResult {
  return createMockTokenSearchResult({
    contractId: "contract001",
    description: "A test token",
    ...overrides,
  });
}

// ─── Reset store between tests ──────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  useTokenStore.setState({
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
  });
});

// ─── Tests ──────────────────────────────────────────────────────────

describe("tokenStore", () => {
  // ─── Initial state ──────────────────────────────────────────

  it("has correct initial state", () => {
    const state = useTokenStore.getState();
    expect(state.tokens).toEqual([]);
    expect(state.searchResults).toEqual([]);
    expect(state.searchKeyword).toBe("");
    expect(state.searchCursor).toBeNull();
    expect(state.searchHasMore).toBe(false);
    expect(state.searching).toBe(false);
    expect(state.loading).toBe(false);
    expect(state.fetching).toBe(false);
    expect(state.refreshing).toBe(false);
    expect(state.error).toBeNull();
    expect(state.sortColumn).toBe("name");
    expect(state.sortOrder).toBe("ascending");
  });

  // ─── loadMyTokenBalances ────────────────────────────────────

  describe("loadMyTokenBalances", () => {
    it("dispatches query and returns task ID", async () => {
      (commands.tokenQueryMyBalances as Mock).mockResolvedValue({
        taskId: "task-100",
      });

      const taskId = await useTokenStore.getState().loadMyTokenBalances();

      expect(taskId).toBe("task-100");
      expect(commands.tokenQueryMyBalances).toHaveBeenCalled();
      expect(useTokenStore.getState().loading).toBe(true);
    });

    it("sets loading state", async () => {
      let resolvePromise: (value: unknown) => void;
      (commands.tokenQueryMyBalances as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolvePromise = resolve;
        }),
      );

      const loadPromise = useTokenStore.getState().loadMyTokenBalances();
      expect(useTokenStore.getState().loading).toBe(true);

      resolvePromise!({ taskId: "task-100" });
      await loadPromise;
    });

    it("handles thrown exception", async () => {
      (commands.tokenQueryMyBalances as Mock).mockRejectedValue(
        new Error("Network error"),
      );

      const taskId = await useTokenStore.getState().loadMyTokenBalances();

      expect(taskId).toBeNull();
      expect(useTokenStore.getState().error).toBe("Network error");
      expect(useTokenStore.getState().loading).toBe(false);
    });
  });

  // ─── searchByKeyword ────────────────────────────────────────

  describe("searchByKeyword", () => {
    it("dispatches search and returns task ID", async () => {
      (commands.tokenQueryDescriptionsByKeyword as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-200" },
      });

      const taskId = await useTokenStore.getState().searchByKeyword("dash");

      expect(taskId).toBe("task-200");
      expect(commands.tokenQueryDescriptionsByKeyword).toHaveBeenCalledWith({
        keyword: "dash",
        startAfter: null,
      });
      expect(useTokenStore.getState().searching).toBe(true);
      expect(useTokenStore.getState().searchKeyword).toBe("dash");
    });

    it("clears previous results when starting new search", async () => {
      useTokenStore.setState({
        searchResults: [makeSearchResult()],
        searchCursor: "cursor123",
        searchHasMore: true,
      });
      (commands.tokenQueryDescriptionsByKeyword as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-201" },
      });

      await useTokenStore.getState().searchByKeyword("new search");

      expect(useTokenStore.getState().searchResults).toEqual([]);
      expect(useTokenStore.getState().searchCursor).toBeNull();
      expect(useTokenStore.getState().searchHasMore).toBe(false);
    });

    it("handles error from backend", async () => {
      (commands.tokenQueryDescriptionsByKeyword as Mock).mockResolvedValue({
        status: "error",
        error: "Invalid keyword",
      });

      const taskId = await useTokenStore.getState().searchByKeyword("");

      expect(taskId).toBeNull();
      expect(useTokenStore.getState().error).toBe("Invalid keyword");
      expect(useTokenStore.getState().searching).toBe(false);
    });

    it("handles thrown exception", async () => {
      (commands.tokenQueryDescriptionsByKeyword as Mock).mockRejectedValue(
        new Error("Connection failed"),
      );

      const taskId =
        await useTokenStore.getState().searchByKeyword("test");

      expect(taskId).toBeNull();
      expect(useTokenStore.getState().error).toBe("Connection failed");
      expect(useTokenStore.getState().searching).toBe(false);
    });
  });

  // ─── searchNextPage ─────────────────────────────────────────

  describe("searchNextPage", () => {
    it("uses cursor for pagination", async () => {
      useTokenStore.setState({
        searchKeyword: "dash",
        searchCursor: "cursor456",
      });
      (commands.tokenQueryDescriptionsByKeyword as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-300" },
      });

      const taskId = await useTokenStore.getState().searchNextPage();

      expect(taskId).toBe("task-300");
      expect(commands.tokenQueryDescriptionsByKeyword).toHaveBeenCalledWith({
        keyword: "dash",
        startAfter: "cursor456",
      });
    });

    it("returns null when no search keyword", async () => {
      const taskId = await useTokenStore.getState().searchNextPage();

      expect(taskId).toBeNull();
      expect(commands.tokenQueryDescriptionsByKeyword).not.toHaveBeenCalled();
    });
  });

  // ─── clearSearch ────────────────────────────────────────────

  describe("clearSearch", () => {
    it("resets all search state", () => {
      useTokenStore.setState({
        searchResults: [makeSearchResult()],
        searchKeyword: "dash",
        searchCursor: "cursor789",
        searchHasMore: true,
        searching: true,
      });

      useTokenStore.getState().clearSearch();

      const state = useTokenStore.getState();
      expect(state.searchResults).toEqual([]);
      expect(state.searchKeyword).toBe("");
      expect(state.searchCursor).toBeNull();
      expect(state.searchHasMore).toBe(false);
      expect(state.searching).toBe(false);
    });
  });

  // ─── fetchTokenByContractId ─────────────────────────────────

  describe("fetchTokenByContractId", () => {
    it("dispatches fetch and returns task ID", async () => {
      (commands.tokenFetchByContractId as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-400" },
      });

      const taskId = await useTokenStore
        .getState()
        .fetchTokenByContractId("contract001");

      expect(taskId).toBe("task-400");
      expect(commands.tokenFetchByContractId).toHaveBeenCalledWith({
        contractId: "contract001",
      });
      expect(useTokenStore.getState().fetching).toBe(true);
    });

    it("returns null and sets error on failure", async () => {
      (commands.tokenFetchByContractId as Mock).mockResolvedValue({
        status: "error",
        error: "Contract not found",
      });

      const taskId = await useTokenStore
        .getState()
        .fetchTokenByContractId("bad-id");

      expect(taskId).toBeNull();
      expect(useTokenStore.getState().error).toBe("Contract not found");
      expect(useTokenStore.getState().fetching).toBe(false);
    });
  });

  // ─── fetchTokenByTokenId ────────────────────────────────────

  describe("fetchTokenByTokenId", () => {
    it("dispatches fetch and returns task ID", async () => {
      (commands.tokenFetchByTokenId as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-500" },
      });

      const taskId = await useTokenStore
        .getState()
        .fetchTokenByTokenId("token001");

      expect(taskId).toBe("task-500");
      expect(commands.tokenFetchByTokenId).toHaveBeenCalledWith({
        tokenId: "token001",
      });
      expect(useTokenStore.getState().fetching).toBe(true);
    });

    it("handles thrown exception", async () => {
      (commands.tokenFetchByTokenId as Mock).mockRejectedValue(
        new Error("Network timeout"),
      );

      const taskId = await useTokenStore
        .getState()
        .fetchTokenByTokenId("token001");

      expect(taskId).toBeNull();
      expect(useTokenStore.getState().error).toBe("Network timeout");
      expect(useTokenStore.getState().fetching).toBe(false);
    });
  });

  // ─── saveTokenLocally ───────────────────────────────────────

  describe("saveTokenLocally", () => {
    it("dispatches save and returns task ID", async () => {
      (commands.tokenSaveLocally as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-600" },
      });

      const taskId = await useTokenStore
        .getState()
        .saveTokenLocally({ name: "MyToken" });

      expect(taskId).toBe("task-600");
      expect(commands.tokenSaveLocally).toHaveBeenCalledWith({
        tokenInfoJson: { name: "MyToken" },
      });
      expect(useTokenStore.getState().fetching).toBe(true);
    });

    it("returns null and sets error on failure", async () => {
      (commands.tokenSaveLocally as Mock).mockResolvedValue({
        status: "error",
        error: "Save failed",
      });

      const taskId = await useTokenStore
        .getState()
        .saveTokenLocally({});

      expect(taskId).toBeNull();
      expect(useTokenStore.getState().error).toBe("Save failed");
    });
  });

  // ─── removeToken ────────────────────────────────────────────

  describe("removeToken", () => {
    it("removes token from list", async () => {
      useTokenStore.setState({
        tokens: [
          makeToken(),
          makeToken({ tokenId: "token002", name: "Other" }),
        ],
      });
      (commands.tokenRemove as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useTokenStore.getState().removeToken("token001");

      const tokens = useTokenStore.getState().tokens;
      expect(tokens).toHaveLength(1);
      expect(tokens[0].tokenId).toBe("token002");
    });

    it("sets error on failure", async () => {
      useTokenStore.setState({ tokens: [makeToken()] });
      (commands.tokenRemove as Mock).mockResolvedValue({
        status: "error",
        error: "Cannot remove token",
      });

      await useTokenStore.getState().removeToken("token001");

      expect(useTokenStore.getState().error).toBe("Cannot remove token");
      expect(useTokenStore.getState().tokens).toHaveLength(1);
    });

    it("handles thrown exception", async () => {
      useTokenStore.setState({ tokens: [makeToken()] });
      (commands.tokenRemove as Mock).mockRejectedValue(
        new Error("DB error"),
      );

      await useTokenStore.getState().removeToken("token001");

      expect(useTokenStore.getState().error).toBe("DB error");
    });
  });

  // ─── loadTokenOrder ─────────────────────────────────────────

  describe("loadTokenOrder", () => {
    it("returns order from backend", async () => {
      const order = [
        { identityId: "id1", tokenId: "t1" },
        { identityId: "id2", tokenId: "t2" },
      ];
      (commands.tokenLoadOrder as Mock).mockResolvedValue({
        status: "ok",
        data: order,
      });

      const result = await useTokenStore.getState().loadTokenOrder();

      expect(result).toEqual(order);
    });

    it("returns empty array on error", async () => {
      (commands.tokenLoadOrder as Mock).mockResolvedValue({
        status: "error",
        error: "No order saved",
      });

      const result = await useTokenStore.getState().loadTokenOrder();

      expect(result).toEqual([]);
      expect(useTokenStore.getState().error).toBe("No order saved");
    });
  });

  // ─── saveTokenOrder ─────────────────────────────────────────

  describe("saveTokenOrder", () => {
    it("saves order to backend", async () => {
      (commands.tokenSaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      const order = [{ identityId: "id1", tokenId: "t1" }];
      await useTokenStore.getState().saveTokenOrder(order);

      expect(commands.tokenSaveOrder).toHaveBeenCalledWith({
        tokenIds: order,
      });
      expect(useTokenStore.getState().error).toBeNull();
    });

    it("sets error on failure", async () => {
      (commands.tokenSaveOrder as Mock).mockResolvedValue({
        status: "error",
        error: "Save order failed",
      });

      await useTokenStore
        .getState()
        .saveTokenOrder([{ identityId: "id1", tokenId: "t1" }]);

      expect(useTokenStore.getState().error).toBe("Save order failed");
    });
  });

  // ─── reorderTokens (drag-and-drop) ─────────────────────────

  describe("reorderTokens", () => {
    it("reorders token groups by moving a token to a new position", async () => {
      const t1 = makeToken({ tokenId: "token_a", name: "Alpha", identityId: "id1" });
      const t2 = makeToken({ tokenId: "token_b", name: "Beta", identityId: "id1" });
      const t3 = makeToken({ tokenId: "token_c", name: "Gamma", identityId: "id1" });
      useTokenStore.setState({ tokens: [t1, t2, t3] });
      (commands.tokenSaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useTokenStore.getState().reorderTokens("token_c", "token_a");

      const tokenIds = useTokenStore.getState().tokens.map((t) => t.tokenId);
      expect(tokenIds).toEqual(["token_c", "token_a", "token_b"]);
    });

    it("does nothing when activeId equals overId", async () => {
      const t1 = makeToken({ tokenId: "token_a" });
      useTokenStore.setState({ tokens: [t1] });

      await useTokenStore.getState().reorderTokens("token_a", "token_a");

      expect(commands.tokenSaveOrder).not.toHaveBeenCalled();
    });

    it("persists the new token order", async () => {
      const t1 = makeToken({ tokenId: "token_a", identityId: "id1" });
      const t2 = makeToken({ tokenId: "token_b", identityId: "id2" });
      useTokenStore.setState({ tokens: [t1, t2] });
      (commands.tokenSaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useTokenStore.getState().reorderTokens("token_b", "token_a");

      expect(commands.tokenSaveOrder).toHaveBeenCalledWith({
        tokenIds: [
          { identityId: "id2", tokenId: "token_b" },
          { identityId: "id1", tokenId: "token_a" },
        ],
      });
    });

    it("preserves multiple entries per token group", async () => {
      const t1a = makeToken({ tokenId: "token_a", identityId: "id1", name: "Alpha" });
      const t1b = makeToken({ tokenId: "token_a", identityId: "id2", name: "Alpha" });
      const t2 = makeToken({ tokenId: "token_b", identityId: "id1", name: "Beta" });
      useTokenStore.setState({ tokens: [t1a, t1b, t2] });
      (commands.tokenSaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useTokenStore.getState().reorderTokens("token_b", "token_a");

      const state = useTokenStore.getState();
      expect(state.tokens.map((t) => t.tokenId)).toEqual(["token_b", "token_a", "token_a"]);
      expect(state.tokens.map((t) => t.identityId)).toEqual(["id1", "id1", "id2"]);
    });
  });

  // ─── setSortColumn ──────────────────────────────────────────

  describe("setSortColumn", () => {
    it("sorts by new column ascending", () => {
      useTokenStore.setState({
        sortColumn: "balance",
        sortOrder: "descending",
        tokens: [
          makeToken({ name: "Zebra" }),
          makeToken({ tokenId: "t2", name: "Alpha" }),
        ],
      });

      useTokenStore.getState().setSortColumn("name");

      const state = useTokenStore.getState();
      expect(state.sortColumn).toBe("name");
      expect(state.sortOrder).toBe("ascending");
      expect(state.tokens[0].name).toBe("Alpha");
      expect(state.tokens[1].name).toBe("Zebra");
    });

    it("toggles direction on same column", () => {
      useTokenStore.setState({
        sortColumn: "name",
        sortOrder: "ascending",
        tokens: [
          makeToken({ name: "Alpha" }),
          makeToken({ tokenId: "t2", name: "Zebra" }),
        ],
      });

      useTokenStore.getState().setSortColumn("name");

      const state = useTokenStore.getState();
      expect(state.sortOrder).toBe("descending");
      expect(state.tokens[0].name).toBe("Zebra");
      expect(state.tokens[1].name).toBe("Alpha");
    });

    it("sorts by balance using BigInt comparison", () => {
      useTokenStore.setState({
        tokens: [
          makeToken({ balance: "999999999999" }),
          makeToken({ tokenId: "t2", balance: "1000000000000" }),
          makeToken({ tokenId: "t3", balance: "1" }),
        ],
      });

      useTokenStore.getState().setSortColumn("balance");

      const balances = useTokenStore.getState().tokens.map((t) => t.balance);
      expect(balances).toEqual(["1", "999999999999", "1000000000000"]);
    });

    it("sorts by owner alias", () => {
      useTokenStore.setState({
        tokens: [
          makeToken({ ownerAlias: "Charlie" }),
          makeToken({ tokenId: "t2", ownerAlias: "Alice" }),
          makeToken({ tokenId: "t3", ownerAlias: null }),
        ],
      });

      useTokenStore.getState().setSortColumn("ownerAlias");

      const aliases = useTokenStore.getState().tokens.map((t) => t.ownerAlias);
      expect(aliases).toEqual([null, "Alice", "Charlie"]);
    });
  });

  // ─── subscribeToUpdates ─────────────────────────────────────

  describe("subscribeToUpdates", () => {
    it("subscribes to task result and error events", async () => {
      await useTokenStore.getState().subscribeToUpdates();

      expect(events.taskResultEvent.listen).toHaveBeenCalledTimes(1);
      expect(events.taskErrorEvent.listen).toHaveBeenCalledTimes(1);
    });

    it("returns unsubscribe function", async () => {
      const unsubResult = vi.fn();
      const unsubError = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(unsubResult);
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unsubError);

      const unsub = await useTokenStore.getState().subscribeToUpdates();
      unsub();

      expect(unsubResult).toHaveBeenCalled();
      expect(unsubError).toHaveBeenCalled();
    });

    it("updates tokens on Token result event with array payload", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useTokenStore.setState({ loading: true });
      await useTokenStore.getState().subscribeToUpdates();

      // Simulate token result event with tokens array
      resultCallback!({
        payload: {
          taskId: "task-100",
          resultType: "Token",
          payload: {
            tokens: [
              {
                tokenId: "t1",
                identityId: "id1",
                contractId: "c1",
                tokenPosition: 0,
                name: "MyToken",
                ownerAlias: "Alice",
                balance: "5000",
                decimals: 8,
              },
            ],
          },
        },
      });

      const state = useTokenStore.getState();
      expect(state.tokens).toHaveLength(1);
      expect(state.tokens[0].tokenId).toBe("t1");
      expect(state.tokens[0].name).toBe("MyToken");
      expect(state.loading).toBe(false);
    });

    it("ignores non-Token result events", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useTokenStore.setState({ fetching: true });
      await useTokenStore.getState().subscribeToUpdates();

      // Simulate an Identity result event
      resultCallback!({
        payload: {
          taskId: "task-456",
          resultType: "Identity",
          payload: null,
        },
      });

      // Fetching should not change for non-Token events
      expect(useTokenStore.getState().fetching).toBe(true);
    });

    it("handles search results in Token event", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      // Mock loadMyTokenBalances for the fallback
      (commands.tokenQueryMyBalances as Mock).mockResolvedValue({
        taskId: "task-reload",
      });

      useTokenStore.setState({ searching: true, searchKeyword: "test" });
      await useTokenStore.getState().subscribeToUpdates();

      // Simulate search result event
      resultCallback!({
        payload: {
          taskId: "task-200",
          resultType: "Token",
          payload: {
            results: [
              { contractId: "c1", description: "Token A" },
              { contractId: "c2", description: "Token B" },
            ],
            nextCursor: "cursor999",
          },
        },
      });

      const state = useTokenStore.getState();
      expect(state.searchResults).toHaveLength(2);
      expect(state.searchResults[0].contractId).toBe("c1");
      expect(state.searchCursor).toBe("cursor999");
      expect(state.searchHasMore).toBe(true);
      expect(state.searching).toBe(false);
    });

    it("appends search results on pagination", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useTokenStore.setState({
        searching: true,
        searchKeyword: "test",
        searchCursor: "cursor-page1",
        searchResults: [makeSearchResult({ contractId: "c0" })],
      });
      await useTokenStore.getState().subscribeToUpdates();

      // Simulate next page result
      resultCallback!({
        payload: {
          taskId: "task-301",
          resultType: "Token",
          payload: {
            results: [{ contractId: "c3", description: "Token C" }],
            nextCursor: null,
          },
        },
      });

      const state = useTokenStore.getState();
      expect(state.searchResults).toHaveLength(2);
      expect(state.searchResults[0].contractId).toBe("c0");
      expect(state.searchResults[1].contractId).toBe("c3");
      expect(state.searchHasMore).toBe(false);
    });

    it("sets error on task error event", async () => {
      let errorCallback: (event: {
        payload: { taskId: string; message: string };
      }) => void;
      (events.taskErrorEvent.listen as Mock).mockImplementation(
        async (
          cb: (event: {
            payload: { taskId: string; message: string };
          }) => void,
        ) => {
          errorCallback = cb;
          return () => {};
        },
      );

      useTokenStore.setState({ fetching: true, searching: true });
      await useTokenStore.getState().subscribeToUpdates();

      errorCallback!({
        payload: { taskId: "task-999", message: "Backend crash" },
      });

      const state = useTokenStore.getState();
      expect(state.error).toBe("Backend crash");
      expect(state.fetching).toBe(false);
      expect(state.loading).toBe(false);
      expect(state.searching).toBe(false);
    });
  });

  // ─── clearError ─────────────────────────────────────────────

  describe("clearError", () => {
    it("clears the error state", () => {
      useTokenStore.setState({ error: "Something went wrong" });

      useTokenStore.getState().clearError();

      expect(useTokenStore.getState().error).toBeNull();
    });
  });
});
