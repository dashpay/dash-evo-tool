//! Kittest coverage for the identity keys list (`KeysScreen`).
//!
//! The screen is the only key-state-independent way into `KeyInfoScreen` for a
//! user identity: every other route is gated on already holding a suitable key,
//! which is false for exactly the identities the restore-stranded-keys offer
//! exists to help. These tests pin that route open, and pin the offer where a
//! user who came looking for missing keys will actually see it.

use crate::support::{fresh_app_context, mount_app, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor, RecoveryPlan};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::components::MessageBanner;
use dash_evo_tool::ui::components::legacy_recovery_section::recovery_item_labels;
use dash_evo_tool::ui::identities::keys::keys_screen::KeysScreen;
use dash_evo_tool::ui::masternodes::{KeyVocabulary, manage_keys_labels};
use dash_evo_tool::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The row control for an authentication key, in the shared role vocabulary.
const AUTH_ROW: &str = "Authentication key ›";
/// The row control for a `TRANSFER`-purpose key on a **user** identity. The
/// same key on a masternode is its payout address; here it is how the user
/// sends funds, and the words follow the identity.
const TRANSFER_ROW: &str = "Transfer key ›";
/// The crumb `KeyInfoScreen` shows for the keys list that opened it.
const PARENT_CRUMB: &str = "Keys";
/// The row control for a `VOTING`-purpose key.
const VOTING_ROW: &str = "Voting key ›";
/// The row control of the recovery offer.
const RESTORE: &str = "Restore keys";
/// The held/not-held disclosure, mirroring `keys_screen.rs`'s own copy. A
/// divergence here is a test bug, not a screen bug — these are the strings the
/// user reads.
const HELD: &str = "This key is saved on this device.";
const NOT_HELD: &str = "This key is not saved on this device.";
/// The banner text a restore that put keys back reports, from
/// `completion_message(true)`.
const RESTORED: &str =
    "Your keys from the previous Dash Evo Tool version have been restored to this identity.";
/// The banner text a restore that put nothing back reports, from
/// `completion_message(false)`.
const NOTHING_RESTORED: &str = "There was nothing left to restore for this identity.";
/// Stands in for the typed error text `AppState` banners when a restore fails.
const RESTORE_FAILED: &str = "Those keys could not be restored. Check the password and try again.";

/// A key with a chosen id and purpose. Deterministic data, because the row
/// labels under test are derived from purpose and id.
fn key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
    use dash_sdk::dpp::platform_value::BinaryData;
    IdentityPublicKeyV0 {
        id,
        key_type: KeyType::ECDSA_HASH160,
        purpose,
        security_level: SecurityLevel::CRITICAL,
        read_only: false,
        data: BinaryData::new(vec![id as u8; 20]),
        disabled_at: None,
        contract_bounds: None,
    }
    .into()
}

/// A plain user identity carrying `purposes` as public keys and **no private
/// key material at all** — the stranded-key state this whole route exists for.
fn stranded_identity(id_byte: u8, purposes: &[Purpose], alias: &str) -> QualifiedIdentity {
    let pv = PlatformVersion::latest();
    let mut identity = Identity::create_basic_identity(Identifier::from([id_byte; 32]), pv)
        .expect("basic identity");
    for (index, purpose) in purposes.iter().enumerate() {
        identity.add_public_key(key(index as KeyID, *purpose));
    }
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some(alias.to_string()),
        // The point of the fixture: the device holds nothing.
        private_keys: KeyStorage::default(),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: dash_sdk::dpp::dashcore::Network::Testnet,
    }
}

/// A user identity holding the private half of one key, filed at `target`.
///
/// `Purpose::VOTING` filed at `PrivateKeyOnMainIdentity` is the shape where the
/// two target conventions in this codebase disagree: `identity_keys` derives the
/// target from which identity it walked, while `impl From<Purpose> for
/// PrivateKeyTarget` maps every voting key to the voter identity regardless of
/// where it actually sits. A voting-purpose key on the main identity is real —
/// `masternode_key_presence` treats it as voting readiness on its own.
fn identity_holding_key(
    id_byte: u8,
    purpose: Purpose,
    target: PrivateKeyTarget,
) -> QualifiedIdentity {
    let public_key = key(0, purpose);
    let identity = Identity::new_with_id_and_keys(
        Identifier::from([id_byte; 32]),
        BTreeMap::from([(public_key.id(), public_key.clone())]),
        PlatformVersion::latest(),
    )
    .expect("identity with one key");
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("held-key".to_string()),
        private_keys: KeyStorage {
            private_keys: BTreeMap::from([(
                (target, public_key.id()),
                (
                    QualifiedIdentityPublicKey::from(public_key),
                    PrivateKeyData::Clear([id_byte; 32]),
                ),
            )]),
        },
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: dash_sdk::dpp::dashcore::Network::Testnet,
    }
}

