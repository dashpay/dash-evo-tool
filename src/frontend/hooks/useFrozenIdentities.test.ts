import { renderHook, act, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, type Mock } from "vitest";
import { useFrozenIdentities } from "./useFrozenIdentities";
import { createMockIdentity } from "@/test/fixtures";

// ─── Mock bindings ───────────────────────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands, events } from "@/bindings";

const mockIdentities = [
  createMockIdentity({
    id: "aabbcc001122",
    alias: "Alice",
    balance: 1000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "active",
    identityType: "user",
  }),
  createMockIdentity({
    id: "ddeeff334455667788990011",
    alias: null,
    balance: 2000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "active",
    identityType: "user",
  }),
  createMockIdentity({
    id: "112233445566",
    alias: "Bob",
    balance: 3000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "active",
    identityType: "user",
  }),
];

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: (sel?: (s: Record<string, unknown>) => unknown) => {
    const s = {
      identities: mockIdentities,
    };
    return sel ? sel(s) : s;
  },
}));

// ─── Helpers ─────────────────────────────────────────────────────────────

/** Retrieve the most recently registered listener for a given event. */
function getLastListener(
  eventMock: { listen: Mock },
): ((event: { payload: unknown }) => void) | null {
  const { calls } = (eventMock.listen as Mock).mock;
  return calls.length > 0 ? calls[calls.length - 1][0] : null;
}

// ─── Tests ──────────────────────────────────────────────────────────────────

describe("useFrozenIdentities", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    (commands.tokenQueryFrozenIdentities as Mock).mockResolvedValue({
      status: "ok",
      data: { taskId: "frozen-task-1" },
    });
  });

  it("starts in loading state", () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));
    expect(result.current.loading).toBe(true);
    expect(result.current.frozenIdentities).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it("dispatches tokenQueryFrozenIdentities with all local identity IDs", async () => {
    renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(commands.tokenQueryFrozenIdentities).toHaveBeenCalledWith({
        tokenId: "token-abc",
        identityIds: ["aabbcc001122", "ddeeff334455667788990011", "112233445566"],
      });
    });
  });

  it("filters identities to only frozen ones on result", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskResultEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskResultEvent)?.({
        payload: {
          taskId: "frozen-task-1",
          result: {
            type: "tokenFrozenIdentities",
            identityIds: ["aabbcc001122", "112233445566"],
          },
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.frozenIdentities).toEqual([
      { id: "aabbcc001122", label: "Alice" },
      { id: "112233445566", label: "Bob" },
    ]);
  });

  it("uses truncated ID for identities without alias", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskResultEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskResultEvent)?.({
        payload: {
          taskId: "frozen-task-1",
          result: {
            type: "tokenFrozenIdentities",
            identityIds: ["ddeeff334455667788990011"],
          },
        },
      });
    });

    expect(result.current.frozenIdentities).toEqual([
      { id: "ddeeff334455667788990011", label: "ddeeff334455..." },
    ]);
  });

  it("returns empty list when no identities are frozen", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskResultEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskResultEvent)?.({
        payload: {
          taskId: "frozen-task-1",
          result: {
            type: "tokenFrozenIdentities",
            identityIds: [],
          },
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.frozenIdentities).toEqual([]);
  });

  it("handles non-frozen result type gracefully", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskResultEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskResultEvent)?.({
        payload: {
          taskId: "frozen-task-1",
          result: { type: "tokenCompleted" },
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.frozenIdentities).toEqual([]);
  });

  it("sets error on task error event", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskErrorEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskErrorEvent)?.({
        payload: {
          taskId: "frozen-task-1",
          domain: "token",
          message: "Network error",
          details: "",
          recoverable: false,
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBe("Network error");
  });

  it("ignores error events from other domains", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskErrorEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskErrorEvent)?.({
        payload: {
          taskId: "frozen-task-1",
          domain: "identity",
          message: "Some identity error",
          details: "",
          recoverable: false,
        },
      });
    });

    // Should still be loading since we got a different domain
    expect(result.current.loading).toBe(true);
  });

  it("sets error when IPC dispatch fails", async () => {
    (commands.tokenQueryFrozenIdentities as Mock).mockResolvedValue({
      status: "error",
      error: "Token not found",
    });

    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toBe("Token not found");
  });

  it("sets loading false when tokenId is empty", async () => {
    const { result } = renderHook(() => useFrozenIdentities(""));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.frozenIdentities).toEqual([]);
    expect(commands.tokenQueryFrozenIdentities).not.toHaveBeenCalled();
  });

  it("ignores task results for other task IDs", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(getLastListener(events.taskResultEvent)).not.toBeNull();
    });

    await act(async () => {
      getLastListener(events.taskResultEvent)?.({
        payload: {
          taskId: "some-other-task",
          result: {
            type: "tokenFrozenIdentities",
            identityIds: ["aabbcc001122"],
          },
        },
      });
    });

    // Should still be loading since we got a different task ID
    expect(result.current.loading).toBe(true);
  });

  it("handles IPC throw gracefully", async () => {
    (commands.tokenQueryFrozenIdentities as Mock).mockRejectedValue(new Error("Connection refused"));

    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toBe("Failed to query frozen identities");
  });
});
