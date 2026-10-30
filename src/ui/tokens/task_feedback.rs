//! User notifications for token task outcomes.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

/// The token domain's reactions.
pub(crate) struct TokensFeedback;

impl DomainFeedback for TokensFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        if let BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight = result {
            MessageBanner::set_global(
                feedback.ctx,
                "Token balances are already refreshing. Wait a moment before refreshing again.",
                MessageType::Info,
            );
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::message_banner::global_banner_texts;
    use crate::ui::task_feedback::ResultDelivery;

    fn react(ctx: &egui::Context, result: &BackendTaskSuccessResult) {
        let contexts = std::collections::BTreeMap::new();
        let feedback = Feedback {
            ctx,
            contexts: &contexts,
            delivery: ResultDelivery {
                sender_in_view: true,
                sender_gone: false,
            },
        };
        TokensFeedback.on_success(&feedback, &BackendTaskContext::Other, result);
    }

    #[test]
    fn a_refresh_already_running_is_reported_once() {
        let ctx = egui::Context::default();
        react(&ctx, &BackendTaskSuccessResult::FetchedTokenBalances);
        assert!(global_banner_texts(&ctx).is_empty());
        react(
            &ctx,
            &BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight,
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("already refreshing"));
    }
}
