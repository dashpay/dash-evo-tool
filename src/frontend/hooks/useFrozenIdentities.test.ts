import { renderHook, act, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useFrozenIdentities } from "./useFrozenIdentities";

// ─── Mocks ──────────────────────────────────────────────────────────────────

const mockTokenQueryFrozenIdentities = vi.fn();
let mockTaskResultListener: ((event: { payload: unknown }) => void) | null = null;
let mockTaskErrorListener: ((event: { payload: unknown }) => void) | null = null;

vi.mock("@/bindings", () => ({
  commands: {
    tokenQueryFrozenIdentities: (...args: unknown[]) =>
      mockTokenQueryFrozenIdentities(...args),
  },
  events: {
    taskResultEvent: {
      listen: vi.fn().mockImplementation((cb) => {
        mockTaskResultListener = cb;
        return Promise.resolve(() => {
          mockTaskResultListener = null;
        });
      }),
    },
    taskErrorEvent: {
      listen: vi.fn().mockImplementation((cb) => {
        mockTaskErrorListener = cb;
        return Promise.resolve(() => {
          mockTaskErrorListener = null;
        });
      }),
    },
  },
}));

const mockIdentities = [
  {
    id: "aabbcc001122",
    alias: "Alice",
    balance: 1000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "Active",
    identityType: "User",
  },
  {
    id: "ddeeff334455667788990011",
    alias: null,
    balance: 2000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "Active",
    identityType: "User",
  },
  {
    id: "112233445566",
    alias: "Bob",
    balance: 3000,
    keys: [],
    dpnsNames: [],
    associatedWalletHashes: [],
    walletIndex: 0,
    topUps: [],
    status: "Active",
    identityType: "User",
  },
];

vi.mock("@/stores/identityStore", () => ({
  useIdentityStore: () => ({
    identities: mockIdentities,
  }),
}));

// ─── Tests ──────────────────────────────────────────────────────────────────

describe("useFrozenIdentities", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockTaskResultListener = null;
    mockTaskErrorListener = null;
    mockTokenQueryFrozenIdentities.mockResolvedValue({
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
      expect(mockTokenQueryFrozenIdentities).toHaveBeenCalledWith({
        tokenId: "token-abc",
        identityIds: ["aabbcc001122", "ddeeff334455667788990011", "112233445566"],
      });
    });
  });

  it("filters identities to only frozen ones on result", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(mockTaskResultListener).not.toBeNull();
    });

    await act(async () => {
      mockTaskResultListener?.({
        payload: {
          taskId: "frozen-task-1",
          resultType: "Token",
          payload: ["aabbcc001122", "112233445566"],
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
      expect(mockTaskResultListener).not.toBeNull();
    });

    await act(async () => {
      mockTaskResultListener?.({
        payload: {
          taskId: "frozen-task-1",
          resultType: "Token",
          payload: ["ddeeff334455667788990011"],
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
      expect(mockTaskResultListener).not.toBeNull();
    });

    await act(async () => {
      mockTaskResultListener?.({
        payload: {
          taskId: "frozen-task-1",
          resultType: "Token",
          payload: [],
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.frozenIdentities).toEqual([]);
  });

  it("handles null payload gracefully", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(mockTaskResultListener).not.toBeNull();
    });

    await act(async () => {
      mockTaskResultListener?.({
        payload: {
          taskId: "frozen-task-1",
          resultType: "Token",
          payload: null,
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.frozenIdentities).toEqual([]);
  });

  it("sets error on task error event", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(mockTaskErrorListener).not.toBeNull();
    });

    await act(async () => {
      mockTaskErrorListener?.({
        payload: {
          taskId: "frozen-task-1",
          message: "Network error",
        },
      });
    });

    expect(result.current.loading).toBe(false);
    expect(result.current.error).toBe("Network error");
  });

  it("sets error when IPC dispatch fails", async () => {
    mockTokenQueryFrozenIdentities.mockResolvedValue({
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
    expect(mockTokenQueryFrozenIdentities).not.toHaveBeenCalled();
  });

  it("ignores task results for other task IDs", async () => {
    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(mockTaskResultListener).not.toBeNull();
    });

    await act(async () => {
      mockTaskResultListener?.({
        payload: {
          taskId: "some-other-task",
          resultType: "Token",
          payload: ["aabbcc001122"],
        },
      });
    });

    // Should still be loading since we got a different task ID
    expect(result.current.loading).toBe(true);
  });

  it("handles IPC throw gracefully", async () => {
    mockTokenQueryFrozenIdentities.mockRejectedValue(new Error("Connection refused"));

    const { result } = renderHook(() => useFrozenIdentities("token-abc"));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toBe("Failed to query frozen identities");
  });
});
