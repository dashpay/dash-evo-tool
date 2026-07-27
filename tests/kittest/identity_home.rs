use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::Screen;
use dash_evo_tool::ui::identity::home::{self, HomeOutcome, HomeState};
use dash_evo_tool::ui::identity::profile_cache::{ProfileCache, ProfileFields};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose, SecurityLevel};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;
use std::sync::Arc;

fn key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
    let mut key = IdentityPublicKey::random_key(id, Some(id as u64), PlatformVersion::latest());
    key.set_id(id);
    key.set_purpose(purpose);
    key.set_security_level(SecurityLevel::CRITICAL);
    key
}

fn seed_identity(
    app_context: &Arc<AppContext>,
    on_chain: Vec<IdentityPublicKey>,
    with_private: Vec<IdentityPublicKey>,
) -> Identifier {
    let public_keys = on_chain.into_iter().map(|key| (key.id(), key)).collect();
    let identity = Identity::new_with_id_and_keys(
        Identifier::from([0xA5; 32]),
        public_keys,
        PlatformVersion::latest(),
    )
    .expect("identity");
    let identity_id = identity.id();

    let mut private_keys = BTreeMap::new();
    for key in with_private {
        private_keys.insert(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, key.id()),
            (
                QualifiedIdentityPublicKey::from(key),
                PrivateKeyData::InVault,
            ),
        );
    }

    let qualified_identity = QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("Home test identity".to_string()),
        private_keys: KeyStorage { private_keys },
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: app_context.network(),
    };

    app_context
        .insert_local_qualified_identity(&qualified_identity, &None)
        .expect("seed identity");
    app_context.set_selected_identity(Some(identity_id));
    identity_id
}

/// Harness state for `home::render`: its own mutable inputs (`HomeState`,
/// `ProfileCache`) plus the `(AppAction, HomeOutcome)` pair the most recent
/// render produced. Capturing the result (rather than discarding it, as a
/// prior version of this harness did) is what lets tests assert a clicked
/// button actually routes to its intended destination, not merely that the
/// click was accepted.
type HomeHarnessState = (HomeState, ProfileCache, Option<(AppAction, HomeOutcome)>);

fn mount_home(app_context: Arc<AppContext>) -> Harness<'static, HomeHarnessState> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_ui_state(
            move |ui, state: &mut HomeHarnessState| {
                let result = home::render(ui, &app_context, &state.0, &mut state.1);
                // `Harness::run`/`step` internally renders extra idle frames
                // (it loops until no immediate repaint is pending), and
                // `home::render` resets its `(AppAction, HomeOutcome)` pair
                // to the no-op value on every call. Only overwrite the
                // captured result on a real (non-no-op) frame, so a click's
                // outcome survives whatever idle re-renders follow it in the
                // same `run()`/`step()` call.
                if !matches!(result.0, AppAction::None) || result.1 != HomeOutcome::None {
                    state.2 = Some(result);
                }
            },
            (HomeState::default(), ProfileCache::default(), None),
        );
    harness.run();
    harness
}

/// The `(AppAction, HomeOutcome)` produced by the most recent render — the
/// frame's result of processing whichever button was clicked before
/// `harness.run()`.
fn last_result<'a>(
    harness: &'a Harness<'static, HomeHarnessState>,
) -> &'a (AppAction, HomeOutcome) {
    harness
        .state()
        .2
        .as_ref()
        .expect("home::render must run at least once before a result is available")
}

#[test]
fn home_send_to_wallet_disabled_for_everyday_without_local_withdrawal_key() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        let button = harness.get_by_label("Send to wallet");
        assert!(button.accesskit_node().is_disabled());

        button.hover();
        harness.run();
        assert!(
            harness
                .query_by_label_contains(
                    "no key available for withdrawal in the current interface mode"
                )
                .is_some(),
            "the disabled tooltip must explain the role-aware capability requirement"
        );
    });
}

/// An on-chain-only key (no locally held private material) can never be
/// used to sign a withdrawal — `resolve_withdrawal_signing_key` enforces
/// this for every role, Developer included, since no signing override is
/// wired into the withdrawal backend task. Enabling the button here used to
/// route Developer-role users into a `WithdrawalScreen` that rendered no
/// form at all (`selected_key` stayed `None`); it must stay disabled instead.
#[test]
fn home_send_to_wallet_disabled_for_developer_with_on_chain_key_only() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Developer);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let harness = mount_home(app_context);
        let button = harness.get_by_label("Send to wallet");
        assert!(button.accesskit_node().is_disabled());
    });
}