/// A plan offering one restorable key, so the section has something to say.
fn plan() -> RecoveryPlan {
    RecoveryPlan {
        items: vec![RecoveryItemDescriptor {
            item: RecoveryItem::Key {
                target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                key_id: 0,
            },
            purpose: Some(Purpose::AUTHENTICATION),
        }],
        excluded: vec![],
    }
}

/// Drive `screen` in a bare harness, capturing the last non-`None` action it
/// returned.
fn harness_for(mut screen: KeysScreen) -> (Harness<'static>, Rc<RefCell<AppAction>>) {
    let action = Rc::new(RefCell::new(AppAction::None));
    let capture = action.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui(move |ui| {
            let act = screen.ui(ui);
            if act != AppAction::None {
                *capture.borrow_mut() = act;
            }
        });
    harness.run_steps(3);
    (harness, action)
}

/// Drive `screen` while keeping a handle on it, so a test can deliver the
/// results and messages `AppState` would between frames.
fn harness_keeping_screen(screen: KeysScreen) -> (Harness<'static>, Rc<RefCell<KeysScreen>>) {
    let screen = Rc::new(RefCell::new(screen));
    let rendered = screen.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui(move |ui| {
            rendered.borrow_mut().ui(ui);
        });
    harness.run_steps(3);
    (harness, screen)
}

/// Drive `screen` with `banner` already set on the harness's own context, the
/// way `AppState` sets one before a screen renders.
fn harness_showing_banner(
    mut screen: KeysScreen,
    banner: &'static str,
    kind: MessageType,
) -> Harness<'static> {
    let mut set = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui(move |ui| {
            if !set {
                MessageBanner::set_global(ui.ctx(), banner, kind);
                set = true;
            }
            screen.ui(ui);
        });
    harness.run_steps(3);
    harness
}

/// The screen must actually render global banners. It is built on
/// `island_central_panel` for this reason: that is the only banner-rendering
/// path a screen gets from the standard chrome (a screen wanting them without
/// it has to call `MessageBanner::show_global` itself, as
/// `contracts_documents_screen` does). On a bare `CentralPanel` every restore
/// outcome would be invisible and a failed restore would look exactly like a
/// successful one — the offer self-extinguishes on the next check either way.
#[test]
fn the_keys_list_renders_global_banners() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x39, &[Purpose::AUTHENTICATION], "banner-channel");

        let success = harness_showing_banner(
            KeysScreen::new(identity.clone(), &app_context),
            RESTORED,
            MessageType::Success,
        );
        assert!(
            success.query_by_label(RESTORED).is_some(),
            "a success banner must be visible on the keys list"
        );

        let failure = harness_showing_banner(
            KeysScreen::new(identity, &app_context),
            RESTORE_FAILED,
            MessageType::Error,
        );
        assert!(
            failure.query_by_label(RESTORE_FAILED).is_some(),
            "an error banner must be visible on the keys list too — a restore that \
             failed must not read as one that worked"
        );
    });
}

/// A finished restore announces itself — and only its own.
///
/// Results reach whichever screen is visible when they arrive, so a completion
/// for another identity must say nothing here: a banner claiming this
/// identity's keys came back when they did not is worse than silence. The
/// *content* of the banner is pinned by
/// `a_restore_reports_success_and_failure_on_the_keys_list_in_the_running_app`,
/// which runs in the app's own context; `fresh_app_context` drops the harness
/// its context belongs to, so text cannot be read back at this level and
/// `has_global` is all this test can honestly assert.
#[test]
fn a_finished_restore_reports_only_its_own_outcome() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x3a, &[Purpose::AUTHENTICATION], "restore-outcome");
        let identity_id = identity.identity.id();
        let mut screen = KeysScreen::new(identity, &app_context);
        let completed = |identity_id| BackendTaskSuccessResult::LegacyRecoveryCompleted {
            identity_id,
            applied: vec![RecoveryItemDescriptor {
                item: RecoveryItem::Key {
                    target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                    key_id: 0,
                },
                purpose: Some(Purpose::AUTHENTICATION),
            }],
            skipped_stale: vec![],
            excluded: vec![],
        };

        MessageBanner::clear_all_global(app_context.egui_ctx());
        screen.display_task_result(completed(Identifier::from([0xee; 32])));
        assert!(
            !MessageBanner::has_global(app_context.egui_ctx()),
            "another identity's restore must not report anything on this screen"
        );

        screen.display_task_result(completed(identity_id));
        assert!(
            MessageBanner::has_global(app_context.egui_ctx()),
            "this identity's own restore must report that it finished"
        );
    });
}

