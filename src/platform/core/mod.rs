mod refresh_wallet_info;

use std::sync::{Arc, RwLock};
use crate::context::AppContext;
use crate::platform::BackendTaskSuccessResult;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::{ChainLock, Network};
use crate::model::wallet::Wallet;

#[derive(Debug, Clone)]
pub(crate) enum CoreTask {
    GetBestChainLock,
    RefreshWalletInfo(Arc<RwLock<Wallet>>),
}
impl PartialEq for CoreTask {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CoreTask::GetBestChainLock, CoreTask::GetBestChainLock) => true,
            (CoreTask::RefreshWalletInfo(_), CoreTask::RefreshWalletInfo(_)) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CoreItem {
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
            CoreTask::RefreshWalletInfo(wallet) => {
                self.refresh_wallet_info(wallet)
            }
        }
    }
}
