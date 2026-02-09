import { renderHook, act, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { useUtxoMonitor } from "./useUtxoMonitor";

// ─── Mock bindings ──────────────────────────────────────────────────

const mockCoreRefreshWalletInfo = vi.fn();
const mockWalletGetHd = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    coreRefreshWalletInfo: (...args: unknown[]) =>
      mockCoreRefreshWalletInfo(...args),
    walletGetHd: (...args: unknown[]) => mockWalletGetHd(...args),
  },
}));

// ─── Helpers ────────────────────────────────────────────────────────

function makeWalletResult(addresses: { address: string; balance: number }[]) {
  return {
    status: "ok" as const,
    data: {
      seedHash: "abc123",
      addresses: addresses.map((a) => ({
        ...a,
        totalReceived: a.balance,
        derivationPath: "m/44'/5'/0'/0/0",
      })),
    },
  };
}

// ─── Tests ──────────────────────────────────────────────────────────

describe("useUtxoMonitor", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockCoreRefreshWalletInfo.mockResolvedValue({ status: "ok" });
    mockWalletGetHd.mockResolvedValue(
      makeWalletResult([{ address: "yTestAddr", balance: 0 }]),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("returns initial state when disabled", () => {
    const { result } = renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: "yTestAddr",
        enabled: false,
      }),
    );
    expect(result.current.fundsReceived).toBe(false);
    expect(result.current.balance).toBe(0);
    expect(result.current.polling).toBe(false);
  });

  it("does not poll when disabled", async () => {
    renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: "yTestAddr",
        enabled: false,
      }),
    );
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(mockCoreRefreshWalletInfo).not.toHaveBeenCalled();
    expect(mockWalletGetHd).not.toHaveBeenCalled();
  });

  it("does not poll when seedHash is null", async () => {
    renderHook(() =>
      useUtxoMonitor({
        seedHash: null,
        address: "yTestAddr",
        enabled: true,
      }),
    );
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(mockCoreRefreshWalletInfo).not.toHaveBeenCalled();
  });

  it("does not poll when address is null", async () => {
    renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: null,
        enabled: true,
      }),
    );
    await act(async () => {
      vi.advanceTimersByTime(10000);
    });
    expect(mockCoreRefreshWalletInfo).not.toHaveBeenCalled();
  });

  it("polls when enabled with valid params", async () => {
    renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: "yTestAddr",
        enabled: true,
        pollIntervalMs: 3000,
      }),
    );

    // Initial check fires immediately
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockCoreRefreshWalletInfo).toHaveBeenCalledWith({
      walletSeedHash: "abc123",
      platformSyncMode: null,
    });
    expect(mockWalletGetHd).toHaveBeenCalledWith("abc123");
  });

  it("detects funds when address balance becomes non-zero", async () => {
    vi.useRealTimers();

    // First call: no funds, second call: funds received
    let callCount = 0;
    mockWalletGetHd.mockImplementation(() => {
      callCount++;
      if (callCount <= 1) {
        return Promise.resolve(
          makeWalletResult([{ address: "yTestAddr", balance: 0 }]),
        );
      }
      return Promise.resolve(
        makeWalletResult([{ address: "yTestAddr", balance: 100_000_000 }]),
      );
    });

    const { result } = renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: "yTestAddr",
        enabled: true,
        pollIntervalMs: 100, // Short interval for real timer test
      }),
    );

    // Wait for funds to be detected
    await waitFor(
      () => {
        expect(result.current.fundsReceived).toBe(true);
        expect(result.current.balance).toBe(100_000_000);
      },
      { timeout: 2000 },
    );
  });

  it("stops polling once funds are detected", async () => {
    mockWalletGetHd.mockResolvedValue(
      makeWalletResult([{ address: "yTestAddr", balance: 50_000_000 }]),
    );

    renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: "yTestAddr",
        enabled: true,
        pollIntervalMs: 3000,
      }),
    );

    // Initial check — funds found immediately
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const callCountAfterDetection = mockCoreRefreshWalletInfo.mock.calls.length;

    // Advance more time — should not poll again
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockCoreRefreshWalletInfo.mock.calls.length).toBe(
      callCountAfterDetection,
    );
  });

  it("resets state when address changes", async () => {
    mockWalletGetHd.mockResolvedValue(
      makeWalletResult([
        { address: "yAddr1", balance: 50_000_000 },
        { address: "yAddr2", balance: 0 },
      ]),
    );

    const { result, rerender } = renderHook(
      (props: { address: string }) =>
        useUtxoMonitor({
          seedHash: "abc123",
          address: props.address,
          enabled: true,
          pollIntervalMs: 3000,
        }),
      { initialProps: { address: "yAddr1" } },
    );

    // Detect funds at addr1
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.fundsReceived).toBe(true);

    // Switch to addr2 — should reset
    rerender({ address: "yAddr2" });
    expect(result.current.fundsReceived).toBe(false);
    expect(result.current.balance).toBe(0);
  });

  it("handles API errors gracefully", async () => {
    mockCoreRefreshWalletInfo.mockRejectedValue(new Error("Network error"));

    const { result } = renderHook(() =>
      useUtxoMonitor({
        seedHash: "abc123",
        address: "yTestAddr",
        enabled: true,
        pollIntervalMs: 3000,
      }),
    );

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Should not crash, stays at initial state
    expect(result.current.fundsReceived).toBe(false);
    expect(result.current.balance).toBe(0);
  });
});
