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
use crate::model::qualified_identity::{QualifiedIdentity, SigningKeySelection, SigningKeyStatus};
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

/// Whether `identity_id` can send token transactions, for a token screen to
/// show. A failed read leaves the actions enabled; the backend enforces the
/// rule either way.
pub fn load_token_action_availability(
    app_context: &AppContext,
    identity_id: &dash_sdk::platform::Identifier,
) -> crate::model::token::TokenActionAvailability {
    app_context
        .token_action_availability(identity_id)
        .unwrap_or_else(|error| {
            tracing::debug!(%error, "Token action availability unreadable; leaving the actions enabled");
            crate::model::token::TokenActionAvailability::Available
        })
}

/// Re-reads `availability` while it waits for the identity's wallet, so an
/// open screen offers its form once the wallet has loaded.
pub fn recheck_token_action_availability(
    app_context: &AppContext,
    identity_id: &dash_sdk::platform::Identifier,
    availability: &mut crate::model::token::TokenActionAvailability,
) {
    if *availability == crate::model::token::TokenActionAvailability::WalletNotOpen {
        *availability = load_token_action_availability(app_context, identity_id);
    }
}

/// Shows why token actions are unavailable, in place of an action form.
/// Returns whether they are unavailable.
pub fn show_token_actions_unavailable(
    ui: &mut egui::Ui,
    availability: crate::model::token::TokenActionAvailability,
) -> bool {
    let Some(reason) = availability.unavailable_reason() else {
        return false;
    };
    ui.label(reason);
    true
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

/// A token form's signing key for one frame, from
/// [`TokenSigningKey::follow_automatic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FollowedKey {
    /// The gate in front of the form. Outside developer mode it opens exactly
    /// when the automatic choice picks a key, so an open form can always
    /// sign; with [`SigningKeyStatus::ManualChoiceOnly`] it opens onto the
    /// key chooser. Developer mode opens it while the identity holds any key.
    status: SigningKeyStatus,
    /// Whether the selected key changed.
    changed: bool,
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

    /// Shows the key chooser; a key the user picks also moves the signing
    /// wallet to that key's wallet, so its unlock prompt follows the choice.
    #[expect(
        clippy::too_many_arguments,
        reason = "the form's chooser inputs and its signing wallet"
    )]
    fn show_chooser(
        &mut self,
        ui: &mut Ui,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
        selected_wallet: &mut Option<Arc<RwLock<Wallet>>>,
        wallet_open_attempted: &mut bool,
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
        if picked {
            self.manually_selected = true;
            crate::ui::identity::sync_signing_wallet(
                Some(identity),
                self.as_ref(),
                selected_wallet,
                wallet_open_attempted,
            );
        }
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

        *self = Self::automatic(
            automatic_selection(identity, transaction_type, contract_id)
                .key
                .cloned(),
        );
    }

    /// Keeps an automatic choice on the key the selector picks now, so a key
    /// that expired since is never signed with; an explicit choice is left
    /// to [`Self::refresh`]. The form's gate status comes from the same
    /// selection, so it agrees with the key.
    fn follow_automatic(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
    ) -> FollowedKey {
        let (automatic, status) = if app_context.user_role().at_least(UserRole::Developer) {
            let status = if identity.identity.public_keys().is_empty() {
                SigningKeyStatus::NoneHeld
            } else {
                SigningKeyStatus::Usable
            };
            // The status does not depend on the automatic key here.
            let automatic = if self.manually_selected {
                None
            } else {
                automatic_selection(identity, transaction_type, contract_id).key
            };
            (automatic, status)
        } else {
            let selection = automatic_selection(identity, transaction_type, contract_id);
            (selection.key, selection.status)
        };
        let changed = !self.manually_selected && self.selected.as_ref() != automatic;
        if changed {
            self.selected = automatic.cloned();
        }
        FollowedKey { status, changed }
    }

    /// [`Self::follow_automatic`], also moving the signing wallet to the new
    /// key's wallet.
    fn follow_automatic_with_wallet(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
        transaction_type: TransactionType,
        contract_id: Identifier,
        selected_wallet: &mut Option<Arc<RwLock<Wallet>>>,
        wallet_open_attempted: &mut bool,
    ) -> FollowedKey {
        let followed = self.follow_automatic(app_context, identity, transaction_type, contract_id);
        if followed.changed {
            crate::ui::identity::sync_signing_wallet(
                Some(identity),
                self.as_ref(),
                selected_wallet,
                wallet_open_attempted,
            );
        }
        followed
    }
}

