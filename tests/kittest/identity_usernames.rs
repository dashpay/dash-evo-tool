//! Kittest coverage for identity usernames (USR-TC-018, 031–038).

use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::OverallConnectionState;
use dash_evo_tool::model::dpns::normalize_dpns_label;
use dash_evo_tool::model::dpns_usernames::{
    RequestPhase, RequestTally, UsernameAvailability, UsernameRequest,
};
use dash_evo_tool::model::fee_estimation::{contest_fee_credits, format_credits_as_dash};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::components::ProgressOverlay;
use dash_evo_tool::ui::identity::home::{self, HomeState};
use dash_evo_tool::ui::identity::profile_cache::ProfileCache;
use dash_evo_tool::ui::identity::register_dpns_name_screen::{
    RegisterDpnsNameScreen, RegisterDpnsNameSource,
};
use dash_evo_tool::ui::identity::settings::SettingsTab;
use dash_evo_tool::ui::identity::username_request_screen::UsernameRequestScreen;
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

fn request(label: &str, phase: RequestPhase) -> UsernameRequest {
    let mut request = UsernameRequest::submitted(
        label,
        1_700_000_000_000,
        dash_evo_tool::model::dpns::ContestDurations {
            total: std::time::Duration::from_secs(14 * 86_400),
            join: std::time::Duration::from_secs(7 * 86_400),
        },
    );
    request.phase = phase;
    request
}

fn mount_settings(app_context: Arc<AppContext>) -> Harness<'static, SettingsTab> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 1200.0))
        .build_ui_state(
            move |ui, tab: &mut SettingsTab| {
                tab.render(ui, &app_context, &mut ProfileCache::default());
            },
            SettingsTab::new(),
        );
    harness.run();
    harness
}

/// USR-TC-020 / 022: every row state renders and no alias stubs remain.
#[test]
fn usernames_card_lists_all_states_without_alias_stubs() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let id = seed_username_identity(
            &app_context,
            0x20,
            "Alice Novak",
            &["alice", "alice-design"],
            0,
            true,
        );
        app_context
            .store_username_requests(
                &id,
                vec![
                    request("ali", RequestPhase::Voting),
                    request("novak", RequestPhase::Joinable),
                    request("al", RequestPhase::Lost),
                    request("aa", RequestPhase::Locked),
                ],
            )
            .expect("store requests");
        let harness = mount_settings(app_context);
        for label in [
            "@alice",
            "@alice-design",
            "Main",
            "@ali",
            "@novak",
            "@al",
            "@aa",
            "Get another username",
        ] {
            assert!(harness.query_by_label(label).is_some(), "missing {label}");
        }
        for text in [
            "Waiting for vote",
            "Open for other requests",
            "Went to someone else",
            "Locked for good",
        ] {
            assert!(
                harness.query_by_label_contains(text).is_some(),
                "missing {text}"
            );
        }
        for stub in [
            "Aliases",
            "Make primary",
            "Remove",
            "Add an alias",
            "View all usernames",
        ] {
            assert!(
                harness.query_by_label(stub).is_none(),
                "stub {stub} remains"
            );
        }
    });
}

/// USR-TC-023: an identity with no username sees the value line and the CTA.
#[test]
fn usernames_card_empty_state() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x23, "Alex", &[], 0, true);
        let harness = mount_settings(app_context);
        assert!(
            harness
                .query_by_label_contains("This identity has no username yet.")
                .is_some()
        );
        assert!(harness.query_by_label("Get a username").is_some());
        assert!(harness.query_by_label("Add a key").is_none());
    });
}

/// USR-TC-024: a view-only identity gets the reason and an `Add a key` link.
#[test]
fn usernames_card_view_only_offers_add_a_key() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        seed_username_identity(&app_context, 0x24, "Viewer", &[], 0, false);
        let harness = mount_settings(app_context);
        assert!(harness.query_by_label("Add a key").is_some());
        assert!(
            harness
                .query_by_label("Add a key to this identity to register usernames.")
                .is_some()
        );
    });
}

fn mount_request(
    app_context: &Arc<AppContext>,
    id: Identifier,
    label: &str,
) -> Harness<'static, UsernameRequestScreen> {
    let screen = UsernameRequestScreen::new(app_context, id, label.to_owned());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 1200.0))
        .build_ui_state(
            |ui, screen: &mut UsernameRequestScreen| {
                screen.ui(ui);
            },
            screen,
        );
    harness.run();
    harness
}

