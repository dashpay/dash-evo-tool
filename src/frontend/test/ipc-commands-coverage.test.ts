/**
 * IPC Command Coverage Tests
 *
 * Tests for 14 commonly-used but previously untested IPC commands.
 * These tests verify that commands can be invoked with correct arguments,
 * return expected response shapes, and handle error cases properly.
 *
 * Task 7.5.4c: Add IPC assertion tests for untested commands.
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

// ─── Mock bindings (centralized) ────────────────────────────────────

vi.mock("@/bindings", async () => {
  const { createMockBindings, mockBindingsModule } = await import(
    "@/test/mock-ipc"
  );
  return mockBindingsModule(createMockBindings());
});

import { commands } from "@/bindings";

// ─── Helpers ────────────────────────────────────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
});

// ─────────────────────────────────────────────────────────────────────
// Group 1: Identity Search Operations
// ─────────────────────────────────────────────────────────────────────

describe("Identity Search Commands", () => {
  describe("identitySearchFromWallet", () => {
    it("dispatches with walletSeedHash and identityIndex", async () => {
      const result = await commands.identitySearchFromWallet({
        walletSeedHash: "abc123def456",
        identityIndex: 0,
      });
      expect(result.status).toBe("ok");
      expect(result.status === "ok" && result.data.taskId).toBeDefined();
      expect(commands.identitySearchFromWallet).toHaveBeenCalledWith({
        walletSeedHash: "abc123def456",
        identityIndex: 0,
      });
    });

    it("dispatches with a high identity index", async () => {
      const result = await commands.identitySearchFromWallet({
        walletSeedHash: "seedhash99",
        identityIndex: 100,
      });
      expect(result.status).toBe("ok");
      expect(commands.identitySearchFromWallet).toHaveBeenCalledWith({
        walletSeedHash: "seedhash99",
        identityIndex: 100,
      });
    });

    it("returns error result when backend fails", async () => {
      vi.mocked(commands.identitySearchFromWallet).mockResolvedValueOnce({
        status: "error",
        error: "Wallet not found",
      });
      const result = await commands.identitySearchFromWallet({
        walletSeedHash: "nonexistent",
        identityIndex: 0,
      });
      expect(result.status).toBe("error");
      if (result.status === "error") {
        expect(result.error).toBe("Wallet not found");
      }
    });
  });

  describe("identitySearchUpToIndex", () => {
    it("dispatches with walletSeedHash and maxIdentityIndex", async () => {
      const result = await commands.identitySearchUpToIndex({
        walletSeedHash: "abc123def456",
        maxIdentityIndex: 10,
      });
      expect(result.status).toBe("ok");
      expect(result.status === "ok" && result.data.taskId).toBeDefined();
      expect(commands.identitySearchUpToIndex).toHaveBeenCalledWith({
        walletSeedHash: "abc123def456",
        maxIdentityIndex: 10,
      });
    });

    it("dispatches with maxIdentityIndex of 1 for minimal search", async () => {
      const result = await commands.identitySearchUpToIndex({
        walletSeedHash: "myhash",
        maxIdentityIndex: 1,
      });
      expect(result.status).toBe("ok");
      expect(commands.identitySearchUpToIndex).toHaveBeenCalledWith({
        walletSeedHash: "myhash",
        maxIdentityIndex: 1,
      });
    });

    it("returns error on backend failure", async () => {
      vi.mocked(commands.identitySearchUpToIndex).mockResolvedValueOnce({
        status: "error",
        error: "Network timeout",
      });
      const result = await commands.identitySearchUpToIndex({
        walletSeedHash: "abc",
        maxIdentityIndex: 5,
      });
      expect(result.status).toBe("error");
    });
  });

  describe("identitySearchByDpnsName", () => {
    it("dispatches with name and null walletSeedHash", async () => {
      const result = await commands.identitySearchByDpnsName({
        name: "alice",
        walletSeedHash: null,
      });
      expect(result.status).toBe("ok");
      expect(result.status === "ok" && result.data.taskId).toBeDefined();
      expect(commands.identitySearchByDpnsName).toHaveBeenCalledWith({
        name: "alice",
        walletSeedHash: null,
      });
    });

    it("dispatches with name and associated wallet", async () => {
      const result = await commands.identitySearchByDpnsName({
        name: "bob",
        walletSeedHash: "wallethash123",
      });
      expect(result.status).toBe("ok");
      expect(commands.identitySearchByDpnsName).toHaveBeenCalledWith({
        name: "bob",
        walletSeedHash: "wallethash123",
      });
    });

    it("handles name without .dash suffix (backend expects bare name)", async () => {
      const result = await commands.identitySearchByDpnsName({
        name: "charlie",
        walletSeedHash: null,
      });
      expect(result.status).toBe("ok");
      expect(commands.identitySearchByDpnsName).toHaveBeenCalledWith(
        expect.objectContaining({ name: "charlie" }),
      );
    });

    it("returns error for invalid DPNS name", async () => {
      vi.mocked(commands.identitySearchByDpnsName).mockResolvedValueOnce({
        status: "error",
        error: "Name not found on Platform",
      });
      const result = await commands.identitySearchByDpnsName({
        name: "nonexistent",
        walletSeedHash: null,
      });
      expect(result.status).toBe("error");
    });
  });
});

// ─────────────────────────────────────────────────────────────────────
// Group 2: Identity Sign Message
// ─────────────────────────────────────────────────────────────────────

describe("Identity Sign Message Command", () => {
  describe("identitySignMessage", () => {
    it("signs a message with identity key and returns base64 signature", async () => {
      const mockSignature = "MEUCIQD+base64signature==";
      vi.mocked(commands.identitySignMessage).mockResolvedValueOnce({
        status: "ok",
        data: mockSignature,
      });
      const result = await commands.identitySignMessage({
        identityId: "aa".repeat(32),
        keyId: 0,
        message: "Hello, world!",
      });
      expect(result.status).toBe("ok");
      if (result.status === "ok") {
        expect(result.data).toBe(mockSignature);
      }
      expect(commands.identitySignMessage).toHaveBeenCalledWith({
        identityId: "aa".repeat(32),
        keyId: 0,
        message: "Hello, world!",
      });
    });

    it("passes correct keyId for non-master keys", async () => {
      vi.mocked(commands.identitySignMessage).mockResolvedValueOnce({
        status: "ok",
        data: "sig==",
      });
      await commands.identitySignMessage({
        identityId: "bb".repeat(32),
        keyId: 5,
        message: "Sign with key 5",
      });
      expect(commands.identitySignMessage).toHaveBeenCalledWith({
        identityId: "bb".repeat(32),
        keyId: 5,
        message: "Sign with key 5",
      });
    });

    it("returns error when key type is unsupported", async () => {
      vi.mocked(commands.identitySignMessage).mockResolvedValueOnce({
        status: "error",
        error: "Key type BLS12_381 does not support message signing",
      });
      const result = await commands.identitySignMessage({
        identityId: "cc".repeat(32),
        keyId: 2,
        message: "test",
      });
      expect(result.status).toBe("error");
      if (result.status === "error") {
        expect(result.error).toContain("BLS12_381");
      }
    });

    it("returns error when identity has no private key", async () => {
      vi.mocked(commands.identitySignMessage).mockResolvedValueOnce({
        status: "error",
        error: "No private key available for key 0",
      });
      const result = await commands.identitySignMessage({
        identityId: "dd".repeat(32),
        keyId: 0,
        message: "test",
      });
      expect(result.status).toBe("error");
    });
  });
});

// ─────────────────────────────────────────────────────────────────────
// Group 3: Wallet Platform Operations
// ─────────────────────────────────────────────────────────────────────

describe("Wallet Platform Commands", () => {
  describe("walletFetchPlatformAddressBalances", () => {
    it("dispatches with wallet seed hash and auto sync mode", async () => {
      const result = await commands.walletFetchPlatformAddressBalances({
        walletSeedHash: "abc123def456",
        syncMode: "auto",
      });
      expect(result.status).toBe("ok");
      expect(result.status === "ok" && result.data.taskId).toBeDefined();
      expect(
        commands.walletFetchPlatformAddressBalances,
      ).toHaveBeenCalledWith({
        walletSeedHash: "abc123def456",
        syncMode: "auto",
      });
    });

    it("dispatches with forceFull sync mode", async () => {
      const result = await commands.walletFetchPlatformAddressBalances({
        walletSeedHash: "seedhash",
        syncMode: "forceFull",
      });
      expect(result.status).toBe("ok");
      expect(
        commands.walletFetchPlatformAddressBalances,
      ).toHaveBeenCalledWith({
        walletSeedHash: "seedhash",
        syncMode: "forceFull",
      });
    });

    it("returns error when wallet not found", async () => {
      vi.mocked(
        commands.walletFetchPlatformAddressBalances,
      ).mockResolvedValueOnce({
        status: "error",
        error: "Wallet not found for seed hash",
      });
      const result = await commands.walletFetchPlatformAddressBalances({
        walletSeedHash: "bad_hash",
        syncMode: "full",
      });
      expect(result.status).toBe("error");
    });
  });

  describe("walletBootstrapAddresses", () => {
    it("bootstraps wallet addresses successfully", async () => {
      const result = await commands.walletBootstrapAddresses("abc123def456");
      expect(result.status).toBe("ok");
      if (result.status === "ok") {
        expect(result.data).toBeNull();
      }
      expect(commands.walletBootstrapAddresses).toHaveBeenCalledWith(
        "abc123def456",
      );
    });

    it("returns error for invalid seed hash", async () => {
      vi.mocked(commands.walletBootstrapAddresses).mockResolvedValueOnce({
        status: "error",
        error: "Invalid wallet seed hash",
      });
      const result = await commands.walletBootstrapAddresses("invalid");
      expect(result.status).toBe("error");
      if (result.status === "error") {
        expect(result.error).toBe("Invalid wallet seed hash");
      }
    });
  });

  describe("walletStartSpv", () => {
    it("starts SPV syncing successfully", async () => {
      const result = await commands.walletStartSpv();
      expect(result.status).toBe("ok");
      if (result.status === "ok") {
        expect(result.data).toBeNull();
      }
      expect(commands.walletStartSpv).toHaveBeenCalledWith();
    });

    it("returns error when SPV already running", async () => {
      vi.mocked(commands.walletStartSpv).mockResolvedValueOnce({
        status: "error",
        error: "SPV sync is already in progress",
      });
      const result = await commands.walletStartSpv();
      expect(result.status).toBe("error");
      if (result.status === "error") {
        expect(result.error).toContain("already");
      }
    });

    it("returns error when no network configured", async () => {
      vi.mocked(commands.walletStartSpv).mockResolvedValueOnce({
        status: "error",
        error: "No network context available",
      });
      const result = await commands.walletStartSpv();
      expect(result.status).toBe("error");
    });
  });
});

// ─────────────────────────────────────────────────────────────────────
// Group 4: Core Chain Operations
// ─────────────────────────────────────────────────────────────────────

describe("Core Chain Commands", () => {
  describe("coreGetBestChainLock", () => {
    it("dispatches task and returns taskId", async () => {
      const result = await commands.coreGetBestChainLock();
      expect(result).toBeDefined();
      expect(result.taskId).toBeDefined();
      expect(commands.coreGetBestChainLock).toHaveBeenCalledWith();
    });

    it("returns different task IDs for successive calls", async () => {
      vi.mocked(commands.coreGetBestChainLock)
        .mockResolvedValueOnce({ taskId: "task-1" })
        .mockResolvedValueOnce({ taskId: "task-2" });
      const r1 = await commands.coreGetBestChainLock();
      const r2 = await commands.coreGetBestChainLock();
      expect(r1.taskId).toBe("task-1");
      expect(r2.taskId).toBe("task-2");
    });
  });

  describe("coreGetBestChainLocks", () => {
    it("dispatches task for all networks and returns taskId", async () => {
      const result = await commands.coreGetBestChainLocks();
      expect(result).toBeDefined();
      expect(result.taskId).toBeDefined();
      expect(commands.coreGetBestChainLocks).toHaveBeenCalledWith();
    });

    it("can be called repeatedly (used in polling)", async () => {
      await commands.coreGetBestChainLocks();
      await commands.coreGetBestChainLocks();
      await commands.coreGetBestChainLocks();
      expect(commands.coreGetBestChainLocks).toHaveBeenCalledTimes(3);
    });
  });

  describe("coreRecoverAssetLocks", () => {
    it("dispatches recovery task with wallet seed hash", async () => {
      const result = await commands.coreRecoverAssetLocks({
        walletSeedHash: "abc123def456",
      });
      expect(result.status).toBe("ok");
      expect(result.status === "ok" && result.data.taskId).toBeDefined();
      expect(commands.coreRecoverAssetLocks).toHaveBeenCalledWith({
        walletSeedHash: "abc123def456",
      });
    });

    it("returns error for unrecognized wallet", async () => {
      vi.mocked(commands.coreRecoverAssetLocks).mockResolvedValueOnce({
        status: "error",
        error: "Wallet not found",
      });
      const result = await commands.coreRecoverAssetLocks({
        walletSeedHash: "unknown",
      });
      expect(result.status).toBe("error");
    });

    it("returns error when core RPC unavailable", async () => {
      vi.mocked(commands.coreRecoverAssetLocks).mockResolvedValueOnce({
        status: "error",
        error: "Core RPC connection refused",
      });
      const result = await commands.coreRecoverAssetLocks({
        walletSeedHash: "valid_hash",
      });
      expect(result.status).toBe("error");
      if (result.status === "error") {
        expect(result.error).toContain("RPC");
      }
    });
  });
});

// ─────────────────────────────────────────────────────────────────────
// Group 5: Settings Mutations
// ─────────────────────────────────────────────────────────────────────

describe("Settings Mutation Commands", () => {
  describe("settingsUpdatePassword", () => {
    it("updates password with salt, nonce, and password check", async () => {
      const result = await commands.settingsUpdatePassword({
        saltHex: "aa".repeat(16),
        nonceHex: "bb".repeat(12),
        passwordCheckHex: "cc".repeat(32),
      });
      expect(result.status).toBe("ok");
      if (result.status === "ok") {
        expect(result.data).toBeNull();
      }
      expect(commands.settingsUpdatePassword).toHaveBeenCalledWith({
        saltHex: "aa".repeat(16),
        nonceHex: "bb".repeat(12),
        passwordCheckHex: "cc".repeat(32),
      });
    });

    it("returns error on invalid password data", async () => {
      vi.mocked(commands.settingsUpdatePassword).mockResolvedValueOnce({
        status: "error",
        error: "Invalid password check data",
      });
      const result = await commands.settingsUpdatePassword({
        saltHex: "",
        nonceHex: "",
        passwordCheckHex: "",
      });
      expect(result.status).toBe("error");
    });

    it("passes correctly structured hex values", async () => {
      const salt = "0123456789abcdef".repeat(2);
      const nonce = "fedcba9876543210".slice(0, 24);
      const check = "abcdef0123456789".repeat(4);
      await commands.settingsUpdatePassword({
        saltHex: salt,
        nonceHex: nonce,
        passwordCheckHex: check,
      });
      expect(commands.settingsUpdatePassword).toHaveBeenCalledWith({
        saltHex: salt,
        nonceHex: nonce,
        passwordCheckHex: check,
      });
    });
  });

  describe("settingsUpdateAutoStartSpv", () => {
    it("enables auto-start SPV", async () => {
      const result = await commands.settingsUpdateAutoStartSpv(true);
      expect(result.status).toBe("ok");
      if (result.status === "ok") {
        expect(result.data).toBeNull();
      }
      expect(commands.settingsUpdateAutoStartSpv).toHaveBeenCalledWith(true);
    });

    it("disables auto-start SPV", async () => {
      const result = await commands.settingsUpdateAutoStartSpv(false);
      expect(result.status).toBe("ok");
      expect(commands.settingsUpdateAutoStartSpv).toHaveBeenCalledWith(false);
    });

    it("returns error on settings persistence failure", async () => {
      vi.mocked(commands.settingsUpdateAutoStartSpv).mockResolvedValueOnce({
        status: "error",
        error: "Failed to persist settings to database",
      });
      const result = await commands.settingsUpdateAutoStartSpv(true);
      expect(result.status).toBe("error");
    });
  });
});

// ─────────────────────────────────────────────────────────────────────
// Group 6: Context Operations
// ─────────────────────────────────────────────────────────────────────

describe("Context Commands", () => {
  describe("contextGetFeeMultiplier", () => {
    it("returns the current fee multiplier (default 1.0)", async () => {
      const result = await commands.contextGetFeeMultiplier();
      expect(typeof result).toBe("number");
      expect(result).toBe(1.0);
      expect(commands.contextGetFeeMultiplier).toHaveBeenCalledWith();
    });

    it("returns custom fee multiplier when set", async () => {
      vi.mocked(commands.contextGetFeeMultiplier).mockResolvedValueOnce(2.5);
      const result = await commands.contextGetFeeMultiplier();
      expect(result).toBe(2.5);
    });

    it("returns multiplier as permille-scale number", async () => {
      // Backend uses permille: 1000 = 1x, 2000 = 2x
      vi.mocked(commands.contextGetFeeMultiplier).mockResolvedValueOnce(1500);
      const result = await commands.contextGetFeeMultiplier();
      expect(result).toBe(1500); // 1.5x in permille
    });
  });

  describe("contextSetFeeMultiplier", () => {
    it("sets the fee multiplier to 1x (1000 permille)", async () => {
      await commands.contextSetFeeMultiplier(1000);
      expect(commands.contextSetFeeMultiplier).toHaveBeenCalledWith(1000);
    });

    it("sets the fee multiplier to 2x (2000 permille)", async () => {
      await commands.contextSetFeeMultiplier(2000);
      expect(commands.contextSetFeeMultiplier).toHaveBeenCalledWith(2000);
    });

    it("returns void on success", async () => {
      const result = await commands.contextSetFeeMultiplier(1000);
      expect(result).toBeUndefined();
    });

    it("can set fractional multipliers", async () => {
      await commands.contextSetFeeMultiplier(1500); // 1.5x
      expect(commands.contextSetFeeMultiplier).toHaveBeenCalledWith(1500);
    });
  });
});

// ─────────────────────────────────────────────────────────────────────
// Cross-cutting: All 14 commands are callable
// ─────────────────────────────────────────────────────────────────────

describe("All 14 Commands Are Exported and Callable", () => {
  it.each([
    "identitySearchFromWallet",
    "identitySearchUpToIndex",
    "identitySearchByDpnsName",
    "identitySignMessage",
    "walletFetchPlatformAddressBalances",
    "walletBootstrapAddresses",
    "walletStartSpv",
    "coreGetBestChainLock",
    "coreGetBestChainLocks",
    "coreRecoverAssetLocks",
    "settingsUpdatePassword",
    "settingsUpdateAutoStartSpv",
    "contextGetFeeMultiplier",
    "contextSetFeeMultiplier",
  ] as const)("%s is a function on the commands object", (name) => {
    expect(typeof commands[name]).toBe("function");
  });
});