/// The key a token form picks without the user's choice, with the status
/// that choice gives.
fn automatic_selection(
    identity: &QualifiedIdentity,
    transaction_type: TransactionType,
    contract_id: Identifier,
) -> SigningKeySelection<'_> {
    match transaction_type {
        TransactionType::TokenClaim | TransactionType::TokenTransfer => {
            identity.single_token_transition_key_selection(contract_id)
        }
        _ => identity.token_action_key_selection(contract_id),
    }
}

/// Why a token action button is disabled while no signing key is selected.
pub(crate) const CHOOSE_SIGNING_KEY: &str = "Choose a signing key to continue.";

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
    use crate::model::token::TokenActionAvailability;
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
            TokenSigningKey::automatic(None)
                .follow_automatic(
                    &ctx,
                    &identity,
                    TransactionType::TokenAction,
                    Identifier::new([0x70; 32]),
                )
                .status,
            SigningKeyStatus::ManualChoiceOnly
        );
    }

    /// A token form gates on the status of the same selection it signs with:
    /// the form opens with a key exactly when that key is selected, and an
    /// explicit choice is kept while the status is still reported.
    #[test]
    fn a_token_forms_status_agrees_with_its_selected_key() {
        use dash_sdk::dpp::identity::Purpose;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_user_role(UserRole::Power);
        let contract_id = Identifier::new([0x70; 32]);
        let mut transfer = auth_key(2, SecurityLevel::CRITICAL);
        transfer.set_purpose(Purpose::TRANSFER);
        let usable = identity_holding(&[auth_key(1, SecurityLevel::CRITICAL), transfer]);
        let group_only = identity_holding(&[bound_to(
            auth_key(1, SecurityLevel::CRITICAL),
            ContractBounds::ContractGroup {
                id: Identifier::new([0x6A; 32]),
            },
        )]);

        for transaction_type in [TransactionType::TokenAction, TransactionType::TokenTransfer] {
            for (case, identity) in [("usable", &usable), ("group only", &group_only)] {
                let mut key = TokenSigningKey::automatic(None);
                let followed = key.follow_automatic(&ctx, identity, transaction_type, contract_id);
                assert_eq!(
                    followed.status == SigningKeyStatus::Usable,
                    key.as_ref().is_some(),
                    "{case}, {transaction_type:?}"
                );
                assert_eq!(followed.changed, key.as_ref().is_some(), "{case}");
                let again = key.follow_automatic(&ctx, identity, transaction_type, contract_id);
                assert_eq!(again.status, followed.status, "{case}");
                assert!(
                    !again.changed,
                    "{case}: the same frame state selects the same key"
                );
            }
        }

        let chosen = usable.identity.public_keys()[&2].clone();
        let mut key = TokenSigningKey {
            selected: Some(chosen.clone()),
            manually_selected: true,
        };
        let followed =
            key.follow_automatic(&ctx, &usable, TransactionType::TokenAction, contract_id);
        assert_eq!(followed.status, SigningKeyStatus::Usable);
        assert!(!followed.changed);
        assert_eq!(key.as_ref(), Some(&chosen));

        ctx.set_user_role(UserRole::Developer);
        let followed =
            key.follow_automatic(&ctx, &group_only, TransactionType::TokenAction, contract_id);
        assert_eq!(followed.status, SigningKeyStatus::Usable);
        assert!(!followed.changed);
        assert_eq!(key.as_ref(), Some(&chosen));
    }

    /// While an identity waits for its wallet, the re-check keeps it waiting
    /// until the wallet has loaded, then makes its actions available.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn availability_recheck_follows_the_wallet_loading() {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let seed = crate::test_support::keys::seed();
        let seed_hash = crate::model::wallet::Wallet::new_from_seed(seed, ctx.network, None, None)
            .expect("build wallet")
            .seed_hash();
        let identity = crate::context::test_support::bare_user_identity(
            dash_sdk::platform::Identifier::from([0x61; 32]),
            ctx.network,
        );
        ctx.insert_local_qualified_identity(&identity, &Some((seed_hash, 0)))
            .expect("link the identity to the wallet");
        let id = identity.identity.id();

        let mut availability = load_token_action_availability(&ctx, &id);
        assert_eq!(availability, TokenActionAvailability::WalletNotOpen);
        recheck_token_action_availability(&ctx, &id, &mut availability);
        assert_eq!(availability, TokenActionAvailability::WalletNotOpen);

        ctx.wallet_backend()
            .expect("backend wired")
            .register_wallet_from_seed(&seed_hash, &seed, None)
            .await
            .expect("register the wallet upstream");
        recheck_token_action_availability(&ctx, &id, &mut availability);
        assert_eq!(availability, TokenActionAvailability::Available);
    }
}
