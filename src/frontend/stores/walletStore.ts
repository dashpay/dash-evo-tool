import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  WalletDto,
  SingleKeyWalletDto,
  WalletRefDto,
  PlatformSyncModeDto,
  TaskResultEvent,
} from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

// ─── Refresh modes (mirrors egui WalletRefreshMode) ─────────────────

export type WalletRefreshMode =
  | "coreOnly"
  | "coreAndPlatformAuto"
  | "coreAndPlatformFull"
  | "coreAndPlatformTerminal"
  | "combined";

// ─── Store state ────────────────────────────────────────────────────

interface WalletState {
  // Data
  hdWallets: WalletDto[];
  singleKeyWallets: SingleKeyWalletDto[];
  selectedWallet: WalletRefDto | null;

  // Loading / error
  loading: boolean;
  refreshing: boolean;
  error: string | null;

  // Refresh mode preference
  refreshMode: WalletRefreshMode;
}

// ─── Store actions ──────────────────────────────────────────────────

interface WalletActions {
  /** Load all wallets from the backend. */
  loadWallets: () => Promise<void>;

  /** Select a wallet (HD or single-key) as the active wallet. */
  selectWallet: (ref: WalletRefDto | null) => Promise<void>;

  /** Refresh a specific HD wallet from Core (and optionally Platform). */
  refreshHdWallet: (seedHash: string) => Promise<void>;

  /** Refresh a specific single-key wallet from Core. */
  refreshSingleKeyWallet: (keyHash: string) => Promise<void>;

  /** Refresh the currently selected wallet. */
  refreshSelectedWallet: () => Promise<void>;

  /** Set the refresh mode preference. */
  setRefreshMode: (mode: WalletRefreshMode) => void;

  /** Rename an HD wallet. */
  setHdWalletAlias: (seedHash: string, alias: string | null) => Promise<void>;

  /** Rename a single-key wallet. */
  setSingleKeyWalletAlias: (
    keyHash: string,
    alias: string | null,
  ) => Promise<void>;

  /** Remove an HD wallet. */
  removeHdWallet: (seedHash: string) => Promise<void>;

  /** Remove a single-key wallet. */
  removeSingleKeyWallet: (keyHash: string) => Promise<void>;

  /** Reload a single HD wallet (after backend mutation). */
  reloadHdWallet: (seedHash: string) => Promise<void>;

  /** Reload a single single-key wallet (after backend mutation). */
  reloadSingleKeyWallet: (keyHash: string) => Promise<void>;

  /** Notify the backend a wallet has been unlocked (non-password wallets only). */
  notifyUnlocked: (seedHash: string) => Promise<void>;

  /** Notify the backend a wallet has been locked (non-password wallets only). */
  notifyLocked: (seedHash: string) => Promise<void>;

  /** Unlock a password-protected wallet. Returns error string or null on success. */
  unlockWallet: (walletRef: WalletRefDto, password: string) => Promise<string | null>;

  /** Lock a wallet (securely erases decrypted key material). */
  lockWallet: (walletRef: WalletRefDto) => Promise<void>;

  /** Subscribe to wallet-updated Tauri events. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Reset all state (used on network switch). */
  resetState: () => void;

  /** Clear error state. */
  clearError: () => void;
}

export type WalletStore = WalletState & WalletActions;

// ─── Helpers ────────────────────────────────────────────────────────

function platformSyncModeForRefresh(
  mode: WalletRefreshMode,
): PlatformSyncModeDto | null {
  switch (mode) {
    case "coreOnly":
      return null;
    case "coreAndPlatformAuto":
    case "combined":
      return "auto";
    case "coreAndPlatformFull":
      return "forceFull";
    case "coreAndPlatformTerminal":
      return "terminalOnly";
  }
}

// ─── Task timeout manager ────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

// ─── Store ──────────────────────────────────────────────────────────

