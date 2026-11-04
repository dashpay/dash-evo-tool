//! User notifications for Settings task outcomes, raised wherever the
//! screen is, and for a task of a network no longer active.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

/// The Settings screen's reactions.
pub(crate) struct SettingsFeedback;

impl DomainFeedback for SettingsFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        match result {
            BackendTaskSuccessResult::NetworkDatabaseCleared { network } => {
                let network = super::chooser_network_label(*network);
                MessageBanner::set_global(
                    feedback.ctx,
                    format!("Cleared {network} database. Restart or resync to rebuild state."),
                    MessageType::Success,
                );
            }
            BackendTaskSuccessResult::PreviousVersionRestored(summary) => {
                let message_type = if summary.has_problems() {
                    MessageType::Warning
                } else {
                    MessageType::Success
                };
                MessageBanner::set_global(feedback.ctx, summary.user_message(), message_type);
            }
            _ => {}
        }
        Vec::new()
    }
}
