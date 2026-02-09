/**
 * TypeScript bindings verification tests.
 *
 * These tests verify that tauri-specta generates complete and correct
 * TypeScript bindings for all Tauri IPC commands, events, and types.
 * They do NOT invoke Tauri at runtime — they only check that the exported
 * shapes (functions, event objects, type aliases) are present and well-typed.
 */
import { describe, it, expect } from "vitest";
import { commands, events } from "./bindings";
import type {
  // Core infrastructure types
  GreetResponse,
  NetworkDto,
  NetworkInfo,
  DispatchTaskResponse,
  Result,

  // Identity types
  QualifiedIdentityDto,
  IdentityKeyDto,
  IdentityStatusDto,
  IdentityTypeDto,
  IdentitySummaryDto,
  DpnsNameInfoDto,
  DpnsNameEntryDto,

  // Wallet types
  WalletDto,
  SingleKeyWalletDto,
  WalletListDto,
  WalletAddressDto,
  WalletTransactionDto,
  AssetLockDto,
  PlatformAddressDto,
  PlatformSyncModeDto,

  // Contract & Document types
  DataContractDto,
  ContractSummaryDto,

  // Token types
  TokenOperationInput,
  TokenPaymentInfoDto,

  // DashPay types
  StoredProfileDto,
  StoredContactDto,
  StoredContactRequestDto,
  StoredPaymentDto,
  ContactPrivateInfoDto,

  // Contested resource / DPNS types
  ScheduledVoteDto,
  VoteChoiceDto,
  VoteEntry,

  // Settings types
  SettingsDto,
  ThemeModeDto,
  UserModeDto,
  CoreBackendModeDto,

  // Event payload types
  TaskResultEvent,
  TaskErrorEvent,
  SpvStatusEvent,
  SpvStatusDto,
  WalletUpdatedEvent,
  ZmqIsLockedTransactionEvent,
  ZmqChainLockedBlockEvent,
  ZmqConnectionStatusEvent,
  ScheduledVoteExecutedEvent,
} from "./bindings";

// ---------------------------------------------------------------------------
// Helper: assert that a value is a function (command wrapper)
// ---------------------------------------------------------------------------
function assertIsFunction(
  obj: Record<string, unknown>,
  name: string,
): void {
  expect(typeof obj[name]).toBe("function");
}

