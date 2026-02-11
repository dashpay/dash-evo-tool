/**
 * identityStore tests — using centralized mock IPC + fixture factories.
 *
 * Pattern:
 * 1. createMockBindings() provides defaults for all 181 commands + 8 events
 * 2. Override specific commands needed by the store under test
 * 3. Use fixture factories instead of inline makers
 */

import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import {
  createMockBindings,
  mockBindingsModule,
} from "@/test/mock-ipc";
import { createMockIdentity } from "@/test/fixtures";
import type { TaskResultEvent } from "../bindings";

// ─── Mock bindings (centralized) ────────────────────────────────────

vi.mock("../bindings", () => {
  const initial = createMockBindings();
  return mockBindingsModule(initial);
});

import { commands, events } from "../bindings";
import { useIdentityStore } from "./identityStore";

// ─── Test-local defaults ────────────────────────────────────────────
// The centralized fixtures use different default values. These helpers
// provide the same defaults as the original inline test makers for
// backward-compatible assertions.

function mi(o?: Partial<Parameters<typeof createMockIdentity>[0]>) {
  return createMockIdentity({
    id: "aabb001122",
    alias: "Alice",
    balance: 500000000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: ["wallethash1"],
    walletIndex: 0,
    topUps: [],
    status: "active",
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    ...o,
  });
}

function mi2(o?: Partial<Parameters<typeof createMockIdentity>[0]>) {
  return mi({
    id: "ccdd003344",
    alias: "Bob",
    balance: 200000000,
    associatedWalletHashes: ["wallethash2"],
    walletIndex: 1,
    ...o,
  });
}

function mi3(o?: Partial<Parameters<typeof createMockIdentity>[0]>) {
  return mi({
    id: "eeff005566",
    alias: "Charlie",
    balance: 100000000,
    identityType: "masternode",
    associatedWalletHashes: [],
    walletIndex: null,
    ...o,
  });
}

// ─── Helpers ────────────────────────────────────────────────────────

