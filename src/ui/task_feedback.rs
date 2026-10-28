//! User feedback on a backend result, decided by the domain the result
//! belongs to. The app makes one call per result; each domain module
//! matches its own results and errors.
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

/// Feedback on a successful result, with what the app still has to do.
pub(crate) fn on_success(
    ctx: &egui::Context,
    contexts: &BTreeMap<Network, Arc<AppContext>>,
    delivery: ResultDelivery,
    context: &BackendTaskContext,
    result: &BackendTaskSuccessResult,
) -> Vec<CacheUpdate> {
    let tracked_top_up = crate::ui::identity::task_feedback::on_success(
        ctx,
        delivery.sender_in_view,
        context,
        result,
    );
    crate::ui::wallets::task_feedback::on_success(
        ctx,
        delivery.sender_gone && !tracked_top_up,
        result,
    );
    crate::ui::tokens::task_feedback::on_success(ctx, result);
    crate::ui::network_chooser_screen::task_feedback::show(ctx, result);
    crate::ui::dpns::task_feedback::show(ctx, contexts, result)
}

/// Feedback on a failed result. Returns whether the failure is quiet: one
/// its domain expects and does not report.
pub(crate) fn on_error(
    ctx: &egui::Context,
    context: &BackendTaskContext,
    error: &TaskError,
) -> bool {
    crate::ui::identity::task_feedback::on_error(ctx, context);
    crate::ui::dpns::task_feedback::is_quiet(error)
}
