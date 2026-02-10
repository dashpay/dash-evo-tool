/**
 * Test fixture factories for wallet-related DTOs.
 *
 * Usage:
 *   import { createMockHdWallet, createMockUtxo } from "@/test/fixtures";
 *
 *   const wallet = createMockHdWallet({ alias: "My Wallet" });
 *   const utxo = createMockUtxo({ amount: 500_000_000 });
 */

import type {
  WalletDto,
  SingleKeyWalletDto,
  WalletAddressDto,
  WalletTransactionDto,
  PlatformAddressDto,
  UtxoDto,
  AssetLockDto,
  AssetLockProofDetailsDto,
  WalletRefDto,
  WalletListDto,
} from "@/bindings";

// ─── Atomic factories ──────────────────────────────────────────────

export function createMockUtxo(overrides?: Partial<UtxoDto>): UtxoDto {
  return {
    txid: "a1b2c3d4e5f60718293a4b5c6d7e8f9001122334455667788990aabbccddeeff",
    vout: 0,
    amount: 100_000_000, // 1 DASH
    ...overrides,
  };
}

export function createMockWalletAddress(
  overrides?: Partial<WalletAddressDto>,
): WalletAddressDto {
  return {
    address: "yWdXnYxGbouNoo8yMvcbZmWsHth3yMERhc",
    balance: 500_000_000, // 5 DASH
    totalReceived: 1_000_000_000,
    derivationPath: "m/44'/5'/0'/0/0",
    ...overrides,
  };
}

export function createMockWalletTransaction(
  overrides?: Partial<WalletTransactionDto>,
): WalletTransactionDto {
  return {
    txid: "f0e1d2c3b4a5968778695a4b3c2d1e0ffaebdccd1a2b3c4d5e6f708192a3b4c5",
    timestamp: 1707500000,
    height: 1_920_000,
    blockHash:
      "00000000000000112233445566778899aabbccddeeff00112233445566778899",
    netAmount: -50_000_000,
    fee: 226,
    label: null,
    isOurs: true,
    ...overrides,
  };
}

export function createMockPlatformAddress(
  overrides?: Partial<PlatformAddressDto>,
): PlatformAddressDto {
  return {
    address: "yXa1b2c3d4e5f6g7h8i9j0kLmNoPqRsTuV",
    balance: 200_000_000, // 2 DASH
    nonce: 0,
    ...overrides,
  };
}

export function createMockAssetLockProofDetails(
  overrides?: Partial<AssetLockProofDetailsDto>,
): AssetLockProofDetailsDto {
  return {
    type: "instantSend",
    instantLockTxid:
      "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    outputIndex: 0,
    ...overrides,
  } as AssetLockProofDetailsDto;
}

export function createMockAssetLock(
  overrides?: Partial<AssetLockDto>,
): AssetLockDto {
  return {
    txid: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    address: "yWdXnYxGbouNoo8yMvcbZmWsHth3yMERhc",
    amount: 100_000_000, // 1 DASH
    hasInstantLock: true,
    hasAssetLockProof: true,
    proofDetails: createMockAssetLockProofDetails(),
    proofHex: "0a0b0c0d0e0f",
    ...overrides,
  };
}

// ─── Wallet factories ──────────────────────────────────────────────

export function createMockHdWallet(
  overrides?: Partial<WalletDto>,
): WalletDto {
  return {
    seedHash: "abc123def456",
    usesPassword: false,
    alias: "Test HD Wallet",
    isMain: true,
    confirmedBalance: 500_000_000, // 5 DASH
    unconfirmedBalance: 0,
    totalBalance: 500_000_000,
    addresses: [createMockWalletAddress()],
    transactions: [],
    unusedAssetLocks: [],
    platformAddresses: [],
    identityIndexes: [0],
    passwordHint: null,
    ...overrides,
  };
}

export function createMockSingleKeyWallet(
  overrides?: Partial<SingleKeyWalletDto>,
): SingleKeyWalletDto {
  return {
    keyHash: "singlekey789abc",
    usesPassword: false,
    publicKey:
      "02a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1",
    address: "yRt5BxWn3oC9xLmvJb4r7pAjd5nYqh2Kzf",
    alias: "Test Single Key Wallet",
    confirmedBalance: 250_000_000, // 2.5 DASH
    unconfirmedBalance: 0,
    totalBalance: 250_000_000,
    utxoCount: 3,
    utxos: [
      createMockUtxo({ amount: 100_000_000 }),
      createMockUtxo({
        txid: "b2c3d4e5f60718293a4b5c6d7e8f9001122334455667788990aabbccddeeff00",
        vout: 1,
        amount: 100_000_000,
      }),
      createMockUtxo({
        txid: "c3d4e5f60718293a4b5c6d7e8f9001122334455667788990aabbccddeeff0011",
        vout: 0,
        amount: 50_000_000,
      }),
    ],
    ...overrides,
  };
}

export function createMockWalletRef(
  overrides?: Partial<{ type: string; seedHash: string; keyHash: string }>,
): WalletRefDto {
  if (overrides?.type === "singleKey") {
    return {
      type: "singleKey",
      keyHash: overrides.keyHash ?? "singlekey789abc",
    };
  }
  return { type: "hd", seedHash: overrides?.seedHash ?? "abc123def456" };
}

export function createMockWalletList(
  overrides?: Partial<WalletListDto>,
): WalletListDto {
  return {
    hdWallets: [createMockHdWallet()],
    singleKeyWallets: [],
    selected: { type: "hd", seedHash: "abc123def456" },
    ...overrides,
  };
}