/// A restore that failed keeps its offer, so a mistyped identity password can
/// be corrected and Restore pressed again rather than the remedy vanishing.
#[test]
fn a_failed_restore_leaves_the_offer_in_place_to_retry() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x3b, &[Purpose::AUTHENTICATION], "restore-retry");
        let identity_id = identity.identity.id();
        let mut screen = KeysScreen::new(identity, &app_context);
        screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCandidates {
            identity_id,
            plan: plan(),
        });

        let (mut harness, screen) = harness_keeping_screen(screen);
        harness.get_by_label(RESTORE).click();
        harness.run_steps(2);
        assert!(
            harness.query_by_label(RESTORE).is_none(),
            "the premise: a restore in flight shows progress, not another Restore"
        );

        // An unrelated task failing while the restore runs must not re-arm it.
        // Results are not screen-affine — any task's error reaches whichever
        // screen is visible — and a re-armed Restore can be pressed again,
        // dispatching the same restore twice.
        screen
            .borrow_mut()
            .display_task_error(&TaskError::WalletStateInconsistent);
        harness.run_steps(2);
        assert!(
            harness.query_by_label(RESTORE).is_none(),
            "an unrelated task's failure must not re-enable Restore mid-restore"
        );

        // The restore's own failure does end it, so a mistyped identity password
        // can be corrected and Restore pressed again.
        screen
            .borrow_mut()
            .display_task_error(&TaskError::LegacyRecoveryIdentityChanged);
        harness.run_steps(2);
        assert!(
            harness.query_by_label(RESTORE).is_some(),
            "a failed restore must leave its offer on screen so it can be retried"
        );
    });
}

/// AC-6: the offer and the key list must name the same key identically. They
/// are rendered by different modules from different types, so the only thing
/// keeping them in step is that both derive their wording from the shared
/// vocabulary — this pins that, since a private re-implementation on either
/// side would read plausibly and drift silently.
#[test]
fn the_offer_and_the_key_list_name_a_key_identically() {
    let voting = key(0, Purpose::VOTING);
    let owner = key(1, Purpose::OWNER);
    let payout = key(2, Purpose::TRANSFER);

    let node = KeyVocabulary::Masternode;
    let list_labels: Vec<String> = manage_keys_labels(
        node,
        &[
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, voting.clone()),
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, owner.clone()),
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, payout.clone()),
        ],
    )
    .into_iter()
    .map(|(label, _)| label)
    .collect();

    let descriptors: Vec<RecoveryItemDescriptor> = [
        (0, Purpose::VOTING),
        (1, Purpose::OWNER),
        (2, Purpose::TRANSFER),
    ]
    .into_iter()
    .map(|(key_id, purpose)| RecoveryItemDescriptor {
        item: RecoveryItem::Key {
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id,
        },
        purpose: Some(purpose),
    })
    .collect();
    let offer_labels: Vec<String> =
        recovery_item_labels(node, &descriptors.iter().collect::<Vec<_>>())
            .into_iter()
            .map(|(label, _)| label)
            .collect();

    assert_eq!(
        list_labels, offer_labels,
        "the keys list and the restore offer must name the same key the same way"
    );
    assert_eq!(
        list_labels,
        vec![
            "Voting key".to_string(),
            "Owner key".to_string(),
            "Payout address key".to_string()
        ],
        "and both must use the DIP-3 role words, not raw Platform purposes"
    );
}

