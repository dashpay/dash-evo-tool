/**
 * Tests for fixture factories.
 *
 * Verifies that every factory produces objects matching the expected DTO shapes
 * from bindings.ts and store types, and that optional overrides work correctly.
 */

import { describe, it, expect } from "vitest";

import {
  // Wallets
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
  // Identities
  createMockIdentityKey,
  createMockDpnsNameInfo,
  createMockTopUpEntry,
  createMockKeySpec,
  createMockContractBounds,
  createMockIdentity,
  createMockMasternodeIdentity,
  createMockIdentitySummary,
  // Tokens
  createMockToken,
  createMockTokenBalance,
  createMockTokenSearchResult,
  createMockTokenOperationInput,
  createMockMintingConfig,
  createMockIdentityTokenIdentifier,
  // Contracts & documents
  createMockContract,
  createMockContractSummary,
  createMockDocumentType,
  createMockDocument,
  createMockDocumentPage,
  createMockWhereClause,
  createMockOrderByClause,
  createMockContractBoundsForContract,
  createMockContractBoundsForDocType,
  // DPNS
  createMockContestant,
  createMockDpnsNameEntry,
  createMockVoteChoice,
  createMockVoteEntry,
  createMockScheduledVote,
  createMockContestedName,
  createMockPastContest,
  createMockLockedContest,
  createMockSelectedVote,
  // Platform
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
} from "../index";

// ─── Wallet fixtures ───────────────────────────────────────────────

