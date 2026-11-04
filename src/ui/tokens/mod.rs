pub mod add_token_by_id_screen;
pub mod burn_tokens_screen;
pub mod claim_tokens_screen;
pub mod destroy_frozen_funds_screen;
pub mod direct_token_purchase_screen;
pub mod freeze_tokens_screen;
pub mod mint_tokens_screen;
pub mod pause_tokens_screen;
pub mod resume_tokens_screen;
pub mod set_token_price_screen;
pub(crate) mod task_feedback;
pub mod token_action_screen;
pub mod tokens_screen;
pub mod transfer_tokens_screen;
pub mod unfreeze_tokens_screen;
pub mod update_token_config;
pub mod view_token_claims_screen;

use crate::context::AppContext;
use crate::model::qualified_identity::{QualifiedIdentity, SigningKeyStatus};
use crate::model::user_role::UserRole;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::IdentityPublicKey;

/// Loads local identities, displaying an error banner on failure.
pub fn load_identities_with_banner(app_context: &AppContext) -> Vec<QualifiedIdentity> {
    use crate::ui::components::ResultBannerExt;
    app_context
        .load_local_qualified_identities()
        .or_show_error(app_context.egui_ctx())
        .unwrap_or_default()
}

/// Convenience wrapper for setting an error banner from a screen constructor.
///
/// Used by token screen constructors to report configuration errors
/// (e.g., "Burning is not allowed on this token") during initialization.
pub fn set_error_banner(app_context: &AppContext, msg: &str) {
    MessageBanner::set_global(app_context.egui_ctx(), msg, MessageType::Error);
}

/// Validates that a signing key is selected before dispatching a backend task.
///
/// Returns the signing key on success, or sets a global error banner and returns
/// `None` so callers can bail out early with `let Some(key) = ... else { return; }`.
pub fn validate_signing_key(
    app_context: &AppContext,
    selected_key: Option<&IdentityPublicKey>,
) -> Option<IdentityPublicKey> {
    match selected_key {
        Some(key) => Some(key.clone()),
        None => {
            MessageBanner::set_global(
                app_context.egui_ctx(),
                "No signing key selected",
                MessageType::Error,
            );
            None
        }
    }
}

/// The gate in front of a token action form (mint, burn, freeze, pause,
/// pricing, purchase, configuration updates). Outside developer mode it opens
/// exactly when [`QualifiedIdentity::token_action_key`] picks a key, the key
/// the form's constructor selects, so an open form can always sign.
pub(crate) fn token_action_key_status(
    app_context: &AppContext,
    identity: &QualifiedIdentity,
    contract_id: dash_sdk::platform::Identifier,
) -> SigningKeyStatus {
    if app_context.user_role().at_least(UserRole::Developer) {
        if identity.identity.public_keys().is_empty() {
            SigningKeyStatus::NoneHeld
        } else {
            SigningKeyStatus::Usable
        }
    } else {
        identity.token_action_key_status(contract_id)
    }
}

/// Why a token action form stays closed for `identity` at `status`.
pub(crate) fn token_action_key_message(
    status: SigningKeyStatus,
    identity: &QualifiedIdentity,
) -> String {
    match status {
        SigningKeyStatus::HeldButUnusable => "No key on this device can sign this token action. The identity's keys are disabled, expired, reserved for another app or token, or not available on this device. Add a new key, or load the wallet that holds one, then try again.".to_string(),
        SigningKeyStatus::Usable | SigningKeyStatus::NoneHeld => format!(
            "No authentication keys with CRITICAL security level found for this {identity_type} identity.",
            identity_type = identity.identity_type,
        ),
    }
}