/// Clicking Back on the keys list pops it off the screen stack — and refreshes
/// what it reveals, since anything the user did above it may have changed the
/// record underneath.
#[test]
fn manage_keys_back_button_pops_the_screen() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x33, &[Purpose::AUTHENTICATION], "back-test");
        let (mut harness, action) = harness_for(KeysScreen::new(identity, &app_context));

        assert!(
            harness.query_by_label("Back").is_some(),
            "the keys list must render a Back control"
        );

        harness.get_by_label("Back").click();
        harness.run_steps(2);

        assert_eq!(
            *action.borrow(),
            AppAction::PopScreenAndRefresh,
            "clicking Back must pop the keys list and refresh the screen it reveals"
        );
    });
}

/// AC-2, and the core of the bug: an identity that holds **no** private keys
/// still gets a per-key control, and it opens `KeyInfoScreen`.
///
/// Every other route into that screen is gated on already holding a suitable
/// key. If this route were gated the same way, the restore offer would be
/// reachable only by identities that do not need it.
#[test]
fn every_key_opens_key_info_even_when_the_device_holds_none() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(
            0x34,
            &[Purpose::AUTHENTICATION, Purpose::TRANSFER],
            "stranded",
        );
        let (mut harness, action) = harness_for(KeysScreen::new(identity, &app_context));

        // Both keys are listed, named in the shared role vocabulary rather
        // than raw `Debug` enum output.
        assert!(
            harness.query_by_label(AUTH_ROW).is_some(),
            "an authentication key must render a row control named in role words"
        );
        assert!(
            harness.query_by_label(TRANSFER_ROW).is_some(),
            "a user identity's transfer key must be named in plain language"
        );
        assert!(
            harness.query_by_label("Payout address key ›").is_none(),
            "and must not be given a masternode's payout-address framing"
        );

        harness.get_by_label(AUTH_ROW).click();
        harness.run_steps(2);

        assert!(
            matches!(
                &*action.borrow(),
                AppAction::AddScreen(Screen::KeyInfoScreen(_))
            ),
            "activating a key row must open the Key Info screen for that key"
        );
    });
}

/// AC-2's second half, both directions: the row reports a key this device holds
/// as held, and one it does not as missing. Asserting only the negative half
/// would pass against a screen that says "not saved" about everything.
#[test]
fn a_held_key_is_reported_as_held_and_a_missing_one_is_not() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();

        // Held state is stated in words, not conveyed by colour alone
        // (WCAG 1.4.1) — it is the single fact a user with stranded keys
        // came to this screen to find.
        let missing = stranded_identity(0x35, &[Purpose::AUTHENTICATION], "held-state");
        let (missing, _) = harness_for(KeysScreen::new(missing, &app_context));
        assert!(
            missing.query_by_label(NOT_HELD).is_some(),
            "a key with no private material must say so in text"
        );
        assert!(
            missing.query_by_label(HELD).is_none(),
            "and must not also claim to be held"
        );

        let held = identity_holding_key(
            0x3d,
            Purpose::AUTHENTICATION,
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
        );
        let (held, _) = harness_for(KeysScreen::new(held, &app_context));
        assert!(
            held.query_by_label(HELD).is_some(),
            "a key this device holds must be reported as held"
        );
        assert!(
            held.query_by_label(NOT_HELD).is_none(),
            "and must not also claim to be missing"
        );
    });
}

/// SEC-001: the keys list and the Key Info screen it opens must agree about
/// whether a key is held — including for a `Purpose::VOTING` key filed on the
/// main identity, the one shape where this codebase's two target conventions
/// disagree.
///
/// The list resolves the target from the identity it walked and hands it over.
/// Key Info re-deriving it from the purpose instead lands on the voter identity,
/// finds nothing there, and reports a key the device demonstrably holds as
/// missing — on the screen built to answer that question, about a key whose
/// private half the user can see listed one screen earlier.
#[test]
fn key_info_agrees_with_the_list_about_a_voting_key_held_on_the_main_identity() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = identity_holding_key(
            0x3e,
            Purpose::VOTING,
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
        );
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("store the identity so Key Info can re-read it");

        // What the list says.
        let (mut list, action) = harness_for(KeysScreen::new(identity, &app_context));
        assert!(
            list.query_by_label(HELD).is_some(),
            "the list must report the key it can see the private half of as held"
        );

        list.get_by_label(VOTING_ROW).click();
        list.run_steps(2);

        // What the screen the list opens says, after it re-reads the record —
        // which it does on arrival, and after any restore lands.
        let opened = std::mem::replace(&mut *action.borrow_mut(), AppAction::None);
        let AppAction::AddScreen(Screen::KeyInfoScreen(mut key_info)) = opened else {
            panic!("the row must open Key Info");
        };
        key_info.refresh_on_arrival();
        assert!(
            key_info.private_key_data.is_some(),
            "Key Info must still hold the key the list handed it — re-deriving the \
             target from the purpose loses a voting key filed on the main identity"
        );
    });
}

