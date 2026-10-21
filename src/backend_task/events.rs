//! Backend completion and cache notifications shared by GUI and headless clients.

use super::error::TaskError;
use super::{BackendTaskContext, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Network;

/// Cached data that changed, independent of any screen's pending request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacheDomain {
    Tokens,
    Identities,
    ContactPayments,
    Dpns,
    All,
}

/// A network-scoped cache invalidation; never a request to reset a form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheUpdate {
    pub network: Network,
    pub domain: CacheDomain,
}

#[derive(Debug)]
pub enum TaskResult {
    Repaint,
    CacheUpdated(CacheUpdate),
    Success {
        context: BackendTaskContext,
        result: Box<BackendTaskSuccessResult>,
    },
    Error {
        context: BackendTaskContext,
        error: TaskError,
    },
}

impl TaskResult {
    pub(crate) fn from_backend_task_result(
        context: BackendTaskContext,
        value: Result<BackendTaskSuccessResult, TaskError>,
    ) -> Self {
        match value {
            Ok(value) => TaskResult::Success {
                context,
                result: Box::new(value),
            },
            Err(error) => TaskResult::Error { context, error },
        }
    }

    pub(crate) fn unattributed_success(result: BackendTaskSuccessResult) -> Self {
        Self::Success {
            context: BackendTaskContext::Unknown,
            result: Box::new(result),
        }
    }

    pub(crate) fn unattributed_error(error: TaskError) -> Self {
        Self::Error {
            context: BackendTaskContext::Unknown,
            error,
        }
    }
}
