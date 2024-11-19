mod refresh_wallet_info;
mod start_dash_qt;

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::{Address, ChainLock, Network, OutPoint, Transaction, TxOut};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub(crate) enum CoreTask {
    GetBestChainLock,
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
            CoreTask::RefreshWalletInfo(wallet) => self.refresh_wallet_info(wallet),
            CoreTask::StartDashQT(network, custom_dash_qt, overwrite_dash_conf) => self
                .start_dash_qt(network, custom_dash_qt, overwrite_dash_conf)
                .map_err(|e| e.to_string())
                .map(|_| BackendTaskSuccessResult::None),
        }
    }
}