/// AC-3: the offer is identity-scoped, so it must be visible without opening
/// any individual key, and it must sit above the key list — a user who arrived
/// because their keys are missing must not have to read past the keys they do
/// have to find the remedy.
#[test]
fn the_restore_offer_renders_above_the_key_list() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x36, &[Purpose::AUTHENTICATION], "offer-placement");
        let identity_id = identity.identity.id();
        let mut screen = KeysScreen::new(identity, &app_context);
        // Answer the detection the screen dispatches on its first frame.
        screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCandidates {
            identity_id,
            plan: plan(),
        });

        let (harness, _) = harness_for(screen);

        let restore = harness
            .query_by_label(RESTORE)
            .expect("the offer must render without opening any key");
        let first_row = harness
            .query_by_label(AUTH_ROW)
            .expect("the key list must render alongside the offer");
        assert!(
            restore.rect().top() < first_row.rect().top(),
            "the identity-scoped offer must sit above the per-key list"
        );
    });
}

/// AC-9: an identity with no keys explains itself instead of rendering an empty
/// table — and does not borrow the per-key "not saved on this device" wording,
/// which says something different (this identity has keys, just not here).
#[test]
fn an_identity_with_no_keys_shows_an_empty_state() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x37, &[], "no-keys");
        let (harness, _) = harness_for(KeysScreen::new(identity, &app_context));

        assert!(
            harness
                .query_by_label("No keys were found for this identity.")
                .is_some(),
            "an empty key list must say what the user is looking at"
        );
    });
}

/// AC-1, end to end through the real app: Default view, a user identity whose
/// keys are stranded, no interface-mode change and no send flow — Identities →
/// Settings → Advanced → Manage keys → a key → Key Info.
///
/// This is the route the bug closed. Component-level tests above prove the
/// screen behaves; this proves a user can actually get to it.
#[test]
fn key_info_is_reachable_from_the_identity_hub_in_default_view() {
    use dash_evo_tool::model::user_role::UserRole;
    use dash_evo_tool::ui::RootScreenType;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        // The Settings tab stacks two columns above the Advanced expander; a
        // short window puts the keys controls below the fold, where a simulated
        // click lands outside the widget.
        harness.set_size(egui::vec2(1280.0, 1800.0));
        let app_context = harness.state().current_app_context().clone();
        // Everyday view: the persona this feature targets, and the one that
        // must not need to raise its own role to reach its own keys.
        app_context.set_user_role(UserRole::Everyday);
        let identity = stranded_identity(0x38, &[Purpose::AUTHENTICATION], "hub-route");
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed the stranded identity");
        harness.run_steps(5);

        // The left nav carries its own "Settings" entry. Both are buttons; only
        // the nav one is a toggle, so that is what tells the hub tab apart.
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Settings")
            .find(|node| node.accesskit_node().toggled().is_none())
            .expect("the hub must render a Settings tab")
            .click();
        harness.run_steps(3);
        harness.get_by_label("Advanced").click();
        harness.run_steps(3);

        harness.get_by_label("Manage keys").click();
        harness.run_steps(3);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeysScreen(_))
            ),
            "'Manage keys' must open the identity keys list"
        );

        harness.get_by_label(AUTH_ROW).click();
        harness.run_steps(3);

        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeyInfoScreen(_))
            ),
            "the keys list must open Key Info — the route this fix restores"
        );
        assert!(
            harness.query_by_label("Key Information").is_some(),
            "the opened Key Info screen must render its heading"
        );
    });
}

