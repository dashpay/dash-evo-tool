use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::dashpay::{DashPayTask, ContactData};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::platform_info::{PlatformInfoTaskRequestType, PlatformInfoTaskResult};
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::context::identity_load_registry::IdentityLoadToken;
use crate::model::masternode_input::decode_identity_id;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::{PlatformAddressUpdates, WalletSeedHash};
use crate::model::grovestark_prover::ProofDataOutput;
use crate::ui::tokens::tokens_screen::{
    ContractDescriptionInfo, IdentityTokenIdentifier, TokenInfo,
};
use crate::utils::egui_mpsc::SenderAsync;
use crate::utils::tasks::TaskManager;
use contested_names::ScheduledDPNSVote;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::evaluate_interval::IntervalEvaluationExplanation;
use dash_sdk::dpp::group::group_action::GroupAction;
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, DocumentQuery, Identifier};
use dash_sdk::query_types::{Documents, IndexMap};
use futures::future::join_all;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use migration::MigrationTask;
use shielded::ShieldedTask;
use tokens::TokenTask;
use grovestark::GroveSTARKTask;

pub mod broadcast_state_transition;
pub mod contested_names;
pub mod contract;
pub mod core;
pub mod dapi_discovery;
pub mod dashpay;
pub mod document;
pub mod error;
pub mod grovestark;
pub mod identity;
pub mod migration;
pub mod platform_info;
pub mod register_contract;
pub mod shielded;
pub mod system_task;
pub mod tokens;
pub mod update_data_contract;
pub mod wallet;

pub(crate) const NETWORK_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
static BACKEND_TASK_DISPATCH_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) async fn await_network_request_with_timeout<T>(
    timeout_duration: Duration,
    request: impl Future<Output = T>,
    timeout_error: impl FnOnce(tokio::time::error::Elapsed) -> TaskError,
) -> Result<T, TaskError> {
    tokio::time::timeout(timeout_duration, request)
        .await
        .map_err(timeout_error)
}

pub(crate) async fn await_managed_network_request_with_timeout<T>(
    task_manager: Arc<TaskManager>,
    reaper_name: &'static str,
    timeout_duration: Duration,
    request: impl Future<Output = T> + Send + 'static,
    timeout_error: impl FnOnce(tokio::time::error::Elapsed) -> TaskError,
) -> Result<T, TaskError>
where
    T: Send + 'static,
{
    let mut task = tokio::spawn(request);
    match tokio::time::timeout(timeout_duration, &mut task).await {
        Ok(result) => result.map_err(|source| TaskError::BackendTaskFailed {
            source: source.into(),
        }),
        Err(source) => {
            task_manager.spawn_sync(reaper_name, async move {
                if let Err(source) = task.await {
                    let error = crate::backend_task::error::BackendTaskJoinError::from(source);
                    tracing::error!(?error, "Timed-out background request stopped unexpectedly");
                }
            });
            Err(timeout_error(source))
        }
    }
}

/// Returns `true` for backend tasks that read or write the
/// `WalletBackend` (and therefore the upstream `SecretStore` / sidecar
/// k/v). These tasks must short-circuit with
/// [`TaskError::WalletStorageNotReady`] while the cold-start migration
/// (`FinishUnwire`) is still in progress so the user sees the "data is
/// still being updated" banner instead of a misleading SDK timeout.
///
/// The list mirrors the family check above the `match` in
/// `run_backend_task`: identity / DashPay / Core / wallet / shielded
/// all funnel through `WalletBackend`. The migration task itself is
/// explicitly excluded — that is the work in progress.
fn is_wallet_touching(task: &BackendTask) -> bool {
    matches!(
        task,
        BackendTask::WalletTask(_)
            | BackendTask::CoreTask(_)
            | BackendTask::IdentityTask(_)
            | BackendTask::DashPayTask(_)
            | BackendTask::ShieldedTask(_)
    )
}

/// The contact-request ID a wallet-touching `DashPayTask` acts on, if it is one
/// of the three paid contact actions the Identity Hub guards while in flight.
///
/// The migration gate uses this to reject such an action with
/// [`TaskError::DashPayContactRequestActionFailed`] so the Hub releases only that
/// request's in-flight guard; every other task names no request and keeps the
/// bare [`TaskError::WalletStorageNotReady`].
pub(crate) fn dashpay_request_id(task: &BackendTask) -> Option<Identifier> {
    let BackendTask::DashPayTask(task) = task else {
        return None;
    };
    match task.as_ref() {
        DashPayTask::AcceptContactRequest { request_id, .. }
        | DashPayTask::RejectContactRequest { request_id, .. }
        | DashPayTask::CancelContactRequest { request_id, .. } => Some(*request_id),
        _ => None,
    }
}

/// The identity-load record `task` was dispatched under, when its caller marked
/// the load `Submitted` and is gating on it. `None` for every other task, and for
/// a load whose caller gates on nothing.
///
/// The id is parsed exactly as `load_identity` parses it, so the record named here
/// is the one that load would claim.
fn identity_load_ticket(task: &BackendTask) -> Option<(Identifier, IdentityLoadToken)> {
    let BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)) = task else {
        return None;
    };
    let token = input.load_token?;
    let identity_id = decode_identity_id(&input.identity_id_input).ok()?;
    Some((identity_id, token))
}

/// Whether a wallet-backend build error is terminal (storage written by a
/// newer/incompatible app build). These must surface their actionable
/// message instead of being logged-and-discarded as a transient deferral
/// (F50); every other init error is retried by the cold-boot bridge.
fn is_terminal_storage_open_error(error: &TaskError) -> bool {
    matches!(
        error,
        TaskError::WalletDataTooNew { .. } | TaskError::WalletDataIncompatible { .. }
    )
}

/// Information about fees for a platform state transition.
#[derive(Debug, Clone, PartialEq)]
pub struct FeeResult {
    /// The fee that was estimated before the operation.
    pub estimated_fee: u64,
    /// The actual fee paid, in credits. `None` when the operation only knows
    /// the estimate (the platform did not report a settled fee).
    pub actual_fee: Option<u64>,
}

impl FeeResult {
    /// Records both an estimate and the settled actual fee.
    pub fn new(estimated_fee: u64, actual_fee: u64) -> Self {
        Self {
            estimated_fee,
            actual_fee: Some(actual_fee),
        }
    }

