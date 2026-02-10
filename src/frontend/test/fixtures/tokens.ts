/**
 * Test fixture factories for token-related DTOs and store types.
 *
 * Usage:
 *   import { createMockToken, createMockTokenSearchResult } from "@/test/fixtures";
 *
 *   const token = createMockToken({ name: "DashToken" });
 */

import type {
  TokenOperationInput,
  MintingConfigDto,
  IdentityTokenIdentifierDto,
} from "@/bindings";
import type {
  TokenEntry,
  TokenSearchResult,
} from "@/stores/tokenStore";

// ─── Token entry factories ─────────────────────────────────────────

export function createMockToken(
  overrides?: Partial<TokenEntry>,
): TokenEntry {
  return {
    identityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    tokenId:
      "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233",
    contractId:
      "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    tokenPosition: 0,
    name: "Test Token",
    ownerAlias: "Alice",
    balance: "1000000000000", // 10,000 tokens with 8 decimals
    decimals: 8,
    ...overrides,
  };
}

export function createMockTokenBalance(
  overrides?: Partial<TokenEntry>,
): TokenEntry {
  return createMockToken({
    balance: "500000000", // 5 tokens with 8 decimals
    ...overrides,
  });
}

// ─── Search result factories ───────────────────────────────────────

export function createMockTokenSearchResult(
  overrides?: Partial<TokenSearchResult>,
): TokenSearchResult {
  return {
    contractId:
      "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    description: "A test token for decentralized payments",
    ...overrides,
  };
}

// ─── Operation input factories ─────────────────────────────────────

export function createMockTokenOperationInput(
  overrides?: Partial<TokenOperationInput>,
): TokenOperationInput {
  return {
    identityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    contractId:
      "1122334455667788990011223344556677889900aabbccddeeff001122334455",
    tokenPosition: 0,
    keyId: 1,
    publicNote: null,
    ...overrides,
  };
}

export function createMockMintingConfig(
  overrides?: Partial<MintingConfigDto>,
): MintingConfigDto {
  return {
    allowChoosingDestination: true,
    defaultDestinationIdentityId: null,
    ...overrides,
  };
}

export function createMockIdentityTokenIdentifier(
  overrides?: Partial<IdentityTokenIdentifierDto>,
): IdentityTokenIdentifierDto {
  return {
    identityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    tokenId:
      "aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff00112233",
    ...overrides,
  };
}
