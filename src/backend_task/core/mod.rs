mod refresh_wallet_info;
mod start_dash_qt;

use crate::backend_task::BackendTaskSuccessResult;
use crate::config::Config;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::{dashcore, RpcApi};
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::{Address, ChainLock, Network, OutPoint, Transaction, TxOut};
use std::sync::{Arc, RwLock};
use dash_sdk::dashcore_rpc::dashcore::address::Payload;
use dash_sdk::dashcore_rpc::dashcore::hashes::{hash160, Hash};
use dash_sdk::dashcore_rpc::dashcore::PubkeyHash;

#[derive(Debug, Clone)]
pub(crate) enum CoreTask {
    GetBestChainLock,
    GetBestChainLocks,
    RefreshWalletInfo(Arc<RwLock<Wallet>>),
    StartDashQT(Network, Option<String>, bool),
    ProRegUpdateTx(String, Address, Address),
}
impl PartialEq for CoreTask {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CoreTask::GetBestChainLock, CoreTask::GetBestChainLock) => true,
            (CoreTask::GetBestChainLocks, CoreTask::GetBestChainLocks) => true,
            (CoreTask::RefreshWalletInfo(_), CoreTask::RefreshWalletInfo(_)) => true,
            (CoreTask::StartDashQT(_, _, _), CoreTask::StartDashQT(_, _, _)) => true,
            (CoreTask::ProRegUpdateTx(_, _, _), CoreTask::ProRegUpdateTx(_, _, _)) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CoreItem {
    ReceivedAvailableUTXOTransaction(Transaction, Vec<(OutPoint, TxOut, Address)>),
    ChainLock(ChainLock, Network),
    ChainLocks(Option<ChainLock>, Option<ChainLock>), // Mainnet, Testnet
    ProRegUpdateTx(String),
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
                let config = match Config::load() {
                    Ok(config) => config,
                    Err(e) => {
                        return Err(format!("Failed to load config: {}", e));
                    }
                };
                let maybe_mainnet_config = config.config_for_network(Network::Dash);
                let maybe_testnet_config = config.config_for_network(Network::Testnet);

                // Get mainnet best chainlock
                let mainnet_result = if let Some(mainnet_config) = maybe_mainnet_config {
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
                    mainnet_client.get_best_chain_lock().map_err(|e| {
                        format!(
                            "Failed to get best chain lock for mainnet: {}",
                            e.to_string()
                        )
                    })
                } else {
                    Err("Mainnet config not found".to_string())
                };

                // Get testnet best chainlock
                let testnet_result = if let Some(testnet_config) = maybe_testnet_config {
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
                    testnet_client.get_best_chain_lock().map_err(|e| {
                        format!(
                            "Failed to get best chain lock for testnet: {}",
                            e.to_string()
                        )
                    })
                } else {
                    Err("Testnet config not found".to_string())
                };

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
            CoreTask::RefreshWalletInfo(wallet) => self.refresh_wallet_info(wallet),
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| e.to_string())
                .map(|_| BackendTaskSuccessResult::None),
            CoreTask::ProRegUpdateTx(pro_tx_hash, voting_address, payout_address) => self
                .core_client
                .get_protx_update_registrar(pro_tx_hash.as_str(), "", payout_address, voting_address, None)
                .map(|pro_tx_hash| {
                    BackendTaskSuccessResult::CoreItem(CoreItem::ProRegUpdateTx(
                        pro_tx_hash.to_string(),
                    ))
                })
                .map_err(|e| e.to_string())
        }
    }
}