/// Both restore outcomes, reported on the real keys list inside the running
/// app — not a component harness with its own context.
///
/// The component tests above prove the screen sets a banner and that it renders
/// whatever banner its context carries. Only this one closes the seam between
/// those two facts: that the context the screen writes its outcome into is the
/// context it renders from. It is worth its own test because the keys list only
/// gained `island_central_panel` in this change, and a restore that reported
/// nothing would be indistinguishable from one that worked — the offer retires
/// itself either way.
#[test]
fn a_restore_reports_success_and_failure_on_the_keys_list_in_the_running_app() {
    use dash_evo_tool::ui::RootScreenType;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        harness.set_size(egui::vec2(1280.0, 1800.0));
        let app_context = harness.state().current_app_context().clone();
        let identity = stranded_identity(0x3c, &[Purpose::AUTHENTICATION], "restore-report");
        let identity_id = identity.identity.id();
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed the stranded identity");
        harness.run_steps(5);

        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Settings")
            .find(|node| node.accesskit_node().toggled().is_none())
            .expect("the hub must render a Settings tab")
            .click();
        harness.run_steps(3);
        harness.get_by_label("Advanced").click();
        harness.run_steps(3);
        harness.get_by_label("Manage keys").click();
        harness.run_steps(3);

        // A restore that put a key back, delivered the way the app delivers one.
        match harness.state_mut().visible_screen_mut() {
            Screen::KeysScreen(screen) => {
                screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCompleted {
                    identity_id,
                    applied: vec![RecoveryItemDescriptor {
                        item: RecoveryItem::Key {
                            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                            key_id: 0,
                        },
                        purpose: Some(Purpose::AUTHENTICATION),
                    }],
                    skipped_stale: vec![],
                    excluded: vec![],
                });
            }
            _ => panic!("the keys list must be the visible screen"),
        }
        harness.run_steps(3);
        assert!(
            harness.query_by_label(RESTORED).is_some(),
            "a restore that landed must say so on the keys list itself"
        );
        assert!(
            harness.query_by_label(NOTHING_RESTORED).is_none(),
            "and must not report that there was nothing to restore"
        );

        // The failing case: `AppState` banners a task error centrally, on this
        // same context. The keys list has to surface that too, or a restore
        // that failed reads exactly like one that worked.
        MessageBanner::clear_all_global(app_context.egui_ctx());
        harness.run_steps(2);
        MessageBanner::set_global(app_context.egui_ctx(), RESTORE_FAILED, MessageType::Error);
        harness.run_steps(3);
        assert!(
            harness.query_by_label(RESTORE_FAILED).is_some(),
            "a restore that failed must say so on the keys list too"
        );
    });
}

/// QA-007: the Expert-only per-key detail row, both directions. Raw key ids,
/// purposes and security levels are Expert material; the Everyday view gets the
/// role word and the held state, which is what it can act on.
#[test]
fn raw_key_detail_is_expert_only() {
    use dash_evo_tool::model::user_role::UserRole;

    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x3f, &[Purpose::AUTHENTICATION], "expert-detail");

        app_context.set_user_role(UserRole::Everyday);
        let (everyday, _) = harness_for(KeysScreen::new(identity.clone(), &app_context));
        assert!(
            everyday.query_by_label_contains("AUTHENTICATION").is_none(),
            "the Everyday view must not show raw key internals"
        );
        assert!(
            everyday.query_by_label(AUTH_ROW).is_some(),
            "but it must still show the key itself, named in role words"
        );

        app_context.set_user_role(UserRole::Power);
        let (expert, _) = harness_for(KeysScreen::new(identity, &app_context));
        assert!(
            expert.query_by_label_contains("AUTHENTICATION").is_some(),
            "the Expert view must show the raw purpose it was promised"
        );
    });
}

