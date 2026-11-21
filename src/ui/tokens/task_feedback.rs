//! User feedback on token results.
use crate::backend_task::BackendTaskContext;
use crate::backend_task::error::TaskError;
use crate::ui::task_feedback::{DomainFeedback, Feedback};

/// The token domain's reactions.
pub(crate) struct TokensFeedback;

impl DomainFeedback for TokensFeedback {
    /// A refresh after a token operation fails quietly: the operation already
    /// reported its result and published its balance, and the next refresh
    /// reads the rest.
    fn on_error(
        &self,
        _feedback: &Feedback<'_>,
        context: &BackendTaskContext,
        error: &TaskError,
    ) -> bool {
        let quiet = *context.operation() == BackendTaskContext::TokenBalanceRefreshAfterOperation;
        if quiet {
            tracing::debug!(
                ?error,
                "Token balance refresh after a token operation failed; the next refresh reads the balances"
            );
        }
        quiet
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTask;
    use crate::backend_task::tokens::TokenTask;
    use crate::ui::task_feedback::ResultDelivery;

    fn quiet(task: TokenTask) -> bool {
        let ctx = egui::Context::default();
        let feedback = Feedback {
            ctx: &ctx,
            contexts: &Default::default(),
            delivery: ResultDelivery {
                sender_in_view: true,
                sender_gone: false,
            },
        };
        let context = BackendTaskContext::from(&BackendTask::TokenTask(Box::new(task)));
        TokensFeedback.on_error(
            &feedback,
            &context,
            &TaskError::TokenBalanceRefreshInProgress,
        )
    }

    #[test]
    fn only_the_refresh_after_an_operation_fails_quietly() {
        assert!(quiet(TokenTask::RefreshBalancesAfterOperation));
        assert!(!quiet(TokenTask::QueryMyTokenBalances));
    }
}
