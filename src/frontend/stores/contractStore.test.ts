import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { useContractStore } from "./contractStore";
import type {
  ContractSummaryDto,
  DataContractDto,
  TaskResultEvent,
} from "../bindings";

// ─── Mock bindings ──────────────────────────────────────────────────

vi.mock("../bindings", () => ({
  commands: {
    contractListLocal: vi.fn(),
    contractGetById: vi.fn(),
    contractSetAlias: vi.fn(),
    contractRemove: vi.fn(),
    contractFetch: vi.fn(),
    contractFetchWithDescriptions: vi.fn(),
    contractSave: vi.fn(),
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

function makeSummary(
  overrides?: Partial<ContractSummaryDto>,
): ContractSummaryDto {
  return {
    id: "contract001",
    alias: "DPNS Contract",
    documentTypeCount: 3,
    tokenCount: 0,
    ...overrides,
  };
}

function makeDetail(overrides?: Partial<DataContractDto>): DataContractDto {
  return {
    id: "contract001",
    ownerId: "owner001",
    alias: "DPNS Contract",
    version: 1,
    documentTypeNames: ["domain", "preorder"],
    tokenCount: 0,
    schemaJson: { type: "object" },
    ...overrides,
  };
}

// ─── Reset store between tests ──────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  useContractStore.setState({
    contracts: [],
    selectedContractId: null,
    selectedContractDetail: null,
    loading: false,
    fetching: false,
    error: null,
  });
});

// ─── Tests ──────────────────────────────────────────────────────────

