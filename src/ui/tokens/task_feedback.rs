//! User notifications for token task outcomes.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::contracts_documents::task_feedback::show_contract_save_warning;
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
        if let BackendTaskSuccessResult::RegisteredTokenContract {
            save_warning: Some(warning),
            ..
        } = result
        {
            show_contract_save_warning(feedback.ctx, warning);
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

    /// A token Platform created but this device could not save is reported
    /// with how to import it, never as a failure to retry.
    #[test]
    fn an_unsaved_token_registration_is_reported_as_a_warning() {
        use crate::backend_task::error::{ContractSaveWarning, TaskError};

        let ctx = egui::Context::default();
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredTokenContract {
                save_warning: None,
                earlier_version: false,
            },
        );
        assert!(global_banner_texts(&ctx).is_empty());

        let contract_id = dash_sdk::platform::Identifier::from([0x42; 32]);
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredTokenContract {
                save_warning: Some(ContractSaveWarning::Token {
                    contract_id,
                    source: std::sync::Arc::new(TaskError::WalletBackendNotYetWired),
                }),
                earlier_version: false,
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("was created on the network"), "{texts:?}");
        assert!(texts[0].contains("Import Token"), "{texts:?}");
        assert!(texts[0].contains(&format!("{contract_id}")), "{texts:?}");
    }

    /// A token Platform holds as an earlier attempt sent it says so.
    #[test]
    fn an_earlier_token_version_created_is_reported() {
        let ctx = egui::Context::default();
        react(
            &ctx,
            &BackendTaskSuccessResult::RegisteredTokenContract {
                save_warning: None,
                earlier_version: true,
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("earlier attempt"), "{texts:?}");
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
