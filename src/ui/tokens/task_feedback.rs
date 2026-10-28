//! User notifications for token task outcomes.
use crate::backend_task::BackendTaskSuccessResult;
use crate::ui::{MessageType, components::MessageBanner};

pub(crate) fn on_success(ctx: &egui::Context, result: &BackendTaskSuccessResult) {
    if let BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight = result {
        MessageBanner::set_global(
            ctx,
            "Token balances are already refreshing. Wait a moment before refreshing again.",
            MessageType::Info,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::message_banner::global_banner_texts;

    #[test]
    fn a_refresh_already_running_is_reported_once() {
        let ctx = egui::Context::default();
        on_success(&ctx, &BackendTaskSuccessResult::FetchedTokenBalances);
        assert!(global_banner_texts(&ctx).is_empty());
        on_success(
            &ctx,
            &BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight,
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("already refreshing"));
    }
}
