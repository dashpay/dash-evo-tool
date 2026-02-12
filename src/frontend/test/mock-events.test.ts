import { describe, it, expect, vi, beforeEach } from "vitest";
import { createMockBindings, type MockBindingsResult } from "./mock-ipc";
import {
  emitTaskResult,
  emitTaskError,
  emitWalletUpdated,
  emitSpvStatus,
  emitZmqChainLockedBlock,
  emitZmqConnectionStatus,
  emitZmqIsLockedTransaction,
  emitScheduledVoteExecuted,
  emitMockEvent,
  getListenerCount,
  getEventListeners,
} from "./mock-events";

describe("mock-events", () => {
  let mocks: MockBindingsResult;

  beforeEach(() => {
    mocks = createMockBindings();
  });

  // ─── emitTaskResult ─────────────────────────────────────────────

  describe("emitTaskResult", () => {
    it("delivers typed TaskResultEvent payload to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.taskResultEvent.listen(cb);

      emitTaskResult(mocks, {
        taskId: "task-1",
        result: { type: "identityCompleted", identityId: "abc123" },
      });

      expect(cb).toHaveBeenCalledWith({
        payload: {
          taskId: "task-1",
          result: { type: "identityCompleted", identityId: "abc123" },
        },
      });
    });
  });

  // ─── emitTaskError ──────────────────────────────────────────────

  describe("emitTaskError", () => {
    it("delivers typed TaskErrorEvent payload to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.taskErrorEvent.listen(cb);

      emitTaskError(mocks, {
        taskId: "task-2",
        domain: "identity",
        message: "Something went wrong",
        details: "Detailed error trace",
        recoverable: true,
      });

      expect(cb).toHaveBeenCalledWith({
        payload: {
          taskId: "task-2",
          domain: "identity",
          message: "Something went wrong",
          details: "Detailed error trace",
          recoverable: true,
        },
      });
    });
  });

  // ─── emitWalletUpdated ──────────────────────────────────────────

  describe("emitWalletUpdated", () => {
    it("delivers WalletUpdatedEvent to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.walletUpdatedEvent.listen(cb);

      emitWalletUpdated(mocks, {
        walletSeedHash: "aabb",
        network: "Testnet",
      });

      expect(cb).toHaveBeenCalledWith({
        payload: { walletSeedHash: "aabb", network: "Testnet" },
      });
    });
  });

  // ─── emitSpvStatus ──────────────────────────────────────────────

  describe("emitSpvStatus", () => {
    it("delivers SpvStatusEvent to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.spvStatusEvent.listen(cb);

      emitSpvStatus(mocks, {
        network: "Mainnet",
        status: "Syncing",
        syncProgressPct: 75.5,
        headerHeight: 123456,
      });

      expect(cb).toHaveBeenCalledWith({
        payload: {
          network: "Mainnet",
          status: "Syncing",
          syncProgressPct: 75.5,
          headerHeight: 123456,
        },
      });
    });
  });

  // ─── emitZmqChainLockedBlock ────────────────────────────────────

  describe("emitZmqChainLockedBlock", () => {
    it("delivers ZmqChainLockedBlockEvent to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.zmqChainLockedBlockEvent.listen(cb);

      emitZmqChainLockedBlock(mocks, {
        network: "Testnet",
        blockHeight: 999,
        blockHash: "00aabb",
        txCount: 5,
        txIds: ["tx1", "tx2"],
        rawBlock: "aabb",
        rawChainLock: "ccdd",
        signature: "sig1",
        isValid: true,
      });

      expect(cb).toHaveBeenCalledWith({
        payload: {
          network: "Testnet",
          blockHeight: 999,
          blockHash: "00aabb",
          txCount: 5,
          txIds: ["tx1", "tx2"],
          rawBlock: "aabb",
          rawChainLock: "ccdd",
          signature: "sig1",
          isValid: true,
        },
      });
    });
  });

  // ─── emitZmqConnectionStatus ────────────────────────────────────

  describe("emitZmqConnectionStatus", () => {
    it("delivers ZmqConnectionStatusEvent to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.zmqConnectionStatusEvent.listen(cb);

      emitZmqConnectionStatus(mocks, {
        network: "Mainnet",
        connected: true,
      });

      expect(cb).toHaveBeenCalledWith({
        payload: { network: "Mainnet", connected: true },
      });
    });
  });

  // ─── emitZmqIsLockedTransaction ─────────────────────────────────

  describe("emitZmqIsLockedTransaction", () => {
    it("delivers ZmqIsLockedTransactionEvent to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.zmqIsLockedTransactionEvent.listen(cb);

      emitZmqIsLockedTransaction(mocks, {
        network: "Testnet",
        txid: "deadbeef",
        rawTx: "0100000001...",
        rawIsLock: "aabb",
        utxoCount: 3,
        isValid: true,
      });

      expect(cb).toHaveBeenCalledWith({
        payload: {
          network: "Testnet",
          txid: "deadbeef",
          rawTx: "0100000001...",
          rawIsLock: "aabb",
          utxoCount: 3,
          isValid: true,
        },
      });
    });
  });

  // ─── emitScheduledVoteExecuted ──────────────────────────────────

  describe("emitScheduledVoteExecuted", () => {
    it("delivers ScheduledVoteExecutedEvent to listeners", async () => {
      const cb = vi.fn();
      await mocks.events.scheduledVoteExecutedEvent.listen(cb);

      emitScheduledVoteExecuted(mocks, {
        contestedName: "alice",
        voterId: "voter-123",
        success: true,
        error: null,
      });

      expect(cb).toHaveBeenCalledWith({
        payload: {
          contestedName: "alice",
          voterId: "voter-123",
          success: true,
          error: null,
        },
      });
    });
  });

  // ─── emitMockEvent (generic) ────────────────────────────────────

  describe("emitMockEvent (generic)", () => {
    it("works for any event name with arbitrary payload", async () => {
      const cb = vi.fn();
      await mocks.events.taskResultEvent.listen(cb);

      emitMockEvent(mocks, "taskResultEvent", { custom: "data" });

      expect(cb).toHaveBeenCalledWith({ payload: { custom: "data" } });
    });
  });

  // ─── Assertion helpers ──────────────────────────────────────────

  describe("getListenerCount", () => {
    it("returns 0 when no listeners registered", () => {
      expect(getListenerCount(mocks, "taskResultEvent")).toBe(0);
    });

    it("returns correct count after registering listeners", async () => {
      await mocks.events.taskResultEvent.listen(vi.fn());
      await mocks.events.taskResultEvent.listen(vi.fn());
      expect(getListenerCount(mocks, "taskResultEvent")).toBe(2);
    });

    it("returns correct count after unsubscribing", async () => {
      const unsub = await mocks.events.taskResultEvent.listen(vi.fn());
      await mocks.events.taskResultEvent.listen(vi.fn());
      unsub();
      expect(getListenerCount(mocks, "taskResultEvent")).toBe(1);
    });
  });

  describe("getEventListeners", () => {
    it("returns empty array when no listeners registered", () => {
      expect(getEventListeners(mocks, "taskErrorEvent")).toEqual([]);
    });

    it("returns a copy of the listeners array", async () => {
      const cb = vi.fn();
      await mocks.events.taskErrorEvent.listen(cb);
      const listeners = getEventListeners(mocks, "taskErrorEvent");
      expect(listeners).toHaveLength(1);

      // Modifying the returned array should not affect the internal state
      listeners.pop();
      expect(getEventListeners(mocks, "taskErrorEvent")).toHaveLength(1);
    });
  });
});