export const useWalletStore = create<WalletStore>((set, get) => ({
  // Initial state
  hdWallets: [],
  singleKeyWallets: [],
  selectedWallet: null,
  loading: false,
  refreshing: false,
  error: null,
  refreshMode: "coreAndPlatformAuto",

  loadWallets: async () => {
    set({ loading: true, error: null });
    try {
      const result = await commands.walletListAll();
      if (result.status === "ok") {
        set({
          hdWallets: result.data.hdWallets,
          singleKeyWallets: result.data.singleKeyWallets,
          selectedWallet: result.data.selected,
          loading: false,
        });
      } else {
        set({ error: result.error, loading: false });
      }
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        loading: false,
      });
    }
  },

  selectWallet: async (ref) => {
    try {
      const result = await commands.walletSelect({ selected: ref });
      if (result.status === "ok") {
        set({ selectedWallet: ref });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  refreshHdWallet: async (seedHash) => {
    const { refreshMode } = get();
    set({ refreshing: true, error: null });
    try {
      const platformSyncMode = platformSyncModeForRefresh(refreshMode);
      const result = await commands.coreRefreshWalletInfo({
        walletSeedHash: seedHash,
        platformSyncMode,
      });
      if (result.status === "error") {
        set({ error: result.error, refreshing: false });
        return;
      }
      // refreshing stays true — cleared when taskResultEvent(walletCompleted) arrives
      timeouts.start(`refreshHd:${seedHash}`, () => {
        set({ refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
      });
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        refreshing: false,
      });
    }
  },

  refreshSingleKeyWallet: async (keyHash) => {
    set({ refreshing: true, error: null });
    try {
      const result = await commands.coreRefreshSingleKeyWalletInfo({
        keyHash,
      });
      if (result.status === "error") {
        set({ error: result.error, refreshing: false });
        return;
      }
      // refreshing stays true — cleared when taskResultEvent(walletCompleted) arrives
      timeouts.start(`refreshSk:${keyHash}`, () => {
        set({ refreshing: false, error: TIMEOUT_ERROR_MESSAGE });
      });
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        refreshing: false,
      });
    }
  },

  refreshSelectedWallet: async () => {
    const { selectedWallet, refreshHdWallet, refreshSingleKeyWallet } = get();
    if (!selectedWallet) return;
    if (selectedWallet.type === "hd") {
      await refreshHdWallet(selectedWallet.seedHash);
    } else {
      await refreshSingleKeyWallet(selectedWallet.keyHash);
    }
  },

  setRefreshMode: (mode) => {
    set({ refreshMode: mode });
  },

  setHdWalletAlias: async (seedHash, alias) => {
    try {
      const result = await commands.walletSetAlias({
        walletSeedHash: seedHash,
        alias,
      });
      if (result.status === "ok") {
        set((state) => ({
          hdWallets: state.hdWallets.map((w) =>
            w.seedHash === seedHash ? { ...w, alias } : w,
          ),
        }));
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  setSingleKeyWalletAlias: async (keyHash, alias) => {
    try {
      const result = await commands.walletSetSingleKeyAlias({
        keyHash,
        alias,
      });
      if (result.status === "ok") {
        set((state) => ({
          singleKeyWallets: state.singleKeyWallets.map((w) =>
            w.keyHash === keyHash ? { ...w, alias } : w,
          ),
        }));
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  removeHdWallet: async (seedHash) => {
    try {
      const result = await commands.walletRemove({
        walletSeedHash: seedHash,
      });
      if (result.status === "ok") {
        set((state) => {
          const newHdWallets = state.hdWallets.filter(
            (w) => w.seedHash !== seedHash,
          );
          // If the removed wallet was selected, deselect
          const wasSelected =
            state.selectedWallet?.type === "hd" &&
            state.selectedWallet.seedHash === seedHash;
          return {
            hdWallets: newHdWallets,
            selectedWallet: wasSelected ? null : state.selectedWallet,
          };
        });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  removeSingleKeyWallet: async (keyHash) => {
    try {
      const result = await commands.walletRemoveSingleKey({ keyHash });
      if (result.status === "ok") {
        set((state) => {
          const newSingleKeyWallets = state.singleKeyWallets.filter(
            (w) => w.keyHash !== keyHash,
          );
          const wasSelected =
            state.selectedWallet?.type === "singleKey" &&
            state.selectedWallet.keyHash === keyHash;
          return {
            singleKeyWallets: newSingleKeyWallets,
            selectedWallet: wasSelected ? null : state.selectedWallet,
          };
        });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  reloadHdWallet: async (seedHash) => {
    try {
      const result = await commands.walletGetHd(seedHash);
      if (result.status === "ok") {
        set((state) => {
          const exists = state.hdWallets.some((w) => w.seedHash === seedHash);
          return {
            hdWallets: exists
              ? state.hdWallets.map((w) =>
                  w.seedHash === seedHash ? result.data : w,
                )
              : [...state.hdWallets, result.data],
          };
        });
      }
    } catch {
      // Silently ignore — wallet may no longer exist
    }
  },

  reloadSingleKeyWallet: async (keyHash) => {
    try {
      const result = await commands.walletGetSingleKey(keyHash);
      if (result.status === "ok") {
        set((state) => {
          const exists = state.singleKeyWallets.some(
            (w) => w.keyHash === keyHash,
          );
          return {
            singleKeyWallets: exists
              ? state.singleKeyWallets.map((w) =>
                  w.keyHash === keyHash ? result.data : w,
                )
              : [...state.singleKeyWallets, result.data],
          };
        });
      }
    } catch {
      // Silently ignore
    }
  },

  notifyUnlocked: async (seedHash) => {
    try {
      await commands.walletNotifyUnlocked(seedHash);
    } catch {
      // Non-critical — best effort
    }
  },

  notifyLocked: async (seedHash) => {
    try {
      await commands.walletNotifyLocked(seedHash);
    } catch {
      // Non-critical — best effort
    }
  },

  unlockWallet: async (walletRef, password) => {
    try {
      const result = await commands.walletUnlock({ walletRef, password });
      if (result.status === "error") {
        return result.error;
      }
      return null;
    } catch (e) {
      return e instanceof Error ? e.message : String(e);
    }
  },

  lockWallet: async (walletRef) => {
    try {
      await commands.walletLock({ walletRef });
    } catch {
      // Best effort
    }
  },

  subscribeToUpdates: async () => {
    // ZMQ real-time balance updates (independent of explicit refresh)
    const unlistenWallet = await events.walletUpdatedEvent.listen(async (event) => {
      const { walletSeedHash, network } = event.payload;

      // Ignore events from other networks
      try {
        const currentNet = await commands.contextGetNetwork();
        if (network !== currentNet) return;
      } catch {
        // If we can't check the network, process the event anyway
      }

      // Clear any pending refresh timeout for this wallet
      timeouts.clear(`refreshHd:${walletSeedHash}`);
      timeouts.clear(`refreshSk:${walletSeedHash}`);

      // Clear refreshing flag now that we have fresh data
      set({ refreshing: false });

      // Reload the updated wallet's data
      const state = get();
      const isHd = state.hdWallets.some(
        (w) => w.seedHash === walletSeedHash,
      );
      if (isHd) {
        state.reloadHdWallet(walletSeedHash);
      } else {
        // Could be a single-key wallet — try reloading from full list
        state.loadWallets();
      }
    });

    // Backend task completion — clears refresh state
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { result } = event.payload;
        if (result.type !== "walletCompleted") return;

        timeouts.clearAll();
        set({ refreshing: false });

        // Reload wallet data
        get().loadWallets();
      },
    );

    // Backend task error — clears refresh state if we were refreshing
    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "core" && event.payload.domain !== "wallet") return;
        if (!get().refreshing) return;

        timeouts.clearAll();
        set({ refreshing: false, error: event.payload.message });
      },
    );

    return () => {
      unlistenWallet();
      unlistenResult();
      unlistenError();
    };
  },

  resetState: () => {
    timeouts.clearAll();
    set({
      hdWallets: [],
      singleKeyWallets: [],
      selectedWallet: null,
      loading: false,
      refreshing: false,
      error: null,
    });
  },

  clearError: () => {
    set({ error: null });
  },
}));
