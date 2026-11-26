//! User notifications for token task outcomes.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

/// The token domain's reactions. A token send whose form is gone is
/// confirmed by a banner, since no form is left to show its success.
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
        if let BackendTaskSuccessResult::TokenCreationOutcome(outcome) = result {
            use crate::model::token_creator::registration::RegistrationStatus;
            let (text, kind) = if outcome.record.status == RegistrationStatus::Confirmed {
                (
                    "The token was created. Open Token Creator to view the saved receipt.",
                    MessageType::Success,
                )
            } else {
                (
                    "Token creation is not yet confirmed. Open Token Creator and check its status before making another attempt.",
                    MessageType::Info,
                )
            };
            MessageBanner::set_global(feedback.ctx, text, kind).disable_auto_dismiss();
        }
        if feedback.delivery.sender_gone
            && let BackendTaskSuccessResult::TransferredTokens(_) = result
        {
            MessageBanner::set_global(
                feedback.ctx,
                "Tokens sent successfully.",
                MessageType::Success,
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
        react_with(ctx, false, result);
    }

    fn react_with(ctx: &egui::Context, sender_gone: bool, result: &BackendTaskSuccessResult) {
        let contexts = std::collections::BTreeMap::new();
        let feedback = Feedback {
            ctx,
            contexts: &contexts,
            delivery: ResultDelivery {
                sender_in_view: !sender_gone,
                sender_gone,
            },
        };
        TokensFeedback.on_success(&feedback, &BackendTaskContext::Other, result);
    }

    /// A token send whose form is gone is confirmed by a banner; one whose
    /// form still takes the result is confirmed by the form alone.
    #[test]
    fn a_closed_forms_token_send_is_confirmed_by_a_banner() {
        let sent =
            BackendTaskSuccessResult::TransferredTokens(crate::backend_task::FeeResult::new(1, 1));
        let ctx = egui::Context::default();
        react_with(&ctx, false, &sent);
        assert!(global_banner_texts(&ctx).is_empty());
        react_with(&ctx, true, &sent);
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts, vec!["Tokens sent successfully.".to_string()]);
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
