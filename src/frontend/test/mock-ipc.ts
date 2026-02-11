/**
 * Centralized Mock IPC Infrastructure
 *
 * Provides a single place to configure mock Tauri IPC command handlers
 * for all 182 commands in bindings.ts. Tests import `createMockBindings()`
 * and pass overrides for the specific commands they care about.
 *
 * Usage in a test file:
 *
 *   import { createMockBindings, type MockCommands } from "@/test/mock-ipc";
 *
 *   const overrides: MockCommands = {
 *     walletListAll: vi.fn().mockResolvedValue({
 *       status: "ok",
 *       data: { hdWallets: [myWallet], singleKeyWallets: [], selected: null },
 *     }),
 *   };
 *   const { commands, events, callHistory } = createMockBindings(overrides);
 *
 *   vi.mock("@/bindings", () => ({ commands, events }));
 *
 * Then assert:
 *   expect(callHistory.get("walletListAll")).toHaveLength(1);
 *   expect(callHistory.get("walletListAll")![0]).toEqual({ ...args });
 */

import { vi, type Mock } from "vitest";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** A record of command name → mock function (subset of all commands). */
export type MockCommands = {
  [K in CommandName]?: Mock;
};

/** A record of event name → mock listen/once/emit. */
export type MockEvents = {
  [K in EventName]?: {
    listen?: Mock;
    once?: Mock;
    emit?: Mock;
  };
};

/** Call history: command name → array of argument objects. */
export type CallHistory = Map<string, unknown[]>;