// ---------------------------------------------------------------------------
// 1. Command completeness — every domain has its commands
// ---------------------------------------------------------------------------
describe("Bindings: commands object", () => {
  it("exports the commands object with expected shape", () => {
    expect(commands).toBeDefined();
    expect(typeof commands).toBe("object");
  });

  it("contains infrastructure commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    assertIsFunction(cmds, "greet");
    assertIsFunction(cmds, "getAppVersion");
    assertIsFunction(cmds, "getNetworkInfo");
    assertIsFunction(cmds, "switchNetwork");
    assertIsFunction(cmds, "getSpvStatus");
  });

  it("contains all 27 identity commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const identityCmds = [
      "identityLoad",
      "identitySearchByDpnsName",
      "identitySearchFromWallet",
      "identitySearchUpToIndex",
      "identityRegisterDpnsName",
      "identityRefresh",
      "identityRefreshDpnsNames",
      "identityWithdraw",
      "identityTransfer",
      "identityAddKey",
      "identityDisableKeys",
      "identityReplaceKey",
      "identityRegister",
      "identityTopUp",
      "identityTopUpFromPlatformAddresses",
      "identityTransferToAddresses",
      "identityListLocal",
      "identityListUser",
      "identityListVoting",
      "identityGetById",
      "identitySetAlias",
      "identityGetAlias",
      "identityLoadOrder",
      "identitySaveOrder",
      "identityDelete",
      "identityListSummaries",
      "identityLocalDpnsNames",
    ];
    for (const cmd of identityCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(identityCmds.length).toBe(27);
  });

  it("contains all 10 core commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const coreCmds = [
      "coreGetBestChainLock",
      "coreGetBestChainLocks",
      "coreRefreshWalletInfo",
      "coreRefreshSingleKeyWalletInfo",
      "coreStartDashQt",
      "coreCreateRegistrationAssetLock",
      "coreCreateTopUpAssetLock",
      "coreSendWalletPayment",
      "coreSendSingleKeyWalletPayment",
      "coreRecoverAssetLocks",
    ];
    for (const cmd of coreCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(coreCmds.length).toBe(10);
  });

  it("contains all 18 wallet commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const walletCmds = [
      "walletGenerateReceiveAddress",
      "walletFetchPlatformAddressBalances",
      "walletTransferPlatformCredits",
      "walletWithdrawFromPlatformAddress",
      "walletFundPlatformAddressFromUtxos",
      "walletListAll",
      "walletGetHd",
      "walletGetSingleKey",
      "walletSelect",
      "walletSetAlias",
      "walletSetSingleKeyAlias",
      "walletRemove",
      "walletRemoveSingleKey",
      "walletStartSpv",
      "walletStopSpv",
      "walletClearSpvData",
      "walletNotifyUnlocked",
      "walletNotifyLocked",
    ];
    for (const cmd of walletCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(walletCmds.length).toBe(18);
  });

  it("contains all 10 contract commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const contractCmds = [
      "contractFetch",
      "contractFetchWithDescriptions",
      "contractFetchActiveGroupActions",
      "contractRegister",
      "contractUpdate",
      "contractSave",
      "contractRemove",
      "contractListLocal",
      "contractGetById",
      "contractSetAlias",
    ];
    for (const cmd of contractCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(contractCmds.length).toBe(10);
  });

  it("contains all 8 document commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const docCmds = [
      "documentBroadcast",
      "documentDelete",
      "documentReplace",
      "documentTransfer",
      "documentPurchase",
      "documentSetPrice",
      "documentFetch",
      "documentFetchPage",
    ];
    for (const cmd of docCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(docCmds.length).toBe(8);
  });

  it("contains all 25 token commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const tokenCmds = [
      "tokenQueryMyBalances",
      "tokenQueryIdentityBalance",
      "tokenQueryFrozenIdentities",
      "tokenQueryDescriptionsByKeyword",
      "tokenFetchByContractId",
      "tokenFetchByTokenId",
      "tokenSaveLocally",
      "tokenQueryPricing",
      "tokenMint",
      "tokenTransfer",
      "tokenBurn",
      "tokenDestroyFrozenFunds",
      "tokenFreeze",
      "tokenUnfreeze",
      "tokenPause",
      "tokenResume",
      "tokenClaim",
      "tokenEstimatePerpetualRewards",
      "tokenUpdateConfig",
      "tokenPurchase",
      "tokenSetDirectPurchasePrice",
      "tokenRegisterContract",
      "tokenRemove",
      "tokenLoadOrder",
      "tokenSaveOrder",
    ];
    for (const cmd of tokenCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(tokenCmds.length).toBe(25);
  });

  it("contains all 23 dashpay commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const dashpayCmds = [
      "dashpayLoadProfile",
      "dashpayUpdateProfile",
      "dashpayLoadContacts",
      "dashpayLoadContactRequests",
      "dashpayFetchContactProfile",
      "dashpaySearchProfiles",
      "dashpaySendContactRequest",
      "dashpaySendContactRequestWithProof",
      "dashpayAcceptContactRequest",
      "dashpayRejectContactRequest",
      "dashpayLoadPaymentHistory",
      "dashpaySendPaymentToContact",
      "dashpayUpdateContactInfo",
      "dashpayRegisterAddresses",
      "dashpayDbLoadProfile",
      "dashpayDbSaveProfile",
      "dashpayDbLoadContacts",
      "dashpayDbLoadPendingRequests",
      "dashpayDbLoadPayments",
      "dashpayDbLoadContactPrivateInfo",
      "dashpayDbSaveContactPrivateInfo",
      "dashpayDbSetContactHidden",
      "dashpayDbSaveAvatarBytes",
    ];
    for (const cmd of dashpayCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(dashpayCmds.length).toBe(23);
  });

  it("contains all 8 contested resource commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const contestedCmds = [
      "contestedQueryDpnsContests",
      "contestedVoteOnDpnsNames",
      "contestedScheduleDpnsVotes",
      "contestedCastScheduledVote",
      "contestedClearAllScheduledVotes",
      "contestedClearExecutedScheduledVotes",
      "contestedDeleteScheduledVote",
      "contestedGetScheduledVotes",
    ];
    for (const cmd of contestedCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(contestedCmds.length).toBe(8);
  });

  it("contains all 8 platform info commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const platformCmds = [
      "platformCurrentEpochInfo",
      "platformTotalCredits",
      "platformVersionVotingState",
      "platformValidatorSetInfo",
      "platformWithdrawalsInQueue",
      "platformRecentlyCompletedWithdrawals",
      "platformBasicInfo",
      "platformFetchAddressBalance",
    ];
    for (const cmd of platformCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(platformCmds.length).toBe(8);
  });

  it("contains all 10 system/mnlist/grovestark commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const systemCmds = [
      "systemWipePlatformData",
      "systemUpdateTheme",
      "mnlistFetchDiff",
      "mnlistFetchQrInfo",
      "mnlistFetchQrInfoWithDmls",
      "mnlistFetchChainLocks",
      "mnlistFetchDiffsChain",
      "grovestarkGenerateProof",
      "grovestarkVerifyProof",
      "broadcastStateTransition",
    ];
    for (const cmd of systemCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(systemCmds.length).toBe(10);
  });

  it("contains all 15 settings/context commands", () => {
    const cmds = commands as unknown as Record<string, unknown>;
    const settingsCmds = [
      "settingsGet",
      "settingsUpdatePassword",
      "settingsUpdateDashCore",
      "settingsUpdateDisableZmq",
      "settingsUpdateOnboardingCompleted",
      "settingsUpdateShowEvonodeTools",
      "settingsUpdateUserMode",
      "settingsUpdateCloseDashQtOnExit",
      "settingsUpdateAutoStartSpv",
      "settingsGetAutoStartSpv",
      "contextIsDeveloperMode",
      "contextEnableDeveloperMode",
      "contextGetFeeMultiplier",
      "contextSetFeeMultiplier",
      "contextGetNetwork",
    ];
    for (const cmd of settingsCmds) {
      assertIsFunction(cmds, cmd);
    }
    expect(settingsCmds.length).toBe(15);
  });

  it("total command count is 167", () => {
    const allKeys = Object.keys(commands);
    expect(allKeys.length).toBe(167);
  });
});

