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
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::dashcore::bls_sig_utils::BLSSignature;
use dash_sdk::dpp::dashcore::network::message_qrinfo::QRInfo;
use dash_sdk::dpp::dashcore::BlockHash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::WalletSeedHash;
use crate::model::grovestark_prover::ProofDataOutput;
use crate::ui::tokens::tokens_screen::{
    ContractDescriptionInfo, IdentityTokenIdentifier, TokenInfo,
};
use crate::utils::egui_mpsc::SenderAsync;
use contested_names::ScheduledDPNSVote;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::dashcore::network::message_sml::MnListDiff;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::evaluate_interval::IntervalEvaluationExplanation;
use dash_sdk::dpp::group::group_action::GroupAction;
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, Identifier};
use dash_sdk::query_types::{Documents, IndexMap};
use futures::future::join_all;
use std::collections::BTreeMap;
use std::sync::Arc;
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
pub mod mnlist;
pub mod platform_info;
pub mod register_contract;
pub mod shielded;
pub mod system_task;
pub mod tokens;
pub mod update_data_contract;
pub mod wallet;

// TODO: Refactor how we handle errors and messages, and remove it from here
pub(crate) const NO_IDENTITIES_FOUND: &str = "No identities found";

/// Returns `true` for backend tasks that read or write the
/// `WalletBackend` (and therefore the upstream `SecretStore` / sidecar
/// k/v). These tasks must short-circuit with
/// [`TaskError::WalletStorageNotReady`] while the cold-start migration
/// (`FinishUnwire`) is still running so the user sees the "data is
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

/// Information about fees paid for a platform state transition
#[derive(Debug, Clone, PartialEq)]
pub struct FeeResult {
    /// The fee that was estimated before the operation
    pub estimated_fee: u64,
    /// The actual fee that was paid (in credits)
    pub actual_fee: u64,
}

