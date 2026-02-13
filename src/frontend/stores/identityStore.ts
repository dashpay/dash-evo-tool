import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  QualifiedIdentityDto,
  TaskResultEvent,
} from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

// ─── Sort types ──────────────────────────────────────────────────────

export type IdentitySortColumn =
  | "alias"
  | "identityId"
  | "inWallet"
  | "type"
  | "balance";

export type IdentitySortOrder = "ascending" | "descending";

// ─── Store state ─────────────────────────────────────────────────────

interface IdentityState {
  /** All loaded identities in display order. */
  identities: QualifiedIdentityDto[];

  /** Currently selected identity ID (hex). */
  selectedIdentityId: string | null;

  /** Loading (initial fetch). */
  loading: boolean;

  /** Set of identity IDs currently being refreshed. */
  refreshingIds: Set<string>;

  /** Whether a bulk refresh-all is in progress. */
  refreshingAll: boolean;

  /** Error message. */
  error: string | null;

  /** Current sort column (only used when useCustomOrder is false). */
  sortColumn: IdentitySortColumn;

  /** Current sort direction. */
  sortOrder: IdentitySortOrder;

  /** Whether custom (user-defined) ordering is active. */
  useCustomOrder: boolean;
}

// ─── Store actions ───────────────────────────────────────────────────

interface IdentityActions {
  /** Load all identities from the backend and apply saved custom order. */
  loadIdentities: () => Promise<void>;

  /** Select an identity by ID. */
  selectIdentity: (identityId: string | null) => void;

  /** Refresh a single identity from Platform (async task). */
  refreshIdentity: (identityId: string) => Promise<void>;

  /** Refresh all identities from Platform. */
  refreshAllIdentities: () => Promise<void>;

  /** Set/clear the alias for an identity. */
  setAlias: (identityId: string, alias: string | null) => Promise<void>;

  /** Move an identity up in the custom order. */
  reorderIdentityUp: (identityId: string) => Promise<void>;

  /** Move an identity down in the custom order. */
  reorderIdentityDown: (identityId: string) => Promise<void>;

  /** Reorder identities by moving an item from one position to another (drag-and-drop). */
  reorderIdentities: (activeId: string, overId: string) => Promise<void>;

  /** Remove an identity from the store and database. */
  removeIdentity: (identityId: string) => Promise<void>;

  /** Sort by a column (switches to ephemeral sort mode). */
  setSortColumn: (column: IdentitySortColumn) => void;

  /** Reload a single identity after a backend mutation. */
  reloadIdentity: (identityId: string) => Promise<void>;

  /** Subscribe to task-result events for identity updates. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Reset all state (used on network switch). */
  resetState: () => void;

  /** Clear error state. */
  clearError: () => void;
}

export type IdentityStore = IdentityState & IdentityActions;

// ─── Helpers ─────────────────────────────────────────────────────────

/** Apply a saved order (list of IDs) to an identity array. */
function applyOrder(
  identities: QualifiedIdentityDto[],
  order: string[],
): QualifiedIdentityDto[] {
  const byId = new Map(identities.map((i) => [i.id, i]));
  const ordered: QualifiedIdentityDto[] = [];

  // First, add identities in the saved order
  for (const id of order) {
    const identity = byId.get(id);
    if (identity) {
      ordered.push(identity);
      byId.delete(id);
    }
  }

  // Then, append any identities not in the saved order
  for (const identity of byId.values()) {
    ordered.push(identity);
  }

  return ordered;
}

/** Sort identities by a column. Returns a new sorted array. */
function sortIdentities(
  identities: QualifiedIdentityDto[],
  column: IdentitySortColumn,
  order: IdentitySortOrder,
): QualifiedIdentityDto[] {
  const sorted = [...identities];
  const dir = order === "ascending" ? 1 : -1;

  sorted.sort((a, b) => {
    switch (column) {
      case "alias":
        return dir * (a.alias ?? "").localeCompare(b.alias ?? "");
      case "identityId":
        return dir * a.id.localeCompare(b.id);
      case "inWallet":
        return (
          dir *
          (a.associatedWalletHashes[0] ?? "").localeCompare(
            b.associatedWalletHashes[0] ?? "",
          )
        );
      case "type":
        return dir * a.identityType.localeCompare(b.identityType);
      case "balance":
        return dir * (a.balance - b.balance);
      default:
        return 0;
    }
  });

  return sorted;
}

// ─── Task timeout manager ────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

// ─── Store ───────────────────────────────────────────────────────────