#[test]
fn home_send_to_wallet_enabled_for_everyday_with_local_transfer_key() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let harness = mount_home(app_context);
        let button = harness.get_by_label("Send to wallet");
        assert!(!button.accesskit_node().is_disabled());
    });
}

/// The prior version of this suite queried only `Send to wallet`, so `render`
/// could omit `Add funds` / `Send to another identity` / `Add contact`, or
/// render one of them twice, without failing anything here. Assert the full
/// four-button row is present, each exactly once.
#[test]
fn home_action_row_renders_all_four_buttons_exactly_once() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let harness = mount_home(app_context);
        for label in [
            "Add funds",
            "Send to wallet",
            "Send to another identity",
            "Add contact",
        ] {
            assert_eq!(
                harness.query_all_by_label(label).count(),
                1,
                "expected exactly one {label:?} button in the action row"
            );
        }
    });
}

/// Clicking `Add funds` must open the top-up screen — not merely be
/// clickable. `mount_home` used to discard `home::render`'s returned
/// `(AppAction, HomeOutcome)`, so a button wired to the wrong destination
/// (or to none at all) would have passed every test in this file.
#[test]
fn home_add_funds_click_opens_top_up_screen() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        harness.get_by_label("Add funds").click();
        // A single settle pass: `home::render` resets `action` to `None` at
        // the top of every call, so an extra idle frame here (no queued
        // input) would overwrite this frame's real result with a no-op one.
        harness.run();

        let (action, _outcome) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(Screen::TopUpIdentityScreen(_))),
            "expected Add funds to open TopUpIdentityScreen, got {action:?}"
        );
    });
}

/// Clicking an enabled `Send to wallet` must open the withdrawal screen.
#[test]
fn home_send_to_wallet_click_opens_withdrawal_screen() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let transfer_key = key(1, Purpose::TRANSFER);
        seed_identity(&app_context, vec![transfer_key.clone()], vec![transfer_key]);

        let mut harness = mount_home(app_context);
        harness.get_by_label("Send to wallet").click();
        harness.run();

        let (action, _outcome) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(Screen::WithdrawalScreen(_))),
            "expected Send to wallet to open WithdrawalScreen, got {action:?}"
        );
    });
}

/// Clicking `Send to another identity` must open the transfer screen.
#[test]
fn home_send_to_another_identity_click_opens_transfer_screen() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        harness.get_by_label("Send to another identity").click();
        harness.run();

        let (action, _outcome) = last_result(&harness);
        assert!(
            matches!(action, AppAction::AddScreen(Screen::TransferScreen(_))),
            "expected Send to another identity to open TransferScreen, got {action:?}"
        );
    });
}

/// `Add contact` is gated behind a social profile (design-spec §B.3). Seed
/// one via `ProfileCache::record_saved` — the same API the hub uses after a
/// real profile load — so the enabled path, and its routing to the Contacts
/// tab, is exercised without a network round-trip.
#[test]
fn home_add_contact_click_routes_to_contacts_when_profile_is_set() {
    with_isolated_data_dir(|| {
        let (_runtime, app_context) = fresh_app_context();
        app_context.set_user_role(UserRole::Everyday);
        let identity_id =
            seed_identity(&app_context, vec![key(1, Purpose::AUTHENTICATION)], vec![]);

        let mut harness = mount_home(app_context);
        harness.state_mut().1.record_saved(
            identity_id,
            ProfileFields {
                display_name: "Alex".to_string(),
                ..Default::default()
            },
        );
        harness.run();

        let button = harness.get_by_label("Add contact");
        assert!(
            !button.accesskit_node().is_disabled(),
            "Add contact must enable once the identity has a social profile"
        );
        button.click();
        harness.run();

        let (_action, outcome) = last_result(&harness);
        assert_eq!(
            *outcome,
            HomeOutcome::GoToContacts,
            "expected Add contact to route to the Contacts tab, got {outcome:?}"
        );
    });
}
