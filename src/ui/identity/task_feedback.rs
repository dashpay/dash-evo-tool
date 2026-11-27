//! User notifications for identity and DashPay task outcomes.
use super::top_up_identity_screen::finish_top_up;
use crate::backend_task::error::{IdentitySaveWarning, TaskError};
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{
    MessageType,
    components::{BannerHandle, MessageBanner},
};

/// The identity domain's reactions. It tracks every top-up in flight,
/// wherever it was sent from, and confirms one that ends out of the user's
/// sight: a top-up's sender in view says so itself and needs no banner.
pub(crate) struct IdentityFeedback;

impl DomainFeedback for IdentityFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        finish_top_up(
            feedback.ctx,
            context,
            true,
            feedback.delivery.sender_in_view,
        );
        clear_profile_saving_banner_after_success(feedback.ctx, context, result);
        show(feedback.ctx, result);
        Vec::new()
    }

    fn on_error(
        &self,
        feedback: &Feedback<'_>,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        finish_top_up(feedback.ctx, context, false, true);
        clear_profile_saving_banner_after_error(feedback.ctx, context);
        false
    }
}

pub(crate) fn show(ctx: &egui::Context, result: &BackendTaskSuccessResult) -> Option<BannerHandle> {
    let (message, kind) = match result {
        BackendTaskSuccessResult::LoadedIdentity {
            warning: Some(warning),
            ..
        } => return Some(show_identity_save_warning(ctx, warning)),
        BackendTaskSuccessResult::RemovedIdentities {
            network,
            associated_cleanup_failed,
            local_data_cleanup_failed,
            cleanup_deferred,
            ..
        } => {
            let (message, kind) = super::removed_identities_banner(
                *associated_cleanup_failed,
                *cleanup_deferred,
                *local_data_cleanup_failed,
            );
            let network = crate::ui::network_chooser_screen::chooser_network_label(*network);
            (format!("Identity removal on {network}: {message}"), kind)
        }
        BackendTaskSuccessResult::DashPayAddressesRegistered {
            addresses,
            contacts,
            errors,
        } => {
            let message = if *errors == 0 {
                format!("Registered {addresses} DashPay addresses for {contacts} contacts.")
            } else {
                format!(
                    "Registered {addresses} DashPay addresses for {contacts} contacts. {errors} addresses could not be registered."
                )
            };
            (message, MessageType::Success)
        }
        BackendTaskSuccessResult::IdentitiesLoaded { count } => {
            let message = if *count == 1 {
                "Successfully loaded 1 identity from your wallet.".to_owned()
            } else {
                format!("Successfully loaded {count} identities from your wallet.")
            };
            (message, MessageType::Success)
        }
        _ => return None,
    };
    let handle = MessageBanner::set_global(ctx, message, kind);
    if kind == MessageType::Warning {
        handle.disable_auto_dismiss();
    }
    Some(handle)
}

/// Tell the user about an identity save the wallet store could not confirm.
/// Carries no identity id: the tray holds five, this banner never
/// auto-dismisses and dedup is by exact text, so one per identity would evict
/// whatever the user was reading. The id is in the save's log line.
pub(crate) fn show_identity_save_warning(
    ctx: &egui::Context,
    warning: &IdentitySaveWarning,
) -> BannerHandle {
    let banner = MessageBanner::set_global(ctx, warning, MessageType::Warning);
    banner.with_details(warning);
    banner.disable_auto_dismiss();
    ctx.request_repaint();
    banner
}

pub(crate) fn clear_profile_saving_banner_after_error(
    ctx: &egui::Context,
    context: &BackendTaskContext,
) {
    if let Some(identity_id) = context.dashpay_profile_update_identity() {
        crate::ui::identity::settings::clear_profile_saving_banner(ctx, &identity_id);
    }
}