export const useIdentityStore = create<IdentityStore>((set, get) => ({
  // Initial state
  identities: [],
  selectedIdentityId: null,
  loading: false,
  refreshingIds: new Set(),
  refreshingAll: false,
  error: null,
  sortColumn: "alias",
  sortOrder: "ascending",
  useCustomOrder: true,

  loadIdentities: async () => {
    set({ loading: true, error: null });
    try {
      const result = await commands.identityListLocal();
      if (result.status === "ok") {
        let identities = result.data;

        // Try to apply saved custom order
        const orderResult = await commands.identityLoadOrder();
        if (orderResult.status === "ok" && orderResult.data.length > 0) {
          identities = applyOrder(identities, orderResult.data);
          set({ identities, loading: false, useCustomOrder: true });
        } else {
          set({ identities, loading: false });
        }

        // Auto-refresh any identities with "unknown" status
        const loaded = get().identities;
        const unknownIds = loaded
          .filter((i) => i.status === "unknown")
          .map((i) => i.id);
        if (unknownIds.length > 0) {
          const currentRefreshing = get().refreshingIds;
          for (const id of unknownIds) {
            if (!currentRefreshing.has(id)) {
              get().refreshIdentity(id);
            }
          }
        }
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

  selectIdentity: (identityId) => {
    set({ selectedIdentityId: identityId });
  },

  refreshIdentity: async (identityId) => {
    set((state) => ({
      refreshingIds: new Set(state.refreshingIds).add(identityId),
      error: null,
    }));
    try {
      const result = await commands.identityRefresh({
        identityId,
      });
      if (result.status === "error") {
        set((state) => {
          const newIds = new Set(state.refreshingIds);
          newIds.delete(identityId);
          return { error: result.error, refreshingIds: newIds };
        });
      } else {
        // On success dispatch, start timeout keyed by identity
        timeouts.start(`refresh:${identityId}`, () => {
          set((s) => {
            const newIds = new Set(s.refreshingIds);
            newIds.delete(identityId);
            return { refreshingIds: newIds, error: TIMEOUT_ERROR_MESSAGE };
          });
        });
      }
      // On success, the TaskResultEvent will trigger reloadIdentity
      // We DON'T clear refreshingIds here — that happens in subscribeToUpdates
    } catch (e) {
      set((state) => {
        const newIds = new Set(state.refreshingIds);
        newIds.delete(identityId);
        return {
          error: e instanceof Error ? e.message : String(e),
          refreshingIds: newIds,
        };
      });
    }
  },

  refreshAllIdentities: async () => {
    const { identities } = get();
    if (identities.length === 0) return;

    set({
      refreshingAll: true,
      refreshingIds: new Set(identities.map((i) => i.id)),
      error: null,
    });
    try {
      // Dispatch refresh for each identity
      const results = await Promise.allSettled(
        identities.map((identity) =>
          commands.identityRefresh({ identityId: identity.id }),
        ),
      );

      // Check for any immediate errors
      const errors = results
        .map((r) => {
          if (r.status === "rejected") return String(r.reason);
          if (r.status === "fulfilled" && r.value.status === "error")
            return r.value.error;
          return null;
        })
        .filter(Boolean);

      if (errors.length > 0) {
        set({ error: `Refresh errors: ${errors[0]}`, refreshingAll: false });
      } else {
        timeouts.start("refreshAll", () => {
          set({ refreshingIds: new Set(), refreshingAll: false, error: TIMEOUT_ERROR_MESSAGE });
        });
      }
      // refreshingAll will be cleared when task results come back
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : String(e),
        refreshingAll: false,
      });
    }
  },

  setAlias: async (identityId, alias) => {
    try {
      const result = await commands.identitySetAlias({
        identityId,
        alias,
      });
      if (result.status === "ok") {
        set((state) => ({
          identities: state.identities.map((i) =>
            i.id === identityId ? { ...i, alias } : i,
          ),
        }));
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  reorderIdentityUp: async (identityId) => {
    const { identities } = get();
    const index = identities.findIndex((i) => i.id === identityId);
    if (index <= 0) return;

    const reordered = [...identities];
    const a = reordered[index];
    const b = reordered[index - 1];
    if (!a || !b) return;
    [reordered[index - 1], reordered[index]] = [a, b];

    set({ identities: reordered, useCustomOrder: true });

    // Persist the new order
    try {
      await commands.identitySaveOrder({
        identityIds: reordered.map((i) => i.id),
      });
    } catch {
      // Best effort — order is already applied in UI
    }
  },

  reorderIdentityDown: async (identityId) => {
    const { identities } = get();
    const index = identities.findIndex((i) => i.id === identityId);
    if (index < 0 || index >= identities.length - 1) return;

    const reordered = [...identities];
    const a = reordered[index];
    const b = reordered[index + 1];
    if (!a || !b) return;
    [reordered[index], reordered[index + 1]] = [b, a];

    set({ identities: reordered, useCustomOrder: true });

    // Persist the new order
    try {
      await commands.identitySaveOrder({
        identityIds: reordered.map((i) => i.id),
      });
    } catch {
      // Best effort
    }
  },

  reorderIdentities: async (activeId, overId) => {
    if (activeId === overId) return;
    const { identities } = get();
    const oldIndex = identities.findIndex((i) => i.id === activeId);
    const newIndex = identities.findIndex((i) => i.id === overId);
    if (oldIndex < 0 || newIndex < 0) return;

    const reordered = [...identities];
    const [moved] = reordered.splice(oldIndex, 1);
    if (!moved) return;
    reordered.splice(newIndex, 0, moved);

    set({ identities: reordered, useCustomOrder: true });

    try {
      await commands.identitySaveOrder({
        identityIds: reordered.map((i) => i.id),
      });
    } catch {
      // Best effort
    }
  },

  removeIdentity: async (identityId) => {
    try {
      // Capture voter identity ID before deletion
      const targetIdentity = get().identities.find((i) => i.id === identityId);
      const voterIdentityId = targetIdentity?.voterIdentityId;

      const result = await commands.identityDelete({ identityId });
      if (result.status === "ok") {
        set((state) => {
          const idsToRemove = new Set([identityId]);
          if (voterIdentityId) idsToRemove.add(voterIdentityId);
          const newIdentities = state.identities.filter(
            (i) => !idsToRemove.has(i.id),
          );
          const wasSelected =
            state.selectedIdentityId === identityId ||
            state.selectedIdentityId === voterIdentityId;
          return {
            identities: newIdentities,
            selectedIdentityId: wasSelected
              ? null
              : state.selectedIdentityId,
          };
        });

        // Persist the updated order
        const { identities } = get();
        try {
          await commands.identitySaveOrder({
            identityIds: identities.map((i) => i.id),
          });
        } catch {
          // Best effort
        }
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  setSortColumn: (column) => {
    const { sortColumn, sortOrder } = get();
    if (column === sortColumn) {
      // Toggle direction
      const newOrder =
        sortOrder === "ascending" ? "descending" : "ascending";
      set((state) => ({
        sortOrder: newOrder,
        useCustomOrder: false,
        identities: sortIdentities(state.identities, column, newOrder),
      }));
    } else {
      // New column, default ascending
      set((state) => ({
        sortColumn: column,
        sortOrder: "ascending",
        useCustomOrder: false,
        identities: sortIdentities(state.identities, column, "ascending"),
      }));
    }
  },

  reloadIdentity: async (identityId) => {
    try {
      const result = await commands.identityGetById(identityId);
      if (result.status === "ok" && result.data) {
        set((state) => {
          const exists = state.identities.some((i) => i.id === identityId);
          return {
            identities: exists
              ? state.identities.map((i) =>
                  i.id === identityId ? result.data! : i,
                )
              : [...state.identities, result.data!],
          };
        });
      }
    } catch {
      // Silently ignore — identity may no longer exist
    }
  },

  subscribeToUpdates: async () => {
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { result } = event.payload;
        console.debug("[identityStore] taskResultEvent received:", result.type, event.payload);

        if (result.type !== "identityCompleted") return;

        const state = get();

        // Handle identity results — reload the affected identity
        const identityId = result.identityId;
        if (identityId) {
          timeouts.clear(`refresh:${identityId}`);
          // Clear this identity from refreshingIds
          set((s) => {
            const newIds = new Set(s.refreshingIds);
            newIds.delete(identityId);
            return { refreshingIds: newIds };
          });
          state.reloadIdentity(identityId);
        } else {
          timeouts.clearAll();
          // Broad refresh — reload all
          state.loadIdentities();
        }

        // Check if we can clear refreshingAll
        const currentState = get();
        if (
          currentState.refreshingAll &&
          currentState.refreshingIds.size === 0
        ) {
          timeouts.clear("refreshAll");
          set({ refreshingAll: false });
        }
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "identity") return;

        timeouts.clearAll();

        // Clear refreshing state on error
        set({
          refreshingIds: new Set(),
          refreshingAll: false,
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
      identities: [],
      selectedIdentityId: null,
      loading: false,
      refreshingIds: new Set(),
      refreshingAll: false,
      error: null,
    });
  },

  clearError: () => {
    set({ error: null });
  },
}));