    /// Records only the estimate; no actual fee is known.
    pub fn estimated_only(estimated_fee: u64) -> Self {
        Self {
            estimated_fee,
            actual_fee: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendTask {
    IdentityTask(IdentityTask),
    DocumentTask(Box<DocumentTask>),
    ContractTask(Box<ContractTask>),
    ContestedResourceTask(ContestedResourceTask),
    CoreTask(CoreTask),
    DashPayTask(Box<DashPayTask>),
    BroadcastStateTransition(StateTransition),
    TokenTask(Box<TokenTask>),
    SystemTask(SystemTask),
    PlatformInfo(PlatformInfoTaskRequestType),
    GroveSTARKTask(GroveSTARKTask),
    WalletTask(WalletTask),
    ShieldedTask(ShieldedTask),
    /// Cold-start data-migration orchestrator. Drains legacy `data.db`
    /// rows into the upstream wallet storage; idempotent across launches.
    MigrationTask(MigrationTask),
    /// Rebuild the Core RPC client and SDK on the current network context.
    /// Dispatched when the user saves a new RPC password so the reinit
    /// (which includes DAPI discovery) runs off the UI thread.
    ReinitCoreClientAndSdk,
    /// Create a new network context and switch to it.
    /// Dispatched to `AppContext::run_backend_task`, which creates the new `AppContext`
    /// and optionally starts SPV sync when `start_spv` is true.
    SwitchNetwork {
        network: Network,
        start_spv: bool,
    },
    /// Discover DAPI nodes from the DCG-operated HTTPS service.
    DiscoverDapiNodes {
        network: Network,
    },
    None,
}

/// Identifies the operation that produced a backend-task error without retaining
/// the complete [`BackendTask`] in the UI result channel. Document operations
/// retain their query because the visible screen needs it for exact matching.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendTaskContext {
    /// One concrete dispatch of an operation that may otherwise be identical to
    /// another in-flight task.
    Dispatched {
        dispatch_id: u64,
        operation: Box<BackendTaskContext>,
    },
    /// A complete document query.
    FetchDocuments(Box<DocumentQuery>),
    /// A paginated document query.
    FetchDocumentsPage(Box<DocumentQuery>),
    /// A refresh of all tracked token balances.
    TokenBalanceRefresh,
    /// A perpetual-reward estimate for one identity-token pair.
    TokenRewardEstimate(IdentityTokenIdentifier),
    /// The destructive per-network database clear.
    ClearNetworkDatabase,
    /// A known backend task that needs no finer UI correlation.
    Other,
    /// An error emitted without an originating backend task.
    Unknown,
}

impl BackendTaskContext {
    pub(crate) fn for_dispatch(task: &BackendTask) -> Self {
        Self::Dispatched {
            dispatch_id: BACKEND_TASK_DISPATCH_ID.fetch_add(1, Ordering::Relaxed),
            operation: Box::new(Self::from(task)),
        }
    }

    fn operation(&self) -> &Self {
        match self {
            Self::Dispatched { operation, .. } => operation,
            operation => operation,
        }
    }

    pub(crate) fn is_fetch_documents(&self) -> bool {
        matches!(self.operation(), Self::FetchDocuments(_))
    }

    pub(crate) fn is_fetch_documents_page(&self) -> bool {
        matches!(self.operation(), Self::FetchDocumentsPage(_))
    }

    pub(crate) fn dispatched_document_fetch(&self) -> bool {
        matches!(
            self,
            Self::Dispatched { operation, .. }
                if matches!(
                    operation.as_ref(),
                    Self::FetchDocuments(_) | Self::FetchDocumentsPage(_)
                )
        )
    }
}

impl From<&BackendTask> for BackendTaskContext {
    fn from(task: &BackendTask) -> Self {
        match task {
            BackendTask::DocumentTask(task) => match task.as_ref() {
                DocumentTask::FetchDocuments(query) => {
                    Self::FetchDocuments(Box::new(query.clone()))
                }
                DocumentTask::FetchDocumentsPage(query) => {
                    Self::FetchDocumentsPage(Box::new(query.clone()))
                }
                _ => Self::Other,
            },
            BackendTask::TokenTask(task)
                if matches!(task.as_ref(), TokenTask::QueryMyTokenBalances) =>
            {
                Self::TokenBalanceRefresh
            }
            BackendTask::TokenTask(task) => match task.as_ref() {
                TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
                    identity_id,
                    token_id,
                } => Self::TokenRewardEstimate(IdentityTokenIdentifier {
                    identity_id: *identity_id,
                    token_id: *token_id,
                }),
                _ => Self::Other,
            },
            BackendTask::SystemTask(SystemTask::ClearNetworkDatabase) => Self::ClearNetworkDatabase,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BackendTaskSuccessResult {
    // General results
    None,
    Refresh,
    NetworkDatabaseCleared {
        network: Network,
    },
    Message(String), // Used for: placeholder messages for
    // not-yet-implemented functionality, and DashPay operations that would need their own typed variants.
    /// Progress updates during long-running operations (e.g. batch identity search).
    /// By convention, the app routes `Progress` results to the visible screen's
    /// `display_task_result` without creating a global banner at the app level
    /// (unlike `Message`). Screens may create a banner handle on first receipt
    /// and update it in-place on subsequent updates to avoid stacking.
    Progress {
        /// Human-readable progress message
        message: String,
        /// Current step (1-based)
        current: u32,
        /// Total steps
        total: u32,
    },
    WalletPayment {
        txid: String,
        /// List of (address, amount) pairs for each recipient
        recipients: Vec<(String, u64)>,
        total_amount: u64,
    },

    // Specific results
    Documents(Documents),
    BroadcastedDocument(Document),
    CoreItem(CoreItem),
    RegisteredIdentity(QualifiedIdentity, FeeResult),
    ToppedUpIdentity(QualifiedIdentity, FeeResult),
    DPNSVoteResults(Vec<(String, ResourceVoteChoice, Result<(), Arc<TaskError>>)>),
    CastScheduledVote(ScheduledDPNSVote),
    /// A scheduled-vote sweep finished without a query, identity or Platform
    /// failure. The app uses this acknowledgement to retire a preserved
    /// migration eligibility cutoff only after the recovery attempt succeeds.
    ScheduledVoteSweepCompleted {
        network: Network,
        preserve_eligibility_since_ms: Option<u64>,
    },
    /// The scheduled votes that the `CastDueScheduledVotes` sweep is about to
    /// cast this cycle, so the Scheduled Votes screen can mark them in progress.
    ScheduledVotesInProgress(Vec<ScheduledDPNSVote>),
    FetchedContract(DataContract),
    FetchedContractWithTokenPosition(
        DataContract,
        dash_sdk::dpp::data_contract::TokenContractPosition,
    ),
    FetchedContracts(Vec<Option<DataContract>>),
    PageDocuments(IndexMap<Identifier, Option<Document>>, Option<Start>),
    DescriptionsByKeyword(Vec<ContractDescriptionInfo>, Option<Start>),
    TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(
        IdentityTokenIdentifier,
        TokenAmount,
        IntervalEvaluationExplanation,
    ),
    ContractsWithDescriptions(
        BTreeMap<Identifier, (Option<ContractDescriptionInfo>, Vec<TokenInfo>)>,
    ),
    ActiveGroupActions(IndexMap<Identifier, GroupAction>),
    TokenPricing {
        token_id: Identifier,
        prices: Option<TokenPricingSchedule>,
    },
    UpdatedThemePreference(crate::ui::theme::ThemeMode),
    PlatformInfo(PlatformInfoTaskResult),

    // DashPay related results
    DashPayProfile(Option<(String, String, String)>), // (display_name, bio, avatar_url)
    DashPayContactProfile(Option<Document>),          // Contact's public profile document
    DashPayProfileSearchResults(Vec<(Identifier, Option<Document>, String)>), // Search results: (identity_id, profile_document, username)
    DashPayContactRequests {
        /// The identity the requests were loaded for. An identity switch cannot
        /// cancel an in-flight load, so the consumer must drop a result whose
        /// identity is no longer the selected one.
        identity: Identifier,
        incoming: Vec<(Identifier, Document)>, // (request_id, document)
        outgoing: Vec<(Identifier, Document)>, // (request_id, document)
    },
    DashPayContacts(Vec<Identifier>), // List of contact identity IDs
    DashPayContactsWithInfo {
        /// The identity the contacts were loaded for — see
        /// [`BackendTaskSuccessResult::DashPayContactRequests`].
        identity: Identifier,
        contacts: Vec<ContactData>,
    },
    DashPayPaymentHistory(Vec<(String, String, u64, bool, String)>), // (tx_id, contact_name, amount, is_incoming, memo)
    DashPayProfileUpdated(Identifier), // Identity ID of updated profile
    DashPayContactRequestSent(String), // Username or ID of recipient
    DashPayContactRequestAccepted(Identifier), // Request ID that was accepted
    DashPayContactRequestRejected(Identifier), // Request ID that was rejected
    DashPayContactRequestCancelled(Identifier), // Request ID whose sent request was withdrawn
    DashPayContactAlreadyEstablished {
        request_id: Identifier,
        contact_id: Identifier,
    },
    DashPayContactInfoUpdated {
        /// Identity that owns the encrypted `contactInfo` document.
        identity: Identifier,
        /// Contact whose private details were updated.
        contact_id: Identifier,
    },
    DashPayPaymentSent(String, String, u64), // (recipient, address, amount in duffs)
    /// Result of a [`FetchAvatar`](crate::backend_task::dashpay::DashPayTask::FetchAvatar):
    /// the validated image bytes for `url`, or `None` when the fetch failed. Routed
    /// into the screen's avatar fetch cache keyed by `url`.
    DashPayAvatar {
        url: String,
        bytes: Option<Vec<u8>>,
    },
    /// Received outputs the `EventBridge` saw on a freshly-detected wallet
    /// transaction. The app dispatches `DetectIncomingContactPayments` for
    /// these — the backend resolves each against the per-identity DashPay
    /// address map and records the ones that match a contact. Non-DashPay
    /// outputs are silently ignored, so this carries every received output.
    DashPayIncomingDetected(Vec<crate::model::dashpay::DetectedIncomingOutput>),
    /// Platform became reachable (masternode list `Synced`), so the wallet
    /// backend asked the frame loop to start the automatic all-wallets identity
    /// discovery sweep. Emitted once per SPV session from the `CoordinatorGate`
    /// fire; the app responds by calling
    /// [`queue_all_wallets_identity_discovery`](crate::context::AppContext::queue_all_wallets_identity_discovery).
    PlatformReadyDiscoverIdentities,
    /// Auto-accept contact QR payload, ready to render. The proof is built
    /// through the JIT chokepoint in the backend so the UI never touches a seed.
    DashPayAutoAcceptQrCode(String),
    GeneratedZKProof(ProofDataOutput),
    VerifiedZKProof(bool, ProofDataOutput),
    GeneratedReceiveAddress {
        seed_hash: WalletSeedHash,
        address: String,
    },
    /// The wallet's tracked asset locks, read off the UI thread through the
    /// upstream `AssetLockManager`. Carries the `seed_hash` so screens cache
    /// and match the result per wallet.
    TrackedAssetLocks {
        seed_hash: WalletSeedHash,
        locks: Vec<platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock>,
    },
    /// Platform address balances fetched from Platform
    PlatformAddressBalances {
        seed_hash: WalletSeedHash,
        /// Map of platform address to (balance, nonce)
        balances: BTreeMap<PlatformAddress, (u64, u32)>,
        /// Network the balances were fetched from
        network: Network,
    },
    /// Pushed by the coordinator after each automatic platform-address sync pass.
    ///
    /// Carries per-wallet per-address funds as raw 20-byte P2PKH hashes so
    /// `EventBridge` (which does not know the network) can populate the result
    /// without coupling to the wallet model. `AppState` routes this to
    /// `AppContext::apply_platform_address_push` which converts the hashes to
    /// `dashcore::Address` and writes them into each wallet's
    /// `platform_address_info`, keeping the per-address tab current
    /// without a manual Refresh.
    PlatformAddressSyncPushed {
        /// Per-wallet owned-address balance updates.
        /// See [`PlatformAddressUpdates`] for the entry layout.
        updates: PlatformAddressUpdates,
    },
    /// Platform credits transferred between addresses
    PlatformCreditsTransferred {
        seed_hash: WalletSeedHash,
    },
    /// Platform address funded from asset lock
    PlatformAddressFunded {
        seed_hash: WalletSeedHash,
    },
    /// Withdrawal from Platform address to Core initiated
    PlatformAddressWithdrawal {
        seed_hash: WalletSeedHash,
    },
    /// A private key derived for on-screen display/export, wrapped in
    /// [`Secret`](crate::model::secret::Secret) end-to-end. The seed never
    /// leaves the backend; only the requested WIF crosses to the UI, which
    /// already shows it on screen (same trust boundary).
    WalletKeyForDisplay {
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
        /// The derived private key as a WIF string, zeroize-on-drop.
        wif: crate::model::secret::Secret,
    },
    /// A fresh Platform (DIP-17/18) receive address generated via the JIT
    /// chokepoint. The seed never leaves the backend; only the Bech32m-encoded
    /// address string crosses to the UI.
    GeneratedPlatformReceiveAddress {
        seed_hash: WalletSeedHash,
        /// The Bech32m-encoded Platform address (DIP-18).
        address: String,
    },
    /// The identity-authentication public-key cache was warmed for
    /// `identity_index` (a completion signal — carries no key material). The
    /// chooser re-reads the now-warm cache on the next frame.
    IdentityAuthPubkeysWarmed {
        identity_index: u32,
    },
    /// A message signed with a wallet-derived key via the JIT chokepoint. Only
    /// the public Base64 signature crosses to the UI — the seed and the derived
    /// private key never leave the backend.
    WalletMessageSigned {
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
        /// The Base64-encoded signature (a public artifact, not a secret).
        signature: String,
    },
    /// An identity private key derived for on-screen display/export, fetched
    /// JIT from the vault (`InVault`). The key bytes never become resident;
    /// only the WIF (zeroize-on-drop) crosses to the UI.
    IdentityKeyForDisplay {
        identity_id: dash_sdk::platform::Identifier,
        target: crate::model::qualified_identity::PrivateKeyTarget,
        key_id: dash_sdk::dpp::identity::KeyID,
        /// The identity private key as a WIF string, zeroize-on-drop.
        wif: crate::model::secret::Secret,
    },
    /// A message signed with a vault-backed identity key via the JIT
    /// chokepoint. Only the public Base64 signature crosses to the UI.
    IdentityMessageSigned {
        identity_id: dash_sdk::platform::Identifier,
        target: crate::model::qualified_identity::PrivateKeyTarget,
        key_id: dash_sdk::dpp::identity::KeyID,
        /// The Base64-encoded signature (a public artifact, not a secret).
        signature: String,
    },

    // Token operation results (replacing string messages)
    PausedTokens(FeeResult),
    ResumedTokens(FeeResult),
    MintedTokens(FeeResult),
    BurnedTokens(FeeResult),
    FrozeTokens(FeeResult),
    UnfrozeTokens(FeeResult),
    TransferredTokens(FeeResult),
    PurchasedTokens(FeeResult),
    SetTokenPrice(FeeResult),
    DestroyedFrozenFunds(FeeResult),
    ClaimedTokens(FeeResult),
    UpdatedTokenConfig(String, FeeResult), // The config item that was updated
    FetchedTokenBalances,
    SavedToken,

    // Identity operation results (replacing string messages)
    AddedKeyToIdentity(FeeResult),
    TransferredCredits(FeeResult),
    WithdrewFromIdentity(FeeResult),
    RegisteredDpnsName(FeeResult),
    RefreshedIdentity(QualifiedIdentity),
    LoadedIdentity(QualifiedIdentity),
    /// This identity's keys were sealed under a password (opt-in).
    /// The `count` keys newly sealed (0 when the task was an idempotent re-run
    /// over an already-protected identity).
    IdentityKeysProtected {
        /// The identity whose keys are now password-protected.
        identity_id: Identifier,
        /// How many keys this run newly sealed Tier-2.
        count: usize,
    },
    /// This identity's key protection was removed (opt-out); signing
    /// is prompt-free again.
    IdentityKeysUnprotected {
        /// The identity whose key protection was removed.
        identity_id: Identifier,
    },

    // Document operation results (replacing string messages)
    DeletedDocument(Identifier, FeeResult),
    ReplacedDocument(Identifier, FeeResult),
    TransferredDocument(Identifier, FeeResult),
    PurchasedDocument(Identifier, FeeResult),
    SetDocumentPrice(Identifier, FeeResult),

    // Contract operation results (replacing string messages)
    UpdatedContract(FeeResult),
    RemovedContract,
    FetchedNonce,
    RegisteredContract(FeeResult),
    RegisteredTokenContract,
    SavedContract,
    ContractNotFound,
    TokenNotFound,
    ProofErrorLogged,
    /// Contract was saved to the local database despite a proof verification error.
    /// Sent by `register_data_contract` / `update_data_contract` when the contract was
    /// successfully fetched from Platform and stored after a `DriveProofError`.
    ContractSavedAfterProofError,

    // Wallet operation results (replacing string messages)
    RefreshedWallet {
        /// Set when Core refresh succeeded but the Platform balance sync
        /// failed; carries the typed error for the banner's details panel.
        warning: Option<Arc<TaskError>>,
    },

    // DPNS operation results (replacing string messages)
    ScheduledVotes,
    RefreshedDpnsContests,
    RefreshedOwnedDpnsNames,

    // Broadcast results
    BroadcastedStateTransition,

    // Mining results (dev mode, Regtest/Devnet only)
    MineBlocksSuccess(u64),

    // Shielded pool results (one per surviving op; sync/init/nullifier-scan
    // are owned by the upstream coordinator now and carry no DET result).
    ShieldedCreditsShielded {
        seed_hash: WalletSeedHash,
        amount: u64,
    },
    ShieldedTransferComplete {
        seed_hash: WalletSeedHash,
        amount: u64,
    },
    ShieldedCreditsUnshielded {
        seed_hash: WalletSeedHash,
        amount: u64,
    },
    ShieldedFromAssetLock {
        seed_hash: WalletSeedHash,
        amount: u64,
    },
    ShieldedWithdrawalComplete {
        seed_hash: WalletSeedHash,
        amount: u64,
    },

    /// Core RPC client and SDK were successfully rebuilt (e.g. after password change).
    CoreClientReinitialized,

    /// A new network context was created asynchronously during a network switch.
    NetworkContextCreated {
        network: Network,
        context: Arc<AppContext>,
        spv_started: bool,
    },

    /// Fresh DAPI node addresses discovered from the DCG service.
    DapiNodesDiscovered {
        network: Network,
        count: usize,
        addresses_csv: String,
    },

    /// An asset-lock funding transaction was broadcast successfully.
    AssetLockBroadcast {
        txid: String,
    },

    /// DashPay contact receiving addresses were registered for an identity.
    DashPayAddressesRegistered {
        addresses: usize,
        contacts: usize,
        errors: usize,
    },

    /// Identities were discovered and loaded from a wallet by index search.
    IdentitiesLoaded {
        count: u32,
    },
}

impl AppContext {
    /// Run backend tasks sequentially
    pub async fn run_backend_tasks_sequential(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: SenderAsync<TaskResult>,
    ) -> Vec<Result<BackendTaskSuccessResult, TaskError>> {
        let mut results = Vec::new();
        for task in tasks {
            results.push(self.run_backend_task(task, sender.clone()).await);
        }
        results
    }

    /// Run backend tasks concurrently
    pub async fn run_backend_tasks_concurrent(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: SenderAsync<TaskResult>,
    ) -> Vec<Result<BackendTaskSuccessResult, TaskError>> {
        let futures = tasks
            .into_iter()
            .map(|task| {
                let cloned_self = Arc::clone(self);
                let cloned_sender = sender.clone();
                async move { cloned_self.run_backend_task(task, cloned_sender).await }
            })
            .collect::<Vec<_>>();

        // Wait for all to finish before returning
        join_all(futures).await
    }

    pub async fn run_backend_task(
        self: &Arc<Self>,
        task: BackendTask,
        sender: SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Refuse a shielded fund movement while shielded operations are
        // unavailable BEFORE `ensure_wallet_backend` runs. That bootstrap does
        // real work for any wallet-touching task — building the backend and, for
        // every loaded wallet, materializing the HD seed, registering upstream,
        // and binding Orchard — none of which should happen for an op the app
        // will refuse anyway. `is_available` is side-effect-free (config read, no
        // lock/await/secret), so it is safe to call before backend init. The
        // in-handler gate in `run_shielded_task` stays as the authoritative check.
        if let BackendTask::ShieldedTask(_) = &task
            && !FeatureGate::ShieldedOperations.is_available(self)
        {
            return Err(TaskError::ShieldedOperationsUnavailable);
        }

        let _contact_request_claim = match dashpay_request_id(&task) {
            Some(request_id) => match self.try_claim_contact_request_action(request_id) {
                Some(claim) => Some(claim),
                None => {
                    return Err(TaskError::DashPayContactRequestActionFailed {
                        request_id,
                        source: Box::new(TaskError::DashPayContactRequestActionInProgress),
                    });
                }
            },
            None => None,
        };

        // A dispatched identity load is recorded `Submitted` before this task
        // exists, and only the load's own claim closes that record out. Both gates
        // below return before the load ever reaches `load_identity`, so without a
        // backstop the record would stay outstanding for the rest of the session and
        // the screen gating on it stuck on "Loading…". Held for the whole task: it
        // covers every way a load can end before its claim, not just these two.
        let _load_dispatch = identity_load_ticket(&task)
            .map(|(identity_id, token)| self.backstop_identity_load(identity_id, token));

        let sdk = self.sdk.load().as_ref().clone();

        // Wallet/identity/DashPay/core/shielded flows go through
        // `WalletBackend`. Build it lazily on first such task (idempotent) —
        // this is where the `AppState`-owned `TaskResult` sender is available.
        // Every wallet-touching family wires the backend on first use, so
        // this mirrors `is_wallet_touching` directly.
        if is_wallet_touching(&task)
            && let Err(e) = self.ensure_wallet_backend(sender.clone()).await
        {
            // A storage-open failure (data written by a newer/incompatible app
            // build) is terminal — restarting won't help and the generic
            // "deferred" banner is misleading. Surface those variants so the
            // user sees the actionable message; every other init error is a
            // transient deferral the cold-boot bridge retries.
            if is_terminal_storage_open_error(&e) {
                return Err(e);
            }
            tracing::warn!(error = %e, "Wallet backend initialization deferred");
        }

        // Short-circuit wallet-touching tasks while the cold-start
        // migration is mid-flight. Reaching the SDK before the legacy
        // drain finishes either races on partially-mirrored sidecars
        // or produces a misleading SDK timeout. `WalletStorageNotReady`
        // is a typed, user-friendly variant whose banner mirrors the
        // migration banner ("data is still being updated").
        if is_wallet_touching(&task) && self.migration_status().state().is_in_progress() {
            tracing::debug!(
                target = "migration::gate",
                task = ?task,
                "Short-circuiting wallet-touching task — migration in progress",
            );
            // A guarded DashPay contact action carries its request ID so the
            // Identity Hub releases only that request's in-flight guard, not every
            // contact action's. Every other wallet-touching task names none and
            // keeps the bare variant. Both surface the same user-facing banner —
            // `DashPayContactRequestActionFailed` forwards its source's `Display`.
            return match dashpay_request_id(&task) {
                Some(request_id) => Err(TaskError::DashPayContactRequestActionFailed {
                    request_id,
                    source: Box::new(TaskError::WalletStorageNotReady),
                }),
                None => Err(TaskError::WalletStorageNotReady),
            };
        }

        match task {
            BackendTask::ContractTask(contract_task) => {
                Ok(self.run_contract_task(*contract_task, &sdk, sender).await?)
            }
            BackendTask::ContestedResourceTask(contested_resource_task) => Ok(self
                .run_contested_resource_task(contested_resource_task, &sdk, sender)
                .await?),
            BackendTask::IdentityTask(identity_task) => {
                Ok(self.run_identity_task(identity_task, &sdk, sender).await?)
            }
            BackendTask::DocumentTask(document_task) => {
                Ok(self.run_document_task(*document_task, &sdk).await?)
            }
            BackendTask::CoreTask(core_task) => Ok(self.run_core_task(core_task).await?),
            BackendTask::DashPayTask(dashpay_task) => {
                Ok(self.run_dashpay_task(*dashpay_task, &sdk).await?)
            }
            BackendTask::BroadcastStateTransition(state_transition) => Ok(self
                .broadcast_state_transition(state_transition, &sdk)
                .await?),
            BackendTask::TokenTask(token_task) => {
                Ok(self.run_token_task(*token_task, &sdk, sender).await?)
            }
            BackendTask::SystemTask(system_task) => {
                Ok(self.run_system_task(system_task, sender).await?)
            }
            BackendTask::PlatformInfo(platform_info_task) => Ok(self
                .run_platform_info_task(platform_info_task, &sdk)
                .await?),
            BackendTask::GroveSTARKTask(grovestark_task) => {
                Ok(grovestark::run_grovestark_task(grovestark_task, &sdk).await?)
            }
            BackendTask::WalletTask(wallet_task) => Ok(self.run_wallet_task(wallet_task).await?),
            BackendTask::ShieldedTask(shielded_task) => {
                Ok(self.run_shielded_task(shielded_task).await?)
            }
            BackendTask::MigrationTask(migration_task) => {
                Ok(self.run_migration_task(migration_task).await?)
            }
            BackendTask::ReinitCoreClientAndSdk => {
                Arc::clone(self).reinit_core_client_and_sdk()?;
                Ok(BackendTaskSuccessResult::CoreClientReinitialized)
            }
            BackendTask::SwitchNetwork { network, start_spv } => {
                // Create a new AppContext for the target network, reusing shared
                // resources (db, subtasks, connection_status) from the current context.
                // Wrapped in block_in_place because AppContext::new() does DB init
                // and file I/O which would block the async runtime.
                let data_dir = self.data_dir.clone();
                let db = self.db.clone();
                let subtasks = self.subtasks.clone();
                let connection_status = self.connection_status.clone();
                let egui_ctx = self.egui_ctx().clone();
                let app_kv = self.app_kv();
                let secret_store = self.secret_store();
                // Share the app-global role cell so the freshly-switched context
                // observes the same value (and live changes) as the rest of the
                // app — never a fresh per-context cell.
                let user_role = self.user_role_cell();
                let new_ctx = tokio::task::block_in_place(|| {
                    AppContext::new(
                        data_dir,
                        network,
                        db,
                        subtasks,
                        connection_status,
                        egui_ctx,
                        app_kv,
                        secret_store,
                        user_role,
                    )
                })
                .ok_or(TaskError::NetworkContextCreationFailed { network })?;

                // Wire the freshly-built context's wallet backend and then start
                // chain sync. The old code called `start_spv()` on an unwired
                // context, which fast-failed with `WalletBackendNotYetWired` and
                // reported `spv_started=false`. Wiring first removes that race so
                // `spv_started` reflects whether sync actually began.
                let spv_started = if start_spv && !self.subtasks.cancellation_token.is_cancelled() {
                    match new_ctx
                        .ensure_wallet_backend_and_start_spv(sender.clone())
                        .await
                    {
                        Ok(()) => {
                            tracing::info!(?network, "SPV started after network switch");
                            true
                        }
                        Err(e) => {
                            tracing::warn!(?network, "SPV start failed after network switch: {e}");
                            false
                        }
                    }
                } else {
                    false
                };
                if self.subtasks.cancellation_token.is_cancelled()
                    && let Ok(backend) = new_ctx.wallet_backend()
                {
                    backend.shutdown().await;
                }
                Ok(BackendTaskSuccessResult::NetworkContextCreated {
                    network,
                    context: new_ctx,
                    spv_started,
                })
            }
            BackendTask::DiscoverDapiNodes { network } => {
                let devnet_name = self
                    .config
                    .read()
                    .map_err(|_| TaskError::LockPoisoned {
                        resource: "NetworkConfig",
                    })?
                    .devnet_name
                    .clone();
                let (count, addresses_csv) =
                    dapi_discovery::discover_and_format(network, devnet_name.as_deref()).await?;
                Ok(BackendTaskSuccessResult::DapiNodesDiscovered {
                    network,
                    count,
                    addresses_csv,
                })
            }
            BackendTask::None => Ok(BackendTaskSuccessResult::None),
        }
    }

    async fn run_wallet_task(
        self: &Arc<Self>,
        task: WalletTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            WalletTask::GenerateReceiveAddress { seed_hash } => {
                self.generate_receive_address(seed_hash).await
            }
            WalletTask::DeriveKeyForDisplay {
                seed_hash,
                derivation_path,
            } => {
                self.derive_key_for_display(seed_hash, derivation_path)
                    .await
            }
            WalletTask::GeneratePlatformReceiveAddress { seed_hash } => {
                self.generate_platform_receive_address(seed_hash).await
            }
            WalletTask::WarmIdentityAuthPubkeys {
                seed_hash,
                identity_index,
                key_count,
            } => {
                self.warm_identity_auth_pubkeys(seed_hash, identity_index, key_count)
                    .await
            }
            WalletTask::SignMessageWithKey {
                seed_hash,
                derivation_path,
                message,
                key_type,
            } => {
                self.sign_message_with_key(seed_hash, derivation_path, message, key_type)
                    .await
            }
            WalletTask::DeriveIdentityKeyForDisplay {
                identity_id,
                target,
                key_id,
            } => {
                self.derive_identity_key_for_display(identity_id, target, key_id)
                    .await
            }
            WalletTask::SignMessageWithIdentityKey {
                identity_id,
                target,
                key_id,
                message,
                key_type,
            } => {
                self.sign_message_with_identity_key(identity_id, target, key_id, message, key_type)
                    .await
            }
            WalletTask::ListTrackedAssetLocks { seed_hash } => {
                let locks = self
                    .wallet_backend()?
                    .list_tracked_asset_locks(&seed_hash)
                    .await?;
                Ok(BackendTaskSuccessResult::TrackedAssetLocks { seed_hash, locks })
            }
            WalletTask::FetchPlatformAddressBalances { seed_hash } => {
                self.fetch_platform_address_balances(seed_hash).await
            }
            WalletTask::TransferPlatformCredits {
                seed_hash,
                inputs,
                outputs,
                fee_payer_index,
            } => {
                self.transfer_platform_credits(seed_hash, inputs, outputs, fee_payer_index)
                    .await
            }
            WalletTask::FundPlatformAddressFromAssetLock {
                seed_hash,
                out_point,
                outputs,
            } => {
                self.fund_platform_address_from_asset_lock(seed_hash, out_point, outputs)
                    .await
            }
            WalletTask::WithdrawFromPlatformAddress {
                seed_hash,
                inputs,
                output_script,
                core_fee_per_byte,
                fee_payer_index,
            } => {
                self.withdraw_from_platform_address(
                    seed_hash,
                    inputs,
                    output_script,
                    core_fee_per_byte,
                    fee_payer_index,
                )
                .await
            }
            WalletTask::FundPlatformAddressFromWalletUtxos {
                seed_hash,
                amount,
                destination,
                fee_deduct_from_output,
            } => {
                self.fund_platform_address_from_wallet_utxos(
                    seed_hash,
                    amount,
                    destination,
                    fee_deduct_from_output,
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_task_context_preserves_document_query_and_fetch_kind() {
        use dash_sdk::dpp::data_contracts::SystemDataContract;
        use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
        use dash_sdk::dpp::version::PlatformVersion;

        let contract =
            load_system_data_contract(SystemDataContract::DPNS, PlatformVersion::latest())
                .expect("DPNS contract");
        let query = DocumentQuery::new(Arc::new(contract), "domain").expect("domain query");

        let fetch =
            BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocuments(query.clone())));
        let page =
            BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocumentsPage(query.clone())));

        assert_eq!(
            BackendTaskContext::from(&fetch),
            BackendTaskContext::FetchDocuments(Box::new(query.clone()))
        );
        assert_eq!(
            BackendTaskContext::from(&page),
            BackendTaskContext::FetchDocumentsPage(Box::new(query))
        );
    }

    #[test]
    fn backend_task_context_preserves_reward_estimate_pair() {
        let identity_token_id = IdentityTokenIdentifier {
            identity_id: Identifier::from([1; 32]),
            token_id: Identifier::from([2; 32]),
        };
        let task = BackendTask::TokenTask(Box::new(
            TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
                identity_id: identity_token_id.identity_id,
                token_id: identity_token_id.token_id,
            },
        ));

        assert_eq!(
            BackendTaskContext::from(&task),
            BackendTaskContext::TokenRewardEstimate(identity_token_id)
        );
    }

    #[test]
    fn backend_task_context_identifies_network_database_clear() {
        let task = BackendTask::SystemTask(SystemTask::ClearNetworkDatabase);

        assert_eq!(
            BackendTaskContext::from(&task),
            BackendTaskContext::ClearNetworkDatabase
        );
    }

    /// `is_wallet_touching` covers every task family that funnels
    /// through `WalletBackend` — the gate in `run_backend_task` relies
    /// on it to short-circuit while the cold-start migration is
    /// in-flight. Locking the matrix here prevents the gate from
    /// silently letting a future task family race the mirror.
    #[test]
    fn wallet_touching_matrix_is_stable() {
        use crate::backend_task::core::CoreTask;
        use crate::backend_task::wallet::WalletTask;
        use dash_sdk::dpp::dashcore::Network;

        let seed_hash = crate::model::wallet::WalletSeedHash::default();

        // Wallet-touching families short-circuit on a running migration.
        assert!(is_wallet_touching(&BackendTask::WalletTask(
            WalletTask::GenerateReceiveAddress { seed_hash },
        )));
        assert!(is_wallet_touching(&BackendTask::CoreTask(
            CoreTask::GetBestChainLocks,
        )));
        assert!(is_wallet_touching(&BackendTask::ShieldedTask(
            shielded::ShieldedTask::ShieldFromBalance {
                seed_hash,
                amount: 1,
            },
        )));
        assert!(is_wallet_touching(&BackendTask::IdentityTask(
            identity::IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
        )));
        assert!(is_wallet_touching(&BackendTask::DashPayTask(Box::new(
            dashpay::DashPayTask::SearchProfiles {
                search_query: String::new(),
            },
        ))));

        // The migration task itself must NOT be gated — that is the
        // work that flips the gate back off.
        assert!(!is_wallet_touching(&BackendTask::MigrationTask(
            MigrationTask::FinishUnwire,
        )));

        // Read-only / network-level tasks are exempt.
        assert!(!is_wallet_touching(&BackendTask::ReinitCoreClientAndSdk));
        assert!(!is_wallet_touching(&BackendTask::SwitchNetwork {
            network: Network::Testnet,
            start_spv: false,
        }));
        assert!(!is_wallet_touching(&BackendTask::DiscoverDapiNodes {
            network: Network::Testnet,
        }));
    }

    /// Regression: a load short-circuited by one of the gates above never reaches
    /// `load_identity`, so it never takes the claim that reports its outcome. The
    /// dispatching screen has already recorded it `Submitted` — without the
    /// backstop that record would stay outstanding for the rest of the session, and
    /// the Masternodes "Load" gate stuck on "Loading…" for that node.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_submitted_load_recovers_when_its_task_never_starts() {
        use crate::backend_task::identity::{IdentityInputToLoad, IdentityLoadMode};
        use crate::context::identity_load_registry::IdentityLoadPhase;
        use crate::context::migration_status::{MigrationState, MigrationStep};
        use crate::context::test_support::test_app_context;
        use crate::model::qualified_identity::IdentityType;
        use crate::model::secret::Secret;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());

        let identity_id = Identifier::from([0x7c; 32]);
        let token = ctx
            .mark_identity_load_submitted(identity_id)
            .expect("submitted");

        ctx.migration_status().set_state(MigrationState::Running {
            step: MigrationStep::Identities,
        });

        let input = IdentityInputToLoad {
            identity_id_input: identity_id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(""),
            owner_private_key_input: Secret::new(""),
            payout_address_private_key_input: Secret::new(""),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::RejectIfExists,
            load_token: Some(token),
        };

        let result = ctx
            .run_backend_task(
                BackendTask::IdentityTask(IdentityTask::LoadIdentity(input)),
                sender,
            )
            .await;
        assert!(
            matches!(result, Err(TaskError::WalletStorageNotReady)),
            "the migration gate short-circuits a wallet-touching task, got {result:?}"
        );
        assert_eq!(
            ctx.identity_load_phase(&identity_id, token),
            Some(IdentityLoadPhase::Failed),
            "a load short-circuited before its task ran reports Failed, offering the user a retry"
        );

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }

