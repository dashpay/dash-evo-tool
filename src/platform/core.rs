use crate::context::AppContext;
use crate::platform::BackendTaskSuccessResult;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::{ChainLock, Network};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CoreTask {
    GetBestChainLock,
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
        }
    }
}
