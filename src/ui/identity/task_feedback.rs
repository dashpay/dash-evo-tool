//! User notifications for identity and DashPay task outcomes.
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult};
use crate::ui::{
    MessageType,
    components::{BannerHandle, MessageBanner},
};

pub(crate) fn show(ctx: &egui::Context, result: &BackendTaskSuccessResult) -> Option<BannerHandle> {
    let (message, kind) = match result {
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