// ---------------------------------------------------------------------------
// 2. Events completeness — all 8 backend→frontend events
// ---------------------------------------------------------------------------
describe("Bindings: events object", () => {
  it("exports the events object", () => {
    expect(events).toBeDefined();
    expect(typeof events).toBe("object");
  });

  it("contains all 8 event listeners", () => {
    const evts = events as unknown as Record<string, unknown>;
    const expectedEvents = [
      "taskResultEvent",
      "taskErrorEvent",
      "zmqIsLockedTransactionEvent",
      "zmqChainLockedBlockEvent",
      "zmqConnectionStatusEvent",
      "spvStatusEvent",
      "walletUpdatedEvent",
      "scheduledVoteExecutedEvent",
    ];
    for (const evt of expectedEvents) {
      expect(evts[evt]).toBeDefined();
    }
  });
});

// ---------------------------------------------------------------------------
// 3. Type structure verification — key DTOs have expected fields
//    (compile-time checks via type assertions; runtime checks via shape)
// ---------------------------------------------------------------------------
describe("Bindings: type structure verification", () => {
  it("NetworkDto is a string union", () => {
    // NetworkDto should accept these literal values
    const values: NetworkDto[] = ["dash", "testnet", "devnet", "regtest"];
    expect(values).toHaveLength(4);
  });

  it("IdentityStatusDto is a string union", () => {
    const values: IdentityStatusDto[] = [
      "unknown",
      "pendingCreation",
      "active",
      "notFound",
      "failedCreation",
    ];
    expect(values).toHaveLength(5);
  });

  it("IdentityTypeDto is a string union", () => {
    const values: IdentityTypeDto[] = ["user", "masternode", "evonode"];
    expect(values).toHaveLength(3);
  });

  it("PlatformSyncModeDto is a string union", () => {
    const values: PlatformSyncModeDto[] = [
      "auto",
      "forceFull",
      "terminalOnly",
    ];
    expect(values).toHaveLength(3);
  });

  it("SpvStatusDto is a string union", () => {
    const values: SpvStatusDto[] = [
      "idle",
      "starting",
      "syncing",
      "running",
      "stopping",
      "stopped",
      "error",
    ];
    expect(values).toHaveLength(7);
  });

  it("ThemeModeDto is a string union", () => {
    const values: ThemeModeDto[] = ["light", "dark", "system"];
    expect(values).toHaveLength(3);
  });

  it("UserModeDto is a string union", () => {
    const values: UserModeDto[] = ["beginner", "advanced"];
    expect(values).toHaveLength(2);
  });

  it("CoreBackendModeDto is a string union", () => {
    const values: CoreBackendModeDto[] = ["spv", "rpc"];
    expect(values).toHaveLength(2);
  });

  it("Result type has ok and error variants", () => {
    const ok: Result<number, string> = { status: "ok", data: 42 };
    const err: Result<number, string> = {
      status: "error",
      error: "fail",
    };
    expect(ok.status).toBe("ok");
    expect(err.status).toBe("error");
  });

  it("GreetResponse has expected fields", () => {
    const resp: GreetResponse = { message: "hi", timestamp_ms: 123 };
    expect(resp.message).toBe("hi");
    expect(resp.timestamp_ms).toBe(123);
  });

  it("DispatchTaskResponse has taskId", () => {
    const resp: DispatchTaskResponse = { taskId: "abc-123" };
    expect(resp.taskId).toBe("abc-123");
  });

  it("NetworkInfo has expected structure", () => {
    const info: NetworkInfo = {
      activeNetwork: "dash",
      availableNetworks: ["dash", "testnet"],
    };
    expect(info.activeNetwork).toBe("dash");
    expect(info.availableNetworks).toHaveLength(2);
  });

  // Compile-time type assertion tests — these won't fail at runtime but
  // verify that the types are structurally sound during typecheck.
  it("QualifiedIdentityDto has identity-related fields", () => {
    // This is a compile-time check — we construct a partial and ensure
    // the types are compatible
    const partial: Partial<QualifiedIdentityDto> = {
      identityId: "abc",
      identityType: "user",
      status: "active",
    };
    expect(partial.identityId).toBe("abc");
  });

  it("WalletDto has wallet-related fields", () => {
    const partial: Partial<WalletDto> = {
      seedHash: "hash",
      walletType: "singleKey",
    };
    expect(partial.seedHash).toBe("hash");
  });

  it("SettingsDto has settings fields", () => {
    const partial: Partial<SettingsDto> = {
      network: "dash",
      themeMode: "dark",
      disableZmq: false,
      onboardingCompleted: true,
    };
    expect(partial.network).toBe("dash");
  });

  it("TaskResultEvent has task result fields", () => {
    const partial: Partial<TaskResultEvent> = {
      taskId: "t1",
    };
    expect(partial.taskId).toBe("t1");
  });

  it("TaskErrorEvent has error fields", () => {
    const partial: Partial<TaskErrorEvent> = {
      taskId: "t1",
      error: "something failed",
    };
    expect(partial.error).toBe("something failed");
  });

  it("SpvStatusEvent has spv fields", () => {
    const partial: Partial<SpvStatusEvent> = {
      network: "testnet",
      status: "syncing",
    };
    expect(partial.status).toBe("syncing");
  });
});