/// The return journey. Every other test in this suite walks *into* Key Info;
/// this one comes back, which is where the stale-offer bug lives.
///
/// A restore dispatched from the pushed Key Info screen writes the record behind
/// this screen's back. Coming back with only the record re-read shows the keys
/// as held while still offering to restore them — and pressing Restore then
/// reports there was nothing left to do. Both halves have to be re-read on the
/// way back, which is what makes `refresh` the arrival hook that has to do both.
#[test]
fn returning_from_key_info_refreshes_both_the_keys_and_the_offer() {
    use dash_evo_tool::ui::RootScreenType;

    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        harness.set_size(egui::vec2(1280.0, 1800.0));
        let app_context = harness.state().current_app_context().clone();
        let identity = stranded_identity(0x40, &[Purpose::AUTHENTICATION], "return-journey");
        let identity_id = identity.identity.id();
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed the identity");
        harness.run_steps(5);

        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Settings")
            .find(|node| node.accesskit_node().toggled().is_none())
            .expect("the hub must render a Settings tab")
            .click();
        harness.run_steps(3);
        harness.get_by_label("Advanced").click();
        harness.run_steps(3);
        harness.get_by_label("Manage keys").click();
        harness.run_steps(3);

        // An offer is on the list, and the key is not held yet.
        match harness.state_mut().visible_screen_mut() {
            Screen::KeysScreen(screen) => {
                screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCandidates {
                    identity_id,
                    plan: plan(),
                });
            }
            _ => panic!("the keys list must be the visible screen"),
        }
        harness.run_steps(3);
        assert!(
            harness.query_by_label(RESTORE).is_some(),
            "the premise: the list is offering to restore this identity's keys"
        );

        // Into Key Info, and restore from there.
        harness.get_by_label(AUTH_ROW).click();
        harness.run_steps(3);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeyInfoScreen(_))
            ),
            "the row must open Key Info"
        );
        // The restore lands: the key is now held, and nothing is left stranded.
        let restored = identity_holding_key(
            0x40,
            Purpose::AUTHENTICATION,
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
        );
        app_context
            .update_local_qualified_identity(&restored)
            .expect("the restore writes the record");
        match harness.state_mut().visible_screen_mut() {
            Screen::KeyInfoScreen(screen) => {
                screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCompleted {
                    identity_id,
                    applied: plan().items,
                    skipped_stale: vec![],
                    excluded: vec![],
                });
            }
            _ => panic!("Key Info must be the visible screen"),
        }
        harness.run_steps(3);

        // Back to the list, via the breadcrumb crumb naming the screen it was
        // opened from. Key Info has no Back control of its own.
        harness.get_by_label(PARENT_CRUMB).click();
        harness.run_steps(5);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeysScreen(_))
            ),
            "Back must return to the keys list"
        );

        assert!(
            harness.query_by_label(HELD).is_some(),
            "the list must show the restored key as held"
        );
        assert!(
            harness.query_by_label(RESTORE).is_none(),
            "and must not still offer to restore a key it just reported as held"
        );
    });
}

/// The contested case both reviewers found in the two-candidate lookup: a main
/// identity holding a `Purpose::VOTING` key at id N, and a linked voting
/// identity holding its own, different key at the same id N.
///
/// An occupied slot proves nothing about whose material is in it. Falling back
/// to the voter slot on key-id alone would report the main identity's key as
/// held — on the strength of a different key's private half — and hand that
/// material to Key Info. The row must be about the key the user clicked.
#[test]
fn a_same_numbered_key_on_the_voter_identity_is_not_mistaken_for_this_one() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();

        // Main identity: a voting-purpose key at id 0, private half NOT held.
        let main_key = key(0, Purpose::VOTING);
        let identity = Identity::new_with_id_and_keys(
            Identifier::from([0x41u8; 32]),
            BTreeMap::from([(main_key.id(), main_key.clone())]),
            PlatformVersion::latest(),
        )
        .expect("main identity");

        // Voter identity: a *different* key, also at id 0, private half held.
        let voter_key = key(0, Purpose::AUTHENTICATION);
        let voter_identity = Identity::new_with_id_and_keys(
            Identifier::from([0x42u8; 32]),
            BTreeMap::from([(voter_key.id(), voter_key.clone())]),
            PlatformVersion::latest(),
        )
        .expect("voter identity");

        let qualified = QualifiedIdentity {
            identity,
            associated_voter_identity: Some((voter_identity, voter_key.clone())),
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: Some("id-collision".to_string()),
            private_keys: KeyStorage {
                private_keys: BTreeMap::from([(
                    (PrivateKeyTarget::PrivateKeyOnVoterIdentity, voter_key.id()),
                    (
                        QualifiedIdentityPublicKey::from(voter_key),
                        PrivateKeyData::Clear([0x42; 32]),
                    ),
                )]),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        };

        let (harness, _) = harness_for(KeysScreen::new(qualified, &app_context));

        // Two rows: the main identity's voting key (not held) and the voter
        // identity's own key (held). Exactly one of each disclosure.
        assert_eq!(
            harness.query_all_by_label(NOT_HELD).count(),
            1,
            "the main identity's voting key holds no private material of its own"
        );
        assert_eq!(
            harness.query_all_by_label(HELD).count(),
            1,
            "only the voter identity's own key is actually held"
        );
    });
}

