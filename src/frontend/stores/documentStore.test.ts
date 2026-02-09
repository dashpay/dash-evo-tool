import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { useDocumentStore, DOCUMENT_PRIVATE_FIELDS } from "./documentStore";
import type { TaskResultEvent } from "../bindings";
import type { DocumentPageEntry } from "./documentStore";

// ─── Mock bindings ──────────────────────────────────────────────────

vi.mock("../bindings", () => ({
  commands: {
    documentFetchPage: vi.fn(),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
    taskErrorEvent: {
      listen: vi.fn().mockResolvedValue(() => {}),
    },
  },
}));

import { commands, events } from "../bindings";

// ─── Test fixtures ──────────────────────────────────────────────────

function makePageEntry(overrides?: Partial<DocumentPageEntry>): DocumentPageEntry {
  return {
    id: "doc001",
    document: {
      id: "doc001",
      ownerId: "owner001",
      documentType: "domain",
      data: { label: "test", normalizedLabel: "test" },
      revision: 1,
      createdAt: 1700000000000,
      updatedAt: null,
      transferredAt: null,
    },
    ...overrides,
  };
}

// ─── Reset store between tests ──────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  useDocumentStore.setState({
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
});

// ─── Tests ──────────────────────────────────────────────────────────

describe("documentStore", () => {
  // ─── Initial state ────────────────────────────────────────────

  it("has correct initial state", () => {
    const state = useDocumentStore.getState();
    expect(state.queryText).toBe("");
    expect(state.whereClauses).toEqual([]);
    expect(state.orderByClauses).toEqual([]);
    expect(state.documents).toEqual([]);
    expect(state.queryStatus).toBe("idle");
    expect(state.queryStartedAt).toBeNull();
    expect(state.queryError).toBeNull();
    expect(state.activeTaskId).toBeNull();
    expect(state.displayMode).toBe("json");
    expect(state.searchFilter).toBe("");
    expect(state.fieldSelection).toEqual({});
    expect(state.currentPage).toBe(1);
    expect(state.nextCursors).toEqual([null]);
    expect(state.hasNextPage).toBe(false);
    expect(state.queryContractId).toBeNull();
    expect(state.queryDocumentType).toBeNull();
  });

  // ─── setQueryText ─────────────────────────────────────────────

  describe("setQueryText", () => {
    it("updates query text", () => {
      useDocumentStore.getState().setQueryText("SELECT * FROM domain");
      expect(useDocumentStore.getState().queryText).toBe("SELECT * FROM domain");
    });
  });

  // ─── setWhereClauses ──────────────────────────────────────────

  describe("setWhereClauses", () => {
    it("updates where clauses", () => {
      const clauses = [{ field: "label", operator: "==", value: "test" }];
      useDocumentStore.getState().setWhereClauses(clauses);
      expect(useDocumentStore.getState().whereClauses).toEqual(clauses);
    });
  });

  // ─── setOrderByClauses ────────────────────────────────────────

  describe("setOrderByClauses", () => {
    it("updates order-by clauses", () => {
      const clauses = [{ field: "label", direction: "asc" }];
      useDocumentStore.getState().setOrderByClauses(clauses);
      expect(useDocumentStore.getState().orderByClauses).toEqual(clauses);
    });
  });

  // ─── setQueryTarget ───────────────────────────────────────────

  describe("setQueryTarget", () => {
    it("sets contract ID and document type", () => {
      useDocumentStore.getState().setQueryTarget("contract001", "domain");
      const state = useDocumentStore.getState();
      expect(state.queryContractId).toBe("contract001");
      expect(state.queryDocumentType).toBe("domain");
    });

    it("clears target when null passed", () => {
      useDocumentStore.setState({ queryContractId: "c1", queryDocumentType: "d1" });
      useDocumentStore.getState().setQueryTarget(null, null);
      const state = useDocumentStore.getState();
      expect(state.queryContractId).toBeNull();
      expect(state.queryDocumentType).toBeNull();
    });
  });

  // ─── fetchDocuments ───────────────────────────────────────────

  describe("fetchDocuments", () => {
    it("dispatches fetch and returns task ID", async () => {
      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-doc-1" },
      });

      const taskId = await useDocumentStore
        .getState()
        .fetchDocuments("contract001", "domain");

      expect(taskId).toBe("task-doc-1");
      expect(commands.documentFetchPage).toHaveBeenCalledWith({
        contractId: "contract001",
        documentTypeName: "domain",
        whereClauses: [],
        orderByClauses: [],
        startAfter: null,
      });
    });

    it("sets waiting state during fetch", async () => {
      let resolvePromise: (value: unknown) => void;
      (commands.documentFetchPage as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolvePromise = resolve;
        }),
      );

      const fetchPromise = useDocumentStore
        .getState()
        .fetchDocuments("contract001", "domain");

      expect(useDocumentStore.getState().queryStatus).toBe("waiting");
      expect(useDocumentStore.getState().queryStartedAt).not.toBeNull();
      expect(useDocumentStore.getState().documents).toEqual([]);
      expect(useDocumentStore.getState().currentPage).toBe(1);

      resolvePromise!({ status: "ok", data: { taskId: "task-1" } });
      await fetchPromise;
    });

    it("resets pagination on new fetch", async () => {
      useDocumentStore.setState({
        currentPage: 3,
        nextCursors: [null, "cursor1", "cursor2"],
        hasNextPage: true,
      });

      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-1" },
      });

      await useDocumentStore.getState().fetchDocuments("contract001", "domain");

      expect(useDocumentStore.getState().currentPage).toBe(1);
      expect(useDocumentStore.getState().nextCursors).toEqual([null]);
      expect(useDocumentStore.getState().hasNextPage).toBe(false);
    });

    it("uses current where/order clauses", async () => {
      useDocumentStore.setState({
        whereClauses: [{ field: "label", operator: "==", value: "test" }],
        orderByClauses: [{ field: "label", direction: "asc" }],
      });

      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-1" },
      });

      await useDocumentStore.getState().fetchDocuments("contract001", "domain");

      expect(commands.documentFetchPage).toHaveBeenCalledWith({
        contractId: "contract001",
        documentTypeName: "domain",
        whereClauses: [{ field: "label", operator: "==", value: "test" }],
        orderByClauses: [{ field: "label", direction: "asc" }],
        startAfter: null,
      });
    });

    it("returns null and sets error on backend error", async () => {
      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "error",
        error: "Invalid document type",
      });

      const taskId = await useDocumentStore
        .getState()
        .fetchDocuments("contract001", "badtype");

      expect(taskId).toBeNull();
      expect(useDocumentStore.getState().queryError).toBe("Invalid document type");
      expect(useDocumentStore.getState().queryStatus).toBe("error");
    });

    it("handles thrown exception", async () => {
      (commands.documentFetchPage as Mock).mockRejectedValue(
        new Error("Network failure"),
      );

      const taskId = await useDocumentStore
        .getState()
        .fetchDocuments("contract001", "domain");

      expect(taskId).toBeNull();
      expect(useDocumentStore.getState().queryError).toBe("Network failure");
      expect(useDocumentStore.getState().queryStatus).toBe("error");
    });
  });

  // ─── fetchPage ────────────────────────────────────────────────

  describe("fetchPage", () => {
    it("dispatches paginated fetch with cursor", async () => {
      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-page-2" },
      });

      const taskId = await useDocumentStore
        .getState()
        .fetchPage("contract001", "domain", "cursor-abc");

      expect(taskId).toBe("task-page-2");
      expect(commands.documentFetchPage).toHaveBeenCalledWith({
        contractId: "contract001",
        documentTypeName: "domain",
        whereClauses: [],
        orderByClauses: [],
        startAfter: "cursor-abc",
      });
    });

    it("sets waiting state", async () => {
      let resolvePromise: (value: unknown) => void;
      (commands.documentFetchPage as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolvePromise = resolve;
        }),
      );

      const promise = useDocumentStore
        .getState()
        .fetchPage("contract001", "domain", null);

      expect(useDocumentStore.getState().queryStatus).toBe("waiting");

      resolvePromise!({ status: "ok", data: { taskId: "task-1" } });
      await promise;
    });
  });

  // ─── goToNextPage ─────────────────────────────────────────────

  describe("goToNextPage", () => {
    it("fetches next page using last document as cursor", async () => {
      useDocumentStore.setState({
        documents: [makePageEntry({ id: "doc-last" })],
        queryContractId: "contract001",
        queryDocumentType: "domain",
        currentPage: 1,
        nextCursors: [null],
        hasNextPage: true,
      });

      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-next" },
      });

      const taskId = await useDocumentStore.getState().goToNextPage();

      expect(taskId).toBe("task-next");
      expect(useDocumentStore.getState().currentPage).toBe(2);
      expect(commands.documentFetchPage).toHaveBeenCalledWith(
        expect.objectContaining({ startAfter: "doc-last" }),
      );
    });

    it("returns null when no more pages", async () => {
      useDocumentStore.setState({
        documents: [makePageEntry()],
        queryContractId: "contract001",
        queryDocumentType: "domain",
        hasNextPage: false,
      });

      const taskId = await useDocumentStore.getState().goToNextPage();
      expect(taskId).toBeNull();
      expect(commands.documentFetchPage).not.toHaveBeenCalled();
    });

    it("returns null when no query target", async () => {
      useDocumentStore.setState({
        documents: [makePageEntry()],
        hasNextPage: true,
        queryContractId: null,
        queryDocumentType: null,
      });

      const taskId = await useDocumentStore.getState().goToNextPage();
      expect(taskId).toBeNull();
    });
  });

  // ─── goToPreviousPage ─────────────────────────────────────────

  describe("goToPreviousPage", () => {
    it("fetches previous page using cursor stack", async () => {
      useDocumentStore.setState({
        queryContractId: "contract001",
        queryDocumentType: "domain",
        currentPage: 2,
        nextCursors: [null, "cursor-page2"],
      });

      (commands.documentFetchPage as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-prev" },
      });

      const taskId = await useDocumentStore.getState().goToPreviousPage();

      expect(taskId).toBe("task-prev");
      expect(useDocumentStore.getState().currentPage).toBe(1);
      expect(commands.documentFetchPage).toHaveBeenCalledWith(
        expect.objectContaining({ startAfter: null }),
      );
    });

    it("returns null on first page", async () => {
      useDocumentStore.setState({
        currentPage: 1,
        queryContractId: "contract001",
        queryDocumentType: "domain",
      });

      const taskId = await useDocumentStore.getState().goToPreviousPage();
      expect(taskId).toBeNull();
      expect(commands.documentFetchPage).not.toHaveBeenCalled();
    });
  });

  // ─── setDisplayMode ───────────────────────────────────────────

  describe("setDisplayMode", () => {
    it("switches to yaml", () => {
      useDocumentStore.getState().setDisplayMode("yaml");
      expect(useDocumentStore.getState().displayMode).toBe("yaml");
    });

    it("switches back to json", () => {
      useDocumentStore.setState({ displayMode: "yaml" });
      useDocumentStore.getState().setDisplayMode("json");
      expect(useDocumentStore.getState().displayMode).toBe("json");
    });
  });

  // ─── setSearchFilter ──────────────────────────────────────────

  describe("setSearchFilter", () => {
    it("updates search filter", () => {
      useDocumentStore.getState().setSearchFilter("alice");
      expect(useDocumentStore.getState().searchFilter).toBe("alice");
    });
  });

  // ─── initFieldSelection ───────────────────────────────────────

  describe("initFieldSelection", () => {
    it("sets schema fields as visible by default", () => {
      useDocumentStore.getState().initFieldSelection(["label", "normalizedLabel"]);

      const sel = useDocumentStore.getState().fieldSelection;
      expect(sel["label"]).toBe(true);
      expect(sel["normalizedLabel"]).toBe(true);
    });

    it("sets $id and $ownerId visible, other private fields hidden", () => {
      useDocumentStore.getState().initFieldSelection(["label"]);

      const sel = useDocumentStore.getState().fieldSelection;
      expect(sel["$id"]).toBe(true);
      expect(sel["$ownerId"]).toBe(true);
      expect(sel["$revision"]).toBe(false);
      expect(sel["$createdAt"]).toBe(false);
      expect(sel["$updatedAt"]).toBe(false);
    });

    it("includes all private fields", () => {
      useDocumentStore.getState().initFieldSelection([]);

      const sel = useDocumentStore.getState().fieldSelection;
      for (const field of DOCUMENT_PRIVATE_FIELDS) {
        expect(field in sel).toBe(true);
      }
    });
  });

  // ─── toggleField ──────────────────────────────────────────────

  describe("toggleField", () => {
    it("toggles a visible field to hidden", () => {
      useDocumentStore.setState({ fieldSelection: { label: true } });
      useDocumentStore.getState().toggleField("label");
      expect(useDocumentStore.getState().fieldSelection["label"]).toBe(false);
    });

    it("toggles a hidden field to visible", () => {
      useDocumentStore.setState({ fieldSelection: { label: false } });
      useDocumentStore.getState().toggleField("label");
      expect(useDocumentStore.getState().fieldSelection["label"]).toBe(true);
    });
  });

  // ─── setAllFields ─────────────────────────────────────────────

  describe("setAllFields", () => {
    it("selects all fields", () => {
      useDocumentStore.setState({
        fieldSelection: { label: false, $id: false, $ownerId: true },
      });
      useDocumentStore.getState().setAllFields(true);

      const sel = useDocumentStore.getState().fieldSelection;
      expect(sel["label"]).toBe(true);
      expect(sel["$id"]).toBe(true);
      expect(sel["$ownerId"]).toBe(true);
    });

    it("deselects all fields", () => {
      useDocumentStore.setState({
        fieldSelection: { label: true, $id: true },
      });
      useDocumentStore.getState().setAllFields(false);

      const sel = useDocumentStore.getState().fieldSelection;
      expect(sel["label"]).toBe(false);
      expect(sel["$id"]).toBe(false);
    });
  });

  // ─── clearResults ─────────────────────────────────────────────

  describe("clearResults", () => {
    it("resets query state", () => {
      useDocumentStore.setState({
        documents: [makePageEntry()],
        queryStatus: "complete",
        queryStartedAt: 123456,
        queryError: "old error",
        activeTaskId: "old-task",
        currentPage: 3,
        nextCursors: [null, "c1", "c2"],
        hasNextPage: true,
        searchFilter: "alice",
      });

      useDocumentStore.getState().clearResults();

      const state = useDocumentStore.getState();
      expect(state.documents).toEqual([]);
      expect(state.queryStatus).toBe("idle");
      expect(state.queryStartedAt).toBeNull();
      expect(state.queryError).toBeNull();
      expect(state.activeTaskId).toBeNull();
      expect(state.currentPage).toBe(1);
      expect(state.nextCursors).toEqual([null]);
      expect(state.hasNextPage).toBe(false);
      expect(state.searchFilter).toBe("");
    });

    it("preserves query text and clauses", () => {
      useDocumentStore.setState({
        queryText: "SELECT * FROM domain",
        whereClauses: [{ field: "label", operator: "==", value: "test" }],
        orderByClauses: [{ field: "label", direction: "asc" }],
        documents: [makePageEntry()],
      });

      useDocumentStore.getState().clearResults();

      expect(useDocumentStore.getState().queryText).toBe("SELECT * FROM domain");
      expect(useDocumentStore.getState().whereClauses).toHaveLength(1);
      expect(useDocumentStore.getState().orderByClauses).toHaveLength(1);
    });
  });

  // ─── subscribeToUpdates ───────────────────────────────────────

  describe("subscribeToUpdates", () => {
    it("subscribes to task result and error events", async () => {
      await useDocumentStore.getState().subscribeToUpdates();

      expect(events.taskResultEvent.listen).toHaveBeenCalledTimes(1);
      expect(events.taskErrorEvent.listen).toHaveBeenCalledTimes(1);
    });

    it("returns unsubscribe function", async () => {
      const unsubResult = vi.fn();
      const unsubError = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(unsubResult);
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unsubError);

      const unsub = await useDocumentStore.getState().subscribeToUpdates();
      unsub();

      expect(unsubResult).toHaveBeenCalled();
      expect(unsubError).toHaveBeenCalled();
    });

    it("processes Document result event with page data", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useDocumentStore.setState({
        queryStatus: "waiting",
        activeTaskId: "task-doc-1",
      });
      await useDocumentStore.getState().subscribeToUpdates();

      const docs = [makePageEntry(), makePageEntry({ id: "doc002" })];
      resultCallback!({
        payload: {
          taskId: "task-doc-1",
          resultType: "Document",
          payload: { documents: docs, hasMore: true },
        },
      });

      const state = useDocumentStore.getState();
      expect(state.queryStatus).toBe("complete");
      expect(state.documents).toEqual(docs);
      expect(state.hasNextPage).toBe(true);
      expect(state.activeTaskId).toBeNull();
    });

    it("handles Document result with null payload", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useDocumentStore.setState({
        queryStatus: "waiting",
        activeTaskId: "task-doc-2",
      });
      await useDocumentStore.getState().subscribeToUpdates();

      resultCallback!({
        payload: {
          taskId: "task-doc-2",
          resultType: "Document",
          payload: null,
        },
      });

      expect(useDocumentStore.getState().queryStatus).toBe("complete");
      expect(useDocumentStore.getState().activeTaskId).toBeNull();
    });

    it("ignores non-Document result events", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useDocumentStore.setState({
        queryStatus: "waiting",
        activeTaskId: "task-1",
      });
      await useDocumentStore.getState().subscribeToUpdates();

      resultCallback!({
        payload: {
          taskId: "task-other",
          resultType: "Contract",
          payload: null,
        },
      });

      // State unchanged
      expect(useDocumentStore.getState().queryStatus).toBe("waiting");
    });

    it("ignores Document results for different task IDs", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useDocumentStore.setState({
        queryStatus: "waiting",
        activeTaskId: "task-active",
      });
      await useDocumentStore.getState().subscribeToUpdates();

      resultCallback!({
        payload: {
          taskId: "task-stale",
          resultType: "Document",
          payload: { documents: [makePageEntry()], hasMore: false },
        },
      });

      // State unchanged — stale result ignored
      expect(useDocumentStore.getState().queryStatus).toBe("waiting");
      expect(useDocumentStore.getState().documents).toEqual([]);
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

      useDocumentStore.setState({
        queryStatus: "waiting",
        activeTaskId: "task-fail",
      });
      await useDocumentStore.getState().subscribeToUpdates();

      errorCallback!({
        payload: { taskId: "task-fail", message: "Query timeout" },
      });

      expect(useDocumentStore.getState().queryStatus).toBe("error");
      expect(useDocumentStore.getState().queryError).toBe("Query timeout");
      expect(useDocumentStore.getState().activeTaskId).toBeNull();
    });

    it("ignores error events for different task IDs", async () => {
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

      useDocumentStore.setState({
        queryStatus: "waiting",
        activeTaskId: "task-active",
      });
      await useDocumentStore.getState().subscribeToUpdates();

      errorCallback!({
        payload: { taskId: "task-unrelated", message: "Some error" },
      });

      // State unchanged
      expect(useDocumentStore.getState().queryStatus).toBe("waiting");
      expect(useDocumentStore.getState().queryError).toBeNull();
    });
  });

  // ─── clearError ───────────────────────────────────────────────

  describe("clearError", () => {
    it("clears the error state", () => {
      useDocumentStore.setState({ queryError: "Something went wrong" });
      useDocumentStore.getState().clearError();
      expect(useDocumentStore.getState().queryError).toBeNull();
    });
  });
});
