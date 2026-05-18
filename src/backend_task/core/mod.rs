mod create_asset_lock;
mod recover_asset_locks;
mod refresh_single_key_wallet_info;
mod refresh_wallet_info;
mod send_single_key_wallet_payment;
mod start_dash_qt;

use crate::app_dir::core_cookie_path;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::{TaskError, is_rpc_auth_error, is_rpc_connection_error};
use crate::config::{Config, NetworkConfig};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::wallet::single_key::SingleKeyWallet;
use dash_sdk::dashcore_rpc;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::{
    Address, Block, ChainLock, InstantLock, Network, OutPoint, Transaction, TxOut,
};
use dash_sdk::dpp::fee::Credits;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub enum CoreTask {
    #[allow(dead_code)] // May be used for getting single chain lock
    GetBestChainLock,
    GetBestChainLocks,
    /// Refresh wallet info from Core. The bool controls whether to also sync
    /// Platform address balances (true = sync Platform, false = Core only).
    RefreshWalletInfo(Arc<RwLock<Wallet>>, bool),
    RefreshSingleKeyWalletInfo(Arc<RwLock<SingleKeyWallet>>),
    StartDashQT(Network, PathBuf, bool),
    CreateRegistrationAssetLock(Arc<RwLock<Wallet>>, Credits, u32), // wallet, amount in credits, identity index
    CreateTopUpAssetLock(Arc<RwLock<Wallet>>, Credits, u32, u32), // wallet, amount in credits, identity index, top up index
    SendWalletPayment {
        wallet: Arc<RwLock<Wallet>>,
        request: WalletPaymentRequest,
    },
    SendSingleKeyWalletPayment {
        wallet: Arc<RwLock<SingleKeyWallet>>,
        request: WalletPaymentRequest,
    },
    RecoverAssetLocks(Arc<RwLock<Wallet>>),
    MineBlocks {
        block_count: u64,
        address: Address,
        wallet: Arc<RwLock<Wallet>>,
    },
    ListCoreWallets,
}
impl PartialEq for CoreTask {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (CoreTask::GetBestChainLock, CoreTask::GetBestChainLock)
                | (CoreTask::GetBestChainLocks, CoreTask::GetBestChainLocks)
                | (
                    CoreTask::RefreshWalletInfo(_, _),
                    CoreTask::RefreshWalletInfo(_, _)
                )
                | (
                    CoreTask::RefreshSingleKeyWalletInfo(_),
                    CoreTask::RefreshSingleKeyWalletInfo(_)
                )
                | (
                    CoreTask::StartDashQT(_, _, _),
                    CoreTask::StartDashQT(_, _, _)
                )
                | (
                    CoreTask::CreateRegistrationAssetLock(_, _, _),
                    CoreTask::CreateRegistrationAssetLock(_, _, _)
                )
                | (
                    CoreTask::CreateTopUpAssetLock(_, _, _, _),
                    CoreTask::CreateTopUpAssetLock(_, _, _, _)
                )
                | (
                    CoreTask::SendWalletPayment { .. },
                    CoreTask::SendWalletPayment { .. },
                )
                | (
                    CoreTask::SendSingleKeyWalletPayment { .. },
                    CoreTask::SendSingleKeyWalletPayment { .. },
                )
                | (
                    CoreTask::RecoverAssetLocks(_),
                    CoreTask::RecoverAssetLocks(_),
                )
                | (CoreTask::MineBlocks { .. }, CoreTask::MineBlocks { .. })
                | (CoreTask::ListCoreWallets, CoreTask::ListCoreWallets)
        )
    }
}

/// A single recipient in a payment request
#[derive(Debug, Clone)]
pub struct PaymentRecipient {
    pub address: String,
    pub amount_duffs: u64,
}