/// A key the device holds stays held after it is disabled on chain.
///
/// The stored copy of a public key is a snapshot taken when its private half was
/// saved. Platform keys are immutable once added with exactly one exception —
/// disabling rewrites `disabled_at` — so full-struct equality between the
/// snapshot and the live key breaks the moment a key is rotated or disabled, and
/// the row reports a key whose private half is demonstrably on this device as
/// missing. Same symptom as SEC-001 from a different trigger.
///
/// This fails safe, understating possession rather than over-, and the guard
/// against the unsafe direction is
/// `a_same_numbered_key_on_the_voter_identity_is_not_mistaken_for_this_one`:
/// the comparison has to survive `disabled_at` moving while still telling two
/// same-numbered keys apart.
#[test]
fn a_disabled_key_whose_private_half_is_saved_is_still_reported_as_held() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();

        // The live key, as Platform reports it after the user disabled it.
        let mut live = key(0, Purpose::AUTHENTICATION);
        let disabled_at = 1_700_000_000_u64;
        match &mut live {
            IdentityPublicKey::V0(v0) => v0.disabled_at = Some(disabled_at),
        }
        let identity = Identity::new_with_id_and_keys(
            Identifier::from([0x43u8; 32]),
            BTreeMap::from([(live.id(), live.clone())]),
            PlatformVersion::latest(),
        )
        .expect("identity with one disabled key");

        // The stored snapshot, written before it was disabled.
        let snapshot = key(0, Purpose::AUTHENTICATION);
        assert_eq!(
            snapshot.disabled_at(),
            None,
            "the premise: the saved copy predates the key being disabled"
        );

        let qualified = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some("disabled-but-held".to_string()),
            private_keys: KeyStorage {
                private_keys: BTreeMap::from([(
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, snapshot.id()),
                    (
                        QualifiedIdentityPublicKey::from(snapshot),
                        PrivateKeyData::Clear([0x43; 32]),
                    ),
                )]),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        };

        let (harness, _) = harness_for(KeysScreen::new(qualified, &app_context));
        assert!(
            harness.query_by_label(HELD).is_some(),
            "disabling a key on chain does not remove its private half from this \
             device, so the row must still report it as saved here"
        );
        assert!(
            harness.query_by_label(NOT_HELD).is_none(),
            "and must not report it as missing"
        );
    });
}

/// QA-008: every screen hosting the offer must name a key as the identity it is
/// showing would.
///
/// Three screens render the same offer — the keys list, Key Info, and masternode
/// detail — and each passes its own vocabulary. Scoping two of them left a user
/// identity's Key Info offer calling its transfer key a "Payout address key",
/// asserting the user owns a masternode, and disagreeing with the keys list one
/// screen away. A required constructor argument makes a *missing* vocabulary a
/// compile error, but nothing in the type system catches a host passing the
/// *wrong* one, so the host is where this has to be pinned: this drives Key Info
/// through the route the user takes and reads the words off the rendered offer.
#[test]
fn key_info_names_a_users_transfer_key_as_the_keys_list_does() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = identity_holding_key(
            0x40,
            Purpose::TRANSFER,
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
        );
        let identity_id = identity.identity.id();
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("store the identity so Key Info can re-read it");

        // Arrive at Key Info the way the user does, from the list row.
        let (mut list, action) = harness_for(KeysScreen::new(identity, &app_context));
        list.get_by_label(TRANSFER_ROW).click();
        list.run_steps(2);
        let opened = std::mem::replace(&mut *action.borrow_mut(), AppAction::None);
        let AppAction::AddScreen(Screen::KeyInfoScreen(mut key_info)) = opened else {
            panic!("the row must open Key Info");
        };

        // Offer Key Info a restorable transfer key, as the check would.
        key_info.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCandidates {
            identity_id,
            plan: RecoveryPlan {
                items: vec![RecoveryItemDescriptor {
                    item: RecoveryItem::Key {
                        target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                        key_id: 0,
                    },
                    purpose: Some(Purpose::TRANSFER),
                }],
                excluded: vec![],
            },
        });

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 900.0))
            .build_ui(move |ui| {
                key_info.ui(ui);
            });
        harness.run_steps(3);

        assert!(
            harness.query_by_label(RESTORE).is_some(),
            "the premise: Key Info is showing the offer"
        );
        assert!(
            harness.query_by_label("Transfer key").is_some(),
            "Key Info's offer must name the key as the keys list does"
        );
        assert!(
            harness.query_by_label("Payout address key").is_none(),
            "and must not tell a plain user their key is a masternode payout address"
        );
    });
}
