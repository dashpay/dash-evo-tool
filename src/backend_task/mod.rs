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
use shielded::ShieldedTask;
use tokens::TokenTask;
use grovestark::GroveSTARKTask;

pub mod broadcast_state_transition;
pub mod contested_names;
pub mod contract;
pub mod core;
pub mod dashpay;
pub mod document;
pub mod error;
pub mod grovestark;
pub mod identity;
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
    None,
}

#[derive(Debug, Clone, PartialEq)]
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
    RecoveredAssetLocks {
        recovered_count: usize,
        total_amount: u64,
    },

    // DPNS operation results (replacing string messages)
    ScheduledVotes,
    RefreshedDpnsContests,
    RefreshedOwnedDpnsNames,

    // Broadcast results
    BroadcastedStateTransition,

    // Mining results (dev mode, Regtest/Devnet only)
    MineBlocksSuccess(u64),

    // Core wallet list (async fetch of loaded Core wallets)
    CoreWalletsList(Vec<String>),

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
                asset_lock_proof,
                asset_lock_address,
                outputs,
            } => {
                self.fund_platform_address_from_asset_lock(
                    seed_hash,
                    *asset_lock_proof,
                    asset_lock_address,
                    outputs,
                )
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