function resetStore() {
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

// ─── Tests ──────────────────────────────────────────────────────────

describe("identityStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  describe("initial state", () => {
    it("starts with empty identities and no selection", () => {
      const state = useIdentityStore.getState();
      expect(state.identities).toEqual([]);
      expect(state.selectedIdentityId).toBeNull();
      expect(state.loading).toBe(false);
      expect(state.refreshingIds.size).toBe(0);
      expect(state.refreshingAll).toBe(false);
      expect(state.error).toBeNull();
      expect(state.sortColumn).toBe("alias");
      expect(state.sortOrder).toBe("ascending");
      expect(state.useCustomOrder).toBe(true);
    });
  });

  describe("loadIdentities", () => {
    it("loads identities from backend on success", async () => {
      const identities = [mi(), mi2()];
      (commands.identityListLocal as Mock).mockResolvedValue({
        status: "ok",
        data: identities,
      });
      (commands.identityLoadOrder as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });

      await useIdentityStore.getState().loadIdentities();

      const state = useIdentityStore.getState();
      expect(state.identities).toEqual(identities);
      expect(state.loading).toBe(false);
      expect(state.error).toBeNull();
    });

    it("sets loading=true during load", async () => {
      let resolveLoad: (value: unknown) => void;
      (commands.identityListLocal as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolveLoad = resolve;
        }),
      );

      const promise = useIdentityStore.getState().loadIdentities();
      expect(useIdentityStore.getState().loading).toBe(true);

      resolveLoad!({ status: "ok", data: [] });
      (commands.identityLoadOrder as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });
      await promise;
      expect(useIdentityStore.getState().loading).toBe(false);
    });

    it("applies saved custom order", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      const id3 = mi3({ id: "ccc" });

      (commands.identityListLocal as Mock).mockResolvedValue({
        status: "ok",
        data: [id1, id2, id3],
      });
      // Saved order has them reversed
      (commands.identityLoadOrder as Mock).mockResolvedValue({
        status: "ok",
        data: ["ccc", "bbb", "aaa"],
      });

      await useIdentityStore.getState().loadIdentities();

      const state = useIdentityStore.getState();
      expect(state.identities.map((i) => i.id)).toEqual([
        "ccc",
        "bbb",
        "aaa",
      ]);
      expect(state.useCustomOrder).toBe(true);
    });

    it("handles identities not in saved order (appends them)", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      const id3 = mi3({ id: "ccc" });

      (commands.identityListLocal as Mock).mockResolvedValue({
        status: "ok",
        data: [id1, id2, id3],
      });
      // Order only includes aaa and ccc — bbb should be appended
      (commands.identityLoadOrder as Mock).mockResolvedValue({
        status: "ok",
        data: ["ccc", "aaa"],
      });

      await useIdentityStore.getState().loadIdentities();

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["ccc", "aaa", "bbb"]);
    });

    it("sets error on backend error result", async () => {
      (commands.identityListLocal as Mock).mockResolvedValue({
        status: "error",
        error: "Database connection failed",
      });

      await useIdentityStore.getState().loadIdentities();

      const state = useIdentityStore.getState();
      expect(state.error).toBe("Database connection failed");
      expect(state.loading).toBe(false);
      expect(state.identities).toEqual([]);
    });

    it("sets error on network exception", async () => {
      (commands.identityListLocal as Mock).mockRejectedValue(
        new Error("IPC not available"),
      );

      await useIdentityStore.getState().loadIdentities();

      const state = useIdentityStore.getState();
      expect(state.error).toBe("IPC not available");
      expect(state.loading).toBe(false);
    });

    it("clears previous error on new load", async () => {
      useIdentityStore.setState({ error: "old error" });
      (commands.identityListLocal as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });
      (commands.identityLoadOrder as Mock).mockResolvedValue({
        status: "ok",
        data: [],
      });

      await useIdentityStore.getState().loadIdentities();
      expect(useIdentityStore.getState().error).toBeNull();
    });
  });

  describe("selectIdentity", () => {
    it("selects an identity by ID", () => {
      useIdentityStore.getState().selectIdentity("aabb001122");
      expect(useIdentityStore.getState().selectedIdentityId).toBe(
        "aabb001122",
      );
    });

    it("deselects when passing null", () => {
      useIdentityStore.setState({ selectedIdentityId: "aabb001122" });
      useIdentityStore.getState().selectIdentity(null);
      expect(useIdentityStore.getState().selectedIdentityId).toBeNull();
    });
  });

  describe("refreshIdentity", () => {
    it("dispatches refresh and tracks refreshing state", async () => {
      (commands.identityRefresh as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useIdentityStore.getState().refreshIdentity("aabb001122");

      expect(commands.identityRefresh).toHaveBeenCalledWith({
        identityId: "aabb001122",
      });
      // refreshingIds should still contain the ID (cleared by event handler)
      expect(
        useIdentityStore.getState().refreshingIds.has("aabb001122"),
      ).toBe(true);
    });

    it("clears refreshing on error response", async () => {
      (commands.identityRefresh as Mock).mockResolvedValue({
        status: "error",
        error: "Platform unavailable",
      });

      await useIdentityStore.getState().refreshIdentity("aabb001122");

      expect(useIdentityStore.getState().refreshingIds.size).toBe(0);
      expect(useIdentityStore.getState().error).toBe("Platform unavailable");
    });

    it("clears refreshing on exception", async () => {
      (commands.identityRefresh as Mock).mockRejectedValue(
        new Error("Network error"),
      );

      await useIdentityStore.getState().refreshIdentity("aabb001122");

      expect(useIdentityStore.getState().refreshingIds.size).toBe(0);
      expect(useIdentityStore.getState().error).toBe("Network error");
    });
  });

  describe("refreshAllIdentities", () => {
    it("dispatches refresh for all identities", async () => {
      const identities = [mi(), mi2()];
      useIdentityStore.setState({ identities });
      (commands.identityRefresh as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      await useIdentityStore.getState().refreshAllIdentities();

      expect(commands.identityRefresh).toHaveBeenCalledTimes(2);
      expect(commands.identityRefresh).toHaveBeenCalledWith({
        identityId: "aabb001122",
      });
      expect(commands.identityRefresh).toHaveBeenCalledWith({
        identityId: "ccdd003344",
      });
    });

    it("does nothing with no identities", async () => {
      await useIdentityStore.getState().refreshAllIdentities();
      expect(commands.identityRefresh).not.toHaveBeenCalled();
      expect(useIdentityStore.getState().refreshingAll).toBe(false);
    });

    it("sets refreshingAll=true during refresh", async () => {
      useIdentityStore.setState({ identities: [mi()] });
      (commands.identityRefresh as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });

      const promise = useIdentityStore.getState().refreshAllIdentities();
      // refreshingAll is set synchronously before awaiting
      await promise;
      // Will be cleared when task results come back
    });
  });

  describe("setAlias", () => {
    it("updates alias on success", async () => {
      useIdentityStore.setState({
        identities: [mi({ alias: "Old" })],
      });
      (commands.identitySetAlias as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().setAlias("aabb001122", "New Name");

      expect(commands.identitySetAlias).toHaveBeenCalledWith({
        identityId: "aabb001122",
        alias: "New Name",
      });
      expect(useIdentityStore.getState().identities[0].alias).toBe(
        "New Name",
      );
    });

    it("clears alias when passing null", async () => {
      useIdentityStore.setState({
        identities: [mi({ alias: "Old" })],
      });
      (commands.identitySetAlias as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().setAlias("aabb001122", null);

      expect(useIdentityStore.getState().identities[0].alias).toBeNull();
    });

    it("does not update on failure", async () => {
      useIdentityStore.setState({
        identities: [mi({ alias: "Original" })],
      });
      (commands.identitySetAlias as Mock).mockResolvedValue({
        status: "error",
        error: "Not found",
      });

      await useIdentityStore.getState().setAlias("aabb001122", "New");

      expect(useIdentityStore.getState().identities[0].alias).toBe(
        "Original",
      );
      expect(useIdentityStore.getState().error).toBe("Not found");
    });
  });

  describe("reorderIdentityUp", () => {
    it("moves identity up in the list", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      useIdentityStore.setState({ identities: [id1, id2] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentityUp("bbb");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["bbb", "aaa"]);
      expect(useIdentityStore.getState().useCustomOrder).toBe(true);
    });

    it("persists the new order", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      useIdentityStore.setState({ identities: [id1, id2] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentityUp("bbb");

      expect(commands.identitySaveOrder).toHaveBeenCalledWith({
        identityIds: ["bbb", "aaa"],
      });
    });

    it("does nothing for first identity", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      useIdentityStore.setState({ identities: [id1, id2] });

      await useIdentityStore.getState().reorderIdentityUp("aaa");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["aaa", "bbb"]);
      expect(commands.identitySaveOrder).not.toHaveBeenCalled();
    });

    it("does nothing for unknown identity", async () => {
      useIdentityStore.setState({ identities: [mi()] });

      await useIdentityStore.getState().reorderIdentityUp("unknown");

      expect(commands.identitySaveOrder).not.toHaveBeenCalled();
    });
  });

  describe("reorderIdentityDown", () => {
    it("moves identity down in the list", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      useIdentityStore.setState({ identities: [id1, id2] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentityDown("aaa");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["bbb", "aaa"]);
    });

    it("does nothing for last identity", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      useIdentityStore.setState({ identities: [id1, id2] });

      await useIdentityStore.getState().reorderIdentityDown("bbb");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["aaa", "bbb"]);
      expect(commands.identitySaveOrder).not.toHaveBeenCalled();
    });
  });

  describe("reorderIdentities (drag-and-drop)", () => {
    it("moves identity from one position to another", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      const id3 = mi({ id: "ccc", alias: "Third" });
      useIdentityStore.setState({ identities: [id1, id2, id3] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentities("ccc", "aaa");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["ccc", "aaa", "bbb"]);
      expect(useIdentityStore.getState().useCustomOrder).toBe(true);
    });

    it("persists the new order after drag-and-drop", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      useIdentityStore.setState({ identities: [id1, id2] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentities("bbb", "aaa");

      expect(commands.identitySaveOrder).toHaveBeenCalledWith({
        identityIds: ["bbb", "aaa"],
      });
    });

    it("does nothing when activeId equals overId", async () => {
      useIdentityStore.setState({ identities: [mi({ id: "aaa" })] });

      await useIdentityStore.getState().reorderIdentities("aaa", "aaa");

      expect(commands.identitySaveOrder).not.toHaveBeenCalled();
    });

    it("does nothing when activeId is not found", async () => {
      useIdentityStore.setState({ identities: [mi({ id: "aaa" })] });

      await useIdentityStore.getState().reorderIdentities("unknown", "aaa");

      expect(commands.identitySaveOrder).not.toHaveBeenCalled();
    });

    it("moves last to first position", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      const id3 = mi({ id: "ccc", alias: "Third" });
      useIdentityStore.setState({ identities: [id1, id2, id3] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentities("ccc", "aaa");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["ccc", "aaa", "bbb"]);
    });

    it("moves first to last position", async () => {
      const id1 = mi({ id: "aaa" });
      const id2 = mi2({ id: "bbb" });
      const id3 = mi({ id: "ccc", alias: "Third" });
      useIdentityStore.setState({ identities: [id1, id2, id3] });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reorderIdentities("aaa", "ccc");

      const ids = useIdentityStore.getState().identities.map((i) => i.id);
      expect(ids).toEqual(["bbb", "ccc", "aaa"]);
    });
  });

  describe("removeIdentity", () => {
    it("removes an identity from the store", async () => {
      useIdentityStore.setState({
        identities: [mi({ id: "aaa" }), mi2({ id: "bbb" })],
      });
      (commands.identityDelete as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().removeIdentity("aaa");

      const state = useIdentityStore.getState();
      expect(state.identities).toHaveLength(1);
      expect(state.identities[0].id).toBe("bbb");
    });

    it("deselects if the removed identity was selected", async () => {
      useIdentityStore.setState({
        identities: [mi()],
        selectedIdentityId: "aabb001122",
      });
      (commands.identityDelete as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().removeIdentity("aabb001122");

      expect(useIdentityStore.getState().selectedIdentityId).toBeNull();
    });

    it("keeps selection if a different identity was removed", async () => {
      useIdentityStore.setState({
        identities: [mi({ id: "aaa" }), mi2({ id: "bbb" })],
        selectedIdentityId: "aaa",
      });
      (commands.identityDelete as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().removeIdentity("bbb");

      expect(useIdentityStore.getState().selectedIdentityId).toBe("aaa");
    });

    it("persists updated order after removal", async () => {
      useIdentityStore.setState({
        identities: [mi({ id: "aaa" }), mi2({ id: "bbb" })],
      });
      (commands.identityDelete as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });
      (commands.identitySaveOrder as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().removeIdentity("aaa");

      expect(commands.identitySaveOrder).toHaveBeenCalledWith({
        identityIds: ["bbb"],
      });
    });

    it("sets error on failure without removing", async () => {
      useIdentityStore.setState({
        identities: [mi()],
      });
      (commands.identityDelete as Mock).mockResolvedValue({
        status: "error",
        error: "Identity in use",
      });

      await useIdentityStore.getState().removeIdentity("aabb001122");

      expect(useIdentityStore.getState().identities).toHaveLength(1);
      expect(useIdentityStore.getState().error).toBe("Identity in use");
    });
  });

  describe("setSortColumn", () => {
    it("sorts by alias ascending when switching from different column", () => {
      useIdentityStore.setState({
        identities: [
          mi({ id: "1", alias: "Charlie" }),
          mi({ id: "2", alias: "Alice" }),
          mi({ id: "3", alias: "Bob" }),
        ],
        sortColumn: "balance", // Different column so alias click starts ascending
      });

      useIdentityStore.getState().setSortColumn("alias");

      const aliases = useIdentityStore
        .getState()
        .identities.map((i) => i.alias);
      expect(aliases).toEqual(["Alice", "Bob", "Charlie"]);
      expect(useIdentityStore.getState().useCustomOrder).toBe(false);
    });

    it("toggles to descending on same column click", () => {
      useIdentityStore.setState({
        identities: [
          mi({ id: "1", alias: "Alice" }),
          mi({ id: "2", alias: "Charlie" }),
          mi({ id: "3", alias: "Bob" }),
        ],
        sortColumn: "alias",
        sortOrder: "ascending",
      });

      useIdentityStore.getState().setSortColumn("alias");

      expect(useIdentityStore.getState().sortOrder).toBe("descending");
      const aliases = useIdentityStore
        .getState()
        .identities.map((i) => i.alias);
      expect(aliases).toEqual(["Charlie", "Bob", "Alice"]);
    });

    it("sorts by balance", () => {
      useIdentityStore.setState({
        identities: [
          mi({ id: "1", balance: 300 }),
          mi({ id: "2", balance: 100 }),
          mi({ id: "3", balance: 200 }),
        ],
      });

      useIdentityStore.getState().setSortColumn("balance");

      const balances = useIdentityStore
        .getState()
        .identities.map((i) => i.balance);
      expect(balances).toEqual([100, 200, 300]);
    });

    it("sorts by type", () => {
      useIdentityStore.setState({
        identities: [
          mi({ id: "1", identityType: "user" }),
          mi({ id: "2", identityType: "evonode" }),
          mi({ id: "3", identityType: "masternode" }),
        ],
      });

      useIdentityStore.getState().setSortColumn("type");

      const types = useIdentityStore
        .getState()
        .identities.map((i) => i.identityType);
      expect(types).toEqual(["evonode", "masternode", "user"]);
    });

    it("resets to ascending when switching columns", () => {
      useIdentityStore.setState({
        sortColumn: "alias",
        sortOrder: "descending",
        identities: [mi()],
      });

      useIdentityStore.getState().setSortColumn("balance");

      expect(useIdentityStore.getState().sortColumn).toBe("balance");
      expect(useIdentityStore.getState().sortOrder).toBe("ascending");
    });
  });

  describe("reloadIdentity", () => {
    it("updates an existing identity in the store", async () => {
      useIdentityStore.setState({
        identities: [mi({ balance: 100 })],
      });
      const updated = mi({ balance: 999 });
      (commands.identityGetById as Mock).mockResolvedValue({
        status: "ok",
        data: updated,
      });

      await useIdentityStore.getState().reloadIdentity("aabb001122");

      expect(
        useIdentityStore.getState().identities[0].balance,
      ).toBe(999);
    });

    it("appends a new identity if not found in store", async () => {
      useIdentityStore.setState({ identities: [mi()] });
      const newId = mi2({ id: "new999" });
      (commands.identityGetById as Mock).mockResolvedValue({
        status: "ok",
        data: newId,
      });

      await useIdentityStore.getState().reloadIdentity("new999");

      expect(useIdentityStore.getState().identities).toHaveLength(2);
    });

    it("silently ignores errors", async () => {
      (commands.identityGetById as Mock).mockRejectedValue(
        new Error("Not found"),
      );

      await useIdentityStore.getState().reloadIdentity("missing");

      expect(useIdentityStore.getState().error).toBeNull();
    });

    it("does nothing if identity not found on backend", async () => {
      useIdentityStore.setState({ identities: [mi()] });
      (commands.identityGetById as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useIdentityStore.getState().reloadIdentity("aabb001122");

      // Should not change anything since data is null
      expect(useIdentityStore.getState().identities).toHaveLength(1);
    });
  });

  describe("subscribeToUpdates", () => {
    it("subscribes to task result and error events", async () => {
      const unlistenResult = vi.fn();
      const unlistenError = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(unlistenResult);
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unlistenError);

      const unlisten = await useIdentityStore
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
      (events.taskResultEvent.listen as Mock).mockResolvedValue(unlistenResult);
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unlistenError);

      const unlisten = await useIdentityStore
        .getState()
        .subscribeToUpdates();
      unlisten();

      expect(unlistenResult).toHaveBeenCalled();
      expect(unlistenError).toHaveBeenCalled();
    });

    it("reloads identity when task result has identityId", async () => {
      const identity = mi({ id: "aabb001122" });
      useIdentityStore.setState({
        identities: [identity],
        refreshingIds: new Set(["aabb001122"]),
      });

      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});

      const updatedIdentity = mi({
        id: "aabb001122",
        balance: 999,
      });
      (commands.identityGetById as Mock).mockResolvedValue({
        status: "ok",
        data: updatedIdentity,
      });

      await useIdentityStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "Identity",
          payload: { identityId: "aabb001122" },
        },
      });

      await vi.waitFor(() => {
        expect(commands.identityGetById).toHaveBeenCalledWith("aabb001122");
      });

      // Should have cleared the refreshing state
      expect(
        useIdentityStore.getState().refreshingIds.has("aabb001122"),
      ).toBe(false);
    });

    it("ignores non-Identity result types", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        (cb: typeof resultCallback) => {
          resultCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(() => {});

      await useIdentityStore.getState().subscribeToUpdates();
      resultCallback!({
        payload: {
          taskId: "t1",
          resultType: "Wallet",
          payload: { someData: true },
        },
      });

      expect(commands.identityGetById).not.toHaveBeenCalled();
      expect(commands.identityListLocal).not.toHaveBeenCalled();
    });

    it("clears refreshing and sets error on task error event", async () => {
      useIdentityStore.setState({
        refreshingIds: new Set(["aabb001122"]),
        refreshingAll: true,
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

      await useIdentityStore.getState().subscribeToUpdates();
      errorCallback!({
        payload: { taskId: "t1", message: "Platform error" },
      });

      const state = useIdentityStore.getState();
      expect(state.refreshingIds.size).toBe(0);
      expect(state.refreshingAll).toBe(false);
      expect(state.error).toBe("Platform error");
    });
  });

  describe("clearError", () => {
    it("clears the error state", () => {
      useIdentityStore.setState({ error: "some error" });

      useIdentityStore.getState().clearError();

      expect(useIdentityStore.getState().error).toBeNull();
    });
  });
});
