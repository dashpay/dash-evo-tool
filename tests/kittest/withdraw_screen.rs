//! Kittest coverage for `WithdrawalScreen` key pre-selection.
//!
//! `WithdrawalScreen::new()` used to pre-select a signing key via
//! `Identity::get_first_public_key_matching(Purpose::TRANSFER, ...)`, scanning
//! on-chain keys unfiltered by local private-key presence — a "ghost key" (an
//! on-chain TRANSFER key with no local private material, e.g. a loaded
//! masternode identity with the payout field left blank) could get silently
//! pre-selected even though the signer can never use it. The fix
//! (`identity.default_withdrawal_key()`, TRANSFER-preferred / OWNER-fallback,
//! private-key-backed only) is unit-tested at the model layer in
//! `src/model/qualified_identity/mod.rs::withdrawal_key_tests`. This file
//! verifies the fix holds when `WithdrawalScreen` is actually constructed and
//! rendered — the layer the model-only tests never touched.
//!
//! `selected_key` is a private field, so it can't be asserted directly from
//! this external integration-test crate. Instead:
//! - the ghost-key case is asserted through the screen's *observable*
//!   behavior: the "no signable key" empty state renders instead of the
//!   withdraw form, and (see `ghost_key_leaks_raw_error_banner` below) a
//!   genuine regression this fix newly exposes at the constructor;
//! - the happy-path / owner-fallback cases are asserted through the rendered
//!   ComboBox's `selected_text`, closing the "constructor picks one thing,
//!   widget renders another" gap the original bug lived in.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::ui::helpers::format_key_label;
use dash_evo_tool::ui::identities::withdraw_screen::WithdrawalScreen;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose, SecurityLevel};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Builds an on-chain public key. Mirrors the `key()` helper in
/// `model/qualified_identity/mod.rs::withdrawal_key_tests` so the two test
/// suites stay in lockstep on fixture shape.
fn key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
    let mut k = IdentityPublicKey::random_key(id, Some(id as u64), PlatformVersion::latest());
    k.set_id(id);
    k.set_purpose(purpose);
    k.set_security_level(SecurityLevel::CRITICAL);
    k
}

/// Builds a `QualifiedIdentity` with `on_chain` public keys, of which
/// `with_private` additionally get local `Clear` private material. Same
/// fixture shape as the model-layer `withdrawal_key_tests::build_identity`
/// helper, extended with a live `AppContext`'s network so the screen renders.
fn build_identity(
    app_context: &Arc<AppContext>,
    identity_type: IdentityType,
    on_chain: Vec<IdentityPublicKey>,
    with_private: Vec<IdentityPublicKey>,
) -> QualifiedIdentity {
    let public_keys: BTreeMap<KeyID, IdentityPublicKey> =
        on_chain.into_iter().map(|k| (k.id(), k)).collect();
    let identity = Identity::new_with_id_and_keys(
        Identifier::random(),
        public_keys,
        PlatformVersion::latest(),
    )
    .expect("identity");

    let mut private_keys = BTreeMap::new();
    for k in with_private {
        private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, k.id()),
            (
                QualifiedIdentityPublicKey::from(k),
                PrivateKeyData::Clear([0u8; 32]),
            ),
        );
    }

    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type,
        alias: None,
        private_keys: KeyStorage { private_keys },
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    }
}

/// Mounts `WithdrawalScreen::ui()` directly (it manages its own top/left/central
/// panels, same as when driven through `AppState`) and runs one settle pass.
fn mount_withdrawal_screen(screen: WithdrawalScreen) -> Harness<'static, WithdrawalScreen> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 850.0))
        .build_ui_state(
            |ui, screen: &mut WithdrawalScreen| {
                screen.ui(ui);
            },
            screen,
        );
    harness.run();
    harness
}

/// Builds a fresh, isolated `AppContext` via the same `AppState::new` factory
/// other kittests use, without mounting a root screen.
fn fresh_context() -> (tokio::runtime::Runtime, Arc<AppContext>) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let guard = rt.enter();
    let mut bootstrap = Harness::builder().with_max_steps(20).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("AppState builds")
            .with_animations(false)
    });
    bootstrap.run_steps(5);
    let app_context = bootstrap.state().current_app_context().clone();
    drop(bootstrap);
    drop(guard);
    (rt, app_context)
}

/// Ghost-key repro: an identity whose only on-chain key is a `Purpose::TRANSFER`
/// key with no local private material (like a loaded masternode identity with
/// the payout field left blank). The withdraw form must never render for it —
/// `available_withdrawal_keys()` (private-key-backed only) is empty, so the
/// screen shows the "no eligible key" empty state, not a Confirm-enabled form.
#[test]
fn ghost_transfer_key_shows_no_keys_empty_state_not_a_form() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_context();

        let transfer = key(1, Purpose::TRANSFER);
        let identity = build_identity(
            &app_context,
            IdentityType::Masternode,
            vec![transfer],
            vec![],
        );

        let screen = WithdrawalScreen::new(identity, &app_context);
        let harness = mount_withdrawal_screen(screen);

        assert!(
            harness
                .query_by_label_contains("You do not have any withdrawal keys loaded")
                .is_some(),
            "a ghost-key-only identity must show the no-keys empty state"
        );
        assert!(
            harness
                .query_by_label_contains("Amount to withdraw")
                .is_none(),
            "the withdraw form (amount/address/key-selection) must never render \
             for an identity with no locally-signable withdrawal key"
        );
        assert!(
            harness.query_by_label_contains("Estimated fee").is_none(),
            "the fee-estimate / Confirm section must never render without a signable key"
        );
    });
}