impl FeeResult {
    pub fn new(estimated_fee: u64, actual_fee: u64) -> Self {
        Self {
            estimated_fee,
            actual_fee,
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
    MnListTask(mnlist::MnListTask),
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

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BackendTaskSuccessResult {
    // General results
    None,
    Refresh,
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
    #[allow(dead_code)] // May be used for individual document operations
    Document(Document),
    Documents(Documents),
    BroadcastedDocument(Document),
    CoreItem(CoreItem),
    RegisteredIdentity(QualifiedIdentity, FeeResult),
    ToppedUpIdentity(QualifiedIdentity, FeeResult),
    #[allow(dead_code)] // May be used for reporting successful votes
    SuccessfulVotes(Vec<Vote>),
    DPNSVoteResults(Vec<(String, ResourceVoteChoice, Result<(), String>)>),
    CastScheduledVote(ScheduledDPNSVote),
    FetchedContract(DataContract),
    FetchedContractWithTokenPosition(
        DataContract,
        dash_sdk::dpp::data_contract::TokenContractPosition,
    ),
    FetchedContracts(Vec<Option<DataContract>>),
    PageDocuments(IndexMap<Identifier, Option<Document>>, Option<Start>),
    #[allow(dead_code)] // May be used for token search results
    TokensByKeyword(Vec<TokenInfo>, Option<Start>),
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
        incoming: Vec<(Identifier, Document)>, // (request_id, document)
        outgoing: Vec<(Identifier, Document)>, // (request_id, document)
    },
    DashPayContacts(Vec<Identifier>), // List of contact identity IDs
    DashPayContactsWithInfo(Vec<ContactData>), // List of contacts with metadata
    DashPayPaymentHistory(Vec<(String, String, u64, bool, String)>), // (tx_id, contact_name, amount, is_incoming, memo)
    DashPayProfileUpdated(Identifier), // Identity ID of updated profile
    DashPayContactRequestSent(String), // Username or ID of recipient
    DashPayContactRequestAccepted(Identifier), // Request ID that was accepted
    DashPayContactRequestRejected(Identifier), // Request ID that was rejected
    DashPayContactAlreadyEstablished(Identifier), // Contact ID that already exists
    DashPayContactInfoUpdated(Identifier), // Contact ID whose info was updated
    DashPayPaymentSent(String, String, f64), // (recipient, address, amount)
    GeneratedZKProof(ProofDataOutput),
    VerifiedZKProof(bool, ProofDataOutput),
    GeneratedReceiveAddress {
        seed_hash: WalletSeedHash,
        address: String,
    },
    /// Platform address balances fetched from Platform
    PlatformAddressBalances {
        seed_hash: WalletSeedHash,
        /// Map of platform address to (balance, nonce)
        balances: BTreeMap<PlatformAddress, (u64, u32)>,
        /// Network the balances were fetched from
        network: Network,
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
    /// A message signed with a wallet-derived key via the JIT chokepoint. Only
    /// the public Base64 signature crosses to the UI — the seed and the derived
    /// private key never leave the backend.
    WalletMessageSigned {
        seed_hash: WalletSeedHash,
        derivation_path: DerivationPath,
        /// The Base64-encoded signature (a public artifact, not a secret).
        signature: String,
    },

    // MNList-specific results
    MnListFetchedDiff {
        base_height: u32,
        height: u32,
        diff: MnListDiff,
    },
    MnListFetchedQrInfo {
        qr_info: QRInfo,
    },
    MnListChainLockSigs {
        entries: Vec<((u32, BlockHash), Option<BLSSignature>)>,
    },
    MnListFetchedDiffs {
        items: Vec<((u32, u32), MnListDiff)>,
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
        /// Optional warning message (e.g., Platform sync failed but Core refresh succeeded)
        warning: Option<String>,
    },

    // DPNS operation results (replacing string messages)
    ScheduledVotes,
    RefreshedDpnsContests,
    RefreshedOwnedDpnsNames,

    // Broadcast results
    BroadcastedStateTransition,

    // Mining results (dev mode, Regtest/Devnet only)
    MineBlocksSuccess(u64),

    // Shielded pool results
    ShieldedInitialized {
        seed_hash: WalletSeedHash,
        balance: u64,
    },
    ShieldedNotesSynced {
        seed_hash: WalletSeedHash,
        new_notes: u32,
        balance: u64,
    },
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
    ShieldedNullifiersChecked {
        seed_hash: WalletSeedHash,
        spent_count: u32,
    },
    ShieldedFromAssetLock {
        seed_hash: WalletSeedHash,
        amount: u64,
    },
    ShieldedWithdrawalComplete {
        seed_hash: WalletSeedHash,
        amount: u64,
    },
    ProvingKeyReady,

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
}

impl BackendTaskSuccessResult {}

impl AppContext {
    /// Run backend tasks sequentially
    pub async fn run_backend_tasks_sequential(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: SenderAsync<TaskResult>,
    ) -> Vec<Result<BackendTaskSuccessResult, TaskError>> {
        let mut results = Vec::new();
        for task in tasks {
            match self.run_backend_task(task, sender.clone()).await {
                Ok(result) => results.push(Ok(result)),
                Err(e) => results.push(Err(e)),
            };
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
        let sdk = self.sdk.load().as_ref().clone();

        // Wallet/identity/DashPay/core flows go through `WalletBackend`.
        // Build it lazily on first such task (idempotent) — this is where
        // the `AppState`-owned `TaskResult` sender is available.
        if matches!(
            task,
            BackendTask::WalletTask(_)
                | BackendTask::CoreTask(_)
                | BackendTask::IdentityTask(_)
                | BackendTask::DashPayTask(_)
        ) && let Err(e) = self.ensure_wallet_backend(sender.clone()).await
        {
            tracing::warn!(error = %e, "Wallet backend initialization deferred");
        }

        // Short-circuit wallet-touching tasks while the cold-start
        // migration is mid-flight. Reaching the SDK before the legacy
        // drain finishes either races on partially-mirrored sidecars
        // or produces a misleading SDK timeout. `WalletStorageNotReady`
        // is a typed, user-friendly variant whose banner mirrors the
        // migration banner ("data is still being updated"). The
        // shielded family also consults the NFR-4 pre-flight gate
        // (legacy shielded rows present but the sidecar has not yet
        // been mirrored) so a read path that pre-dates the orchestrator
        // run cannot race the mirror.
        if is_wallet_touching(&task) && self.migration_status().state().is_running() {
            tracing::debug!(
                target = "migration::gate",
                task = ?task,
                "Short-circuiting wallet-touching task — migration in progress",
            );
            return Err(TaskError::WalletStorageNotReady);
        }
        if matches!(task, BackendTask::ShieldedTask(_))
            && crate::backend_task::migration::finish_unwire::legacy_shielded_present_but_sidecar_empty(
                self,
            )?
        {
            tracing::debug!(
                target = "migration::gate",
                "Short-circuiting shielded task — legacy shielded rows still need to be mirrored",
            );
            return Err(TaskError::WalletStorageNotReady);
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
            BackendTask::MnListTask(mnlist_task) => {
                Ok(mnlist::run_mnlist_task(self, mnlist_task).await?)
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
                let new_ctx = tokio::task::block_in_place(|| {
                    AppContext::new(
                        data_dir,
                        network,
                        db,
                        subtasks,
                        connection_status,
                        egui_ctx,
                        app_kv,
                    )
                })
                .ok_or(TaskError::NetworkContextCreationFailed {
                    network,
                    detail: "AppContext::new() returned None".into(),
                })?;

                // Wire the freshly-built context's wallet backend and then start
                // chain sync. The old code called `start_spv()` on an unwired
                // context, which fast-failed with `WalletBackendNotYetWired` and
                // reported `spv_started=false`. Wiring first removes that race so
                // `spv_started` reflects whether sync actually began.
                let spv_started = if start_spv {
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
            WalletTask::SignMessageWithKey {
                seed_hash,
                derivation_path,
                message,
                key_type,
            } => {
                self.sign_message_with_key(seed_hash, derivation_path, message, key_type)
                    .await
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
            CoreTask::GetBestChainLock,
        )));
        assert!(is_wallet_touching(&BackendTask::ShieldedTask(
            shielded::ShieldedTask::InitializeShieldedWallet { seed_hash },
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
}
