/**
 * Centralized test fixture factories for all DTO types.
 *
 * Import from "@/test/fixtures" to create realistic mock data:
 *
 *   import {
 *     createMockHdWallet,
 *     createMockIdentity,
 *     createMockToken,
 *     createMockContract,
 *     createMockContestedName,
 *     createMockSettings,
 *   } from "@/test/fixtures";
 *
 * All factories accept optional `Partial<T>` overrides to customize
 * specific fields while keeping sensible defaults for everything else.
 */

// Wallet fixtures
export {
  createMockUtxo,
  createMockWalletAddress,
  createMockWalletTransaction,
  createMockPlatformAddress,
  createMockAssetLock,
  createMockAssetLockProofDetails,
  createMockHdWallet,
  createMockSingleKeyWallet,
  createMockWalletRef,
  createMockWalletList,
} from "./wallets";

// Identity fixtures
export {
  createMockIdentityKey,
  createMockDpnsNameInfo,
  createMockTopUpEntry,
  createMockKeySpec,
  createMockContractBounds,
  createMockIdentity,
  createMockMasternodeIdentity,
  createMockIdentitySummary,
} from "./identities";

// Token fixtures
export {
  createMockToken,
  createMockTokenBalance,
  createMockTokenSearchResult,
  createMockTokenOperationInput,
  createMockMintingConfig,
  createMockIdentityTokenIdentifier,
} from "./tokens";

// Contract & document fixtures
export {
  createMockContract,
  createMockContractSummary,
  createMockDocumentType,
  createMockDocument,
  createMockDocumentPage,
  createMockWhereClause,
  createMockOrderByClause,
  createMockContractBoundsForContract,
  createMockContractBoundsForDocType,
} from "./contracts";

// DPNS & contest fixtures
export {
  createMockContestant,
  createMockDpnsNameEntry,
  createMockVoteChoice,
  createMockVoteEntry,
  createMockScheduledVote,
  createMockContestedName,
  createMockPastContest,
  createMockLockedContest,
  createMockSelectedVote,
} from "./dpns";

// Platform, settings, events & DashPay fixtures
export {
  createMockNetworkInfo,
  createMockSettings,
  createMockSpvStatus,
  createMockWalletUpdatedEvent,
  createMockZmqChainLockedBlock,
  createMockZmqConnectionStatus,
  createMockZmqIsLockedTransaction,
  createMockScheduledVoteExecutedEvent,
  createMockTaskResult,
  createMockTaskError,
  createMockDispatchResponse,
  createMockStoredProfile,
  createMockStoredContact,
  createMockContactRequest,
  createMockStoredPayment,
  createMockContactPrivateInfo,
  createMockGreetResponse,
  createMockDiffChainEntry,
  createMockPlatformAddressAmount,
} from "./platform";