describe("contractStore", () => {
  // ─── Initial state ────────────────────────────────────────────

  it("has correct initial state", () => {
    const state = useContractStore.getState();
    expect(state.contracts).toEqual([]);
    expect(state.selectedContractId).toBeNull();
    expect(state.selectedContractDetail).toBeNull();
    expect(state.loading).toBe(false);
    expect(state.fetching).toBe(false);
    expect(state.error).toBeNull();
  });

  // ─── loadContracts ────────────────────────────────────────────

  describe("loadContracts", () => {
    it("loads contracts from backend", async () => {
      const contracts = [makeSummary(), makeSummary({ id: "contract002", alias: "DashPay" })];
      (commands.contractListLocal as Mock).mockResolvedValue({
        status: "ok",
        data: contracts,
      });

      await useContractStore.getState().loadContracts();

      expect(commands.contractListLocal).toHaveBeenCalled();
      const state = useContractStore.getState();
      expect(state.contracts).toEqual(contracts);
      expect(state.loading).toBe(false);
      expect(state.error).toBeNull();
    });

    it("sets loading state during load", async () => {
      let resolvePromise: (value: unknown) => void;
      (commands.contractListLocal as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolvePromise = resolve;
        }),
      );

      const loadPromise = useContractStore.getState().loadContracts();
      expect(useContractStore.getState().loading).toBe(true);

      resolvePromise!({ status: "ok", data: [] });
      await loadPromise;
      expect(useContractStore.getState().loading).toBe(false);
    });

    it("handles error from backend", async () => {
      (commands.contractListLocal as Mock).mockResolvedValue({
        status: "error",
        error: "Database connection failed",
      });

      await useContractStore.getState().loadContracts();

      const state = useContractStore.getState();
      expect(state.error).toBe("Database connection failed");
      expect(state.loading).toBe(false);
    });

    it("handles thrown exception", async () => {
      (commands.contractListLocal as Mock).mockRejectedValue(
        new Error("Network error"),
      );

      await useContractStore.getState().loadContracts();

      const state = useContractStore.getState();
      expect(state.error).toBe("Network error");
      expect(state.loading).toBe(false);
    });
  });

  // ─── getContractById ──────────────────────────────────────────

  describe("getContractById", () => {
    it("returns contract detail on success", async () => {
      const detail = makeDetail();
      (commands.contractGetById as Mock).mockResolvedValue({
        status: "ok",
        data: detail,
      });

      const result =
        await useContractStore.getState().getContractById("contract001");

      expect(result).toEqual(detail);
    });

    it("returns null when contract not found", async () => {
      (commands.contractGetById as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      const result =
        await useContractStore.getState().getContractById("nonexistent");

      expect(result).toBeNull();
    });

    it("returns null and sets error on failure", async () => {
      (commands.contractGetById as Mock).mockResolvedValue({
        status: "error",
        error: "Not found",
      });

      const result =
        await useContractStore.getState().getContractById("contract001");

      expect(result).toBeNull();
      expect(useContractStore.getState().error).toBe("Not found");
    });
  });

  // ─── selectContract ───────────────────────────────────────────

  describe("selectContract", () => {
    it("selects a contract and loads its detail", async () => {
      const detail = makeDetail();
      (commands.contractGetById as Mock).mockResolvedValue({
        status: "ok",
        data: detail,
      });

      await useContractStore.getState().selectContract("contract001");

      const state = useContractStore.getState();
      expect(state.selectedContractId).toBe("contract001");
      expect(state.selectedContractDetail).toEqual(detail);
    });

    it("clears selection when null passed", async () => {
      useContractStore.setState({
        selectedContractId: "contract001",
        selectedContractDetail: makeDetail(),
      });

      await useContractStore.getState().selectContract(null);

      const state = useContractStore.getState();
      expect(state.selectedContractId).toBeNull();
      expect(state.selectedContractDetail).toBeNull();
    });

    it("clears old detail immediately when selecting new contract", async () => {
      useContractStore.setState({
        selectedContractId: "old",
        selectedContractDetail: makeDetail({ id: "old" }),
      });

      let resolvePromise: (value: unknown) => void;
      (commands.contractGetById as Mock).mockReturnValue(
        new Promise((resolve) => {
          resolvePromise = resolve;
        }),
      );

      const selectPromise =
        useContractStore.getState().selectContract("contract001");

      // Detail should be cleared while loading
      expect(useContractStore.getState().selectedContractDetail).toBeNull();
      expect(useContractStore.getState().selectedContractId).toBe(
        "contract001",
      );

      resolvePromise!({ status: "ok", data: makeDetail() });
      await selectPromise;
    });
  });

  // ─── setAlias ─────────────────────────────────────────────────

  describe("setAlias", () => {
    it("updates alias in contract list", async () => {
      useContractStore.setState({ contracts: [makeSummary()] });
      (commands.contractSetAlias as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useContractStore.getState().setAlias("contract001", "New Name");

      const contract = useContractStore.getState().contracts[0];
      expect(contract.alias).toBe("New Name");
    });

    it("updates alias in selected contract detail", async () => {
      useContractStore.setState({
        contracts: [makeSummary()],
        selectedContractId: "contract001",
        selectedContractDetail: makeDetail(),
      });
      (commands.contractSetAlias as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useContractStore.getState().setAlias("contract001", "Renamed");

      expect(useContractStore.getState().selectedContractDetail?.alias).toBe(
        "Renamed",
      );
    });

    it("clears alias when null passed", async () => {
      useContractStore.setState({
        contracts: [makeSummary({ alias: "Old Name" })],
      });
      (commands.contractSetAlias as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useContractStore.getState().setAlias("contract001", null);

      expect(useContractStore.getState().contracts[0].alias).toBeNull();
    });

    it("sets error on failure", async () => {
      useContractStore.setState({ contracts: [makeSummary()] });
      (commands.contractSetAlias as Mock).mockResolvedValue({
        status: "error",
        error: "Permission denied",
      });

      await useContractStore.getState().setAlias("contract001", "New");

      expect(useContractStore.getState().error).toBe("Permission denied");
      // Original alias unchanged
      expect(useContractStore.getState().contracts[0].alias).toBe(
        "DPNS Contract",
      );
    });
  });

  // ─── removeContract ───────────────────────────────────────────

  describe("removeContract", () => {
    it("removes contract from list", async () => {
      useContractStore.setState({
        contracts: [
          makeSummary(),
          makeSummary({ id: "contract002", alias: "Other" }),
        ],
      });
      (commands.contractRemove as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useContractStore.getState().removeContract("contract001");

      const contracts = useContractStore.getState().contracts;
      expect(contracts).toHaveLength(1);
      expect(contracts[0].id).toBe("contract002");
    });

    it("clears selection when removing selected contract", async () => {
      useContractStore.setState({
        contracts: [makeSummary()],
        selectedContractId: "contract001",
        selectedContractDetail: makeDetail(),
      });
      (commands.contractRemove as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useContractStore.getState().removeContract("contract001");

      const state = useContractStore.getState();
      expect(state.selectedContractId).toBeNull();
      expect(state.selectedContractDetail).toBeNull();
    });

    it("preserves selection when removing different contract", async () => {
      useContractStore.setState({
        contracts: [
          makeSummary(),
          makeSummary({ id: "contract002" }),
        ],
        selectedContractId: "contract001",
        selectedContractDetail: makeDetail(),
      });
      (commands.contractRemove as Mock).mockResolvedValue({
        status: "ok",
        data: null,
      });

      await useContractStore.getState().removeContract("contract002");

      expect(useContractStore.getState().selectedContractId).toBe(
        "contract001",
      );
      expect(
        useContractStore.getState().selectedContractDetail,
      ).not.toBeNull();
    });

    it("sets error on failure", async () => {
      useContractStore.setState({ contracts: [makeSummary()] });
      (commands.contractRemove as Mock).mockResolvedValue({
        status: "error",
        error: "Cannot remove system contract",
      });

      await useContractStore.getState().removeContract("contract001");

      expect(useContractStore.getState().error).toBe(
        "Cannot remove system contract",
      );
      expect(useContractStore.getState().contracts).toHaveLength(1);
    });
  });

  // ─── fetchContracts ───────────────────────────────────────────

  describe("fetchContracts", () => {
    it("dispatches fetch and returns task ID", async () => {
      (commands.contractFetch as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-123" },
      });

      const taskId = await useContractStore
        .getState()
        .fetchContracts(["id1", "id2"]);

      expect(taskId).toBe("task-123");
      expect(commands.contractFetch).toHaveBeenCalledWith({
        contractIds: ["id1", "id2"],
      });
      expect(useContractStore.getState().fetching).toBe(true);
    });

    it("sets fetching state", async () => {
      (commands.contractFetch as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-123" },
      });

      await useContractStore.getState().fetchContracts(["id1"]);

      expect(useContractStore.getState().fetching).toBe(true);
    });

    it("returns null and sets error on failure", async () => {
      (commands.contractFetch as Mock).mockResolvedValue({
        status: "error",
        error: "Invalid contract ID",
      });

      const taskId = await useContractStore
        .getState()
        .fetchContracts(["bad-id"]);

      expect(taskId).toBeNull();
      expect(useContractStore.getState().error).toBe("Invalid contract ID");
      expect(useContractStore.getState().fetching).toBe(false);
    });
  });

  // ─── fetchContractsWithDescriptions ───────────────────────────

  describe("fetchContractsWithDescriptions", () => {
    it("dispatches fetch and returns task ID", async () => {
      (commands.contractFetchWithDescriptions as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-456" },
      });

      const taskId = await useContractStore
        .getState()
        .fetchContractsWithDescriptions(["id1"]);

      expect(taskId).toBe("task-456");
    });
  });

  // ─── saveContract ─────────────────────────────────────────────

  describe("saveContract", () => {
    it("dispatches save and returns task ID", async () => {
      (commands.contractSave as Mock).mockResolvedValue({
        status: "ok",
        data: { taskId: "task-789" },
      });

      const taskId = await useContractStore
        .getState()
        .saveContract({ type: "object" }, "My Contract", false);

      expect(taskId).toBe("task-789");
      expect(commands.contractSave).toHaveBeenCalledWith({
        contractJson: { type: "object" },
        alias: "My Contract",
        insertTokens: false,
      });
    });

    it("returns null and sets error on failure", async () => {
      (commands.contractSave as Mock).mockResolvedValue({
        status: "error",
        error: "Save failed",
      });

      const taskId = await useContractStore
        .getState()
        .saveContract({}, null, false);

      expect(taskId).toBeNull();
      expect(useContractStore.getState().error).toBe("Save failed");
    });
  });

  // ─── subscribeToUpdates ───────────────────────────────────────

  describe("subscribeToUpdates", () => {
    it("subscribes to task result and error events", async () => {
      await useContractStore.getState().subscribeToUpdates();

      expect(events.taskResultEvent.listen).toHaveBeenCalledTimes(1);
      expect(events.taskErrorEvent.listen).toHaveBeenCalledTimes(1);
    });

    it("returns unsubscribe function", async () => {
      const unsubResult = vi.fn();
      const unsubError = vi.fn();
      (events.taskResultEvent.listen as Mock).mockResolvedValue(unsubResult);
      (events.taskErrorEvent.listen as Mock).mockResolvedValue(unsubError);

      const unsub = await useContractStore.getState().subscribeToUpdates();
      unsub();

      expect(unsubResult).toHaveBeenCalled();
      expect(unsubError).toHaveBeenCalled();
    });

    it("reloads contracts on Contract result event", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      // Set up loadContracts mock
      (commands.contractListLocal as Mock).mockResolvedValue({
        status: "ok",
        data: [makeSummary()],
      });

      useContractStore.setState({ fetching: true });
      await useContractStore.getState().subscribeToUpdates();

      // Simulate a contract result event
      resultCallback!({
        payload: {
          taskId: "task-123",
          resultType: "Contract",
          payload: null,
        },
      });

      expect(useContractStore.getState().fetching).toBe(false);
      expect(commands.contractListLocal).toHaveBeenCalled();
    });

    it("ignores non-Contract result events", async () => {
      let resultCallback: (event: { payload: TaskResultEvent }) => void;
      (events.taskResultEvent.listen as Mock).mockImplementation(
        async (cb: (event: { payload: TaskResultEvent }) => void) => {
          resultCallback = cb;
          return () => {};
        },
      );

      useContractStore.setState({ fetching: true });
      await useContractStore.getState().subscribeToUpdates();

      // Simulate an Identity result event
      resultCallback!({
        payload: {
          taskId: "task-456",
          resultType: "Identity",
          payload: null,
        },
      });

      // Fetching should not change for non-Contract events
      expect(useContractStore.getState().fetching).toBe(true);
      expect(commands.contractListLocal).not.toHaveBeenCalled();
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

      useContractStore.setState({ fetching: true });
      await useContractStore.getState().subscribeToUpdates();

      errorCallback!({
        payload: { taskId: "task-999", message: "Backend crash" },
      });

      expect(useContractStore.getState().error).toBe("Backend crash");
      expect(useContractStore.getState().fetching).toBe(false);
    });
  });

  // ─── clearError ───────────────────────────────────────────────

  describe("clearError", () => {
    it("clears the error state", () => {
      useContractStore.setState({ error: "Something went wrong" });

      useContractStore.getState().clearError();

      expect(useContractStore.getState().error).toBeNull();
    });
  });
});
