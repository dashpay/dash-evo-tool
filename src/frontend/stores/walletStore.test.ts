/**
 * walletStore tests — using centralized mock IPC + fixture factories.
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
import {
  createMockHdWallet,
  createMockSingleKeyWallet,
  createMockWalletList,
} from "@/test/fixtures";

// ─── Mock bindings (centralized) ────────────────────────────────────

vi.mock("../bindings", () => {
  const initial = createMockBindings();
  return mockBindingsModule(initial);
});

import { commands, events } from "../bindings";
import { useWalletStore } from "./walletStore";
import type { WalletRefDto } from "../bindings";

// ─── Test-local defaults ────────────────────────────────────────────
// The centralized fixtures use different default hashes. These helpers
// provide the same defaults as the original inline test makers for
// backward-compatible assertions.
const HD_HASH = "abc123";
const SK_HASH = "def456";

function hw(o?: Partial<Parameters<typeof createMockHdWallet>[0]>) {
  return createMockHdWallet({ seedHash: HD_HASH, ...o });
}
function skw(o?: Partial<Parameters<typeof createMockSingleKeyWallet>[0]>) {
  return createMockSingleKeyWallet({ keyHash: SK_HASH, ...o });
}
function wl(o?: Partial<Parameters<typeof createMockWalletList>[0]>) {
  return createMockWalletList({
    hdWallets: [hw()],
    singleKeyWallets: [skw()],
    selected: { type: "hd", seedHash: HD_HASH },
    ...o,
  });
}

// ─── Reset store between tests ──────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  useWalletStore.setState({
    hdWallets: [],
    singleKeyWallets: [],
    selectedWallet: null,
    loading: false,
    refreshing: false,
    error: null,
    refreshMode: "coreAndPlatformAuto",
  });
});

// ─── Tests ──────────────────────────────────────────────────────────

describe("walletStore", () => {
  describe("initial state", () => {
    it("starts with empty wallets and no selection", () => {
      const state = useWalletStore.getState();
      expect(state.hdWallets).toEqual([]);
      expect(state.singleKeyWallets).toEqual([]);
      expect(state.selectedWallet).toBeNull();
      expect(state.loading).toBe(false);
      expect(state.refreshing).toBe(false);
      expect(state.error).toBeNull();
      expect(state.refreshMode).toBe("coreAndPlatformAuto");
    });
  });

  describe("loadWallets", () => {
    it("loads wallets from backend on success", async () => {
      const walletList = wl();
      (commands.walletListAll as Mock).mockResolvedValue({
        status: "ok",
        data: walletList,
      });

      await useWalletStore.getState().loadWallets();

      const state = useWalletStore.getState();
      expect(state.hdWallets).toEqual(walletList.hdWallets);
      expect(state.singleKeyWallets).toEqual(walletList.singleKeyWallets);
      expect(state.selectedWallet).toEqual(walletList.selected);
      expect(state.loading).toBe(false);
      expect(state.error).toBeNull();
    });

    it("sets loading=true during load", async () => {
      let resolveLoad: (value: unknown) => void;
      (commands.walletListAll as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolveLoad = resolve;
        }),
      );

      const promise = useWalletStore.getState().loadWallets();
      expect(useWalletStore.getState().loading).toBe(true);

      resolveLoad!({ status: "ok", data: wl() });
      await promise;
      expect(useWalletStore.getState().loading).toBe(false);
    });

    it("sets error on backend error result", async () => {
      (commands.walletListAll as Mock).mockResolvedValue({
        status: "error",
        error: "Database connection failed",
      });

      await useWalletStore.getState().loadWallets();

      const state = useWalletStore.getState();
      expect(state.error).toBe("Database connection failed");
      expect(state.loading).toBe(false);
      expect(state.hdWallets).toEqual([]);
    });

    it("sets error on network exception", async () => {
      (commands.walletListAll as Mock).mockRejectedValue(
        new Error("IPC not available"),
      );

      await useWalletStore.getState().loadWallets();

      const state = useWalletStore.getState();
      expect(state.error).toBe("IPC not available");
      expect(state.loading).toBe(false);
    });

    it("clears previous error on new load", async () => {
      useWalletStore.setState({ error: "old error" });
      (commands.walletListAll as Mock).mockResolvedValue({
        status: "ok",
        data: wl(),
      });

      await useWalletStore.getState().loadWallets();
      expect(useWalletStore.getState().error).toBeNull();
    });
  });

  describe("selectWallet", () => {
    it("selects an HD wallet", async () => {
      (commands.walletSelect as Mock).mockResolvedValue({ status: "ok", data: null });

      const ref: WalletRefDto = { type: "hd", seedHash: HD_HASH };
      await useWalletStore.getState().selectWallet(ref);

      expect(commands.walletSelect).toHaveBeenCalledWith({ selected: ref });
      expect(useWalletStore.getState().selectedWallet).toEqual(ref);
    });

    it("selects a single-key wallet", async () => {
      (commands.walletSelect as Mock).mockResolvedValue({ status: "ok", data: null });

      const ref: WalletRefDto = { type: "singleKey", keyHash: SK_HASH };
      await useWalletStore.getState().selectWallet(ref);

      expect(useWalletStore.getState().selectedWallet).toEqual(ref);
    });

    it("deselects wallet when passing null", async () => {
      useWalletStore.setState({
        selectedWallet: { type: "hd", seedHash: HD_HASH },
      });
      (commands.walletSelect as Mock).mockResolvedValue({ status: "ok", data: null });

      await useWalletStore.getState().selectWallet(null);

      expect(useWalletStore.getState().selectedWallet).toBeNull();
    });

    it("sets error on failure without changing selection", async () => {
      const original: WalletRefDto = { type: "hd", seedHash: HD_HASH };
      useWalletStore.setState({ selectedWallet: original });
      (commands.walletSelect as Mock).mockResolvedValue({
        status: "error",
        error: "Permission denied",
      });

      await useWalletStore.getState().selectWallet({
        type: "singleKey",
        keyHash: SK_HASH,
      });

      expect(useWalletStore.getState().selectedWallet).toEqual(original);
      expect(useWalletStore.getState().error).toBe("Permission denied");
    });
  });

  describe("refreshHdWallet", () => {
    it("refreshes wallet and updates store with new data", async () => {
      const wallet = hw({ totalBalance: 500000000 });
      useWalletStore.setState({ hdWallets: [wallet] });

      const refreshedWallet = hw({ totalBalance: 600000000 });
      (commands.coreRefreshWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: refreshedWallet,
      });

      await useWalletStore.getState().refreshHdWallet(HD_HASH);

      const state = useWalletStore.getState();
      expect(state.hdWallets[0].totalBalance).toBe(600000000);
      expect(state.refreshing).toBe(false);
    });

    it("uses correct platform sync mode based on refreshMode", async () => {
      useWalletStore.setState({
        hdWallets: [hw()],
        refreshMode: "coreOnly",
      });
      (commands.coreRefreshWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: hw(),
      });

      await useWalletStore.getState().refreshHdWallet(HD_HASH);

      expect(commands.coreRefreshWalletInfo).toHaveBeenCalledWith({
        walletSeedHash: HD_HASH,
        platformSyncMode: null,
      });
    });

    it("passes forceFull for coreAndPlatformFull mode", async () => {
      useWalletStore.setState({
        hdWallets: [hw()],
        refreshMode: "coreAndPlatformFull",
      });
      (commands.coreRefreshWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: hw(),
      });

      await useWalletStore.getState().refreshHdWallet(HD_HASH);

      expect(commands.coreRefreshWalletInfo).toHaveBeenCalledWith({
        walletSeedHash: HD_HASH,
        platformSyncMode: "forceFull",
      });
    });

    it("passes terminalOnly for coreAndPlatformTerminal mode", async () => {
      useWalletStore.setState({
        hdWallets: [hw()],
        refreshMode: "coreAndPlatformTerminal",
      });
      (commands.coreRefreshWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: hw(),
      });

      await useWalletStore.getState().refreshHdWallet(HD_HASH);

      expect(commands.coreRefreshWalletInfo).toHaveBeenCalledWith({
        walletSeedHash: HD_HASH,
        platformSyncMode: "terminalOnly",
      });
    });

    it("sets refreshing=true during refresh", async () => {
      useWalletStore.setState({ hdWallets: [hw()] });
      let resolveRefresh: (value: unknown) => void;
      (commands.coreRefreshWalletInfo as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolveRefresh = resolve;
        }),
      );

      const promise = useWalletStore.getState().refreshHdWallet(HD_HASH);
      expect(useWalletStore.getState().refreshing).toBe(true);

      resolveRefresh!({ status: "ok", data: { taskId: "t1" } });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: hw(),
      });
      await promise;
      expect(useWalletStore.getState().refreshing).toBe(false);
    });

    it("sets error on refresh failure", async () => {
      useWalletStore.setState({ hdWallets: [hw()] });
      (commands.coreRefreshWalletInfo as Mock).mockResolvedValue({
        status: "error",
        error: "Core not connected",
      });

      await useWalletStore.getState().refreshHdWallet(HD_HASH);

      expect(useWalletStore.getState().error).toBe("Core not connected");
      expect(useWalletStore.getState().refreshing).toBe(false);
    });
  });

  describe("refreshSingleKeyWallet", () => {
    it("refreshes and updates single-key wallet", async () => {
      const wallet = skw({ totalBalance: 100 });
      useWalletStore.setState({ singleKeyWallets: [wallet] });

      const refreshed = skw({ totalBalance: 200 });
      (commands.coreRefreshSingleKeyWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t2" },
      });
      (commands.walletGetSingleKey as Mock).mockResolvedValue({
        status: "ok",
        data: refreshed,
      });

      await useWalletStore.getState().refreshSingleKeyWallet(SK_HASH);

      expect(useWalletStore.getState().singleKeyWallets[0].totalBalance).toBe(200);
      expect(useWalletStore.getState().refreshing).toBe(false);
    });
  });

  describe("refreshSelectedWallet", () => {
    it("refreshes selected HD wallet", async () => {
      useWalletStore.setState({
        hdWallets: [hw()],
        selectedWallet: { type: "hd", seedHash: HD_HASH },
      });
      (commands.coreRefreshWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t1" },
      });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: hw(),
      });

      await useWalletStore.getState().refreshSelectedWallet();

      expect(commands.coreRefreshWalletInfo).toHaveBeenCalledWith({
        walletSeedHash: HD_HASH,
        platformSyncMode: "auto",
      });
    });

    it("refreshes selected single-key wallet", async () => {
      useWalletStore.setState({
        singleKeyWallets: [skw()],
        selectedWallet: { type: "singleKey", keyHash: SK_HASH },
      });
      (commands.coreRefreshSingleKeyWalletInfo as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "t2" },
      });
      (commands.walletGetSingleKey as Mock).mockResolvedValue({
        status: "ok",
        data: skw(),
      });

      await useWalletStore.getState().refreshSelectedWallet();

      expect(commands.coreRefreshSingleKeyWalletInfo).toHaveBeenCalledWith({
        keyHash: SK_HASH,
      });
    });

    it("does nothing when no wallet is selected", async () => {
      await useWalletStore.getState().refreshSelectedWallet();
      expect(commands.coreRefreshWalletInfo).not.toHaveBeenCalled();
      expect(commands.coreRefreshSingleKeyWalletInfo).not.toHaveBeenCalled();
    });
  });

  describe("setRefreshMode", () => {
    it("updates the refresh mode", () => {
      useWalletStore.getState().setRefreshMode("coreOnly");
      expect(useWalletStore.getState().refreshMode).toBe("coreOnly");
    });

    it("accepts all valid refresh modes", () => {
      const modes: Array<
        ReturnType<typeof useWalletStore.getState>["refreshMode"]
      > = [
        "coreOnly",
        "coreAndPlatformAuto",
        "coreAndPlatformFull",
        "coreAndPlatformTerminal",
        "combined",
      ];
      for (const mode of modes) {
        useWalletStore.getState().setRefreshMode(mode);
        expect(useWalletStore.getState().refreshMode).toBe(mode);
      }
    });
  });

  describe("setHdWalletAlias", () => {
    it("renames an HD wallet", async () => {
      useWalletStore.setState({ hdWallets: [hw()] });
      (commands.walletSetAlias as Mock).mockResolvedValue({ status: "ok", data: null });

      await useWalletStore.getState().setHdWalletAlias(HD_HASH, "New Name");

      expect(commands.walletSetAlias).toHaveBeenCalledWith({
        walletSeedHash: HD_HASH,
        alias: "New Name",
      });
      expect(useWalletStore.getState().hdWallets[0].alias).toBe("New Name");
    });

    it("clears alias when passing null", async () => {
      useWalletStore.setState({
        hdWallets: [hw({ alias: "Old Name" })],
      });
      (commands.walletSetAlias as Mock).mockResolvedValue({ status: "ok", data: null });

      await useWalletStore.getState().setHdWalletAlias(HD_HASH, null);

      expect(useWalletStore.getState().hdWallets[0].alias).toBeNull();
    });

    it("does not update on failure", async () => {
      useWalletStore.setState({
        hdWallets: [hw({ alias: "Original" })],
      });
      (commands.walletSetAlias as Mock).mockResolvedValue({
        status: "error",
        error: "Not found",
      });

      await useWalletStore.getState().setHdWalletAlias(HD_HASH, "New Name");

      expect(useWalletStore.getState().hdWallets[0].alias).toBe("Original");
      expect(useWalletStore.getState().error).toBe("Not found");
    });
  });

  describe("setSingleKeyWalletAlias", () => {
    it("renames a single-key wallet", async () => {
      useWalletStore.setState({
        singleKeyWallets: [skw()],
      });
      (commands.walletSetSingleKeyAlias as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useWalletStore
        .getState()
        .setSingleKeyWalletAlias(SK_HASH, "Key Wallet");

      expect(commands.walletSetSingleKeyAlias).toHaveBeenCalledWith({
        keyHash: SK_HASH,
        alias: "Key Wallet",
      });
      expect(useWalletStore.getState().singleKeyWallets[0].alias).toBe(
        "Key Wallet",
      );
    });
  });

  describe("removeHdWallet", () => {
    it("removes an HD wallet from the store", async () => {
      useWalletStore.setState({
        hdWallets: [
          hw({ seedHash: "abc123" }),
          hw({ seedHash: "xyz789" }),
        ],
      });
      (commands.walletRemove as Mock).mockResolvedValue({ status: "ok", data: null });

      await useWalletStore.getState().removeHdWallet("abc123");

      const state = useWalletStore.getState();
      expect(state.hdWallets).toHaveLength(1);
      expect(state.hdWallets[0].seedHash).toBe("xyz789");
    });

    it("deselects if the removed wallet was selected", async () => {
      useWalletStore.setState({
        hdWallets: [hw()],
        selectedWallet: { type: "hd", seedHash: HD_HASH },
      });
      (commands.walletRemove as Mock).mockResolvedValue({ status: "ok", data: null });

      await useWalletStore.getState().removeHdWallet(HD_HASH);

      expect(useWalletStore.getState().selectedWallet).toBeNull();
    });

    it("keeps selection if a different wallet was removed", async () => {
      const selected: WalletRefDto = { type: "hd", seedHash: HD_HASH };
      useWalletStore.setState({
        hdWallets: [
          hw({ seedHash: HD_HASH }),
          hw({ seedHash: "xyz789" }),
        ],
        selectedWallet: selected,
      });
      (commands.walletRemove as Mock).mockResolvedValue({ status: "ok", data: null });

      await useWalletStore.getState().removeHdWallet("xyz789");

      expect(useWalletStore.getState().selectedWallet).toEqual(selected);
    });

    it("sets error on failure", async () => {
      useWalletStore.setState({ hdWallets: [hw()] });
      (commands.walletRemove as Mock).mockResolvedValue({
        status: "error",
        error: "Wallet in use",
      });

      await useWalletStore.getState().removeHdWallet(HD_HASH);

      expect(useWalletStore.getState().hdWallets).toHaveLength(1);
      expect(useWalletStore.getState().error).toBe("Wallet in use");
    });
  });

  describe("removeSingleKeyWallet", () => {
    it("removes a single-key wallet from the store", async () => {
      useWalletStore.setState({
        singleKeyWallets: [skw()],
        selectedWallet: { type: "singleKey", keyHash: SK_HASH },
      });
      (commands.walletRemoveSingleKey as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useWalletStore.getState().removeSingleKeyWallet(SK_HASH);

      expect(useWalletStore.getState().singleKeyWallets).toHaveLength(0);
      expect(useWalletStore.getState().selectedWallet).toBeNull();
    });
  });

  describe("reloadHdWallet", () => {
    it("updates an existing HD wallet in the store", async () => {
      useWalletStore.setState({ hdWallets: [hw()] });
      const updated = hw({ totalBalance: 999 });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: updated,
      });

      await useWalletStore.getState().reloadHdWallet(HD_HASH);

      expect(useWalletStore.getState().hdWallets[0].totalBalance).toBe(999);
    });

    it("appends a new HD wallet if not found in store", async () => {
      useWalletStore.setState({ hdWallets: [hw()] });
      const newWallet = hw({ seedHash: "new999" });
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: newWallet,
      });

      await useWalletStore.getState().reloadHdWallet("new999");

      expect(useWalletStore.getState().hdWallets).toHaveLength(2);
    });

    it("silently ignores errors", async () => {
      (commands.walletGetHd as Mock).mockRejectedValue(
        new Error("Not found"),
      );

      await useWalletStore.getState().reloadHdWallet("missing");

      expect(useWalletStore.getState().error).toBeNull();
    });
  });

  describe("reloadSingleKeyWallet", () => {
    it("updates an existing single-key wallet in the store", async () => {
      useWalletStore.setState({
        singleKeyWallets: [skw()],
      });
      const updated = skw({ totalBalance: 555 });
      (commands.walletGetSingleKey as Mock).mockResolvedValue({
        status: "ok",
        data: updated,
      });

      await useWalletStore.getState().reloadSingleKeyWallet(SK_HASH);

      expect(useWalletStore.getState().singleKeyWallets[0].totalBalance).toBe(
        555,
      );
    });

    it("appends a new single-key wallet if not found", async () => {
      useWalletStore.setState({ singleKeyWallets: [] });
      const newWallet = skw({ keyHash: "new111" });
      (commands.walletGetSingleKey as Mock).mockResolvedValue({
        status: "ok",
        data: newWallet,
      });

      await useWalletStore.getState().reloadSingleKeyWallet("new111");

      expect(useWalletStore.getState().singleKeyWallets).toHaveLength(1);
    });
  });

  describe("notifyUnlocked / notifyLocked", () => {
    it("calls backend notifyUnlocked", async () => {
      (commands.walletNotifyUnlocked as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useWalletStore.getState().notifyUnlocked(HD_HASH);

      expect(commands.walletNotifyUnlocked).toHaveBeenCalledWith(HD_HASH);
    });

    it("calls backend notifyLocked", async () => {
      (commands.walletNotifyLocked as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useWalletStore.getState().notifyLocked(HD_HASH);

      expect(commands.walletNotifyLocked).toHaveBeenCalledWith(HD_HASH);
    });

    it("does not set error on notify failure", async () => {
      (commands.walletNotifyUnlocked as Mock).mockRejectedValue(
        new Error("IPC error"),
      );

      await useWalletStore.getState().notifyUnlocked(HD_HASH);

      expect(useWalletStore.getState().error).toBeNull();
    });
  });

  describe("subscribeToUpdates", () => {
    it("subscribes to wallet update events", async () => {
      const unlistenFn = vi.fn();
      (events.walletUpdatedEvent.listen as Mock).mockResolvedValue(
        unlistenFn,
      );

      const unlisten = await useWalletStore
        .getState()
        .subscribeToUpdates();

      expect(events.walletUpdatedEvent.listen).toHaveBeenCalledWith(
        expect.any(Function),
      );
      expect(typeof unlisten).toBe("function");
    });

    it("reloads HD wallet when update event received for known HD wallet", async () => {
      const wallet = hw();
      useWalletStore.setState({ hdWallets: [wallet] });

      let eventCallback: (event: { payload: { walletSeedHash: string } }) => void;
      (events.walletUpdatedEvent.listen as Mock).mockImplementation(
        (cb: typeof eventCallback) => {
          eventCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (commands.walletGetHd as Mock).mockResolvedValue({
        status: "ok",
        data: hw({ totalBalance: 777 }),
      });

      await useWalletStore.getState().subscribeToUpdates();
      eventCallback!({ payload: { walletSeedHash: HD_HASH } });

      // Wait for async reload
      await vi.waitFor(() => {
        expect(commands.walletGetHd).toHaveBeenCalledWith(HD_HASH);
      });
    });

    it("calls loadWallets when update event for unknown wallet", async () => {
      useWalletStore.setState({ hdWallets: [] });

      let eventCallback: (event: { payload: { walletSeedHash: string } }) => void;
      (events.walletUpdatedEvent.listen as Mock).mockImplementation(
        (cb: typeof eventCallback) => {
          eventCallback = cb;
          return Promise.resolve(() => {});
        },
      );
      (commands.walletListAll as Mock).mockResolvedValue({
        status: "ok",
        data: wl(),
      });

      await useWalletStore.getState().subscribeToUpdates();
      eventCallback!({ payload: { walletSeedHash: "unknown999" } });

      await vi.waitFor(() => {
        expect(commands.walletListAll).toHaveBeenCalled();
      });
    });
  });

  describe("clearError", () => {
    it("clears the error state", () => {
      useWalletStore.setState({ error: "some error" });

      useWalletStore.getState().clearError();

      expect(useWalletStore.getState().error).toBeNull();
    });
  });
});
