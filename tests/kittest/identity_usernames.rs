//! Kittest coverage for identity usernames (USR-TC-018, 031–038).

use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::OverallConnectionState;
use dash_evo_tool::model::dpns_usernames::UsernameAvailability;
use dash_evo_tool::model::fee_estimation::{contest_fee_credits, format_credits_as_dash};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::components::ProgressOverlay;
use dash_evo_tool::ui::identity::register_dpns_name_screen::{
    RegisterDpnsNameScreen, RegisterDpnsNameSource,
};
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{Identity, Purpose, SecurityLevel};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Seed a selected identity with an authentication key this device can sign with.
pub(crate) fn seed_username_identity(
    app_context: &Arc<AppContext>,
    byte: u8,
    alias: &str,
    names: &[&str],
    balance: u64,
    with_key: bool,
) -> Identifier {
    let mut key =
        IdentityPublicKey::random_key(1, Some(u64::from(byte)), PlatformVersion::latest());
    key.set_id(1);
    key.set_purpose(Purpose::AUTHENTICATION);
    key.set_security_level(SecurityLevel::HIGH);
    let mut identity = Identity::new_with_id_and_keys(
        Identifier::from([byte; 32]),
        BTreeMap::from([(key.id(), key.clone())]),
        PlatformVersion::latest(),
    )
    .expect("identity");
    identity.set_balance(balance);
    let id = identity.id();
    let mut private_keys = BTreeMap::new();
    if with_key {
        private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key),
                PrivateKeyData::InVault,
            ),
        );
    }
    let qualified = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(alias.to_owned()),
        private_keys: KeyStorage::from(private_keys),
        dpns_names: names
            .iter()
            .map(|name| DPNSNameInfo {
                name: (*name).to_owned(),
                acquired_at: 0,
            })
            .collect(),
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    };
    app_context
        .insert_local_qualified_identity(&qualified, &None)
        .expect("seed identity");
    app_context.set_selected_identity(Some(id));
    id
}

fn mount(screen: RegisterDpnsNameScreen) -> Harness<'static, RegisterDpnsNameScreen> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(
            |ui, screen: &mut RegisterDpnsNameScreen| {
                screen.ui(ui);
            },
            screen,
        );
    harness.run();
    harness
}

fn answered(
    app_context: &Arc<AppContext>,
    label: &str,
    availability: UsernameAvailability,
) -> RegisterDpnsNameScreen {
    let mut screen = RegisterDpnsNameScreen::new(app_context, RegisterDpnsNameSource::Identities);
    screen.type_label_for_test(label);
    let _ = screen.flush_debounce_for_test();
    screen.display_task_result(BackendTaskSuccessResult::UsernameAvailability {
        label: label.to_owned(),
        availability,
    });
    screen
}

/// USR-TC-031: opened from an identity, no identity picker; the title names it.
#[test]
fn get_username_names_the_identity_without_a_picker() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x31, "Alice Novak", &["alice"], 0, true);
        let harness = mount(RegisterDpnsNameScreen::new(
            &app_context,
            RegisterDpnsNameSource::Identities,
        ));
        assert!(
            harness
                .query_by_label_contains("For Alice Novak. People will use it to find and pay you.")
                .is_some()
        );
        assert!(harness.query_by_label("Get another username").is_some());
        assert!(harness.query_by_label("Identity:").is_none());
        assert!(harness.query_by_label_contains("Select Identity").is_none());
    });
}

/// USR-TC-032 / 033: the answered row renders alone, with suggestions for a vote.
#[test]
fn availability_row_and_suggestions_render() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x32, "Alice Novak", &[], 0, true);
        let harness = mount(answered(
            &app_context,
            "nova",
            UsernameAvailability::Joinable {
                contenders: 1,
                join_end: 0,
            },
        ));
        assert!(
            harness
                .query_by_label_contains("1 other person already asked for @nova.")
                .is_some()
        );
        assert!(harness.query_by_label_contains("is available").is_none());
        assert!(harness.query_by_label("Try one without a vote:").is_some());
        assert!(harness.query_by_label("@nova2").is_some());
    });
}

/// USR-TC-018: a pay-time re-check that finds the name locked returns to the
/// choose step with the locked row; nothing is dispatched.
#[test]
fn pay_time_recheck_returns_to_choose_with_locked_row() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x18, "Alice Novak", &[], 0, true);
        let mut screen = answered(&app_context, "alice", UsernameAvailability::NeedsVote);
        screen.open_confirm_for_test();
        let ctx = egui::Context::default();
        screen.raise_progress_overlay_for_test(&ctx);
        let handled = screen.display_task_error(&TaskError::UsernameNoLongerAvailable {
            availability: UsernameAvailability::Locked,
        });
        assert!(handled, "the screen explains the change inline");
        assert!(!ProgressOverlay::has_global(&ctx));
        assert!(!screen.is_confirming_for_test());
        let harness = mount(screen);
        assert!(
            harness
                .query_by_label_contains(
                    "@alice can't be registered. A community vote locked this name for good."
                )
                .is_some()
        );
    });
}

/// USR-TC-035 / 036 / 037: confirm rows, low balance, and the sync gate.
#[test]
fn confirm_step_shows_fees_low_balance_and_sync_gate() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x35, "Alice Novak", &[], 0, true);
        let mut screen = answered(&app_context, "nova", UsernameAvailability::NeedsVote);
        screen.open_confirm_for_test();
        let mut harness = mount(screen);

        let contest_fee =
            format_credits_as_dash(contest_fee_credits(app_context.sdk_platform_version()));
        assert!(
            harness
                .query_by_label("Community vote fee (not returned)")
                .is_some()
        );
        assert!(harness.query_by_label(&contest_fee).is_some());
        assert!(harness.query_by_label("Community vote").is_some());
        assert!(
            harness
                .query_by_label_contains(&format!(
                    "This identity has {}. Add at least",
                    format_credits_as_dash(0)
                ))
                .is_some()
        );
        assert!(harness.query_by_label("Top up").is_some());
        assert!(harness.query_by_label_contains("deposit").is_none());

        assert_ne!(
            app_context.connection_status().overall_state(),
            OverallConnectionState::Synced
        );
        harness.get_by_label_contains("and request @nova").click();
        harness.run();
        assert!(
            harness.state().is_confirming_for_test(),
            "an unsynced, underfunded identity must not start paying"
        );
    });
}

/// USR-FR-005: an identity without a signing key is told how to proceed.
#[test]
fn view_only_identity_is_told_to_add_a_key() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x05, "Viewer", &[], 0, false);
        let harness = mount(RegisterDpnsNameScreen::new(
            &app_context,
            RegisterDpnsNameSource::Identities,
        ));
        assert!(
            harness
                .query_by_label_contains("Add a key to this identity to register usernames.")
                .is_some()
        );
    });
}
