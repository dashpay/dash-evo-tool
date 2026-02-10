import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  createMockBindings,
  mockBindingsModule,
  type MockBindingsResult,
} from "./mock-ipc";

describe("mock-ipc", () => {
  let mocks: MockBindingsResult;

  beforeEach(() => {
    mocks = createMockBindings();
  });

  // ─── createMockBindings ──────────────────────────────────────────

  describe("createMockBindings", () => {
    it("provides a commands object with all 181 command names", () => {
      const names = Object.keys(mocks.commands);
      expect(names.length).toBe(181);
    });

    it("provides an events object with all 8 event names", () => {
      const names = Object.keys(mocks.events);
      expect(names).toEqual([
        "scheduledVoteExecutedEvent",
        "spvStatusEvent",
        "taskErrorEvent",
        "taskResultEvent",
        "walletUpdatedEvent",
        "zmqChainLockedBlockEvent",
        "zmqConnectionStatusEvent",
        "zmqIsLockedTransactionEvent",
      ]);
    });

    it("every command is a callable mock function", () => {
      for (const [, fn] of Object.entries(mocks.commands)) {
        expect(typeof fn).toBe("function");
        // vi.fn() instances have a `mock` property
        expect(fn).toHaveProperty("mock");
      }
    });
  });

  // ─── Default command responses ───────────────────────────────────

  describe("default command responses", () => {
    it("walletListAll returns ok with empty lists", async () => {
      const result = await mocks.commands.walletListAll();
      expect(result).toEqual({
        status: "ok",
        data: { hdWallets: [], singleKeyWallets: [], selected: null },
      });
    });

    it("identityListLocal returns ok with empty array", async () => {
      const result = await mocks.commands.identityListLocal();
      expect(result).toEqual({ status: "ok", data: [] });
    });

    it("settingsGet returns ok with default settings", async () => {
      const result = await mocks.commands.settingsGet();
      expect(result).toEqual({
        status: "ok",
        data: expect.objectContaining({
          theme: "Dark",
          developerMode: false,
          onboardingCompleted: false,
        }),
      });
    });

    it("dispatch-style commands return ok with taskId", async () => {
      const result = await mocks.commands.identityLoad({ identityId: "abc" });
      expect(result).toEqual({
        status: "ok",
        data: { taskId: "mock-task-id" },
      });
    });

    it("getAppVersion returns a version string", async () => {
      const result = await mocks.commands.getAppVersion();
      expect(result).toBe("0.0.0-test");
    });

    it("contextIsDeveloperMode returns boolean", async () => {
      const result = await mocks.commands.contextIsDeveloperMode();
      expect(result).toBe(false);
    });

    it("walletStopSpv returns undefined (void)", async () => {
      const result = await mocks.commands.walletStopSpv();
      expect(result).toBeUndefined();
    });
  });

  // ─── Command overrides ──────────────────────────────────────────

  describe("command overrides", () => {
    it("accepts overrides at creation time", async () => {
      const customHandler = vi.fn().mockResolvedValue({
        status: "ok",
        data: { hdWallets: [{ seedHash: "abc" }], singleKeyWallets: [], selected: "abc" },
      });

      const custom = createMockBindings({ walletListAll: customHandler });
      const result = await custom.commands.walletListAll();
      expect(result).toEqual({
        status: "ok",
        data: { hdWallets: [{ seedHash: "abc" }], singleKeyWallets: [], selected: "abc" },
      });
    });

    it("overrides only the specified command, others use defaults", async () => {
      const custom = createMockBindings({
        walletListAll: vi.fn().mockResolvedValue("custom"),
      });
      // Override works
      expect(await custom.commands.walletListAll()).toBe("custom");
      // Other commands still use defaults
      const version = await custom.commands.getAppVersion();
      expect(version).toBe("0.0.0-test");
    });

    it("configureMock replaces a command handler at runtime", async () => {
      // Default first
      const before = await mocks.commands.getAppVersion();
      expect(before).toBe("0.0.0-test");

      // Override
      mocks.configureMock(
        "getAppVersion",
        vi.fn().mockResolvedValue("1.2.3"),
      );
      const after = await mocks.commands.getAppVersion();
      expect(after).toBe("1.2.3");
    });
  });

  // ─── Call history ───────────────────────────────────────────────

  describe("call history", () => {
    it("records arguments for default commands", async () => {
      await mocks.commands.identityLoad({ identityId: "test-id-123" });
      const history = mocks.callHistory.get("identityLoad");
      expect(history).toHaveLength(1);
      expect(history![0]).toEqual({ identityId: "test-id-123" });
    });

    it("records multiple calls", async () => {
      await mocks.commands.identitySetAlias({ identityId: "a", alias: "Alice" });
      await mocks.commands.identitySetAlias({ identityId: "b", alias: "Bob" });
      const history = mocks.callHistory.get("identitySetAlias");
      expect(history).toHaveLength(2);
      expect(history![0]).toEqual({ identityId: "a", alias: "Alice" });
      expect(history![1]).toEqual({ identityId: "b", alias: "Bob" });
    });

    it("records undefined for commands with no arguments", async () => {
      await mocks.commands.getAppVersion();
      const history = mocks.callHistory.get("getAppVersion");
      expect(history).toHaveLength(1);
      expect(history![0]).toBeUndefined();
    });

    it("call history is empty initially", () => {
      expect(mocks.callHistory.size).toBe(0);
    });
  });

  // ─── Event mocks ───────────────────────────────────────────────

  describe("event mocks", () => {
    it("listen returns an unsubscribe function", async () => {
      const cb = vi.fn();
      const unsub = await mocks.events.taskResultEvent.listen(cb);
      expect(typeof unsub).toBe("function");
    });

    it("listen tracks callback in eventListeners", async () => {
      const cb = vi.fn();
      await mocks.events.taskResultEvent.listen(cb);
      const listeners = mocks.eventListeners.get("taskResultEvent");
      expect(listeners).toHaveLength(1);
    });

    it("unsubscribe removes the listener", async () => {
      const cb = vi.fn();
      const unsub = await mocks.events.taskResultEvent.listen(cb);
      unsub();
      const listeners = mocks.eventListeners.get("taskResultEvent");
      expect(listeners).toHaveLength(0);
    });

    it("supports multiple simultaneous listeners", async () => {
      const cb1 = vi.fn();
      const cb2 = vi.fn();
      await mocks.events.taskResultEvent.listen(cb1);
      await mocks.events.taskResultEvent.listen(cb2);
      const listeners = mocks.eventListeners.get("taskResultEvent");
      expect(listeners).toHaveLength(2);
    });

    it("once listener fires once and auto-removes", async () => {
      const cb = vi.fn();
      await mocks.events.taskResultEvent.once(cb);

      // Emit — should fire
      mocks.emitMockEvent("taskResultEvent", { taskId: "t1" });
      expect(cb).toHaveBeenCalledTimes(1);
      expect(cb).toHaveBeenCalledWith({ payload: { taskId: "t1" } });

      // Second emit — should NOT fire
      mocks.emitMockEvent("taskResultEvent", { taskId: "t2" });
      expect(cb).toHaveBeenCalledTimes(1);

      // And listener is removed
      expect(mocks.eventListeners.get("taskResultEvent")).toHaveLength(0);
    });
  });

  // ─── emitMockEvent ──────────────────────────────────────────────

  describe("emitMockEvent", () => {
    it("delivers payload to all registered listeners", async () => {
      const cb1 = vi.fn();
      const cb2 = vi.fn();
      await mocks.events.taskResultEvent.listen(cb1);
      await mocks.events.taskResultEvent.listen(cb2);

      mocks.emitMockEvent("taskResultEvent", {
        taskId: "task-42",
        resultType: "Wallet",
        payload: { wallet: "data" },
      });

      expect(cb1).toHaveBeenCalledWith({
        payload: {
          taskId: "task-42",
          resultType: "Wallet",
          payload: { wallet: "data" },
        },
      });
      expect(cb2).toHaveBeenCalledWith({
        payload: {
          taskId: "task-42",
          resultType: "Wallet",
          payload: { wallet: "data" },
        },
      });
    });

    it("does not fail when no listeners are registered", () => {
      // Should not throw
      mocks.emitMockEvent("taskErrorEvent", { taskId: "x", message: "err", details: "" });
    });
  });

  // ─── resetMocks ─────────────────────────────────────────────────

  describe("resetMocks", () => {
    it("clears call history", async () => {
      await mocks.commands.walletListAll();
      expect(mocks.callHistory.size).toBeGreaterThan(0);

      mocks.resetMocks();
      expect(mocks.callHistory.size).toBe(0);
    });

    it("clears mock call counts", async () => {
      await mocks.commands.walletListAll();
      expect(mocks.commands.walletListAll).toHaveBeenCalledTimes(1);

      mocks.resetMocks();
      expect(mocks.commands.walletListAll).toHaveBeenCalledTimes(0);
    });

    it("removes all event listeners", async () => {
      const cb = vi.fn();
      await mocks.events.taskResultEvent.listen(cb);
      expect(mocks.eventListeners.get("taskResultEvent")).toHaveLength(1);

      mocks.resetMocks();
      expect(mocks.eventListeners.get("taskResultEvent")).toHaveLength(0);
    });
  });

  // ─── mockBindingsModule ─────────────────────────────────────────

  describe("mockBindingsModule", () => {
    it("returns object with commands and events", () => {
      const module = mockBindingsModule(mocks);
      expect(module).toHaveProperty("commands");
      expect(module).toHaveProperty("events");
      expect(Object.keys(module.commands).length).toBe(181);
      expect(Object.keys(module.events).length).toBe(8);
    });
  });
});
