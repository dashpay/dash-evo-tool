import { create } from "zustand";
import { commands, events } from "../bindings";
import type {
  WhereClauseDto,
  OrderByClauseDto,
  TaskResultEvent,
} from "../bindings";
import type { JsonValue } from "../bindings";
import { TaskTimeoutManager, TIMEOUT_ERROR_MESSAGE } from "../lib/taskTimeout";

// ─── Document types (from task result payload) ─────────────────────

/** A document as returned in task result payloads. */
export interface DocumentEntry {
  id: string;
  ownerId: string;
  documentType: string;
  data: JsonValue;
  revision: number;
  createdAt: number | null;
  updatedAt: number | null;
  transferredAt: number | null;
}

/** A page entry (document may be null if deleted). */
export interface DocumentPageEntry {
  id: string;
  document: DocumentEntry | null;
}

/** Display mode for document results. */
export type DocumentDisplayMode = "json" | "yaml";

/** Query status tracking. */
export type DocumentQueryStatus =
  | "idle"
  | "waiting"
  | "complete"
  | "error";

// ─── Helpers ──────────────────────────────────────────────────────

/** Extract an identifier from a DPP-serialized value (byte array or string). */
function extractIdString(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    return value.map((b: number) => b.toString(16).padStart(2, "0")).join("");
  }
  return "";
}

// ─── Special document fields ───────────────────────────────────────

/** System fields that exist on all documents but aren't in the document type schema. */
export const DOCUMENT_PRIVATE_FIELDS = [
  "$id",
  "$ownerId",
  "$version",
  "$revision",
  "$createdAt",
  "$updatedAt",
  "$transferredAt",
  "$createdAtBlockHeight",
  "$updatedAtBlockHeight",
  "$transferredAtBlockHeight",
  "$createdAtCoreBlockHeight",
  "$updatedAtCoreBlockHeight",
  "$transferredAtCoreBlockHeight",
] as const;

/** Default private fields to show initially. */
const DEFAULT_VISIBLE_PRIVATE_FIELDS = new Set(["$id", "$ownerId"]);

// ─── Store state ────────────────────────────────────────────────────

interface DocumentState {
  /** SQL-like query text. */
  queryText: string;
  /** Where clauses for structured query. */
  whereClauses: WhereClauseDto[];
  /** Order-by clauses. */
  orderByClauses: OrderByClauseDto[];
  /** Fetched document results for the current page. */
  documents: DocumentPageEntry[];
  /** Current query status. */
  queryStatus: DocumentQueryStatus;
  /** Timestamp (ms) when query started, for elapsed time display. */
  queryStartedAt: number | null;
  /** Error message from last query. */
  queryError: string | null;
  /** Task ID of the active fetch operation. */
  activeTaskId: string | null;
  /** Display mode for results. */
  displayMode: DocumentDisplayMode;
  /** Client-side search filter on displayed results. */
  searchFilter: string;
  /** Map of field name -> visible for field selection UI. */
  fieldSelection: Record<string, boolean>;
  /** Current page number (1-indexed). */
  currentPage: number;
  /** Cursor stack for forward pagination. */
  nextCursors: (string | null)[];
  /** Whether there are more pages. */
  hasNextPage: boolean;
  /** The contract ID currently being queried. */
  queryContractId: string | null;
  /** The document type currently being queried. */
  queryDocumentType: string | null;
}

// ─── Store actions ──────────────────────────────────────────────────

interface DocumentActions {
  /** Set the query text. */
  setQueryText: (text: string) => void;

  /** Set where clauses. */
  setWhereClauses: (clauses: WhereClauseDto[]) => void;

  /** Set order-by clauses. */
  setOrderByClauses: (clauses: OrderByClauseDto[]) => void;

  /** Set the contract and document type for the query. */
  setQueryTarget: (contractId: string | null, documentType: string | null) => void;

  /** Fetch documents (first page). Dispatches async task. */
  fetchDocuments: (
    contractId: string,
    documentTypeName: string,
  ) => Promise<string | null>;

  /** Fetch a specific page. */
  fetchPage: (
    contractId: string,
    documentTypeName: string,
    startAfter: string | null,
  ) => Promise<string | null>;

  /** Go to next page (uses cursor from current results). */
  goToNextPage: () => Promise<string | null>;

  /** Go to previous page (uses cursor stack). */
  goToPreviousPage: () => Promise<string | null>;

  /** Set display mode (json/yaml). */
  setDisplayMode: (mode: DocumentDisplayMode) => void;

  /** Set the client-side search filter. */
  setSearchFilter: (filter: string) => void;

  /** Initialize field selection for a document type schema. */
  initFieldSelection: (schemaFields: string[]) => void;

  /** Toggle visibility of a single field. */
  toggleField: (field: string) => void;