/** All known command names (matching keys on `commands` in bindings.ts). */
export type CommandName =
  | "greet"
  | "getAppVersion"
  | "getNetworkInfo"
  | "switchNetwork"
  | "getSpvStatus"
  // Identity
  | "identityLoad"
  | "identitySearchByDpnsName"
  | "identitySearchFromWallet"
  | "identitySearchUpToIndex"
  | "identityRegisterDpnsName"
  | "identityRefresh"
  | "identityRefreshDpnsNames"
  | "identityWithdraw"
  | "identityTransfer"
  | "identityAddKey"
  | "identityDisableKeys"
  | "identityReplaceKey"
  | "identityRegister"
  | "identityTopUp"
  | "identityTopUpFromPlatformAddresses"
  | "identityTransferToAddresses"
  | "identityListLocal"
  | "identityListUser"
  | "identityListVoting"
  | "identityGetById"
  | "identitySetAlias"
  | "identityGetAlias"
  | "identityLoadOrder"
  | "identitySaveOrder"
  | "identityDelete"
  | "identityListSummaries"
  | "identityLocalDpnsNames"
  | "identitySignMessage"
  // Core
  | "coreGetBestChainLock"
  | "coreGetBestChainLocks"
  | "coreRefreshWalletInfo"
  | "coreRefreshSingleKeyWalletInfo"
  | "coreStartDashQt"
  | "coreCreateRegistrationAssetLock"
  | "coreCreateTopUpAssetLock"
  | "coreSendWalletPayment"
  | "coreSendSingleKeyWalletPayment"
  | "coreRecoverAssetLocks"
  // Wallet
  | "walletGenerateReceiveAddress"
  | "walletFetchPlatformAddressBalances"
  | "walletTransferPlatformCredits"
  | "walletWithdrawFromPlatformAddress"
  | "walletFundPlatformAddressFromUtxos"
  | "walletFundPlatformFromAssetLock"
  | "walletCreate"
  | "walletImportMnemonic"
  | "walletImportPrivateKey"
  | "walletListAll"
  | "walletGetHd"
  | "walletGetSingleKey"
  | "walletSelect"
  | "walletSetAlias"
  | "walletSetSingleKeyAlias"
  | "walletRemove"
  | "walletRemoveSingleKey"
  | "walletStartSpv"
  | "walletStopSpv"
  | "walletClearSpvData"
  | "walletBootstrapAddresses"
  | "walletNotifyUnlocked"
  | "walletNotifyLocked"
  | "walletGetPrivateKey"
  // Contract
  | "contractFetch"
  | "contractFetchWithDescriptions"
  | "contractFetchActiveGroupActions"
  | "contractRegister"
  | "contractUpdate"
  | "contractSave"
  | "contractRemove"
  | "contractListLocal"
  | "contractGetById"
  | "contractGetByTokenId"
  | "contractSetAlias"
  // Document
  | "documentBroadcast"
  | "documentDelete"
  | "documentReplace"
  | "documentTransfer"
  | "documentPurchase"
  | "documentSetPrice"
  | "documentFetch"
  | "documentFetchPage"
  // Token
  | "tokenQueryMyBalances"
  | "tokenQueryIdentityBalance"
  | "tokenQueryFrozenIdentities"
  | "tokenQueryDescriptionsByKeyword"
  | "tokenFetchByContractId"
  | "tokenFetchByTokenId"
  | "tokenSaveLocally"
  | "tokenQueryPricing"
  | "tokenMint"
  | "tokenTransfer"
  | "tokenBurn"
  | "tokenDestroyFrozenFunds"
  | "tokenFreeze"
  | "tokenUnfreeze"
  | "tokenPause"
  | "tokenResume"
  | "tokenClaim"
  | "tokenEstimatePerpetualRewards"
  | "tokenQueryClaims"
  | "tokenUpdateConfig"
  | "tokenPurchase"
  | "tokenSetDirectPurchasePrice"
  | "tokenRegisterContract"
  | "tokenRemove"
  | "tokenLoadOrder"
  | "tokenSaveOrder"
  | "tokenGetMintingConfig"
  // DashPay
  | "dashpayLoadProfile"
  | "dashpayUpdateProfile"
  | "dashpayLoadContacts"
  | "dashpayLoadContactRequests"
  | "dashpayFetchContactProfile"
  | "dashpaySearchProfiles"
  | "dashpaySendContactRequest"
  | "dashpaySendContactRequestWithProof"
  | "dashpayAcceptContactRequest"
  | "dashpayRejectContactRequest"
  | "dashpayLoadPaymentHistory"
  | "dashpaySendPaymentToContact"
  | "dashpayUpdateContactInfo"
  | "dashpayRegisterAddresses"
  | "dashpayDbLoadProfile"
  | "dashpayDbSaveProfile"
  | "dashpayDbLoadContacts"
  | "dashpayDbLoadPendingRequests"
  | "dashpayDbLoadPayments"
  | "dashpayDbLoadContactPrivateInfo"
  | "dashpayDbSaveContactPrivateInfo"
  | "dashpayDbSetContactHidden"
  | "dashpayDbSaveAvatarBytes"
  // Contested / DPNS
  | "contestedQueryDpnsContests"
  | "contestedVoteOnDpnsNames"
  | "contestedScheduleDpnsVotes"
  | "contestedCastScheduledVote"
  | "contestedClearAllScheduledVotes"
  | "contestedClearExecutedScheduledVotes"
  | "contestedDeleteScheduledVote"
  | "contestedGetScheduledVotes"
  // Parsers
  | "parseDataContract"
  | "parseDocument"
  | "parseGrovedbProof"
  // Platform Info
  | "platformCurrentEpochInfo"
  | "platformTotalCredits"
  | "platformVersionVotingState"
  | "platformValidatorSetInfo"
  | "platformWithdrawalsInQueue"
  | "platformRecentlyCompletedWithdrawals"
  | "platformBasicInfo"
  | "platformFetchAddressBalance"
  // System
  | "systemWipePlatformData"
  | "systemUpdateTheme"
  // Masternode
  | "mnlistFetchDiff"
  | "mnlistFetchQrInfo"
  | "mnlistFetchQrInfoWithDmls"
  | "mnlistFetchChainLocks"
  | "mnlistFetchDiffsChain"
  // GroveSTARK
  | "grovestarkGenerateProof"
  | "grovestarkVerifyProof"
  // Broadcast
  | "broadcastStateTransition"
  // Settings
  | "settingsGet"
  | "settingsUpdatePassword"
  | "settingsUpdateDashCore"
  | "settingsUpdateDisableZmq"
  | "settingsUpdateOnboardingCompleted"
  | "settingsUpdateShowEvonodeTools"
  | "settingsUpdateUserMode"
  | "settingsUpdateCloseDashQtOnExit"
  | "settingsUpdateAutoStartSpv"
  | "settingsGetAutoStartSpv"
  // Context
  | "contextIsDeveloperMode"
  | "contextEnableDeveloperMode"
  | "contextGetFeeMultiplier"
  | "contextSetFeeMultiplier"
  | "contextGetNetwork"
  | "contextGetCoreBackendMode"
  | "contextSetCoreBackendMode";

