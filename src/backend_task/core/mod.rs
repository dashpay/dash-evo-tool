mod refresh_wallet_info;
mod start_dash_qt;

use crate::app_dir::{core_cookie_path, core_user_data_dir_path};
use crate::backend_task::BackendTaskSuccessResult;
use crate::config::{Config, NetworkConfig};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::{Address, ChainLock, Network, OutPoint, Transaction, TxOut};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub(crate) enum CoreTask {
    GetBestChainLock,
    GetBestChainLocks,
    RefreshWalletInfo(Arc<RwLock<Wallet>>),
    StartDashQT(Network, Option<String>, bool),
}
impl PartialEq for CoreTask {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CoreTask::GetBestChainLock, CoreTask::GetBestChainLock) => true,
            (CoreTask::GetBestChainLocks, CoreTask::GetBestChainLocks) => true,
            (CoreTask::RefreshWalletInfo(_), CoreTask::RefreshWalletInfo(_)) => true,
            (CoreTask::StartDashQT(_, _, _), CoreTask::StartDashQT(_, _, _)) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CoreItem {
    ReceivedAvailableUTXOTransaction(Transaction, Vec<(OutPoint, TxOut, Address)>),
    ChainLock(ChainLock, Network),
    ChainLocks(Option<ChainLock>, Option<ChainLock>), // Mainnet, Testnet
}

impl AppContext {
    pub async fn run_core_task(&self, task: CoreTask) -> Result<BackendTaskSuccessResult, String> {
        match task {
            CoreTask::GetBestChainLock => self
                .core_client
                .get_best_chain_lock()
                .map(|chain_lock| {
                    BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                        chain_lock,
                        self.network,
                    ))
                })
                .map_err(|e| e.to_string()),
            CoreTask::GetBestChainLocks => {
                tracing::info!("Getting best chain locks for testnet and mainnet");

                // Load configs
                let config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;

                // Get chain locks
                let mainnet_result =
                    Self::get_best_chain_lock(config.config_for_network(Network::Dash), Network::Dash);
                let testnet_result = Self::get_best_chain_lock(
                    config.config_for_network(Network::Testnet),
                    Network::Testnet,
                );

                // Handle results
                match (mainnet_result, testnet_result) {
                    (Ok(mainnet_chainlock), Ok(testnet_chainlock)) => {
                        Ok(BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                            Some(mainnet_chainlock),
                            Some(testnet_chainlock),
                        )))
                    }
                    (Ok(mainnet_chainlock), Err(_)) => Ok(BackendTaskSuccessResult::CoreItem(
                        CoreItem::ChainLocks(Some(mainnet_chainlock), None),
                    )),
                    (Err(_), Ok(testnet_chainlock)) => Ok(BackendTaskSuccessResult::CoreItem(
                        CoreItem::ChainLocks(None, Some(testnet_chainlock)),
                    )),
                    (Err(_), Err(_)) => {
                        Err("Failed to get best chain lock for both mainnet and testnet"
                            .to_string())
                    }
                }
            }
            CoreTask::RefreshWalletInfo(wallet) => self
                .refresh_wallet_info(wallet)
                .map_err(|e| format!("Error refreshing wallet: {}", e.to_string())),
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| e.to_string())
                .map(|_| BackendTaskSuccessResult::None),
        }
    }

    fn get_best_chain_lock(
        config: &Option<NetworkConfig>,
        network: Network,
    ) -> Result<ChainLock, String> {
        if let Some(network_config) = config {
            let addr = format!(
                "http://{}:{}",
                network_config.core_host, network_config.core_rpc_port
            );

            let cookie_path = core_cookie_path(network, &network_config.devnet_name)
                .map_err(|e| format!("Failed to get core cookie path: {}", e))?;

            // Try cookie authentication first
            let client = match Client::new(&addr, Auth::CookieFile(cookie_path.clone())) {
                Ok(client) => Ok(client),
                Err(_) => {
                    tracing::info!(
                        "Failed to authenticate using .cookie file at {:?}, falling back to user/pass",
                        cookie_path
                    );
                    Client::new(
                        &addr,
                        Auth::UserPass(
                            network_config.core_rpc_user.to_string(),
                            network_config.core_rpc_password.to_string(),
                        ),
                    )
                }
            }
                .map_err(|_| format!("Failed to create {} client", network.to_string()))?;

            client.get_best_chain_lock().map_err(|e| {
                format!(
                    "Failed to get best chain lock for {}: {}",
                    network.to_string(),
                    e.to_string()
                )
            })
        } else {
            Err(format!("{} config not found", network.to_string()))
        }
    }
}
