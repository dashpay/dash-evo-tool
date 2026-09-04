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
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::ui::helpers::format_key_label;
use dash_evo_tool::ui::identity::withdraw_screen::WithdrawalScreen;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{Identity, KeyID, KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::BinaryData;
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

/// Builds the on-chain TRANSFER key a masternode's registered Core payout
/// address is derived from — `ECDSA_HASH160` over `hash`, the shape
/// `QualifiedIdentity::masternode_payout_address` matches on.
fn payout_key(id: KeyID, hash: [u8; 20]) -> IdentityPublicKey {
    let mut k = key(id, Purpose::TRANSFER);
    k.set_key_type(KeyType::ECDSA_HASH160);
    k.set_data(BinaryData::new(hash.to_vec()));
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
        private_keys: KeyStorage::from(private_keys),
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
    crate::support::wait_for_screens(&mut bootstrap);
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
                .query_by_label_contains("This identity has no loaded key")
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

/// Regression lock for a bug this fix (default_withdrawal_key) briefly
/// introduced and a follow-up fix (2edbc18e, "skip wallet resolution when no
/// signable key exists") closed: constructing `WithdrawalScreen` for a
/// ghost-key-only identity used to leak a raw, technical error banner
/// ("No key provided when getting selected wallet") from
/// `get_selected_wallet()`. Before the original fix `selected_key` was almost
/// always `Some(..)` (unfiltered on-chain scan), so that `Err` branch in
/// `get_selected_wallet` was rarely reached; once `selected_key` correctly
/// started becoming `None` whenever no locally-signable key exists, every such
/// identity tripped this internal-string leak on screen open — violating this
/// project's own error-message conventions (CLAUDE.md "Error messages": no
/// raw/internal strings in user-facing banners, must be actionable).
/// `WithdrawalScreen::new()` now only calls `get_selected_wallet` when
/// `selected_key` is `Some`, making that `Err` path structurally unreachable
/// here rather than merely avoided by luck — and the no-keys empty state must
/// still render correctly for the same identity.
#[test]
fn ghost_key_construction_does_not_leak_raw_error_banner() {
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

        let screen = WithdrawalScreen::new(identity, &app_context);

        assert!(
            !MessageBanner::has_global(app_context.egui_ctx()),
            "WithdrawalScreen::new() must not leak get_selected_wallet's raw \
             \"No key provided...\" error as a global banner just from opening \
             the screen for an identity with no signable key (regression: \
             2edbc18e guards the call on selected_key being Some)"
        );

        // The no-keys empty state must still render correctly for this identity
        // — the fix must not have traded the banner leak for a broken empty state.
        let harness = mount_withdrawal_screen(screen);
        assert!(
            harness
                .query_by_label_contains("This identity has no loaded key")
                .is_some(),
            "the no-keys empty state must still render after the banner-leak fix"
        );
        assert!(
            harness
                .query_by_label_contains("Amount to withdraw")
                .is_none(),
            "the withdraw form must still not render for a ghost-key-only identity"
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
                .query_by_label_contains("This identity has no loaded key")
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

/// TC-FR9-06 / MN-007 — an owner-key withdrawal's destination is fixed to the
/// node's registered Core payout address and is never user-editable, at ANY
/// role. The address field used to stay a free-text `TextEdit` whenever the
/// role was Power or above — and since the Masternodes page (the only route to
/// this screen with an owner key selected) is itself gated at Power, that
/// escape hatch was open to *every* user who could reach it, contradicting both
/// the user story and the test spec. The backend's
/// `OwnerKeyWithdrawalNotAllowed` guard only fired after the user had typed an
/// address, confirmed, and submitted.
#[test]
fn owner_key_destination_is_locked_at_every_role() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_context();
        MessageBanner::clear_all_global(app_context.egui_ctx());

        // Only the OWNER key is signable locally; the payout key is on-chain
        // only, so `default_withdrawal_key()` falls back to OWNER while
        // `masternode_payout_address()` still resolves the forced destination.
        let owner = key(1, Purpose::OWNER);
        let payout = payout_key(2, [0x11; 20]);
        let identity = build_identity(
            &app_context,
            IdentityType::Masternode,
            vec![owner.clone(), payout],
            vec![owner],
        );
        let payout_address = identity
            .masternode_payout_address(app_context.network())
            .expect("the fixture registers a payout address");

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            app_context.set_user_role(role);
            let screen = WithdrawalScreen::new(identity.clone(), &app_context);
            let harness = mount_withdrawal_screen(screen);

            assert!(
                harness
                    .query_by_label_contains(&format!(
                        "Masternode payout address: {payout_address}"
                    ))
                    .is_some(),
                "{role:?}: the forced payout destination must be shown"
            );
            assert!(
                harness.query_by_label("Address:").is_none(),
                "{role:?}: an owner-key withdrawal must not offer a free-text \
                 destination field — the destination is fixed to the payout address"
            );
            assert!(
                harness
                    .query_by_label_contains("Leave empty to use masternode payout address")
                    .is_none(),
                "{role:?}: the opt-out hint belongs to the editable field, which \
                 must not render for an owner key"
            );
        }
    });
}

/// TC-FR9-07 — the mirror image: withdrawing with the transfer/payout key
/// accepts any user-chosen address. The owner-key lock must not over-reach and
/// freeze the destination for every masternode withdrawal.
#[test]
fn transfer_key_destination_stays_editable() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_context();
        MessageBanner::clear_all_global(app_context.egui_ctx());
        app_context.set_user_role(UserRole::Power);

        // Both keys signable: TRANSFER wins, so the destination is free.
        let owner = key(1, Purpose::OWNER);
        let payout = payout_key(2, [0x11; 20]);
        let identity = build_identity(
            &app_context,
            IdentityType::Masternode,
            vec![owner.clone(), payout.clone()],
            vec![owner, payout],
        );

        let screen = WithdrawalScreen::new(identity, &app_context);
        let harness = mount_withdrawal_screen(screen);

        assert!(
            harness.query_by_label("Address:").is_some(),
            "a transfer/payout-key withdrawal must keep the free-text destination field"
        );
    });
}