describe("Wallet fixtures", () => {
  it("createMockHdWallet returns valid WalletDto shape", () => {
    const wallet = createMockHdWallet();
    expect(wallet).toHaveProperty("seedHash");
    expect(wallet).toHaveProperty("usesPassword");
    expect(wallet).toHaveProperty("alias");
    expect(wallet).toHaveProperty("isMain");
    expect(wallet).toHaveProperty("confirmedBalance");
    expect(wallet).toHaveProperty("unconfirmedBalance");
    expect(wallet).toHaveProperty("totalBalance");
    expect(wallet).toHaveProperty("addresses");
    expect(wallet).toHaveProperty("transactions");
    expect(wallet).toHaveProperty("unusedAssetLocks");
    expect(wallet).toHaveProperty("platformAddresses");
    expect(wallet).toHaveProperty("identityIndexes");
    expect(wallet).toHaveProperty("passwordHint");
    expect(typeof wallet.seedHash).toBe("string");
    expect(typeof wallet.totalBalance).toBe("number");
    expect(Array.isArray(wallet.addresses)).toBe(true);
  });

  it("createMockHdWallet accepts overrides", () => {
    const wallet = createMockHdWallet({
      alias: "Custom Wallet",
      totalBalance: 999,
    });
    expect(wallet.alias).toBe("Custom Wallet");
    expect(wallet.totalBalance).toBe(999);
    expect(wallet.seedHash).toBe("abc123def456"); // default preserved
  });

  it("createMockSingleKeyWallet returns valid shape", () => {
    const wallet = createMockSingleKeyWallet();
    expect(wallet).toHaveProperty("keyHash");
    expect(wallet).toHaveProperty("publicKey");
    expect(wallet).toHaveProperty("address");
    expect(wallet).toHaveProperty("utxoCount");
    expect(wallet).toHaveProperty("utxos");
    expect(Array.isArray(wallet.utxos)).toBe(true);
    expect(wallet.utxos.length).toBe(3);
    expect(wallet.utxoCount).toBe(3);
  });

  it("createMockUtxo returns valid UtxoDto shape", () => {
    const utxo = createMockUtxo();
    expect(utxo.txid).toHaveLength(64); // 32-byte hex
    expect(typeof utxo.vout).toBe("number");
    expect(typeof utxo.amount).toBe("number");
    expect(utxo.amount).toBe(100_000_000);
  });

  it("createMockAssetLock includes proof details", () => {
    const lock = createMockAssetLock();
    expect(lock.hasInstantLock).toBe(true);
    expect(lock.hasAssetLockProof).toBe(true);
    expect(lock.proofDetails).not.toBeNull();
    expect(lock.proofDetails!.type).toBe("instantSend");
  });

  it("createMockAssetLockProofDetails creates instant send proof", () => {
    const proof = createMockAssetLockProofDetails();
    expect(proof.type).toBe("instantSend");
    expect(
      (proof as { type: "instantSend"; instantLockTxid: string })
        .instantLockTxid,
    ).toBeDefined();
  });

  it("createMockWalletRef defaults to HD", () => {
    const ref = createMockWalletRef();
    expect(ref.type).toBe("hd");
    expect((ref as { type: "hd"; seedHash: string }).seedHash).toBe(
      "abc123def456",
    );
  });

  it("createMockWalletRef creates single-key ref", () => {
    const ref = createMockWalletRef({ type: "singleKey" });
    expect(ref.type).toBe("singleKey");
  });

  it("createMockWalletList returns valid shape with defaults", () => {
    const list = createMockWalletList();
    expect(list.hdWallets).toHaveLength(1);
    expect(list.singleKeyWallets).toHaveLength(0);
    expect(list.selected).not.toBeNull();
  });

  it("createMockWalletAddress has derivation path", () => {
    const addr = createMockWalletAddress();
    expect(addr.derivationPath).toMatch(/^m\/44'/);
  });

  it("createMockPlatformAddress has nonce", () => {
    const addr = createMockPlatformAddress();
    expect(typeof addr.nonce).toBe("number");
  });

  it("createMockWalletTransaction has required fields", () => {
    const tx = createMockWalletTransaction();
    expect(tx.txid).toHaveLength(64);
    expect(typeof tx.timestamp).toBe("number");
    expect(typeof tx.netAmount).toBe("number");
    expect(typeof tx.isOurs).toBe("boolean");
  });
});

// ─── Identity fixtures ─────────────────────────────────────────────

describe("Identity fixtures", () => {
  it("createMockIdentity returns valid QualifiedIdentityDto shape", () => {
    const id = createMockIdentity();
    expect(id).toHaveProperty("id");
    expect(id).toHaveProperty("identityType");
    expect(id).toHaveProperty("alias");
    expect(id).toHaveProperty("balance");
    expect(id).toHaveProperty("keys");
    expect(id).toHaveProperty("dpnsNames");
    expect(id).toHaveProperty("associatedWalletHashes");
    expect(id).toHaveProperty("walletIndex");
    expect(id).toHaveProperty("topUps");
    expect(id).toHaveProperty("status");
    expect(id).toHaveProperty("network");
    expect(id.identityType).toBe("user");
    expect(id.status).toBe("active");
    expect(id.keys.length).toBeGreaterThan(0);
    expect(id.dpnsNames.length).toBeGreaterThan(0);
  });

  it("createMockIdentity accepts overrides", () => {
    const id = createMockIdentity({ alias: "Bob", balance: 42 });
    expect(id.alias).toBe("Bob");
    expect(id.balance).toBe(42);
    expect(id.identityType).toBe("user"); // default preserved
  });

  it("createMockMasternodeIdentity returns masternode type", () => {
    const id = createMockMasternodeIdentity();
    expect(id.identityType).toBe("masternode");
    expect(id.voterIdentityId).not.toBeNull();
    expect(id.dpnsNames).toHaveLength(0);
  });

  it("createMockIdentityKey returns valid shape", () => {
    const key = createMockIdentityKey();
    expect(typeof key.keyId).toBe("number");
    expect(typeof key.keyType).toBe("string");
    expect(typeof key.purpose).toBe("string");
    expect(typeof key.securityLevel).toBe("string");
    expect(typeof key.data).toBe("string");
    expect(typeof key.isDisabled).toBe("boolean");
    expect(typeof key.hasPrivateKey).toBe("boolean");
  });

  it("createMockIdentitySummary has displayName", () => {
    const summary = createMockIdentitySummary();
    expect(typeof summary.displayName).toBe("string");
    expect(typeof summary.balance).toBe("number");
  });

  it("createMockKeySpec has contractBounds", () => {
    const spec = createMockKeySpec();
    expect(spec.contractBounds).toBeNull();
    const specWithBounds = createMockKeySpec({
      contractBounds: createMockContractBounds(),
    });
    expect(specWithBounds.contractBounds).not.toBeNull();
  });

  it("createMockDpnsNameInfo has name and timestamp", () => {
    const info = createMockDpnsNameInfo();
    expect(info.name).toContain(".dash");
    expect(typeof info.acquiredAt).toBe("number");
  });

  it("createMockTopUpEntry has index and amount", () => {
    const entry = createMockTopUpEntry();
    expect(typeof entry.index).toBe("number");
    expect(typeof entry.amount).toBe("number");
    expect(entry.amount).toBeGreaterThan(0);
  });
});

// ─── Token fixtures ────────────────────────────────────────────────

describe("Token fixtures", () => {
  it("createMockToken returns valid TokenEntry shape", () => {
    const token = createMockToken();
    expect(token).toHaveProperty("identityId");
    expect(token).toHaveProperty("tokenId");
    expect(token).toHaveProperty("contractId");
    expect(token).toHaveProperty("tokenPosition");
    expect(token).toHaveProperty("name");
    expect(token).toHaveProperty("ownerAlias");
    expect(token).toHaveProperty("balance");
    expect(token).toHaveProperty("decimals");
    expect(typeof token.balance).toBe("string"); // u128 as string
    expect(typeof token.decimals).toBe("number");
  });

  it("createMockTokenBalance has lower balance", () => {
    const token = createMockTokenBalance();
    expect(BigInt(token.balance)).toBeLessThan(
      BigInt(createMockToken().balance),
    );
  });

  it("createMockTokenSearchResult has description", () => {
    const result = createMockTokenSearchResult();
    expect(typeof result.contractId).toBe("string");
    expect(typeof result.description).toBe("string");
  });

  it("createMockTokenOperationInput has required fields", () => {
    const input = createMockTokenOperationInput();
    expect(typeof input.identityId).toBe("string");
    expect(typeof input.contractId).toBe("string");
    expect(typeof input.tokenPosition).toBe("number");
    expect(typeof input.keyId).toBe("number");
  });

  it("createMockMintingConfig has destination fields", () => {
    const config = createMockMintingConfig();
    expect(typeof config.allowChoosingDestination).toBe("boolean");
    expect(config.defaultDestinationIdentityId).toBeNull();
  });

  it("createMockIdentityTokenIdentifier has identity and token IDs", () => {
    const id = createMockIdentityTokenIdentifier();
    expect(typeof id.identityId).toBe("string");
    expect(typeof id.tokenId).toBe("string");
  });
});

// ─── Contract fixtures ─────────────────────────────────────────────

describe("Contract fixtures", () => {
  it("createMockContract returns valid DataContractDto shape", () => {
    const contract = createMockContract();
    expect(contract).toHaveProperty("id");
    expect(contract).toHaveProperty("ownerId");
    expect(contract).toHaveProperty("alias");
    expect(contract).toHaveProperty("version");
    expect(contract).toHaveProperty("documentTypeNames");
    expect(contract).toHaveProperty("tokenCount");
    expect(contract).toHaveProperty("schemaJson");
    expect(Array.isArray(contract.documentTypeNames)).toBe(true);
    expect(contract.documentTypeNames.length).toBeGreaterThan(0);
  });

  it("createMockContractSummary has counts", () => {
    const summary = createMockContractSummary();
    expect(typeof summary.documentTypeCount).toBe("number");
    expect(typeof summary.tokenCount).toBe("number");
  });

  it("createMockDocumentType has schema and indexes", () => {
    const dt = createMockDocumentType();
    expect(typeof dt.name).toBe("string");
    expect(dt.schema).not.toBeNull();
    expect(Array.isArray(dt.indexes)).toBe(true);
  });

  it("createMockDocument returns valid DocumentEntry shape", () => {
    const doc = createMockDocument();
    expect(doc).toHaveProperty("id");
    expect(doc).toHaveProperty("ownerId");
    expect(doc).toHaveProperty("documentType");
    expect(doc).toHaveProperty("data");
    expect(doc).toHaveProperty("revision");
    expect(doc).toHaveProperty("createdAt");
    expect(doc).toHaveProperty("updatedAt");
    expect(doc).toHaveProperty("transferredAt");
    expect(typeof doc.revision).toBe("number");
  });

  it("createMockDocumentPage wraps document", () => {
    const page = createMockDocumentPage();
    expect(page.document).not.toBeNull();
    expect(page.document!.id).toBe(page.id);
  });

  it("createMockContractBoundsForContract creates single contract bound", () => {
    const bounds = createMockContractBoundsForContract();
    expect(bounds.type).toBe("singleContract");
  });

  it("createMockContractBoundsForDocType creates doc type bound", () => {
    const bounds = createMockContractBoundsForDocType(undefined, "preorder");
    expect(bounds.type).toBe("singleContractDocumentType");
    expect(
      (bounds as { documentTypeName: string }).documentTypeName,
    ).toBe("preorder");
  });

  it("createMockWhereClause has field, operator, value", () => {
    const clause = createMockWhereClause();
    expect(typeof clause.field).toBe("string");
    expect(typeof clause.operator).toBe("string");
    expect(clause.value).toBeDefined();
  });

  it("createMockOrderByClause has field and direction", () => {
    const clause = createMockOrderByClause();
    expect(typeof clause.field).toBe("string");
    expect(typeof clause.direction).toBe("string");
  });
});

// ─── DPNS fixtures ─────────────────────────────────────────────────

describe("DPNS fixtures", () => {
  it("createMockContestedName returns valid shape", () => {
    const contest = createMockContestedName();
    expect(contest).toHaveProperty("normalizedContestedName");
    expect(contest).toHaveProperty("contestants");
    expect(contest).toHaveProperty("lockedVotes");
    expect(contest).toHaveProperty("abstainVotes");
    expect(contest).toHaveProperty("awardedTo");
    expect(contest).toHaveProperty("endTime");
    expect(contest).toHaveProperty("state");
    expect(contest).toHaveProperty("lastUpdated");
    expect(contest.contestants).not.toBeNull();
    expect(contest.contestants!.length).toBe(2);
  });

  it("createMockPastContest has winner", () => {
    const past = createMockPastContest();
    expect(past.awardedTo).not.toBeNull();
    expect(typeof past.state).toBe("object"); // { wonBy: ... }
  });

  it("createMockLockedContest has locked state", () => {
    const locked = createMockLockedContest();
    expect(locked.state).toBe("locked");
    expect(locked.lockedVotes).toBeGreaterThan(0);
  });

  it("createMockScheduledVote returns valid shape", () => {
    const vote = createMockScheduledVote();
    expect(typeof vote.contestedName).toBe("string");
    expect(typeof vote.voterId).toBe("string");
    expect(typeof vote.unixTimestamp).toBe("number");
    expect(typeof vote.executedSuccessfully).toBe("boolean");
    expect(vote.choice).toBeDefined();
  });

  it("createMockVoteChoice creates all three types", () => {
    const towards = createMockVoteChoice("towards");
    expect(typeof towards).toBe("object");
    expect(
      (towards as { towardsIdentity: { identityId: string } })
        .towardsIdentity,
    ).toBeDefined();

    const abstain = createMockVoteChoice("abstain");
    expect(abstain).toBe("abstain");

    const lock = createMockVoteChoice("lock");
    expect(lock).toBe("lock");
  });

  it("createMockContestant has vote count", () => {
    const contestant = createMockContestant();
    expect(typeof contestant.votes).toBe("number");
    expect(typeof contestant.id).toBe("string");
    expect(typeof contestant.documentId).toBe("string");
  });

  it("createMockDpnsNameEntry has name and timestamp", () => {
    const entry = createMockDpnsNameEntry();
    expect(entry.name).toContain(".dash");
    expect(typeof entry.acquiredAt).toBe("number");
  });

  it("createMockVoteEntry has name and choice", () => {
    const entry = createMockVoteEntry();
    expect(typeof entry.contestedName).toBe("string");
    expect(entry.choice).toBeDefined();
  });

  it("createMockSelectedVote has endTime", () => {
    const vote = createMockSelectedVote();
    expect(typeof vote.contestedName).toBe("string");
    expect(vote.choice).toBeDefined();
    expect(typeof vote.endTime).toBe("number");
  });
});

// ─── Platform fixtures ─────────────────────────────────────────────

describe("Platform fixtures", () => {
  it("createMockNetworkInfo returns valid shape", () => {
    const info = createMockNetworkInfo();
    expect(info.activeNetwork).toBe("testnet");
    expect(info.availableNetworks).toHaveLength(4);
    expect(info.availableNetworks).toContain("dash");
    expect(info.availableNetworks).toContain("testnet");
  });

  it("createMockSettings returns valid SettingsDto shape", () => {
    const settings = createMockSettings();
    expect(settings).toHaveProperty("network");
    expect(settings).toHaveProperty("themeMode");
    expect(settings).toHaveProperty("overwriteDashConf");
    expect(settings).toHaveProperty("disableZmq");
    expect(settings).toHaveProperty("onboardingCompleted");
    expect(settings).toHaveProperty("showEvonodeTools");
    expect(settings).toHaveProperty("userMode");
    expect(settings).toHaveProperty("closeDashQtOnExit");
    expect(settings).toHaveProperty("coreBackendMode");
    expect(settings).toHaveProperty("hasPassword");
    expect(settings).toHaveProperty("dashQtPath");
  });

  it("createMockSettings accepts overrides", () => {
    const settings = createMockSettings({ themeMode: "dark" });
    expect(settings.themeMode).toBe("dark");
    expect(settings.network).toBe("testnet"); // default preserved
  });

  it("createMockSpvStatus has sync progress fields", () => {
    const status = createMockSpvStatus();
    expect(status.status).toBe("running");
    expect(status.syncProgressPct).toBe(100);
    expect(typeof status.connectedPeers).toBe("number");
  });

  it("createMockTaskResult has result payload", () => {
    const result = createMockTaskResult();
    expect(typeof result.taskId).toBe("string");
    expect(result.result).toBeDefined();
    expect(result.result.type).toBe("identityCompleted");
  });

  it("createMockTaskError has error details", () => {
    const err = createMockTaskError();
    expect(typeof err.domain).toBe("string");
    expect(typeof err.message).toBe("string");
    expect(typeof err.details).toBe("string");
    expect(typeof err.recoverable).toBe("boolean");
  });

  it("createMockDispatchResponse has taskId", () => {
    const response = createMockDispatchResponse();
    expect(typeof response.taskId).toBe("string");
  });

  it("createMockWalletUpdatedEvent has wallet hash and network", () => {
    const event = createMockWalletUpdatedEvent();
    expect(typeof event.walletSeedHash).toBe("string");
    expect(typeof event.network).toBe("string");
  });

  it("createMockZmqIsLockedTransaction has txid and raw", () => {
    const event = createMockZmqIsLockedTransaction();
    expect(typeof event.txid).toBe("string");
    expect(typeof event.rawTx).toBe("string");
    expect(typeof event.affectedUtxoCount).toBe("number");
  });

  it("createMockGreetResponse has message and timestamp", () => {
    const response = createMockGreetResponse();
    expect(typeof response.message).toBe("string");
    expect(typeof response.timestamp_ms).toBe("number");
  });

  it("createMockDiffChainEntry has heights and hashes", () => {
    const entry = createMockDiffChainEntry();
    expect(typeof entry.baseHeight).toBe("number");
    expect(typeof entry.baseHash).toBe("string");
    expect(typeof entry.height).toBe("number");
    expect(typeof entry.hash).toBe("string");
    expect(entry.height).toBeGreaterThan(entry.baseHeight);
  });

  it("createMockPlatformAddressAmount has address and amount", () => {
    const addr = createMockPlatformAddressAmount();
    expect(typeof addr.address).toBe("string");
    expect(typeof addr.amount).toBe("number");
  });
});

// ─── DashPay fixtures ──────────────────────────────────────────────

describe("DashPay fixtures", () => {
  it("createMockStoredProfile returns valid shape", () => {
    const profile = createMockStoredProfile();
    expect(typeof profile.identityId).toBe("string");
    expect(typeof profile.displayName).toBe("string");
    expect(typeof profile.createdAt).toBe("number");
  });

  it("createMockStoredContact has contact status", () => {
    const contact = createMockStoredContact();
    expect(typeof contact.contactStatus).toBe("string");
    expect(typeof contact.ownerIdentityId).toBe("string");
    expect(typeof contact.contactIdentityId).toBe("string");
  });

  it("createMockContactRequest has request type", () => {
    const request = createMockContactRequest();
    expect(typeof request.requestType).toBe("string");
    expect(typeof request.status).toBe("string");
    expect(typeof request.fromIdentityId).toBe("string");
  });

  it("createMockStoredPayment has amount and memo", () => {
    const payment = createMockStoredPayment();
    expect(typeof payment.amount).toBe("number");
    expect(payment.amount).toBeGreaterThan(0);
    expect(typeof payment.memo).toBe("string");
  });

  it("createMockContactPrivateInfo has nickname and notes", () => {
    const info = createMockContactPrivateInfo();
    expect(typeof info.nickname).toBe("string");
    expect(typeof info.notes).toBe("string");
    expect(typeof info.isHidden).toBe("boolean");
  });
});

// ─── Event fixtures ────────────────────────────────────────────────

describe("Event fixtures", () => {
  it("createMockZmqChainLockedBlock has block info", () => {
    const event = createMockZmqChainLockedBlock();
    expect(typeof event.blockHeight).toBe("number");
    expect(typeof event.blockHash).toBe("string");
    expect(typeof event.txCount).toBe("number");
  });

  it("createMockZmqConnectionStatus has connected flag", () => {
    const event = createMockZmqConnectionStatus();
    expect(typeof event.connected).toBe("boolean");
  });

  it("createMockScheduledVoteExecutedEvent has success flag", () => {
    const event = createMockScheduledVoteExecutedEvent();
    expect(typeof event.success).toBe("boolean");
    expect(event.error).toBeNull();
  });
});

// ─── Cross-fixture consistency ─────────────────────────────────────

describe("Cross-fixture consistency", () => {
  it("identity wallet hash matches wallet seed hash", () => {
    const identity = createMockIdentity();
    const wallet = createMockHdWallet();
    expect(identity.associatedWalletHashes).toContain(wallet.seedHash);
  });

  it("token identityId matches identity id", () => {
    const identity = createMockIdentity();
    const token = createMockToken();
    expect(token.identityId).toBe(identity.id);
  });

  it("document ownerId matches identity id", () => {
    const identity = createMockIdentity();
    const doc = createMockDocument();
    expect(doc.ownerId).toBe(identity.id);
  });

  it("contest contestant id matches identity id", () => {
    const identity = createMockIdentity();
    const contest = createMockContestedName();
    expect(contest.contestants![0].id).toBe(identity.id);
  });

  it("stored contact owner matches identity id", () => {
    const identity = createMockIdentity();
    const contact = createMockStoredContact();
    expect(contact.ownerIdentityId).toBe(identity.id);
  });
});
