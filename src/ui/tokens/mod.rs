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
pub mod tokens_screen;
pub mod transfer_tokens_screen;
pub mod unfreeze_tokens_screen;
pub mod update_token_config;
pub mod view_token_claims_screen;

use crate::context::AppContext;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use dash_sdk::platform::IdentityPublicKey;

/// Validates that a signing key is selected before dispatching a backend task.
///
/// Returns the signing key on success, or sets a global error banner and returns
/// `None` so callers can bail out early with `let Some(key) = ... else { return; }`.
pub fn validate_signing_key(
    app_context: &AppContext,
    selected_key: &Option<IdentityPublicKey>,
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