pub(crate) fn clear_profile_saving_banner_after_success(
    ctx: &egui::Context,
    context: &BackendTaskContext,
    result: &BackendTaskSuccessResult,
) {
    if let BackendTaskSuccessResult::DashPayProfileUpdated(snapshot) = result
        && context.dashpay_profile_update_identity() == Some(snapshot.owner)
    {
        crate::ui::identity::settings::clear_profile_saving_banner(ctx, &snapshot.owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::platform::Identifier;

    fn profile_update_context(dispatch_id: u64, identity_byte: u8) -> BackendTaskContext {
        BackendTaskContext::Dispatched {
            dispatch_id,
            operation: Box::new(BackendTaskContext::DashPayProfileUpdate(Identifier::from(
                [identity_byte; 32],
            ))),
        }
    }

    /// A load the GUI started shows the save warning its result carries,
    /// and keeps it up until the user dismisses it.
    #[test]
    fn a_loaded_identity_shows_its_save_warning() {
        use crate::ui::task_feedback::{DomainFeedback, Feedback, ResultDelivery};
        let ctx = egui::Context::default();
        let warning = IdentitySaveWarning::WalletFreeUnconfirmed {
            source: std::sync::Arc::new(TaskError::TaskManagerShuttingDown),
        };
        let result = BackendTaskSuccessResult::LoadedIdentity {
            identity: crate::context::test_support::bare_user_identity(
                Identifier::from([3; 32]),
                Network::Testnet,
            ),
            warning: Some(warning.clone()),
        };
        let feedback = Feedback {
            ctx: &ctx,
            active_network: Network::Testnet,
            contexts: &Default::default(),
            delivery: ResultDelivery {
                sender_in_view: true,
                sender_gone: false,
            },
        };

        IdentityFeedback.on_success(&feedback, &BackendTaskContext::Unknown, &result);

        assert_eq!(
            crate::ui::components::message_banner::global_banner_texts(&ctx),
            vec![warning.to_string()]
        );
    }

    #[test]
    fn profile_update_error_clears_only_its_saving_banner() {
        let ctx = egui::Context::default();
        let identity_id = Identifier::from([1; 32]);
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, identity_id);
        let saving = MessageBanner::set_global(
            &ctx,
            crate::ui::identity::settings::PROFILE_SAVING,
            MessageType::Info,
        );

        clear_profile_saving_banner_after_error(&ctx, &profile_update_context(1, 1));

        assert!(
            saving.elapsed().is_none(),
            "a failed profile update must dismiss its persistent progress banner"
        );
    }

    #[test]
    fn profile_update_success_clears_its_saving_banner() {
        let ctx = egui::Context::default();
        let identity_id = Identifier::from([1; 32]);
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, identity_id);
        let saving = MessageBanner::set_global(
            &ctx,
            crate::ui::identity::settings::PROFILE_SAVING,
            MessageType::Info,
        );

        clear_profile_saving_banner_after_success(
            &ctx,
            &profile_update_context(1, 1),
            &BackendTaskSuccessResult::DashPayProfileUpdated(
                crate::model::dashpay::ProfileSnapshot {
                    network: Network::Testnet,
                    owner: identity_id,
                    revision: 0,
                    profile: None,
                },
            ),
        );

        assert!(
            saving.elapsed().is_none(),
            "a successful profile update must dismiss its persistent progress banner"
        );
    }

    #[test]
    fn unrelated_error_does_not_clear_profile_saving_banner() {
        let ctx = egui::Context::default();
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, Identifier::from([1; 32]));
        let saving = MessageBanner::set_global(
            &ctx,
            crate::ui::identity::settings::PROFILE_SAVING,
            MessageType::Info,
        );

        clear_profile_saving_banner_after_error(&ctx, &BackendTaskContext::Other);

        assert!(saving.elapsed().is_some());
    }

    #[test]
    fn one_identity_error_does_not_clear_another_identity_saving_banner() {
        let ctx = egui::Context::default();
        let identity_a = profile_update_context(1, 1);
        let identity_b = profile_update_context(2, 2);
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, Identifier::from([1; 32]));
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, Identifier::from([2; 32]));
        let saving = MessageBanner::set_global(
            &ctx,
            crate::ui::identity::settings::PROFILE_SAVING,
            MessageType::Info,
        );

        clear_profile_saving_banner_after_error(&ctx, &identity_a);

        assert!(
            saving.elapsed().is_some(),
            "identity A's error must not dismiss identity B's progress banner"
        );

        clear_profile_saving_banner_after_error(&ctx, &identity_b);
        assert!(
            saving.elapsed().is_none(),
            "identity B's error must dismiss identity B's progress banner"
        );
    }

    #[test]
    fn one_identity_success_does_not_clear_another_identity_saving_banner() {
        let ctx = egui::Context::default();
        let identity_a = profile_update_context(1, 1);
        let identity_b = profile_update_context(2, 2);
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, Identifier::from([1; 32]));
        crate::ui::identity::settings::show_profile_saving_banner(&ctx, Identifier::from([2; 32]));
        let saving = MessageBanner::set_global(
            &ctx,
            crate::ui::identity::settings::PROFILE_SAVING,
            MessageType::Info,
        );

        clear_profile_saving_banner_after_success(
            &ctx,
            &identity_a,
            &BackendTaskSuccessResult::DashPayProfileUpdated(
                crate::model::dashpay::ProfileSnapshot {
                    network: Network::Testnet,
                    owner: Identifier::from([1; 32]),
                    revision: 0,
                    profile: None,
                },
            ),
        );

        assert!(
            saving.elapsed().is_some(),
            "identity A's success must not dismiss identity B's progress banner"
        );

        clear_profile_saving_banner_after_success(
            &ctx,
            &identity_b,
            &BackendTaskSuccessResult::DashPayProfileUpdated(
                crate::model::dashpay::ProfileSnapshot {
                    network: Network::Testnet,
                    owner: Identifier::from([2; 32]),
                    revision: 0,
                    profile: None,
                },
            ),
        );
        assert!(
            saving.elapsed().is_none(),
            "identity B's success must dismiss identity B's progress banner"
        );
    }
}
