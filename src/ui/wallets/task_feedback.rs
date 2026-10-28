//! User notifications for wallet task outcomes.
use crate::backend_task::BackendTaskSuccessResult;
use crate::ui::{MessageType, components::MessageBanner};

/// `closed_form` says the form that sent the task is gone and nothing else
/// confirms the outcome, so the banner tells the user how the send ended.
pub(crate) fn on_success(
    ctx: &egui::Context,
    closed_form: bool,
    result: &BackendTaskSuccessResult,
) {
    if let BackendTaskSuccessResult::AssetLockBroadcast { txid } = result {
        MessageBanner::set_global(
            ctx,
            format!("Asset lock transaction broadcast successfully. Transaction ID: {txid}"),
            MessageType::Success,
        );
    }
    if closed_form && let Some(message) = super::send_screen::closed_send_form_message(result) {
        MessageBanner::set_global(ctx, message, MessageType::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::FeeResult;
    use crate::ui::components::message_banner::global_banner_texts;

    fn topped_up() -> BackendTaskSuccessResult {
        let identity = crate::context::test_support::bare_user_identity(
            dash_sdk::platform::Identifier::from([0x61; 32]),
            dash_sdk::dpp::dashcore::Network::Testnet,
        );
        BackendTaskSuccessResult::ToppedUpIdentity(identity, FeeResult::new(1, 1))
    }

    /// A send whose form is gone is reported once by the form's own words;
    /// one still watched by its form, or confirmed elsewhere, is not.
    #[test]
    fn a_closed_forms_send_is_reported_by_the_banner_alone() {
        let ctx = egui::Context::default();
        on_success(&ctx, false, &topped_up());
        assert!(global_banner_texts(&ctx).is_empty());
        on_success(&ctx, true, &topped_up());
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("Identity topped up successfully!"));
    }

    #[test]
    fn a_broadcast_asset_lock_names_its_transaction() {
        let ctx = egui::Context::default();
        on_success(
            &ctx,
            true,
            &BackendTaskSuccessResult::AssetLockBroadcast {
                txid: "ab".repeat(32),
            },
        );
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains(&"ab".repeat(32)));
    }
}
