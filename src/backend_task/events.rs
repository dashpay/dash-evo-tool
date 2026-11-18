//! Backend completion and cache notifications shared by GUI and headless clients.

use super::error::TaskError;
use super::{BackendTaskContext, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Network;
use std::future::Future;

tokio::task_local! {
    /// The context of the request running on the current task. The app's
    /// dispatcher sets it around each `run_backend_task` ([`as_request`]) so a
    /// report the request sends mid-task can name its request without the
    /// context being threaded through every call. Unset outside a dispatch —
    /// MCP, bootstrap, tests — where an interim report falls back to
    /// [`BackendTaskContext::Unknown`], today's behaviour.
    static CURRENT_REQUEST: BackendTaskContext;
}

/// Run `future` as the request `request`, so the interim reports it sends
/// name that request.
pub(crate) async fn as_request<F: Future>(request: BackendTaskContext, future: F) -> F::Output {
    CURRENT_REQUEST.scope(request, future).await
}

/// Carry the running request into `future`, for a subtask the request
/// spawns: a task-local does not cross `tokio::spawn`, so an interim report
/// built inside a spawned subtask would otherwise lose its request.
pub(crate) fn in_current_request<F: Future>(future: F) -> impl Future<Output = F::Output> {
    match current_request() {
        Some(request) => futures::future::Either::Left(CURRENT_REQUEST.scope(request, future)),
        None => futures::future::Either::Right(future),
    }
}

/// The running request, or `None` outside a dispatch.
fn current_request() -> Option<BackendTaskContext> {
    CURRENT_REQUEST.try_with(Clone::clone).ok()
}

/// The context of a report sent mid-task by the running request on `network`.
/// Outside a dispatch — or in a subtask not carried into one — the report
/// names no request and is routed as an undispatched result.
fn interim_context(network: Network) -> BackendTaskContext {
    let request = current_request().unwrap_or_else(|| {
        tracing::debug!(
            ?network,
            "Interim report sent outside a dispatched request; it names no sender"
        );
        BackendTaskContext::Unknown
    });
    BackendTaskContext::interim(network, request)
}

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

    /// An error with no context at all. Every producer names at least the
    /// network it runs on; only tests emit one without.
    #[cfg(test)]
    pub(crate) fn unattributed_error(error: TaskError) -> Self {
        Self::Error {
            context: BackendTaskContext::Unknown,
            error,
        }
    }

    /// A report from a request still running on `network`: progress or a
    /// partial result. Routed like the request's result, to the screen that
    /// sent the request, but ends nothing: the request's sender stays on
    /// record and its wait goes on. Outside a dispatch the report names no
    /// request and goes to the screen in view, as every undispatched result.
    pub(crate) fn interim_success(network: Network, result: BackendTaskSuccessResult) -> Self {
        Self::Success {
            context: interim_context(network),
            result: Box::new(result),
        }
    }

    /// A failure of one step of a request still running on `network`, such
    /// as a sub-query's. Reported like an error of the request, but ends
    /// nothing: see [`Self::interim_success`].
    pub(crate) fn interim_error(network: Network, error: TaskError) -> Self {
        Self::Error {
            context: interim_context(network),
            error,
        }
    }

    /// An event one network's wallet backend raised from chain sync, not from
    /// a request. It belongs to `network` whether or not that network is the
    /// active one.
    pub(crate) fn wallet_event(network: Network, result: BackendTaskSuccessResult) -> Self {
        Self::Success {
            context: BackendTaskContext::WalletSync { network },
            result: Box::new(result),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTask;

    fn request() -> BackendTaskContext {
        BackendTaskContext::for_dispatch(&BackendTask::None)
    }

    fn context_of(result: &TaskResult) -> &BackendTaskContext {
        let (TaskResult::Success { context, .. } | TaskResult::Error { context, .. }) = result
        else {
            panic!("an interim report is a success or an error");
        };
        context
    }

    fn interim_request(result: &TaskResult) -> &BackendTaskContext {
        context_of(result)
            .interim_request()
            .expect("an interim report names its request")
    }

    #[tokio::test]
    async fn an_interim_report_names_the_request_it_is_sent_from() {
        let request = request();
        let (progress, failure) = as_request(request.clone(), async {
            (
                TaskResult::interim_success(Network::Testnet, BackendTaskSuccessResult::None),
                TaskResult::interim_error(Network::Testnet, TaskError::NoIdentitiesFound),
            )
        })
        .await;

        assert_eq!(interim_request(&progress), &request);
        assert_eq!(interim_request(&failure), &request);
        assert_eq!(
            context_of(&progress).undispatched_network(),
            Some(Network::Testnet)
        );
    }

    #[tokio::test]
    async fn an_interim_report_outside_a_dispatch_names_no_request() {
        let progress =
            TaskResult::interim_success(Network::Testnet, BackendTaskSuccessResult::None);
        assert_eq!(interim_request(&progress), &BackendTaskContext::Unknown);
        assert_eq!(context_of(&progress).dispatch_id(), None);
    }

    #[tokio::test]
    async fn a_spawned_subtask_keeps_the_request_it_is_carried_into() {
        let request = request();
        let (carried, dropped) = as_request(request.clone(), async {
            let carried = tokio::spawn(in_current_request(async {
                TaskResult::interim_success(Network::Testnet, BackendTaskSuccessResult::None)
            }))
            .await
            .expect("subtask");
            let dropped = tokio::spawn(async {
                TaskResult::interim_success(Network::Testnet, BackendTaskSuccessResult::None)
            })
            .await
            .expect("subtask");
            (carried, dropped)
        })
        .await;

        assert_eq!(interim_request(&carried), &request);
        assert_eq!(
            interim_request(&dropped),
            &BackendTaskContext::Unknown,
            "a bare spawn loses the request: producers must carry it"
        );
    }
}