/** All known event names (matching keys on `events` in bindings.ts). */
export type EventName =
  | "scheduledVoteExecutedEvent"
  | "spvStatusEvent"
  | "taskErrorEvent"
  | "taskResultEvent"
  | "walletUpdatedEvent"
  | "zmqChainLockedBlockEvent"
  | "zmqConnectionStatusEvent"
  | "zmqIsLockedTransactionEvent";

// ---------------------------------------------------------------------------
// Default response helpers
// ---------------------------------------------------------------------------

/** Shorthand for `{ status: "ok", data }`. */
function ok<T>(data: T) {
  return { status: "ok" as const, data };
}

/** Dispatch task response — a backend task ID string. */
const DISPATCH_OK = ok({ taskId: "mock-task-id" });


// ---------------------------------------------------------------------------
// Default handlers for every command
// ---------------------------------------------------------------------------

/**
 * Returns a map of command name → default mock handler.
 * Each handler returns a realistic empty/default response matching the
 * real binding's return type.
 *
 * The `tracker` function wraps each handler to record call arguments.
 */
function buildDefaultCommands(history: CallHistory): Record<CommandName, Mock> {
  function tracked(name: string, impl: (...args: unknown[]) => unknown): Mock {
    const fn = vi.fn((...args: unknown[]) => {
      const calls = history.get(name) ?? [];
      calls.push(args.length === 1 ? args[0] : args.length === 0 ? undefined : args);
      history.set(name, calls);
      return impl(...args);
    });
    return fn;
  }

  // Convenience: creates a tracked mock that resolves to `value`.
  function resolves(name: string, value: unknown): Mock {
    return tracked(name, () => Promise.resolve(value));
  }

  // Convenience: creates a tracked mock that resolves to `{ status: "ok", data }`.
  function resolvesOk(name: string, data: unknown): Mock {
    return resolves(name, ok(data));
  }

  // Convenience: creates a tracked mock for dispatch-style commands.
  function dispatchOk(name: string): Mock {
    return resolves(name, DISPATCH_OK);
  }

  return {
    // -- General --
    greet: resolves("greet", { message: "Hello, Mock!" }),
    getAppVersion: resolves("getAppVersion", "0.0.0-test"),
    getNetworkInfo: resolves("getNetworkInfo", {
      network: "Testnet",
      coreVersion: "21.0.0",
      platformVersion: "1.0.0",
      connected: true,
    }),
    switchNetwork: resolvesOk("switchNetwork", null),
    getSpvStatus: resolves("getSpvStatus", []),

    // -- Identity --
    identityLoad: dispatchOk("identityLoad"),
    identitySearchByDpnsName: dispatchOk("identitySearchByDpnsName"),
    identitySearchFromWallet: dispatchOk("identitySearchFromWallet"),
    identitySearchUpToIndex: dispatchOk("identitySearchUpToIndex"),
    identityRegisterDpnsName: dispatchOk("identityRegisterDpnsName"),
    identityRefresh: dispatchOk("identityRefresh"),
    identityRefreshDpnsNames: resolves("identityRefreshDpnsNames", { taskId: "mock-task-id" }),
    identityWithdraw: dispatchOk("identityWithdraw"),
    identityTransfer: dispatchOk("identityTransfer"),
    identityAddKey: dispatchOk("identityAddKey"),
    identityDisableKeys: dispatchOk("identityDisableKeys"),
    identityReplaceKey: dispatchOk("identityReplaceKey"),
    identityRegister: dispatchOk("identityRegister"),
    identityTopUp: dispatchOk("identityTopUp"),
    identityTopUpFromPlatformAddresses: dispatchOk("identityTopUpFromPlatformAddresses"),
    identityTransferToAddresses: dispatchOk("identityTransferToAddresses"),
    identityListLocal: resolvesOk("identityListLocal", []),
    identityListUser: resolvesOk("identityListUser", []),
    identityListVoting: resolvesOk("identityListVoting", []),
    identityGetById: resolvesOk("identityGetById", null),
    identitySetAlias: resolvesOk("identitySetAlias", null),
    identityGetAlias: resolvesOk("identityGetAlias", null),
    identityLoadOrder: resolvesOk("identityLoadOrder", []),
    identitySaveOrder: resolvesOk("identitySaveOrder", null),
    identityDelete: resolvesOk("identityDelete", null),
    identityListSummaries: resolvesOk("identityListSummaries", []),
    identityLocalDpnsNames: resolvesOk("identityLocalDpnsNames", []),
    identitySignMessage: resolvesOk("identitySignMessage", ""),

    // -- Core --
    coreGetBestChainLock: resolves("coreGetBestChainLock", { taskId: "mock-task-id" }),
    coreGetBestChainLocks: resolves("coreGetBestChainLocks", { taskId: "mock-task-id" }),
    coreRefreshWalletInfo: dispatchOk("coreRefreshWalletInfo"),
    coreRefreshSingleKeyWalletInfo: dispatchOk("coreRefreshSingleKeyWalletInfo"),
    coreStartDashQt: resolves("coreStartDashQt", { taskId: "mock-task-id" }),
    coreCreateRegistrationAssetLock: dispatchOk("coreCreateRegistrationAssetLock"),
    coreCreateTopUpAssetLock: dispatchOk("coreCreateTopUpAssetLock"),
    coreSendWalletPayment: dispatchOk("coreSendWalletPayment"),
    coreSendSingleKeyWalletPayment: dispatchOk("coreSendSingleKeyWalletPayment"),
    coreRecoverAssetLocks: dispatchOk("coreRecoverAssetLocks"),

    // -- Wallet --
    walletGenerateReceiveAddress: dispatchOk("walletGenerateReceiveAddress"),
    walletFetchPlatformAddressBalances: dispatchOk("walletFetchPlatformAddressBalances"),
    walletTransferPlatformCredits: dispatchOk("walletTransferPlatformCredits"),
    walletWithdrawFromPlatformAddress: dispatchOk("walletWithdrawFromPlatformAddress"),
    walletFundPlatformAddressFromUtxos: dispatchOk("walletFundPlatformAddressFromUtxos"),
    walletFundPlatformFromAssetLock: dispatchOk("walletFundPlatformFromAssetLock"),
    walletCreate: resolvesOk("walletCreate", {
      seedHash: "mock-seed-hash",
      alias: "Mock Wallet",
      identityRegistrations: [],
      accounts: [],
      utxos: [],
      assetLocks: [],
    }),
    walletImportMnemonic: resolvesOk("walletImportMnemonic", {
      seedHash: "mock-seed-hash",
      alias: "Imported Wallet",
      identityRegistrations: [],
      accounts: [],
      utxos: [],
      assetLocks: [],
    }),
    walletImportPrivateKey: resolvesOk("walletImportPrivateKey", {
      keyHash: "mock-key-hash",
      alias: "Imported Key",
      address: "yMockAddress123",
      balanceSatoshis: 0,
      utxos: [],
    }),
    walletListAll: resolvesOk("walletListAll", {
      hdWallets: [],
      singleKeyWallets: [],
      selected: null,
    }),
    walletGetHd: resolvesOk("walletGetHd", null),
    walletGetSingleKey: resolvesOk("walletGetSingleKey", null),
    walletSelect: resolvesOk("walletSelect", null),
    walletSetAlias: resolvesOk("walletSetAlias", null),
    walletSetSingleKeyAlias: resolvesOk("walletSetSingleKeyAlias", null),
    walletRemove: resolvesOk("walletRemove", null),
    walletRemoveSingleKey: resolvesOk("walletRemoveSingleKey", null),
    walletStartSpv: resolvesOk("walletStartSpv", null),
    walletStopSpv: resolves("walletStopSpv", undefined),
    walletClearSpvData: resolvesOk("walletClearSpvData", null),
    walletBootstrapAddresses: resolvesOk("walletBootstrapAddresses", null),
    walletNotifyUnlocked: resolvesOk("walletNotifyUnlocked", null),
    walletNotifyLocked: resolvesOk("walletNotifyLocked", null),
    walletGetPrivateKey: resolvesOk("walletGetPrivateKey", ""),

    // -- Contract --
    contractFetch: dispatchOk("contractFetch"),
    contractFetchWithDescriptions: dispatchOk("contractFetchWithDescriptions"),
    contractFetchActiveGroupActions: dispatchOk("contractFetchActiveGroupActions"),
    contractRegister: dispatchOk("contractRegister"),
    contractUpdate: dispatchOk("contractUpdate"),
    contractSave: dispatchOk("contractSave"),
    contractRemove: resolvesOk("contractRemove", null),
    contractListLocal: resolvesOk("contractListLocal", []),
    contractGetById: resolvesOk("contractGetById", null),
    contractGetByTokenId: resolvesOk("contractGetByTokenId", null),
    contractSetAlias: resolvesOk("contractSetAlias", null),

    // -- Document --
    documentBroadcast: dispatchOk("documentBroadcast"),
    documentDelete: dispatchOk("documentDelete"),
    documentReplace: dispatchOk("documentReplace"),
    documentTransfer: dispatchOk("documentTransfer"),
    documentPurchase: dispatchOk("documentPurchase"),
    documentSetPrice: dispatchOk("documentSetPrice"),
    documentFetch: dispatchOk("documentFetch"),
    documentFetchPage: dispatchOk("documentFetchPage"),

    // -- Token --
    tokenQueryMyBalances: resolves("tokenQueryMyBalances", { taskId: "mock-task-id" }),
    tokenQueryIdentityBalance: dispatchOk("tokenQueryIdentityBalance"),
    tokenQueryFrozenIdentities: dispatchOk("tokenQueryFrozenIdentities"),
    tokenQueryDescriptionsByKeyword: dispatchOk("tokenQueryDescriptionsByKeyword"),
    tokenFetchByContractId: dispatchOk("tokenFetchByContractId"),
    tokenFetchByTokenId: dispatchOk("tokenFetchByTokenId"),
    tokenSaveLocally: dispatchOk("tokenSaveLocally"),
    tokenQueryPricing: dispatchOk("tokenQueryPricing"),
    tokenMint: dispatchOk("tokenMint"),
    tokenTransfer: dispatchOk("tokenTransfer"),
    tokenBurn: dispatchOk("tokenBurn"),
    tokenDestroyFrozenFunds: dispatchOk("tokenDestroyFrozenFunds"),
    tokenFreeze: dispatchOk("tokenFreeze"),
    tokenUnfreeze: dispatchOk("tokenUnfreeze"),
    tokenPause: dispatchOk("tokenPause"),
    tokenResume: dispatchOk("tokenResume"),
    tokenClaim: dispatchOk("tokenClaim"),
    tokenEstimatePerpetualRewards: dispatchOk("tokenEstimatePerpetualRewards"),
    tokenQueryClaims: dispatchOk("tokenQueryClaims"),
    tokenUpdateConfig: dispatchOk("tokenUpdateConfig"),
    tokenPurchase: dispatchOk("tokenPurchase"),
    tokenSetDirectPurchasePrice: dispatchOk("tokenSetDirectPurchasePrice"),
    tokenRegisterContract: dispatchOk("tokenRegisterContract"),
    tokenRemove: resolvesOk("tokenRemove", null),
    tokenLoadOrder: resolvesOk("tokenLoadOrder", []),
    tokenSaveOrder: resolvesOk("tokenSaveOrder", null),
    tokenGetMintingConfig: resolvesOk("tokenGetMintingConfig", {
      allowChoosingDestination: true,
      defaultDestinationIdentityId: null,
    }),

    // -- DashPay --
    dashpayLoadProfile: dispatchOk("dashpayLoadProfile"),
    dashpayUpdateProfile: dispatchOk("dashpayUpdateProfile"),
    dashpayLoadContacts: dispatchOk("dashpayLoadContacts"),
    dashpayLoadContactRequests: dispatchOk("dashpayLoadContactRequests"),
    dashpayFetchContactProfile: dispatchOk("dashpayFetchContactProfile"),
    dashpaySearchProfiles: dispatchOk("dashpaySearchProfiles"),
    dashpaySendContactRequest: dispatchOk("dashpaySendContactRequest"),
    dashpaySendContactRequestWithProof: dispatchOk("dashpaySendContactRequestWithProof"),
    dashpayAcceptContactRequest: dispatchOk("dashpayAcceptContactRequest"),
    dashpayRejectContactRequest: dispatchOk("dashpayRejectContactRequest"),
    dashpayLoadPaymentHistory: dispatchOk("dashpayLoadPaymentHistory"),
    dashpaySendPaymentToContact: dispatchOk("dashpaySendPaymentToContact"),
    dashpayUpdateContactInfo: dispatchOk("dashpayUpdateContactInfo"),
    dashpayRegisterAddresses: dispatchOk("dashpayRegisterAddresses"),
    dashpayDbLoadProfile: resolvesOk("dashpayDbLoadProfile", null),
    dashpayDbSaveProfile: resolvesOk("dashpayDbSaveProfile", null),
    dashpayDbLoadContacts: resolvesOk("dashpayDbLoadContacts", []),
    dashpayDbLoadPendingRequests: resolvesOk("dashpayDbLoadPendingRequests", []),
    dashpayDbLoadPayments: resolvesOk("dashpayDbLoadPayments", []),
    dashpayDbLoadContactPrivateInfo: resolvesOk("dashpayDbLoadContactPrivateInfo", {
      nickname: "",
      notes: "",
      isHidden: false,
    }),
    dashpayDbSaveContactPrivateInfo: resolvesOk("dashpayDbSaveContactPrivateInfo", null),
    dashpayDbSetContactHidden: resolvesOk("dashpayDbSetContactHidden", null),
    dashpayDbSaveAvatarBytes: resolvesOk("dashpayDbSaveAvatarBytes", null),

    // -- Contested / DPNS --
    contestedQueryDpnsContests: resolves("contestedQueryDpnsContests", { taskId: "mock-task-id" }),
    contestedVoteOnDpnsNames: dispatchOk("contestedVoteOnDpnsNames"),
    contestedScheduleDpnsVotes: dispatchOk("contestedScheduleDpnsVotes"),
    contestedCastScheduledVote: dispatchOk("contestedCastScheduledVote"),
    contestedClearAllScheduledVotes: resolves("contestedClearAllScheduledVotes", { taskId: "mock-task-id" }),
    contestedClearExecutedScheduledVotes: resolves("contestedClearExecutedScheduledVotes", { taskId: "mock-task-id" }),
    contestedDeleteScheduledVote: dispatchOk("contestedDeleteScheduledVote"),
    contestedGetScheduledVotes: resolvesOk("contestedGetScheduledVotes", []),

    // -- Parsers --
    parseDataContract: resolvesOk("parseDataContract", { json: "{}", id: "" }),
    parseDocument: resolvesOk("parseDocument", { json: "{}" }),
    parseGrovedbProof: resolvesOk("parseGrovedbProof", { text: "" }),

    // -- Platform Info --
    platformCurrentEpochInfo: resolves("platformCurrentEpochInfo", { taskId: "mock-task-id" }),
    platformTotalCredits: resolves("platformTotalCredits", { taskId: "mock-task-id" }),
    platformVersionVotingState: resolves("platformVersionVotingState", { taskId: "mock-task-id" }),
    platformValidatorSetInfo: resolves("platformValidatorSetInfo", { taskId: "mock-task-id" }),
    platformWithdrawalsInQueue: resolves("platformWithdrawalsInQueue", { taskId: "mock-task-id" }),
    platformRecentlyCompletedWithdrawals: resolves("platformRecentlyCompletedWithdrawals", { taskId: "mock-task-id" }),
    platformBasicInfo: resolves("platformBasicInfo", { taskId: "mock-task-id" }),
    platformFetchAddressBalance: resolves("platformFetchAddressBalance", { taskId: "mock-task-id" }),

    // -- System --
    systemWipePlatformData: resolves("systemWipePlatformData", { taskId: "mock-task-id" }),
    systemUpdateTheme: resolves("systemUpdateTheme", { taskId: "mock-task-id" }),

    // -- Masternode --
    mnlistFetchDiff: dispatchOk("mnlistFetchDiff"),
    mnlistFetchQrInfo: dispatchOk("mnlistFetchQrInfo"),
    mnlistFetchQrInfoWithDmls: dispatchOk("mnlistFetchQrInfoWithDmls"),
    mnlistFetchChainLocks: resolves("mnlistFetchChainLocks", { taskId: "mock-task-id" }),
    mnlistFetchDiffsChain: dispatchOk("mnlistFetchDiffsChain"),

    // -- GroveSTARK --
    grovestarkGenerateProof: dispatchOk("grovestarkGenerateProof"),
    grovestarkVerifyProof: dispatchOk("grovestarkVerifyProof"),

    // -- Broadcast --
    broadcastStateTransition: dispatchOk("broadcastStateTransition"),

    // -- Settings --
    settingsGet: resolvesOk("settingsGet", {
      theme: "Dark",
      developerMode: false,
      disableZmq: false,
      onboardingCompleted: false,
      showEvonodeTools: false,
      userMode: "Basic",
      closeDashQtOnExit: false,
      autoStartSpv: false,
    }),
    settingsUpdatePassword: resolvesOk("settingsUpdatePassword", null),
    settingsUpdateDashCore: resolvesOk("settingsUpdateDashCore", null),
    settingsUpdateDisableZmq: resolvesOk("settingsUpdateDisableZmq", null),
    settingsUpdateOnboardingCompleted: resolvesOk("settingsUpdateOnboardingCompleted", null),
    settingsUpdateShowEvonodeTools: resolvesOk("settingsUpdateShowEvonodeTools", null),
    settingsUpdateUserMode: resolvesOk("settingsUpdateUserMode", null),
    settingsUpdateCloseDashQtOnExit: resolvesOk("settingsUpdateCloseDashQtOnExit", null),
    settingsUpdateAutoStartSpv: resolvesOk("settingsUpdateAutoStartSpv", null),
    settingsGetAutoStartSpv: resolvesOk("settingsGetAutoStartSpv", false),

    // -- Context --
    contextIsDeveloperMode: resolves("contextIsDeveloperMode", false),
    contextEnableDeveloperMode: resolves("contextEnableDeveloperMode", undefined),
    contextGetFeeMultiplier: resolves("contextGetFeeMultiplier", 1.0),
    contextSetFeeMultiplier: resolves("contextSetFeeMultiplier", undefined),
    contextGetNetwork: resolves("contextGetNetwork", "Testnet"),
    contextGetCoreBackendMode: resolves("contextGetCoreBackendMode", "Rpc"),
    contextSetCoreBackendMode: resolvesOk("contextSetCoreBackendMode", null),
  };
}

