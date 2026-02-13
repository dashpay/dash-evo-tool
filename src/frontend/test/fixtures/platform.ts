/**
 * Test fixture factories for platform, settings, SPV, and event DTOs.
 *
 * Usage:
 *   import { createMockNetworkInfo, createMockSettings } from "@/test/fixtures";
 *
 *   const info = createMockNetworkInfo({ activeNetwork: "testnet" });
 *   const settings = createMockSettings({ themeMode: "dark" });
 */

import type {
  NetworkInfo,
  NetworkDto,
  SettingsDto,
  ThemeModeDto,
  UserModeDto,
  CoreBackendModeDto,
  SpvStatusDto,
  SpvStatusEvent,
  WalletUpdatedEvent,
  ZmqChainLockedBlockEvent,
  ZmqConnectionStatusEvent,
  ZmqIsLockedTransactionEvent,
  ScheduledVoteExecutedEvent,
  TaskResultEvent,
  TaskErrorEvent,
  DispatchTaskResponse,
  GreetResponse,
  DiffChainEntry,
  PlatformAddressAmountDto,
  StoredProfileDto,
  StoredContactDto,
  StoredContactRequestDto,
  StoredPaymentDto,
  ContactPrivateInfoDto,
} from "@/bindings";

// ─── Network & settings factories ──────────────────────────────────

export function createMockNetworkInfo(
  overrides?: Partial<NetworkInfo>,
): NetworkInfo {
  return {
    activeNetwork: "testnet" as NetworkDto,
    availableNetworks: [
      "dash",
      "testnet",
      "devnet",
      "regtest",
    ] as NetworkDto[],
    ...overrides,
  };
}

export function createMockSettings(
  overrides?: Partial<SettingsDto>,
): SettingsDto {
  return {
    network: "testnet" as NetworkDto,
    themeMode: "system" as ThemeModeDto,
    overwriteDashConf: false,
    disableZmq: false,
    onboardingCompleted: true,
    showEvonodeTools: false,
    userMode: "advanced" as UserModeDto,
    closeDashQtOnExit: false,
    coreBackendMode: "spv" as CoreBackendModeDto,
    hasPassword: false,
    dashQtPath: null,
    ...overrides,
  };
}

// ─── SPV status factories ──────────────────────────────────────────

export function createMockSpvStatus(
  overrides?: Partial<SpvStatusEvent>,
): SpvStatusEvent {
  return {
    network: "testnet" as NetworkDto,
    status: "running" as SpvStatusDto,
    syncProgressPct: 100,
    headerHeight: 1_920_000,
    connectedPeers: 8,
    error: null,
    ...overrides,
  };
}

// ─── Event factories ───────────────────────────────────────────────

export function createMockWalletUpdatedEvent(
  overrides?: Partial<WalletUpdatedEvent>,
): WalletUpdatedEvent {
  return {
    walletSeedHash: "abc123def456",
    network: "testnet" as NetworkDto,
    ...overrides,
  };
}

export function createMockZmqChainLockedBlock(
  overrides?: Partial<ZmqChainLockedBlockEvent>,
): ZmqChainLockedBlockEvent {
  return {
    network: "testnet" as NetworkDto,
    blockHeight: 1_920_001,
    blockHash:
      "00000000000000112233445566778899aabbccddeeff00112233445566778899",
    txCount: 5,
    ...overrides,
  };
}

export function createMockZmqConnectionStatus(
  overrides?: Partial<ZmqConnectionStatusEvent>,
): ZmqConnectionStatusEvent {
  return {
    network: "testnet" as NetworkDto,
    connected: true,
    ...overrides,
  };
}

export function createMockZmqIsLockedTransaction(
  overrides?: Partial<ZmqIsLockedTransactionEvent>,
): ZmqIsLockedTransactionEvent {
  return {
    network: "testnet" as NetworkDto,
    txid: "a1b2c3d4e5f60718293a4b5c6d7e8f9001122334455667788990aabbccddeeff",
    rawTx: "01000000010000000000000000000000000000000000000000000000000000000000000000",
    affectedUtxoCount: 1,
    ...overrides,
  };
}

export function createMockScheduledVoteExecutedEvent(
  overrides?: Partial<ScheduledVoteExecutedEvent>,
): ScheduledVoteExecutedEvent {
  return {
    contestedName: "alice",
    voterId: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
    success: true,
    error: null,
    ...overrides,
  };
}