    /// A shared MCP request dispatches through `run_backend_task`, so a wallet
    /// task must remain gated while migration is paused for a desktop password.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wallet_task_is_rejected_while_migration_awaits_password() {
        use crate::backend_task::wallet::WalletTask;
        use crate::context::migration_status::MigrationState;
        use crate::context::test_support::test_app_context;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        let seed_hash = [0x5a; 32];

        ctx.migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![seed_hash],
            });

        let result = ctx
            .run_backend_task(
                BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash }),
                sender,
            )
            .await;
        assert!(
            matches!(result, Err(TaskError::WalletStorageNotReady)),
            "an MCP-style wallet dispatch must stay gated during password collection, got {result:?}",
        );

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }

    /// The shielded pre-check runs before the migration gate, so an *unavailable*
    /// shielded write is refused with `ShieldedOperationsUnavailable` even while a
    /// storage update collects wallet passwords — the accurate, actionable message
    /// ("shielded is not available") rather than the misleading "wait for the
    /// update", since waiting will never make shielded available.
    ///
    /// The migration gate for shielded still applies once shielded operations
    /// ship (the pre-check passes, then the gate short-circuits); its
    /// `is_wallet_touching` membership is pinned by
    /// [`wallet_touching_matrix_is_stable`]. Sibling of
    /// [`wallet_task_is_rejected_while_migration_awaits_password`], which pins the
    /// migration gate for a task with no pre-check.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unavailable_shielded_write_is_refused_before_the_migration_gate() {
        use crate::backend_task::shielded::ShieldedTask;
        use crate::context::migration_status::MigrationState;
        use crate::context::test_support::test_app_context;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        assert!(!FeatureGate::ShieldedOperations.is_available(&ctx));
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        let seed_hash = [0x5a; 32];

        ctx.migration_status()
            .set_state(MigrationState::AwaitingWalletPasswords {
                wallets: vec![seed_hash],
            });

        let result = ctx
            .run_backend_task(
                BackendTask::ShieldedTask(ShieldedTask::ShieldFromBalance {
                    seed_hash,
                    amount: 100_000,
                }),
                sender,
            )
            .await;
        assert!(
            matches!(result, Err(TaskError::ShieldedOperationsUnavailable)),
            "the shielded pre-check must refuse an unavailable write before the migration gate, got {result:?}",
        );

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }

    fn qualified_identity(byte: u8) -> crate::model::qualified_identity::QualifiedIdentity {
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use std::collections::BTreeMap;

        let identity = Identity::create_basic_identity(
            Identifier::from([byte; 32]),
            PlatformVersion::latest(),
        )
        .expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// `dashpay_request_id` names a request only for the three guarded contact
    /// actions; every other DashPay task — and every non-DashPay task — names
    /// none, so the migration gate keeps its bare variant for them.
    #[test]
    fn dashpay_request_id_extracts_only_the_three_contact_actions() {
        use crate::backend_task::dashpay::DashPayTask;
        use crate::model::dashpay::UnreadableContactInfoPolicy;

        let request_id = Identifier::from([0x11; 32]);
        let dashpay = |t: DashPayTask| BackendTask::DashPayTask(Box::new(t));

        assert_eq!(
            dashpay_request_id(&dashpay(DashPayTask::AcceptContactRequest {
                identity: qualified_identity(1),
                request_id,
            })),
            Some(request_id)
        );
        assert_eq!(
            dashpay_request_id(&dashpay(DashPayTask::RejectContactRequest {
                identity: qualified_identity(1),
                request_id,
                unreadable: UnreadableContactInfoPolicy::Abort,
            })),
            Some(request_id)
        );
        assert_eq!(
            dashpay_request_id(&dashpay(DashPayTask::CancelContactRequest {
                identity: qualified_identity(1),
                request_id,
                unreadable: UnreadableContactInfoPolicy::Abort,
            })),
            Some(request_id)
        );
        assert_eq!(
            dashpay_request_id(&dashpay(DashPayTask::SearchProfiles {
                search_query: String::new(),
            })),
            None,
            "a non-contact-action DashPay task names no request"
        );
        assert_eq!(
            dashpay_request_id(&BackendTask::ReinitCoreClientAndSdk),
            None,
            "a non-DashPay task names no request"
        );
    }

    #[test]
    fn app_context_allows_only_one_backend_execution_per_contact_request() {
        use crate::context::test_support::test_app_context;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let request_id = Identifier::from([0x33; 32]);
        let first = ctx
            .try_claim_contact_request_action(request_id)
            .expect("first execution claims the request");

        assert!(ctx.contact_request_action_is_in_flight(&request_id));
        assert!(
            ctx.try_claim_contact_request_action(request_id).is_none(),
            "another UI surface must not start the same paid request action"
        );

        drop(first);
        assert!(!ctx.contact_request_action_is_in_flight(&request_id));
        assert!(ctx.try_claim_contact_request_action(request_id).is_some());
    }

    /// End-to-end: the migration gate wraps a rejected DashPay contact action in
    /// `DashPayContactRequestActionFailed` carrying its request ID (so the Hub
    /// un-sticks only that row), while a non-contact wallet-touching task keeps
    /// the bare `WalletStorageNotReady`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gate_rejection_wraps_a_contact_action_but_not_other_tasks() {
        use crate::backend_task::dashpay::DashPayTask;
        use crate::context::migration_status::{MigrationState, MigrationStep};
        use crate::context::test_support::test_app_context;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        ctx.migration_status().set_state(MigrationState::Running {
            step: MigrationStep::Identities,
        });

        let request_id = Identifier::from([0x22; 32]);

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let contact_result = ctx
            .run_backend_task(
                BackendTask::DashPayTask(Box::new(DashPayTask::AcceptContactRequest {
                    identity: qualified_identity(9),
                    request_id,
                })),
                SenderAsync::new(tx, ctx.egui_ctx().clone()),
            )
            .await;
        assert!(
            matches!(
                &contact_result,
                Err(TaskError::DashPayContactRequestActionFailed { request_id: r, source })
                    if *r == request_id
                        && matches!(source.as_ref(), TaskError::WalletStorageNotReady)
            ),
            "a gate-rejected contact action must name its request: {contact_result:?}"
        );

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let search_result = ctx
            .run_backend_task(
                BackendTask::DashPayTask(Box::new(DashPayTask::SearchProfiles {
                    search_query: String::new(),
                })),
                SenderAsync::new(tx, ctx.egui_ctx().clone()),
            )
            .await;
        assert!(
            matches!(&search_result, Err(TaskError::WalletStorageNotReady)),
            "a non-contact wallet-touching task stays bare: {search_result:?}"
        );

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }

    /// Only the storage-open variants (data from a newer/incompatible
    /// build) are terminal; every other init error is a transient deferral.
    #[test]
    fn terminal_storage_open_errors_are_classified() {
        assert!(is_terminal_storage_open_error(
            &TaskError::WalletDataTooNew {
                found: 99,
                max_supported: 1,
            }
        ));
        // A transient pre-wire state must NOT be treated as terminal.
        assert!(!is_terminal_storage_open_error(
            &TaskError::WalletBackendNotYetWired
        ));
        assert!(!is_terminal_storage_open_error(
            &TaskError::WalletStorageNotReady
        ));
    }
}
