//! Identity Home state, including correlated payment previews and receive sharing.

use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::qr_code_image::QrCodeImage;
use crate::ui::state::PendingRequest;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

/// Cached Home content for the currently selected identity.
#[derive(Default)]
pub struct HomeState {
    pub dismissed_checklist: bool,
    pub receive_open: bool,
    pub receive_qr: Option<QrCodeImage>,
    pub payments: Vec<(String, String, u64, bool, String)>,
    requested_payments: bool,
    pending_payments: PendingRequest,
    /// The identity this content belongs to, once Home has rendered it.
    owner: Option<(Network, Identifier)>,
}

impl HomeState {
    /// Bind the content to the identity on screen. The selection can change
    /// without the hub hearing of it, so Home calls this before it renders;
    /// content of another identity is dropped, the same identity keeps its own.
    pub fn reconcile_owner(&mut self, identity: &QualifiedIdentity) {
        let owner = Some((identity.network, identity.identity.id()));
        if self.owner != owner {
            *self = Self {
                owner,
                ..Self::default()
            };
        }
    }

    /// Refresh on arrival while retaining the visible preview.
    pub fn refresh_payments(&mut self) {
        self.requested_payments = false;
        self.pending_payments.clear();
    }

    /// Whether the payment history has not been asked for since its last refresh.
    pub fn payments_load_is_due(&self) -> bool {
        !self.requested_payments
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
        self.pending_payments.dispatch(task)
    }

    /// Accept only the response belonging to this Home request.
    pub fn record_payments(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        if let BackendTaskSuccessResult::DashPayPaymentHistory(payments) = result
            && self.pending_payments.settle(context).is_some()
        {
            self.payments = payments.clone();
        }
    }
}
