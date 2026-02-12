import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  ContractSummaryDto,
  DataContractDto,
  JsonValue,
  TaskResultEvent,
} from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

// ─── Store state ────────────────────────────────────────────────────

interface ContractState {
  /** Local contract summaries (from DB). */
  contracts: ContractSummaryDto[];
  /** Currently selected contract ID (hex). */
  selectedContractId: string | null;
  /** Loaded contract details keyed by contract ID. */
  contractDetails: Record<string, DataContractDto>;
  /** Whether the initial load is in progress. */
  loading: boolean;
  /** Whether a contract fetch/save operation is in progress. */
  fetching: boolean;
  /** Error message from the last operation. */
  error: string | null;
}

// ─── Store actions ──────────────────────────────────────────────────

interface ContractActions {
  /** Load all local contract summaries from the backend DB. */
  loadContracts: () => Promise<void>;

  /** Get full contract data by ID. */
  getContractById: (contractId: string) => Promise<DataContractDto | null>;

  /** Select a contract and load its full detail. */
  selectContract: (contractId: string | null) => Promise<void>;

  /** Set or clear alias for a contract. */
  setAlias: (contractId: string, alias: string | null) => Promise<void>;

  /** Remove a contract from local DB. */
  removeContract: (contractId: string) => Promise<void>;

  /** Fetch contracts from Platform by IDs (async — result via event). */
  fetchContracts: (contractIds: string[]) => Promise<string | null>;

  /** Fetch contracts with descriptions from Platform (async — result via event). */
  fetchContractsWithDescriptions: (contractIds: string[]) => Promise<string | null>;

  /** Save a contract to local DB (async — result via event). */
  saveContract: (contractJson: unknown, alias: string | null, insertTokens: boolean) => Promise<string | null>;

  /** Subscribe to task result events for contract updates. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Reset all state (used on network switch). */
  resetState: () => void;

  /** Clear the error state. */
  clearError: () => void;
}

// ─── Combined store type ────────────────────────────────────────────

export type ContractStore = ContractState & ContractActions;

// ─── Task timeout manager ────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

// ─── Store implementation ───────────────────────────────────────────

export const useContractStore = create<ContractStore>((set, get) => ({
  // Initial state
  contracts: [],
  selectedContractId: null,
  contractDetails: {},
  loading: false,
  fetching: false,
  error: null,

  loadContracts: async () => {
    set({ loading: true, error: null });
    try {
      const result = await commands.contractListLocal();
      if (result.status === "ok") {
        set({ contracts: result.data, loading: false });
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

  getContractById: async (contractId: string) => {
    try {
      const result = await commands.contractGetById(contractId);
      if (result.status === "ok") {
        return result.data;
      }
      set({ error: result.error });
      return null;
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
      return null;
    }
  },

  selectContract: async (contractId: string | null) => {
    if (!contractId) {
      set({ selectedContractId: null });
      return;
    }
    set({ selectedContractId: contractId });
    // Skip fetch if already loaded
    if (get().contractDetails[contractId]) return;
    const detail = await get().getContractById(contractId);
    if (detail) {
      set((state) => ({
        contractDetails: { ...state.contractDetails, [contractId]: detail },
      }));
    }
  },

  setAlias: async (contractId: string, alias: string | null) => {
    try {
      const result = await commands.contractSetAlias({
        contractId,
        alias,
      });
      if (result.status === "ok") {
        // Update local state
        set((state) => {
          const updatedDetails = { ...state.contractDetails };
          if (updatedDetails[contractId]) {
            updatedDetails[contractId] = { ...updatedDetails[contractId], alias };
          }
          return {
            contracts: state.contracts.map((c) =>
              c.id === contractId ? { ...c, alias } : c,
            ),
            contractDetails: updatedDetails,
          };
        });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  removeContract: async (contractId: string) => {
    try {
      const result = await commands.contractRemove({ contractId });
      if (result.status === "ok") {
        set((state) => {
          const { [contractId]: _, ...remainingDetails } = state.contractDetails;
          return {
            contracts: state.contracts.filter((c) => c.id !== contractId),
            selectedContractId:
              state.selectedContractId === contractId
                ? null
                : state.selectedContractId,
            contractDetails: remainingDetails,
          };
        });
      } else {
        set({ error: result.error });
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    }
  },

  fetchContracts: async (contractIds: string[]) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.contractFetch({ contractIds });
      if (result.status === "ok") {
        timeouts.start("fetch", () => {
          set({ fetching: false, error: TIMEOUT_ERROR_MESSAGE });
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

  fetchContractsWithDescriptions: async (contractIds: string[]) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.contractFetchWithDescriptions({
        contractIds,
      });
      if (result.status === "ok") {
        timeouts.start("fetchWithDesc", () => {
          set({ fetching: false, error: TIMEOUT_ERROR_MESSAGE });
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

  saveContract: async (
    contractJson: unknown,
    alias: string | null,
    insertTokens: boolean,
  ) => {
    set({ fetching: true, error: null });
    try {
      const result = await commands.contractSave({
        contractJson: contractJson as JsonValue,
        alias,
        insertTokens,
      });
      if (result.status === "ok") {
        timeouts.start("save", () => {
          set({ fetching: false, error: TIMEOUT_ERROR_MESSAGE });
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

  subscribeToUpdates: async () => {
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { result } = event.payload;

        if (result.type !== "contractCompleted") return;

        timeouts.clearAll();

        // Contract result received — reload local contracts list
        set({ fetching: false });
        get().loadContracts();
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "contract") return;

        timeouts.clearAll();

        set({
          fetching: false,
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
      contracts: [],
      selectedContractId: null,
      contractDetails: {},
      loading: false,
      fetching: false,
      error: null,
    });
  },

  clearError: () => {
    set({ error: null });
  },
}));
