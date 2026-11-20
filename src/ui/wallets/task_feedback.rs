//! User notifications for wallet task outcomes.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

/// The wallet domain's reactions. A send whose form is gone is reported by
/// the banner alone, in the form's own words; a top-up is the identity
/// domain's to confirm, so [`closed_send_form_message`] leaves it out.
pub(crate) struct WalletsFeedback;

impl DomainFeedback for WalletsFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        if let BackendTaskSuccessResult::AssetLockBroadcast { txid } = result {
            MessageBanner::set_global(
                feedback.ctx,
                format!("Asset lock transaction broadcast successfully. Transaction ID: {txid}"),
                MessageType::Success,
            );
        }
        if feedback.delivery.sender_gone
            && let Some(message) = closed_send_form_message(result)
        {
            MessageBanner::set_global(feedback.ctx, message, MessageType::Success);
        }
        Vec::new()
    }
}

/// What to tell the user about a send whose form was closed before it
/// finished: the text the form would have shown, or `None` for a result that
/// does not complete a send. A top-up is left out: the identity domain
/// tracks every top-up in flight and confirms one that ends out of sight.
pub(crate) fn closed_send_form_message(result: &BackendTaskSuccessResult) -> Option<String> {
    match result {
        BackendTaskSuccessResult::ToppedUpIdentity(..) => None,
        result => super::send_screen::closed_send_form_message(result),
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

    fn transferred() -> BackendTaskSuccessResult {
        BackendTaskSuccessResult::TransferredCredits(FeeResult::new(1, 1))
    }

    fn react(ctx: &egui::Context, sender_gone: bool, result: &BackendTaskSuccessResult) {
        let contexts = std::collections::BTreeMap::new();
        let feedback = Feedback {
            ctx,
            contexts: &contexts,
            delivery: crate::ui::task_feedback::ResultDelivery {
                sender_in_view: !sender_gone,
                sender_gone,
            },
        };
        WalletsFeedback.on_success(&feedback, &BackendTaskContext::Other, result);
    }

    /// A send whose form is gone is reported once by the form's own words;
    /// one still watched by its form is not.
    #[test]
    fn a_closed_forms_send_is_reported_by_the_banner_alone() {
        let ctx = egui::Context::default();
        react(&ctx, false, &transferred());
        assert!(global_banner_texts(&ctx).is_empty());
        react(&ctx, true, &transferred());
        let texts = global_banner_texts(&ctx);
        assert_eq!(texts.len(), 1, "{texts:?}");
        assert!(texts[0].contains("Credits transferred successfully!"));
    }

    /// A top-up from a closed form is the identity domain's to confirm: the
    /// wallet domain says nothing, whether the form is gone or not.
    #[test]
    fn a_closed_forms_top_up_is_left_to_the_identity_domain() {
        let ctx = egui::Context::default();
        react(&ctx, true, &topped_up());
        assert!(global_banner_texts(&ctx).is_empty());
        assert_eq!(closed_send_form_message(&topped_up()), None);
    }

    #[test]
    fn a_broadcast_asset_lock_names_its_transaction() {
        let ctx = egui::Context::default();
        react(
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
