mod refresh_wallet_info;
mod start_dash_qt;

use crate::backend_task::BackendTaskSuccessResult;
use crate::config::Config;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::{Address, ChainLock, Network, OutPoint, Transaction, TxOut};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub(crate) enum CoreTask {
    GetBestChainLock,
    GetBothBestChainLocks,
    RefreshWalletInfo(Arc<RwLock<Wallet>>),
    StartDashQT(Network, Option<String>, bool),
}
impl PartialEq for CoreTask {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CoreTask::GetBestChainLock, CoreTask::GetBestChainLock) => true,
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
    BothChainLocks(ChainLock, ChainLock),
}

impl AppContext {
    pub async fn run_core_task(&self, task: CoreTask) -> Result<BackendTaskSuccessResult, String> {
        match task {
            CoreTask::GetBestChainLock => {
                let network_string = match self.network {
                    Network::Dash => "mainnet",
                    Network::Testnet => "testnet",
                    _ => "network",
                };

                self.core_client
                    .get_best_chain_lock()
                    .map(|chain_lock| {
                        BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                            chain_lock,
                            self.network,
                        ))
                    })
                    .map_err(|e| {
                        format!(
                            "Failed to get best chain lock for {}: {}",
                            network_string,
                            e.to_string()
                        )
                    })
            }
            CoreTask::GetBothBestChainLocks => {
                // Load config
                let config = match Config::load() {
                    Ok(config) => config,
                    Err(e) => {
                        return Err(format!("Failed to load config: {}", e));
                    }
                };

                // Get mainnet best chainlock
                let mainnet_config = config
                    .config_for_network(Network::Dash)
                    .clone()
                    .ok_or("Failed to get mainnet config".to_string())?;
                let mainnet_addr = format!(
                    "http://{}:{}",
                    mainnet_config.core_host, mainnet_config.core_rpc_port
                );
                let mainnet_client = Client::new(
                    &mainnet_addr,
                    Auth::UserPass(
                        mainnet_config.core_rpc_user.to_string(),
                        mainnet_config.core_rpc_password.to_string(),
                    ),
                )
                .map_err(|_| "Failed to create mainnet client".to_string())?;
                let mainnet_result = mainnet_client.get_best_chain_lock().map_err(|e| {
                    format!(
                        "Failed to get best chain lock for mainnet: {}",
                        e.to_string()
                    )
                });

                // Get testnet best chainlock
                let testnet_config = config
                    .config_for_network(Network::Testnet)
                    .clone()
                    .ok_or("Failed to get testnet config".to_string())?;
                let testnet_addr = format!(
                    "http://{}:{}",
                    testnet_config.core_host, testnet_config.core_rpc_port
                );
                let testnet_client = Client::new(
                    &testnet_addr,
                    Auth::UserPass(
                        testnet_config.core_rpc_user.to_string(),
                        testnet_config.core_rpc_password.to_string(),
                    ),
                )
                .map_err(|_| "Failed to create testnet client".to_string())?;
                let testnet_result = testnet_client.get_best_chain_lock().map_err(|e| {
                    format!(
                        "Failed to get best chain lock for testnet: {}",
                        e.to_string()
                    )
                });

                // Handle results
                match (mainnet_result, testnet_result) {
                    (Ok(mainnet_chainlock), Ok(testnet_chainlock)) => {
                        Ok(BackendTaskSuccessResult::CoreItem(
                            CoreItem::BothChainLocks(mainnet_chainlock, testnet_chainlock),
                        ))
                    }
                    (Ok(mainnet_chainlock), Err(testnet_err)) => {
                        tracing::error!("{}", testnet_err);
                        Ok(BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                            mainnet_chainlock,
                            Network::Dash,
                        )))
                    }
                    (Err(mainnet_err), Ok(testnet_chainlock)) => {
                        tracing::error!("{}", mainnet_err);
                        Ok(BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(
                            testnet_chainlock,
                            Network::Testnet,
                        )))
                    }
                    (Err(_), Err(_)) => {
                        tracing::error!(
                            "Failed to get best chain lock for both mainnet and testnet",
                        );
                        Err("Failed to get best chain lock for both mainnet and testnet"
                            .to_string())
                    }
                }
            }
            CoreTask::RefreshWalletInfo(wallet) => self.refresh_wallet_info(wallet),
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| e.to_string())
                .map(|_| BackendTaskSuccessResult::None),
        }
    }
}
