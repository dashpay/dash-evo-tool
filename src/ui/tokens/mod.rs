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

use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::identity_key_usability::{SigningScope, key_caveats, now_ms};
use crate::model::qualified_identity::{QualifiedIdentity, SigningKeyStatus};
use crate::model::user_role::UserRole;
use crate::model::wallet::Wallet;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::helpers::{
    TransactionType, add_key_chooser_with_selection, offered_for_manual_choice,
};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui::Ui;
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
    contract_id: Identifier,
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

/// Keeps the user's explicit signing key choice across identity refreshes.
struct TokenSigningKey {
    selected: Option<IdentityPublicKey>,
    manually_selected: bool,
}

impl TokenSigningKey {
    fn automatic(selected: Option<IdentityPublicKey>) -> Self {
        Self {
            selected,
            manually_selected: false,
        }
    }

    fn as_ref(&self) -> Option<&IdentityPublicKey> {
        self.selected.as_ref()
    }

    fn show_chooser(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
    ) -> AppAction {
        let (action, picked) = add_key_chooser_with_selection(
            ui,
            app_context,
            identity,
            &mut self.selected,
            transaction_type,
            &transaction_type.allowed_security_levels(),
            SigningScope::ContractWide { contract_id },
        );
        self.manually_selected |= picked;
        action
    }

    fn refresh(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
    ) {
        let scope = SigningScope::ContractWide { contract_id };
        if self.manually_selected
            && let Some(live) = self
                .selected
                .as_ref()
                .and_then(|key| identity.current_public_key(key))
            && identity.holds_obtainable_key(live)
            && !live.is_disabled()
            && offered_for_manual_choice(&key_caveats(live, scope, now_ms()))
            && (app_context.user_role().at_least(UserRole::Developer)
                || (transaction_type
                    .allowed_purposes()
                    .contains(&live.purpose())
                    && transaction_type
                        .allowed_security_levels()
                        .contains(&live.security_level())
                    && transaction_type
                        .allowed_key_types()
                        .is_none_or(|types| types.contains(&live.key_type()))))
        {
            self.selected = Some(live.clone());
            return;
        }

        *self = Self::automatic(automatic_key(identity, transaction_type, contract_id).cloned());
    }

    /// Keeps an automatic choice on the key the selector picks now, so a key
    /// that expired since is never signed with; an explicit choice is left
    /// to [`Self::refresh`]. Returns whether the selection changed.
    fn follow_automatic(
        &mut self,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
    ) -> bool {
        let automatic = automatic_key(identity, transaction_type, contract_id);
        if self.manually_selected || self.selected.as_ref() == automatic {
            return false;
        }
        self.selected = automatic.cloned();
        true
    }

    /// [`Self::follow_automatic`], also moving the signing wallet to the new
    /// key's wallet. Returns whether the selection changed.
    fn follow_automatic_with_wallet(
        &mut self,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
        selected_wallet: &mut Option<Arc<RwLock<Wallet>>>,
        wallet_open_attempted: &mut bool,
    ) -> bool {
        let changed = self.follow_automatic(identity, transaction_type, contract_id);
        if changed {
            crate::ui::identity::sync_signing_wallet(
                Some(identity),
                self.as_ref(),
                selected_wallet,
                wallet_open_attempted,
            );
        }
        changed
    }
}

/// The key a token form picks without the user's choice.
fn automatic_key(
    identity: &QualifiedIdentity,
    transaction_type: TransactionType,
    contract_id: Identifier,
) -> Option<&IdentityPublicKey> {
    match transaction_type {
        TransactionType::TokenClaim | TransactionType::TokenTransfer => {
            identity.single_token_transition_key(contract_id)
        }
        _ => identity.token_action_key(contract_id),
    }
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
    use std::sync::RwLock;

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
        store_identity_with_wallet_key(
            ctx,
            identity_holding(&[auth_key(1, SecurityLevel::CRITICAL)]),
            1,
        )
    }

    pub(super) fn store_identity_with_wallet_key(
        ctx: &AppContext,
        mut identity: QualifiedIdentity,
        key_id: u32,
    ) -> crate::model::wallet::WalletSeedHash {
        use crate::wallet_backend::DetKv;
        use crate::wallet_backend::kv_test_support::InMemoryKv;
        let wallet = crate::model::wallet::test_support::open_wallet();
        let seed_hash = wallet.seed_hash();
        ctx.wallet_context()
            .insert_test_wallet(seed_hash, Arc::new(RwLock::new(wallet)));
        crate::context::test_support::hold_as_wallet_reference(&mut identity, key_id, seed_hash);
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