// ---------------------------------------------------------------------------
// Default event mocks
// ---------------------------------------------------------------------------

const ALL_EVENTS: EventName[] = [
  "scheduledVoteExecutedEvent",
  "spvStatusEvent",
  "taskErrorEvent",
  "taskResultEvent",
  "walletUpdatedEvent",
  "zmqChainLockedBlockEvent",
  "zmqConnectionStatusEvent",
  "zmqIsLockedTransactionEvent",
];

/**
 * Builds event mocks. Each event gets `listen`, `once`, and `emit` mocks.
 * `listen` and `once` return a resolved unsubscribe function.
 * Listeners are tracked so `emitMockEvent` can invoke them.
 */
function buildDefaultEvents(): {
  events: Record<EventName, { listen: Mock; once: Mock; emit: Mock }>;
  listeners: Map<string, Array<(event: { payload: unknown }) => void>>;
} {
  const listeners = new Map<string, Array<(event: { payload: unknown }) => void>>();

  const events = {} as Record<EventName, { listen: Mock; once: Mock; emit: Mock }>;

  for (const name of ALL_EVENTS) {
    listeners.set(name, []);

    events[name] = {
      listen: vi.fn((cb: (event: { payload: unknown }) => void) => {
        const arr = listeners.get(name)!;
        arr.push(cb);
        // Return unsubscribe function
        return Promise.resolve(() => {
          const idx = arr.indexOf(cb);
          if (idx >= 0) arr.splice(idx, 1);
        });
      }),
      once: vi.fn((cb: (event: { payload: unknown }) => void) => {
        const arr = listeners.get(name)!;
        const wrapper = (event: { payload: unknown }) => {
          cb(event);
          const idx = arr.indexOf(wrapper);
          if (idx >= 0) arr.splice(idx, 1);
        };
        arr.push(wrapper);
        return Promise.resolve(() => {
          const idx = arr.indexOf(wrapper);
          if (idx >= 0) arr.splice(idx, 1);
        });
      }),
      emit: vi.fn(),
    };
  }

  return { events, listeners };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface MockBindingsResult {
  /** Mock `commands` object — drop-in replacement for `commands` from bindings.ts. */
  commands: Record<CommandName, Mock>;
  /** Mock `events` object — drop-in replacement for `events` from bindings.ts. */
  events: Record<EventName, { listen: Mock; once: Mock; emit: Mock }>;
  /** Call history per command: `history.get("walletListAll")` → array of args. */
  callHistory: CallHistory;
  /** Event listeners registered via `events.*.listen()`. */
  eventListeners: Map<string, Array<(event: { payload: unknown }) => void>>;
  /** Override a specific command's mock implementation. */
  configureMock: (name: CommandName, handler: Mock) => void;
  /** Reset all mocks to defaults and clear call history. */
  resetMocks: () => void;
  /** Emit a mock event to all registered listeners. */
  emitMockEvent: (name: EventName, payload: unknown) => void;
}

/**
 * Creates a complete set of mock bindings for Tauri IPC commands and events.
 *
 * @param commandOverrides - Optional overrides for specific commands.
 *   Each override replaces the default handler for that command.
 * @param eventOverrides - Optional overrides for specific events.
 *
 * @returns An object with `commands`, `events`, call tracking, and utilities.
 *
 * @example
 * ```ts
 * const mocks = createMockBindings({
 *   walletListAll: vi.fn().mockResolvedValue({
 *     status: "ok",
 *     data: { hdWallets: [wallet], singleKeyWallets: [], selected: null },
 *   }),
 * });
 *
 * vi.mock("@/bindings", () => ({
 *   commands: mocks.commands,
 *   events: mocks.events,
 * }));
 * ```
 */
export function createMockBindings(
  commandOverrides?: MockCommands,
  eventOverrides?: MockEvents,
): MockBindingsResult {
  const callHistory: CallHistory = new Map();
  const defaultCommands = buildDefaultCommands(callHistory);
  const { events: defaultEvents, listeners } = buildDefaultEvents();

  // Apply command overrides
  if (commandOverrides) {
    for (const [name, handler] of Object.entries(commandOverrides)) {
      if (handler) {
        defaultCommands[name as CommandName] = handler;
      }
    }
  }

  // Apply event overrides
  if (eventOverrides) {
    for (const [name, overrides] of Object.entries(eventOverrides)) {
      if (overrides) {
        const eventName = name as EventName;
        if (overrides.listen) defaultEvents[eventName].listen = overrides.listen;
        if (overrides.once) defaultEvents[eventName].once = overrides.once;
        if (overrides.emit) defaultEvents[eventName].emit = overrides.emit;
      }
    }
  }

  return {
    commands: defaultCommands,
    events: defaultEvents,
    callHistory,
    eventListeners: listeners,

    configureMock(name: CommandName, handler: Mock) {
      defaultCommands[name] = handler;
    },

    resetMocks() {
      callHistory.clear();
      for (const mock of Object.values(defaultCommands)) {
        mock.mockClear();
      }
      for (const eventMocks of Object.values(defaultEvents)) {
        eventMocks.listen.mockClear();
        eventMocks.once.mockClear();
        eventMocks.emit.mockClear();
      }
      for (const arr of listeners.values()) {
        arr.length = 0;
      }
    },

    emitMockEvent(name: EventName, payload: unknown) {
      const cbs = listeners.get(name) ?? [];
      for (const cb of [...cbs]) {
        cb({ payload });
      }
    },
  };
}

/**
 * Convenience: returns a `vi.mock("@/bindings")` factory function.
 *
 * Usage:
 * ```ts
 * const mocks = createMockBindings({ walletListAll: myHandler });
 * vi.mock("@/bindings", () => mockBindingsModule(mocks));
 * ```
 */
export function mockBindingsModule(result: MockBindingsResult) {
  return {
    commands: result.commands,
    events: result.events,
  };
}
