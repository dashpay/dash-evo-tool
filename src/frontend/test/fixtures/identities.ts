/**
 * Test fixture factories for identity-related DTOs.
 *
 * Usage:
 *   import { createMockIdentity, createMockIdentityKey } from "@/test/fixtures";
 *
 *   const identity = createMockIdentity({ alias: "Alice" });
 *   const key = createMockIdentityKey({ purpose: "VOTING" });
 */

import type {
  QualifiedIdentityDto,
  IdentityKeyDto,
  IdentityTypeDto,
  IdentityStatusDto,
  DpnsNameInfoDto,
  IdentitySummaryDto,
  TopUpEntryDto,
  KeySpecDto,
  ContractBoundsDto,
} from "@/bindings";

// ─── Atomic factories ──────────────────────────────────────────────

export function createMockIdentityKey(
  overrides?: Partial<IdentityKeyDto>,
): IdentityKeyDto {
  return {
    keyId: 0,
    keyType: "ECDSA_SECP256K1",
    purpose: "AUTHENTICATION",
    securityLevel: "MASTER",
    data: "02b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4",
    isDisabled: false,
    disabledAt: null,
    hasPrivateKey: true,
    contractBounds: null,
    ...overrides,
  };
}

export function createMockDpnsNameInfo(
  overrides?: Partial<DpnsNameInfoDto>,
): DpnsNameInfoDto {
  return {
    name: "alice.dash",
    acquiredAt: 1707500000,
    ...overrides,
  };
}

export function createMockTopUpEntry(
  overrides?: Partial<TopUpEntryDto>,
): TopUpEntryDto {
  return {
    index: 0,
    amount: 100_000_000,
    ...overrides,
  };
}

export function createMockKeySpec(
  overrides?: Partial<KeySpecDto>,
): KeySpecDto {
  return {
    keyType: "ECDSA_SECP256K1",
    purpose: "AUTHENTICATION",
    securityLevel: "HIGH",
    contractBounds: null,
    ...overrides,
  };
}

export function createMockContractBounds(
  overrides?: Partial<{
    type: string;
    contractId: string;
    documentTypeName: string;
  }>,
): ContractBoundsDto {
  if (overrides?.type === "singleContractDocumentType") {
    return {
      type: "singleContractDocumentType",
      contractId:
        overrides.contractId ??
        "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
      documentTypeName: overrides.documentTypeName ?? "domain",
    };
  }
  return {
    type: "singleContract",
    contractId:
      overrides?.contractId ??
      "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec",
  } as ContractBoundsDto;
}

// ─── Identity factories ────────────────────────────────────────────

export function createMockIdentity(
  overrides?: Partial<QualifiedIdentityDto>,
): QualifiedIdentityDto {
  return {
    id: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    identityType: "user" as IdentityTypeDto,
    alias: "Alice",
    balance: 1_000_000_000, // 0.01 DASH in credits
    keys: [
      createMockIdentityKey({ keyId: 0, securityLevel: "MASTER" }),
      createMockIdentityKey({
        keyId: 1,
        purpose: "AUTHENTICATION",
        securityLevel: "HIGH",
      }),
    ],
    dpnsNames: [createMockDpnsNameInfo()],
    associatedWalletHashes: ["abc123def456"],
    walletIndex: 0,
    topUps: [],
    status: "active" as IdentityStatusDto,
    network: "testnet",
    voterIdentityId: null,
    operatorIdentityId: null,
    masternodePayoutAddress: null,
    ...overrides,
  };
}

export function createMockMasternodeIdentity(
  overrides?: Partial<QualifiedIdentityDto>,
): QualifiedIdentityDto {
  return createMockIdentity({
    id: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
    identityType: "masternode",
    alias: null,
    balance: 500_000_000,
    keys: [
      createMockIdentityKey({ keyId: 0, securityLevel: "MASTER" }),
      createMockIdentityKey({
        keyId: 1,
        purpose: "VOTING",
        securityLevel: "HIGH",
      }),
    ],
    dpnsNames: [],
    walletIndex: null,
    voterIdentityId: "8CgY3Lrw9oqSGeTf063nZtuXG42rAROir7Led6Abde23",
    ...overrides,
  });
}

export function createMockIdentitySummary(
  overrides?: Partial<IdentitySummaryDto>,
): IdentitySummaryDto {
  return {
    id: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    displayName: "Alice",
    identityType: "user" as IdentityTypeDto,
    balance: 1_000_000_000,
    ...overrides,
  };
}
