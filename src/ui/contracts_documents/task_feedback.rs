//! Completion feedback for contract tasks whose originating screen was closed.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

pub(crate) struct ContractsFeedback;

impl DomainFeedback for ContractsFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        if feedback.delivery.sender_gone
            && matches!(result, BackendTaskSuccessResult::ContractFeesClaimed { .. })
        {
            MessageBanner::set_global(
                feedback.ctx,
                "The fees were paid out.",
                MessageType::Success,
            );
        }
        Vec::new()
    }
}
