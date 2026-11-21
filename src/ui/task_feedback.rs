//! User feedback on a backend result, decided by the domain the result
//! belongs to. The app makes one call per result; each domain's
//! `task_feedback` module implements [`DomainFeedback`] for its own results
//! and errors, and the fan-out here holds no decision of its own.
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::context::AppContext;
use dash_sdk::dpp::dashcore::Network;
use std::{collections::BTreeMap, sync::Arc};

/// Where a result went, as far as feedback needs to know.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResultDelivery {
    /// The screen that sent the task is the one in view.
    pub sender_in_view: bool,
    /// The screen that sent the task was closed, or sent it on a network no
    /// longer active: no screen takes the result.
    pub sender_gone: bool,
}

/// What every domain is told about a result, beyond the result itself.
pub(crate) struct Feedback<'a> {
    pub ctx: &'a egui::Context,
    /// Every network's context, for a result that names its network.
    pub contexts: &'a BTreeMap<Network, Arc<AppContext>>,
    pub delivery: ResultDelivery,
}

/// A domain's reactions to the results and errors it owns: settle what it
/// tracks about the task and tell the user what they need to know. A result
/// another domain owns is left alone, by the defaults.
pub(crate) trait DomainFeedback {
    /// React to a successful result. Returns the cache changes the result
    /// implies, for the app to deliver as notifications.
    fn on_success(
        &self,
        _feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        _result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        Vec::new()
    }

    /// React to a failure. Returns whether the failure is quiet: one the
    /// domain expects and does not report.
    fn on_error(
        &self,
        _feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        false
    }
}

/// Every domain, in the order their reactions run.
const DOMAINS: &[&dyn DomainFeedback] = &[
    &crate::ui::identity::task_feedback::IdentityFeedback,
    &crate::ui::wallets::task_feedback::WalletsFeedback,
    &crate::ui::network_chooser_screen::task_feedback::SettingsFeedback,
    &crate::ui::dpns::task_feedback::DpnsFeedback,
];

/// Feedback on a successful result, with the cache changes it implies.
pub(crate) fn on_success(
    feedback: &Feedback<'_>,
    context: &BackendTaskContext,
    result: &BackendTaskSuccessResult,
) -> Vec<CacheUpdate> {
    DOMAINS
        .iter()
        .flat_map(|domain| domain.on_success(feedback, context, result))
        .collect()
}

/// Feedback on a failed result. Returns whether the failure is quiet: one
/// its domain expects and does not report. Every domain reacts, quiet or not.
pub(crate) fn on_error(
    feedback: &Feedback<'_>,
    context: &BackendTaskContext,
    error: &TaskError,
) -> bool {
    // A loop, not `any`: every domain reacts, quiet or not.
    let mut quiet = false;
    for domain in DOMAINS {
        quiet |= domain.on_error(feedback, context, error);
    }
    quiet
}
