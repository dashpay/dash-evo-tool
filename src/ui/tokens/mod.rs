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
use crate::model::wallet::Wallet;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use std::sync::{Arc, RwLock};

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
/// the form's constructor selects, so an open form can always sign; with
/// [`SigningKeyStatus::ManualChoiceOnly`] it opens onto the key chooser.
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

/// After `identity` was refreshed, picks the automatic signing key with
/// `pick` and its unlock wallet when no key is chosen yet, so a key that
/// became usable since the form opened is the one it signs with. A chosen
/// key, picked automatically or by hand, is kept.
pub(crate) fn pick_key_if_none(
    identity: &QualifiedIdentity,
    key: &mut Option<IdentityPublicKey>,
    wallet: &mut Option<Arc<RwLock<Wallet>>>,
    pick: impl FnOnce(&QualifiedIdentity) -> Option<&IdentityPublicKey>,
) {
    if key.is_some() {
        return;
    }
    *key = pick(identity).cloned();
    *wallet = crate::ui::identity::wallet_for_signing_key(identity, key.as_ref());
}

/// Why a token form opened with its key chooser shown
/// ([`SigningKeyStatus::ManualChoiceOnly`]).
pub(crate) const CONTRACT_GROUP_KEY_NOTE: &str = "No key can be chosen automatically for this action, because the only key that can sign it is limited to a contract group. Choose that key below if the group includes this token's contract.";

/// Why a token action form stays closed for `identity` at `status`.
pub(crate) fn token_action_key_message(
    status: SigningKeyStatus,
    identity: &QualifiedIdentity,
) -> String {
    match status {
        SigningKeyStatus::HeldButUnusable => "No key on this device can sign this token action. The identity's keys are disabled, expired, reserved for another app or token, or not available on this device. Add a new key, or load the wallet that holds one, then try again.".to_string(),
        SigningKeyStatus::Usable
        | SigningKeyStatus::ManualChoiceOnly
        | SigningKeyStatus::NoneHeld => format!(
            "No authentication keys with CRITICAL security level found for this {identity_type} identity.",
            identity_type = identity.identity_type,
        ),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::context::test_support::{auth_key, bound_to, identity_holding};
    use dash_sdk::dpp::identity::SecurityLevel;
    use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
    use dash_sdk::platform::Identifier;

    /// The identity [`identity_holding`] builds, holding only an expired
    /// CRITICAL authentication key `1`: no key can sign a token action.
    pub(crate) fn identity_with_expired_key() -> QualifiedIdentity {
        identity_holding(&[auth_key(1, SecurityLevel::CRITICAL).with_limits(None, Some(1))])
    }

    /// Stores in `ctx` the same identity holding a live key `1` from a wallet
    /// loaded in `ctx`, as a screen reads it on refresh once the key became
    /// usable, and returns that wallet's seed hash.
    pub(crate) fn store_identity_with_usable_wallet_key(
        ctx: &AppContext,
    ) -> crate::model::wallet::WalletSeedHash {
        use crate::wallet_backend::DetKv;
        use crate::wallet_backend::kv_test_support::InMemoryKv;
        use std::sync::{Arc, RwLock};
        let wallet = crate::model::wallet::test_support::open_wallet();
        let seed_hash = wallet.seed_hash();
        ctx.wallet_context()
            .insert_test_wallet(seed_hash, Arc::new(RwLock::new(wallet)));
        let mut identity = identity_holding(&[auth_key(1, SecurityLevel::CRITICAL)]);
        crate::context::test_support::hold_as_wallet_reference(&mut identity, 1, seed_hash);
        ctx.set_det_kv_override_for_test(DetKv::from_store(Arc::new(InMemoryKv::default())));
        ctx.insert_local_qualified_identity(&identity, &Some((seed_hash, 0)))
            .expect("store identity");
        seed_hash
    }

    /// Regression: for a Power user whose only key is bound to a contract
    /// group, the token action gate closed before the Advanced chooser that
    /// offers that key.
    #[test]
    fn a_power_users_contract_group_key_opens_the_chooser() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_user_role(UserRole::Power);
        let identity = identity_holding(&[bound_to(
            auth_key(1, SecurityLevel::CRITICAL),
            ContractBounds::ContractGroup {
                id: Identifier::new([0x6A; 32]),
            },
        )]);
        assert_eq!(
            token_action_key_status(&ctx, &identity, Identifier::new([0x70; 32])),
            SigningKeyStatus::ManualChoiceOnly
        );
    }
}