#[derive(Debug, Clone)]
pub struct WalletPaymentRequest {
    pub recipients: Vec<PaymentRecipient>,
    pub subtract_fee_from_amount: bool,
    pub memo: Option<String>,
    /// Override fee to use instead of calculated fee (for retry after min relay fee error)
    pub override_fee: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreItem {
    InstantLockedTransaction(Transaction, Vec<(OutPoint, TxOut, Address)>, InstantLock),
    ReceivedAvailableUTXOTransaction(Transaction, Vec<(OutPoint, TxOut, Address)>),
    ChainLock(ChainLock, Network),
    ChainLocks(
        Option<ChainLock>,
        Option<ChainLock>,
        Option<ChainLock>,
        Option<ChainLock>,
        Option<String>,
    ), // Mainnet, Testnet, Devnet, Local, active network RPC error
    ChainLockedBlock(Block, ChainLock),
}

impl AppContext {
    pub async fn run_core_task(
        self: &Arc<Self>,
        task: CoreTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            CoreTask::GetBestChainLock => self
                .core_client
                .read()?
                .get_best_chain_lock()
                .map(|chain_lock| {
                    BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                        chain_lock,
                        self.network,
                    ))
                })
                .map_err(|e| self.rpc_error_with_url(e)),
            CoreTask::GetBestChainLocks => {
                // Load configs
                let config = Config::load_from(&self.data_dir)?;

                let maybe_mainnet_config = config.config_for_network(Network::Mainnet);
                let maybe_testnet_config = config.config_for_network(Network::Testnet);
                let maybe_devnet_config = config.config_for_network(Network::Devnet);
                let maybe_local_config = config.config_for_network(Network::Regtest);

                let mainnet_result =
                    Self::get_best_chain_lock(maybe_mainnet_config, Network::Mainnet);
                let testnet_result =
                    Self::get_best_chain_lock(maybe_testnet_config, Network::Testnet);
                let devnet_result = Self::get_best_chain_lock(maybe_devnet_config, Network::Devnet);
                let local_result = Self::get_best_chain_lock(maybe_local_config, Network::Regtest);

                // Surface auth and connection errors on the active network
                // instead of silently degrading to "Disconnected".
                let (active_result, active_config) = match self.network {
                    Network::Mainnet => (&mainnet_result, maybe_mainnet_config),
                    Network::Testnet => (&testnet_result, maybe_testnet_config),
                    Network::Devnet => (&devnet_result, maybe_devnet_config),
                    Network::Regtest => (&local_result, maybe_local_config),
                };
                let active_rpc_error = if let Err(e) = active_result {
                    if let Some(task_err) =
                        Self::chain_lock_rpc_error(active_config, self.network, e)
                    {
                        return Err(task_err);
                    }
                    // Non-auth, non-connection error — show the actual error
                    // in the Networks page status display for debugging.
                    tracing::warn!(network = ?self.network, error = %e, "Chain lock query failed on active network");
                    Some(format!("RPC error: {e}"))
                } else {
                    // Successful chain lock fetch — clear any lingering RPC error
                    // so the connection status recovers after a transient outage.
                    self.connection_status.set_rpc_last_error(None);
                    None
                };

                // Convert each to Option<ChainLock> (flatten Ok(None) and Err into None)
                let mainnet_chainlock = mainnet_result.ok().flatten();
                let testnet_chainlock = testnet_result.ok().flatten();
                let devnet_chainlock = devnet_result.ok().flatten();
                let local_chainlock = local_result.ok().flatten();

                // Return whatever we have — even all-None is valid.
                Ok(BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                    mainnet_chainlock,
                    testnet_chainlock,
                    devnet_chainlock,
                    local_chainlock,
                    active_rpc_error,
                )))
            }
            CoreTask::RefreshWalletInfo(_wallet, _sync_platform) => {
                Err(TaskError::WalletBackendNotYetWired)
            }
            CoreTask::RefreshSingleKeyWalletInfo(_wallet) => {
                Err(TaskError::SingleKeyWalletsUnsupported)
            }
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| TaskError::DashCoreStartError { source: e })
                .map(|_| BackendTaskSuccessResult::None),
            CoreTask::CreateRegistrationAssetLock(_wallet, _amount, _identity_index) => {
                Err(TaskError::WalletBackendNotYetWired)
            }
            CoreTask::CreateTopUpAssetLock(_wallet, _amount, _identity_index, _top_up_index) => {
                Err(TaskError::WalletBackendNotYetWired)
            }
            CoreTask::SendWalletPayment {
                wallet: _,
                request: _,
            } => Err(TaskError::WalletBackendNotYetWired),
            CoreTask::SendSingleKeyWalletPayment {
                wallet: _,
                request: _,
            } => Err(TaskError::SingleKeyWalletsUnsupported),
            CoreTask::RecoverAssetLocks(_wallet) => Err(TaskError::WalletBackendNotYetWired),
            CoreTask::MineBlocks {
                block_count,
                address,
                wallet,
            } => {
                if !matches!(self.network, Network::Regtest | Network::Devnet) {
                    return Err(TaskError::OperationNotAvailableOnNetwork {
                        operation: "Mining",
                        allowed_networks: "Regtest and Devnet",
                    });
                }
                let ctx = self.clone();
                let mined = tokio::task::spawn_blocking(move || {
                    ctx.core_client
                        .read()
                        .map_err(TaskError::from)?
                        .generate_to_address(block_count, &address)
                        .map_err(TaskError::from)
                })
                .await??;

                let _ = wallet;
                let mined_count = mined.len() as u64;
                // Balances refresh once the wallet backend is wired (P2).
                Ok(BackendTaskSuccessResult::MineBlocksSuccess(mined_count))
            }
            CoreTask::ListCoreWallets => {
                // Named Core wallets are RPC-only; the RPC wallet backend was
                // removed. Returns empty until the UI entry point is dropped (P4).
                Ok(BackendTaskSuccessResult::CoreWalletsList(Vec::new()))
            }
        }
    }
    fn get_best_chain_lock(
        config: &Option<NetworkConfig>,
        network: Network,
    ) -> Result<Option<ChainLock>, dashcore_rpc::Error> {
        let Some(network_config) = config else {
            return Ok(None);
        };

        let addr = format!(
            "http://{}:{}",
            network_config.rpc_host(),
            network_config.rpc_port(network)
        );

        let cookie_path = match core_cookie_path(network, &network_config.devnet_name) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get core cookie path for {network}: {e}");
                return Ok(None);
            }
        };

        // Try cookie authentication first
        let client = match Client::new(&addr, Auth::CookieFile(cookie_path.clone())) {
            Ok(client) => client,
            Err(_) => {
                tracing::trace!(
                    "Cookie auth unavailable at {:?}, using user/pass",
                    cookie_path
                );
                match Client::new(
                    &addr,
                    Auth::UserPass(
                        network_config.core_rpc_user.clone().unwrap_or_default(),
                        network_config.core_rpc_password.clone().unwrap_or_default(),
                    ),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("Failed to create {network} client: {e}");
                        return Ok(None);
                    }
                }
            }
        };

        client.get_best_chain_lock().map(Some)
    }

    /// Convert a `dashcore_rpc::Error` from `get_best_chain_lock` into a
    /// `TaskError`, enriching connection failures with host:port.
    fn chain_lock_rpc_error(
        config: &Option<NetworkConfig>,
        network: Network,
        e: &dashcore_rpc::Error,
    ) -> Option<TaskError> {
        if is_rpc_auth_error(e) {
            return Some(TaskError::CoreRpcAuthFailed);
        }
        if is_rpc_connection_error(e) {
            let url = config
                .as_ref()
                .map(|c| format!("{}:{}", c.rpc_host(), c.rpc_port(network)))
                .unwrap_or_else(|| "unknown".to_string());
            return Some(TaskError::CoreRpcConnectionFailed { url, source: None });
        }
        None
    }
}