/// Regression surfaced BY the fix: constructing `WithdrawalScreen` for a
/// ghost-key-only identity leaks a raw, technical error banner
/// ("No key provided when getting selected wallet") from
/// `get_selected_wallet()`. Before the fix `selected_key` was almost always
/// `Some(..)` (unfiltered on-chain scan), so this `Err` branch in
/// `get_selected_wallet` was rarely reached; now that `selected_key` correctly
/// becomes `None` whenever no locally-signable key exists, every such
/// identity trips this internal-string leak on screen open — violating this
/// project's own error-message conventions (CLAUDE.md "Error messages": no
/// raw/internal strings in user-facing banners, must be actionable).
#[test]
fn ghost_key_construction_leaks_raw_error_banner() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_context();
        // Bootstrap (AppState::new + a few settle frames) can itself raise
        // unrelated startup banners (e.g. connection-status warnings); clear
        // them so this test observes only what `WithdrawalScreen::new()` adds.
        MessageBanner::clear_all_global(app_context.egui_ctx());

        let transfer = key(1, Purpose::TRANSFER);
        let identity = build_identity(
            &app_context,
            IdentityType::Masternode,
            vec![transfer],
            vec![],
        );

        assert!(
            !MessageBanner::has_global(app_context.egui_ctx()),
            "precondition: no banner before construction"
        );

        let _screen = WithdrawalScreen::new(identity, &app_context);

        assert!(
            MessageBanner::has_global(app_context.egui_ctx()),
            "WithdrawalScreen::new() must not leak get_selected_wallet's raw \
             \"No key provided...\" error as a global banner just from opening \
             the screen for an identity with no signable key"
        );
    });
}

/// Happy path: an identity with a TRANSFER key backed by real local private
/// material. `selected_key` must be `Some` and the rendered key-selection
/// ComboBox must display that exact key — the constructor and the widget must
/// agree, closing the exact gap the original bug lived in.
#[test]
fn private_backed_transfer_key_is_selected_and_rendered_in_combo() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_context();
        MessageBanner::clear_all_global(app_context.egui_ctx());

        let transfer = key(1, Purpose::TRANSFER);
        let identity = build_identity(
            &app_context,
            IdentityType::User,
            vec![transfer.clone()],
            vec![transfer.clone()],
        );

        let screen = WithdrawalScreen::new(identity, &app_context);
        assert!(
            !MessageBanner::has_global(app_context.egui_ctx()),
            "no error banner expected when a private-key-backed key is pre-selected"
        );

        let mut harness = mount_withdrawal_screen(screen);

        assert!(
            harness
                .query_by_label_contains("Amount to withdraw")
                .is_some(),
            "the withdraw form must render when a locally-signable key exists"
        );
        assert!(
            harness
                .query_by_label_contains("You do not have any withdrawal keys loaded")
                .is_none()
        );

        // Reveal key selection (section 3) via "Show Advanced Options".
        harness.get_by_label("Show Advanced Options").click();
        harness.run_steps(3);

        assert!(
            harness
                .query_by_label_contains("3. Select the key to sign with")
                .is_some(),
            "the key-selection section must render once Advanced Options is shown"
        );

        // The ComboBox exposes its current selection via accesskit's `value`
        // (a plain Label like "Withdraw" uses `label`; egui's ComboBox does not).
        let expected_label = format_key_label(&transfer);
        assert!(
            harness.query_by_value(&expected_label).is_some(),
            "the key-selection ComboBox must display the pre-selected TRANSFER \
             key's label ({expected_label}); constructor and widget disagreeing \
             is exactly the class of bug this fix targets"
        );
        assert!(
            harness.query_by_value("Select Key…").is_none(),
            "the combo must not show the unselected placeholder when a key was pre-selected"
        );
    });
}

/// Owner fallback: a masternode-type identity with only a private-key-backed
/// OWNER key, no TRANSFER key at all. `default_withdrawal_key()` must fall
/// back to OWNER, and the rendered ComboBox must reflect that exact key.
#[test]
fn owner_key_fallback_is_selected_and_rendered_in_combo_when_no_transfer_key() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_context();
        MessageBanner::clear_all_global(app_context.egui_ctx());

        let owner = key(2, Purpose::OWNER);
        let identity = build_identity(
            &app_context,
            IdentityType::Masternode,
            vec![owner.clone()],
            vec![owner.clone()],
        );

        let screen = WithdrawalScreen::new(identity, &app_context);
        assert!(
            !MessageBanner::has_global(app_context.egui_ctx()),
            "no error banner expected when the OWNER-fallback key is pre-selected"
        );

        let mut harness = mount_withdrawal_screen(screen);

        assert!(
            harness
                .query_by_label_contains("Amount to withdraw")
                .is_some(),
            "the withdraw form must render for the OWNER-fallback case"
        );

        harness.get_by_label("Show Advanced Options").click();
        harness.run_steps(3);

        let expected_label = format_key_label(&owner);
        assert!(
            harness.query_by_value(&expected_label).is_some(),
            "the key-selection ComboBox must display the pre-selected OWNER \
             fallback key's label ({expected_label})"
        );
        assert!(harness.query_by_value("Select Key…").is_none());
    });
}