  /** Select/deselect all fields. */
  setAllFields: (visible: boolean) => void;

  /** Clear query results and reset query state. */
  clearResults: () => void;

  /** Subscribe to task result events for document updates. Returns unsubscribe fn. */
  subscribeToUpdates: () => Promise<() => void>;

  /** Reset all state (used on network switch). */
  resetState: () => void;

  /** Clear the error state. */
  clearError: () => void;
}

// ─── Combined store type ────────────────────────────────────────────

export type DocumentStore = DocumentState & DocumentActions;

// ─── Task timeout manager ────────────────────────────────────────────

const timeouts = new TaskTimeoutManager();

// ─── Store implementation ───────────────────────────────────────────

export const useDocumentStore = create<DocumentStore>((set, get) => ({
  // Initial state
  queryText: "",
  whereClauses: [],
  orderByClauses: [],
  documents: [],
  queryStatus: "idle",
  queryStartedAt: null,
  queryError: null,
  activeTaskId: null,
  displayMode: "json",
  searchFilter: "",
  fieldSelection: {},
  currentPage: 1,
  nextCursors: [null], // First page has no start-after cursor
  hasNextPage: false,
  queryContractId: null,
  queryDocumentType: null,

  setQueryText: (text: string) => {
    set({ queryText: text });
  },

  setWhereClauses: (clauses: WhereClauseDto[]) => {
    set({ whereClauses: clauses });
  },

  setOrderByClauses: (clauses: OrderByClauseDto[]) => {
    set({ orderByClauses: clauses });
  },

  setQueryTarget: (contractId: string | null, documentType: string | null) => {
    set({
      queryContractId: contractId,
      queryDocumentType: documentType,
    });
  },

  fetchDocuments: async (
    contractId: string,
    documentTypeName: string,
  ) => {
    set({
      queryStatus: "waiting",
      queryStartedAt: Date.now(),
      queryError: null,
      documents: [],
      currentPage: 1,
      nextCursors: [null],
      hasNextPage: false,
      queryContractId: contractId,
      queryDocumentType: documentTypeName,
    });

    try {
      const { whereClauses, orderByClauses } = get();
      const result = await commands.documentFetchPage({
        contractId,
        documentTypeName,
        whereClauses,
        orderByClauses,
        startAfter: null,
      });

      if (result.status === "ok") {
        set({ activeTaskId: result.data.taskId });
        timeouts.start("query", () => {
          set({ queryStatus: "error", queryError: TIMEOUT_ERROR_MESSAGE, queryStartedAt: null, activeTaskId: null });
        });
        return result.data.taskId;
      }

      set({
        queryStatus: "error",
        queryError: result.error,
        queryStartedAt: null,
      });
      return null;
    } catch (e) {
      set({
        queryStatus: "error",
        queryError: e instanceof Error ? e.message : String(e),
        queryStartedAt: null,
      });
      return null;
    }
  },

  fetchPage: async (
    contractId: string,
    documentTypeName: string,
    startAfter: string | null,
  ) => {
    set({
      queryStatus: "waiting",
      queryStartedAt: Date.now(),
      queryError: null,
    });

    try {
      const { whereClauses, orderByClauses } = get();
      const result = await commands.documentFetchPage({
        contractId,
        documentTypeName,
        whereClauses,
        orderByClauses,
        startAfter,
      });

      if (result.status === "ok") {
        set({ activeTaskId: result.data.taskId });
        timeouts.start("query", () => {
          set({ queryStatus: "error", queryError: TIMEOUT_ERROR_MESSAGE, queryStartedAt: null, activeTaskId: null });
        });
        return result.data.taskId;
      }

      set({
        queryStatus: "error",
        queryError: result.error,
        queryStartedAt: null,
      });
      return null;
    } catch (e) {
      set({
        queryStatus: "error",
        queryError: e instanceof Error ? e.message : String(e),
        queryStartedAt: null,
      });
      return null;
    }
  },

  goToNextPage: async () => {
    const { documents, queryContractId, queryDocumentType, currentPage, nextCursors, hasNextPage } = get();
    if (!hasNextPage || !queryContractId || !queryDocumentType) return null;

    // The cursor for the next page is the last document ID on the current page
    const lastDoc = documents[documents.length - 1];
    if (!lastDoc) return null;

    const cursor = lastDoc.id;

    // Save the cursor so we can go back
    const newCursors = [...nextCursors];
    if (newCursors.length <= currentPage) {
      newCursors.push(cursor);
    }

    set({
      currentPage: currentPage + 1,
      nextCursors: newCursors,
    });

    return get().fetchPage(queryContractId, queryDocumentType, cursor);
  },

  goToPreviousPage: async () => {
    const { currentPage, nextCursors, queryContractId, queryDocumentType } = get();
    if (currentPage <= 1 || !queryContractId || !queryDocumentType) return null;

    const prevPage = currentPage - 1;
    const cursor = nextCursors[prevPage - 1] ?? null; // Page 1 has cursor at index 0 (null)

    set({ currentPage: prevPage });

    return get().fetchPage(queryContractId, queryDocumentType, cursor);
  },

  setDisplayMode: (mode: DocumentDisplayMode) => {
    set({ displayMode: mode });
  },

  setSearchFilter: (filter: string) => {
    set({ searchFilter: filter });
  },

  initFieldSelection: (schemaFields: string[]) => {
    const selection: Record<string, boolean> = {};

    // Schema fields are visible by default
    for (const field of schemaFields) {
      selection[field] = true;
    }

    // Private fields: only $id and $ownerId visible by default
    for (const field of DOCUMENT_PRIVATE_FIELDS) {
      selection[field] = DEFAULT_VISIBLE_PRIVATE_FIELDS.has(field);
    }

    set({ fieldSelection: selection });
  },

  toggleField: (field: string) => {
    set((state) => ({
      fieldSelection: {
        ...state.fieldSelection,
        [field]: !state.fieldSelection[field],
      },
    }));
  },

  setAllFields: (visible: boolean) => {
    set((state) => {
      const updated: Record<string, boolean> = {};
      for (const key of Object.keys(state.fieldSelection)) {
        updated[key] = visible;
      }
      return { fieldSelection: updated };
    });
  },

  clearResults: () => {
    timeouts.clearAll();
    set({
      documents: [],
      queryStatus: "idle",
      queryStartedAt: null,
      queryError: null,
      activeTaskId: null,
      currentPage: 1,
      nextCursors: [null],
      hasNextPage: false,
      searchFilter: "",
    });
  },

  subscribeToUpdates: async () => {
    const unlistenResult = await events.taskResultEvent.listen(
      (event: { payload: TaskResultEvent }) => {
        const { result, taskId } = event.payload;
        const state = get();

        // Handle document page results
        if (result.type === "documentPage") {
          // Only process results for the active query
          if (state.activeTaskId && taskId !== state.activeTaskId) return;

          timeouts.clear("query");

          const rawDocs = result.documents as Record<string, unknown>[];
          const hasMore = result.hasMore;

          // Convert flat DPP-serialized docs to DocumentPageEntry format
          const documents: DocumentPageEntry[] = rawDocs.map((doc) => {
            const id = extractIdString(doc["$id"]);
            return {
              id,
              document: {
                id,
                ownerId: extractIdString(doc["$ownerId"]),
                documentType: "",
                data: doc as JsonValue,
                revision: (doc["$revision"] as number) ?? 0,
                createdAt: (doc["$createdAt"] as number) ?? null,
                updatedAt: (doc["$updatedAt"] as number) ?? null,
                transferredAt: (doc["$transferredAt"] as number) ?? null,
              },
            };
          });

          // Empty result handling
          if (documents.length === 0) {
            if (get().currentPage > 1) {
              // Went past last page — revert to previous page (its data is still in store)
              set({
                hasNextPage: false,
                currentPage: get().currentPage - 1,
                queryStatus: "complete",
                queryStartedAt: null,
                activeTaskId: null,
              });
            } else {
              // Page 1 is empty — no results at all
              set({
                documents,
                hasNextPage: false,
                queryStatus: "complete",
                queryStartedAt: null,
                activeTaskId: null,
              });
            }
            return;
          }

          set({
            documents,
            hasNextPage: hasMore,
            queryStatus: "complete",
            queryStartedAt: null,
            activeTaskId: null,
          });
          return;
        }

        if (result.type === "documentCompleted") {
          if (state.activeTaskId && taskId !== state.activeTaskId) return;

          timeouts.clear("query");

          set({
            queryStatus: "complete",
            queryStartedAt: null,
            activeTaskId: null,
          });
        }
      },
    );

    const unlistenError = await events.taskErrorEvent.listen(
      (event: { payload: { taskId: string; domain: string; message: string } }) => {
        if (event.payload.domain !== "document") return;

        const state = get();
        if (state.activeTaskId && event.payload.taskId !== state.activeTaskId) return;

        timeouts.clearAll();

        set({
          queryStatus: "error",
          queryError: event.payload.message,
          queryStartedAt: null,
          activeTaskId: null,
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
      queryText: "",
      whereClauses: [],
      orderByClauses: [],
      documents: [],
      queryStatus: "idle",
      queryStartedAt: null,
      queryError: null,
      activeTaskId: null,
      displayMode: "json",
      searchFilter: "",
      fieldSelection: {},
      currentPage: 1,
      nextCursors: [null],
      hasNextPage: false,
      queryContractId: null,
      queryDocumentType: null,
    });
  },

  clearError: () => {
    set({ queryError: null });
  },
}));