// ─── Task system factories ─────────────────────────────────────────

export function createMockTaskResult(
  overrides?: Partial<TaskResultEvent>,
): TaskResultEvent {
  return {
    taskId: "task-001",
    result: { type: "identityCompleted", identityId: null },
    ...overrides,
  };
}

export function createMockTaskError(
  overrides?: Partial<TaskErrorEvent>,
): TaskErrorEvent {
  return {
    taskId: "task-001",
    domain: "identity",
    message: "Operation failed",
    details: "Connection timed out after 30 seconds",
    recoverable: true,
    ...overrides,
  };
}

export function createMockDispatchResponse(
  overrides?: Partial<DispatchTaskResponse>,
): DispatchTaskResponse {
  return {
    taskId: "task-001",
    ...overrides,
  };
}

// ─── DashPay factories ─────────────────────────────────────────────

export function createMockStoredProfile(
  overrides?: Partial<StoredProfileDto>,
): StoredProfileDto {
  return {
    identityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    displayName: "Alice",
    bio: "Dash enthusiast and developer",
    avatarUrl: null,
    publicMessage: null,
    createdAt: 1707500000,
    updatedAt: 1707500000,
    ...overrides,
  };
}

export function createMockStoredContact(
  overrides?: Partial<StoredContactDto>,
): StoredContactDto {
  return {
    ownerIdentityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    contactIdentityId: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
    username: "bob.dash",
    displayName: "Bob",
    avatarUrl: null,
    publicMessage: null,
    contactStatus: "accepted",
    createdAt: 1707500000,
    updatedAt: 1707500000,
    lastSeen: 1707600000,
    ...overrides,
  };
}

export function createMockContactRequest(
  overrides?: Partial<StoredContactRequestDto>,
): StoredContactRequestDto {
  return {
    id: 1,
    fromIdentityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    toIdentityId: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
    toUsername: "bob.dash",
    fromUsername: null,
    accountLabel: "Account 0",
    requestType: "outgoing",
    status: "pending",
    createdAt: 1707500000,
    respondedAt: null,
    expiresAt: null,
    ...overrides,
  };
}

export function createMockStoredPayment(
  overrides?: Partial<StoredPaymentDto>,
): StoredPaymentDto {
  return {
    id: 1,
    txId: "a1b2c3d4e5f60718293a4b5c6d7e8f9001122334455667788990aabbccddeeff",
    fromIdentityId: "4EfA9Jrvv3nnCFdSf7fad59851kYrUWF21pYPKhq6Ycis",
    toIdentityId: "7BfX2Kqv8npRFdTe952mYsTWF31qZQNhq6Kdm5Zabc12",
    amount: 50_000_000, // 0.5 DASH
    memo: "Thanks for lunch!",
    paymentType: "standard",
    status: "confirmed",
    createdAt: 1707500000,
    confirmedAt: 1707500060,
    ...overrides,
  };
}

export function createMockContactPrivateInfo(
  overrides?: Partial<ContactPrivateInfoDto>,
): ContactPrivateInfoDto {
  return {
    nickname: "Bobby",
    notes: "Met at Dash conference 2024",
    isHidden: false,
    ...overrides,
  };
}

// ─── Misc platform factories ───────────────────────────────────────

export function createMockGreetResponse(
  overrides?: Partial<GreetResponse>,
): GreetResponse {
  return {
    message: "Hello from Tauri!",
    timestamp_ms: Date.now(),
    ...overrides,
  };
}

export function createMockDiffChainEntry(
  overrides?: Partial<DiffChainEntry>,
): DiffChainEntry {
  return {
    baseHeight: 1_919_000,
    baseHash:
      "00000000000000001122334455667788aabbccddeeff0011223344556677889900",
    height: 1_920_000,
    hash: "00000000000000112233445566778899aabbccddeeff00112233445566778899",
    ...overrides,
  };
}

export function createMockPlatformAddressAmount(
  overrides?: Partial<PlatformAddressAmountDto>,
): PlatformAddressAmountDto {
  return {
    address: "yXa1b2c3d4e5f6g7h8i9j0kLmNoPqRsTuV",
    amount: 200_000_000,
    ...overrides,
  };
}
