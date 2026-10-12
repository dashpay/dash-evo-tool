//! Identity Home state, including correlated payment previews and receive sharing.

use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::model::qualified_identity::QualifiedIdentity;

/// Cached Home content for the currently selected identity.
#[derive(Default)]
pub struct HomeState {
    pub dismissed_checklist: bool,
    pub receive_open: bool,
    pub receive_qr: Option<egui::TextureHandle>,
    pub payments: Vec<(String, String, u64, bool, String)>,
    requested_payments: bool,
    pending_payments: Option<BackendTaskContext>,
}

impl HomeState {
    /// Refresh on arrival while retaining the visible preview.
    pub fn refresh_payments(&mut self) {
        self.requested_payments = false;
        self.pending_payments = None;
    }

    /// Request once per arrival; the dispatch identity rejects late results after switching.
    pub fn load_payments(&mut self, identity: &QualifiedIdentity) -> AppAction {
        if self.requested_payments {
            return AppAction::None;
        }
        self.requested_payments = true;
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadPaymentHistory {
            identity: identity.clone(),
        }));
        let context = BackendTaskContext::for_dispatch(&task);
        self.pending_payments = Some(context.clone());
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Accept only the response belonging to this Home request.
    pub fn record_payments(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        if self.pending_payments.as_ref() == Some(context)
            && let BackendTaskSuccessResult::DashPayPaymentHistory(payments) = result
        {
            self.payments = payments.clone();
            self.pending_payments = None;
        }
    }
}