// ---------------------------------------------------------------------------
// 4. Type existence verification — ensure key types are importable
//    These are purely compile-time: if the import doesn't resolve, tsc fails.
// ---------------------------------------------------------------------------
describe("Bindings: type imports compile", () => {
  it("all key domain types are importable", () => {
    // The fact that these variables can be typed means the imports resolved
    const _identity: QualifiedIdentityDto | null = null;
    const _key: IdentityKeyDto | null = null;
    const _summary: IdentitySummaryDto | null = null;
    const _dpns: DpnsNameInfoDto | null = null;
    const _dpnsEntry: DpnsNameEntryDto | null = null;
    const _wallet: WalletDto | null = null;
    const _skw: SingleKeyWalletDto | null = null;
    const _walletList: WalletListDto | null = null;
    const _addr: WalletAddressDto | null = null;
    const _tx: WalletTransactionDto | null = null;
    const _lock: AssetLockDto | null = null;
    const _platAddr: PlatformAddressDto | null = null;
    const _contract: DataContractDto | null = null;
    const _contractSummary: ContractSummaryDto | null = null;
    const _tokenOp: TokenOperationInput | null = null;
    const _tokenPay: TokenPaymentInfoDto | null = null;
    const _profile: StoredProfileDto | null = null;
    const _contact: StoredContactDto | null = null;
    const _request: StoredContactRequestDto | null = null;
    const _payment: StoredPaymentDto | null = null;
    const _contactInfo: ContactPrivateInfoDto | null = null;
    const _vote: ScheduledVoteDto | null = null;
    const _voteChoice: VoteChoiceDto | null = null;
    const _voteEntry: VoteEntry | null = null;
    const _settings: SettingsDto | null = null;
    const _taskResult: TaskResultEvent | null = null;
    const _taskError: TaskErrorEvent | null = null;
    const _walletUpdated: WalletUpdatedEvent | null = null;
    const _zmqIsl: ZmqIsLockedTransactionEvent | null = null;
    const _zmqClb: ZmqChainLockedBlockEvent | null = null;
    const _zmqConn: ZmqConnectionStatusEvent | null = null;
    const _schedVote: ScheduledVoteExecutedEvent | null = null;

    // Suppress unused variable warnings
    expect([
      _identity,
      _key,
      _summary,
      _dpns,
      _dpnsEntry,
      _wallet,
      _skw,
      _walletList,
      _addr,
      _tx,
      _lock,
      _platAddr,
      _contract,
      _contractSummary,
      _tokenOp,
      _tokenPay,
      _profile,
      _contact,
      _request,
      _payment,
      _contactInfo,
      _vote,
      _voteChoice,
      _voteEntry,
      _settings,
      _taskResult,
      _taskError,
      _walletUpdated,
      _zmqIsl,
      _zmqClb,
      _zmqConn,
      _schedVote,
    ]).toHaveLength(32);
  });
});