/// USR-TC-029 / 030: timeline, tally, the four rules, and no voter link for Alex.
#[test]
fn request_status_page_explains_the_vote() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let id = seed_username_identity(&app_context, 0x29, "Alice Novak", &[], 0, true);
        let mut pending = request("ali", RequestPhase::Voting);
        pending.tally = RequestTally {
            you: 31,
            others: vec![(Identifier::from([7; 32]), 18)],
            lock: 8,
            abstain: 3,
        };
        app_context
            .store_username_requests(&id, vec![pending])
            .expect("store request");
        let harness = mount_request(&app_context, id, &normalize_dpns_label("ali"));
        for text in [
            "Your request for @ali",
            "Requested ",
            "Community vote. Ends around",
            "Result: Not decided yet.",
            "Leading",
            "Other request (",
            "Lock, so no one gets it",
            "Evonodes count as 4 votes.",
            "becomes yours automatically.",
            "the most recent request wins.",
            "no one can ever register @ali.",
            "The community vote fee isn't returned in any case.",
        ] {
            assert!(
                harness.query_by_label_contains(text).is_some(),
                "missing {text}"
            );
        }
        assert!(
            harness
                .query_by_label("Your nodes can vote on this name")
                .is_none()
        );
    });
}

/// USR-TC-030: a Power user with a voting node sees the link to vote.
#[test]
fn request_status_page_links_voters() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let id = seed_username_identity(&app_context, 0x30, "Alice Novak", &[], 0, true);
        app_context
            .store_username_requests(&id, vec![request("ali", RequestPhase::Voting)])
            .expect("store request");
        let mut node = app_context
            .load_local_user_identities()
            .expect("identities")
            .into_iter()
            .find(|qi| qi.identity.id() == id)
            .expect("seeded");
        let voter = Identity::create_basic_identity(
            Identifier::from([0x31; 32]),
            PlatformVersion::latest(),
        )
        .expect("voter");
        let key = IdentityPublicKey::random_key(9, Some(9), PlatformVersion::latest());
        node.associated_voter_identity = Some((voter, key));
        app_context
            .update_local_qualified_identity(&node)
            .expect("store node");
        app_context.set_user_role(UserRole::Power);
        let harness = mount_request(&app_context, id, &normalize_dpns_label("ali"));
        assert!(
            harness
                .query_by_label("Your nodes can vote on this name")
                .is_some()
        );
    });
}

fn mount_home(app_context: Arc<AppContext>) -> Harness<'static, (HomeState, ProfileCache)> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 1200.0))
        .build_ui_state(
            move |ui, state: &mut (HomeState, ProfileCache)| {
                let _ = home::render(ui, &app_context, &state.0, &mut state.1);
            },
            (HomeState::default(), ProfileCache::default()),
        );
    harness.run();
    harness
}

/// USR-TC-021: "Show as main" changes the shown name locally, with no backend task.
#[test]
fn show_as_main_changes_the_hero_name() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let id = seed_username_identity(&app_context, 0x21, "", &["a-name", "b-name"], 0, true);
        app_context
            .set_main_username(&id, "b-name")
            .expect("set main");
        let identity = app_context
            .load_local_user_identities()
            .expect("identities")
            .into_iter()
            .find(|qi| qi.identity.id() == id)
            .expect("seeded");
        assert_eq!(
            app_context.main_username(&identity).as_deref(),
            Some("b-name")
        );
        let harness = mount_home(app_context);
        assert!(
            harness
                .query_all_by_label_contains("b-name")
                .next()
                .is_some()
        );
        assert!(
            harness
                .query_all_by_label_contains("a-name")
                .next()
                .is_none()
        );
    });
}

/// USR-TC-025 / 026: with no registered name, the pending request shows in the
/// header and as a Home card with the standing sentence.
#[test]
fn home_shows_pending_request_header_and_card() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let id = seed_username_identity(&app_context, 0x25, "Alex", &[], 0, true);
        let mut pending = request("ali", RequestPhase::Voting);
        pending.tally = RequestTally {
            you: 5,
            others: vec![(Identifier::from([9; 32]), 2)],
            lock: 0,
            abstain: 0,
        };
        app_context
            .store_username_requests(&id, vec![pending])
            .expect("store request");
        let harness = mount_home(app_context);
        assert!(harness.query_by_label("@ali").is_some());
        assert!(
            harness
                .query_all_by_label_contains("Waiting for vote")
                .next()
                .is_some()
        );
        assert!(
            harness
                .query_by_label_contains("@ali is waiting for a community vote. It ends around")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("You're leading right now.")
                .is_some()
        );
        assert!(harness.query_by_label("View status").is_some());
    });
}

/// USR-TC-027: an outcome banner shows the first time only.
#[test]
fn outcome_banner_shows_once() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let id = seed_username_identity(&app_context, 0x27, "Alex", &[], 0, true);
        app_context
            .store_username_requests(&id, vec![request("ali", RequestPhase::Won)])
            .expect("store request");
        let banner = "You're @ali. People can now find and pay you by this name.";
        let first = mount_home(app_context.clone());
        assert!(first.query_by_label_contains(banner).is_some());
        drop(first);
        let second = mount_home(app_context);
        assert!(second.query_by_label_contains(banner).is_none());
    });
}
