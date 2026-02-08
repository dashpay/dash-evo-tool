use crate::app::TaskResult;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contract::{ContractResult, ContractTask};
use crate::backend_task::core::{CoreResult, CoreTask};
use crate::backend_task::dashpay::{DashPayResult, DashPayTask};
use crate::backend_task::document::{DocumentResult, DocumentTask};
use crate::backend_task::identity::{IdentityResult, IdentityTask};
use crate::backend_task::platform_info::{PlatformInfoTaskRequestType, PlatformInfoTaskResult};
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::wallet::{WalletResult, WalletTask};
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::utils::egui_mpsc::SenderAsync;
use dash_sdk::dpp::state_transition::StateTransition;
use futures::future::join_all;
use grovestark::GroveSTARKTask;
use std::sync::Arc;
use tokens::TokenTask;

pub mod broadcast_state_transition;
pub mod contested_names;
pub mod contract;
pub mod core;
pub mod dashpay;
pub mod document;
pub mod grovestark;
pub mod identity;
pub mod mnlist;
pub mod platform_info;
pub mod register_contract;
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
    None,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum BackendTaskSuccessResult {
    // General results
    None,
    Refresh,
    Message(String), // Used for: progress messages during long operations, placeholder messages for
    // not-yet-implemented functionality, and DashPay operations that would need their own typed variants.

    // Wallet/Core domain results
    Wallet(WalletResult),
    Core(CoreResult),

    // Identity domain results
    Identity(IdentityResult),

    // Document domain results
    Document(DocumentResult),

    // Contract domain results
    Contract(ContractResult),

    // DPNS/Contest results
    Contest(contested_names::ContestResult),

    // Other domain results
    UpdatedThemePreference(crate::ui::theme::ThemeMode),
    PlatformInfo(PlatformInfoTaskResult),
    DashPay(DashPayResult),
    GroveSTARK(grovestark::GroveSTARKResult),
    MnList(mnlist::MnListResult),
    Token(tokens::TokenResult),

    // Broadcast results
    BroadcastedStateTransition,
}

impl BackendTaskSuccessResult {}

impl AppContext {
    /// Run backend tasks sequentially
    pub async fn run_backend_tasks_sequential(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: SenderAsync<TaskResult>,
    ) -> Vec<Result<BackendTaskSuccessResult, String>> {
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
    ) -> Vec<Result<BackendTaskSuccessResult, String>> {
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
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = {
            let guard = self.sdk.read_or_recover();
            guard.clone()
        };
        match task {
            BackendTask::ContractTask(contract_task) => {
                self.run_contract_task(*contract_task, &sdk, sender).await
            }
            BackendTask::ContestedResourceTask(contested_resource_task) => {
                self.run_contested_resource_task(contested_resource_task, &sdk, sender)
                    .await
            }
            BackendTask::IdentityTask(identity_task) => {
                self.run_identity_task(identity_task, &sdk, sender).await
            }
            BackendTask::DocumentTask(document_task) => {
                self.run_document_task(*document_task, &sdk).await
            }
            BackendTask::CoreTask(core_task) => self.run_core_task(core_task).await,
            BackendTask::DashPayTask(dashpay_task) => {
                self.run_dashpay_task(*dashpay_task, &sdk).await
            }
            BackendTask::BroadcastStateTransition(state_transition) => {
                self.broadcast_state_transition(state_transition, &sdk)
                    .await
            }
            BackendTask::TokenTask(token_task) => {
                self.run_token_task(*token_task, &sdk, sender).await
            }
            BackendTask::SystemTask(system_task) => self.run_system_task(system_task, sender).await,
            BackendTask::MnListTask(mnlist_task) => mnlist::run_mnlist_task(self, mnlist_task)
                .await
                .map(BackendTaskSuccessResult::MnList),
            BackendTask::PlatformInfo(platform_info_task) => {
                self.run_platform_info_task(platform_info_task).await
            }
            BackendTask::GroveSTARKTask(grovestark_task) => {
                grovestark::run_grovestark_task(grovestark_task, &sdk)
                    .await
                    .map(BackendTaskSuccessResult::GroveSTARK)
            }
            BackendTask::WalletTask(wallet_task) => self.run_wallet_task(wallet_task).await,
            BackendTask::None => Ok(BackendTaskSuccessResult::None),
        }
    }

    async fn run_wallet_task(
        self: &Arc<Self>,
        task: WalletTask,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            WalletTask::GenerateReceiveAddress { seed_hash } => {
                self.generate_receive_address(seed_hash).await
            }
            WalletTask::FetchPlatformAddressBalances {
                seed_hash,
                sync_mode,
            } => {
                self.fetch_platform_address_balances(seed_hash, sync_mode)
                    .await
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
