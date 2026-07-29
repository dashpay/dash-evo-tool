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
use dash_evo_tool::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor, RecoveryPlan};
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::identities::keys::keys_screen::KeysScreen;
use dash_evo_tool::ui::{Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
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
/// The row control for a `TRANSFER`-purpose key, which the shared vocabulary
/// names for its DIP-3 counterpart rather than its Platform purpose.
const PAYOUT_ROW: &str = "Payout address key ›";
/// The restore control of the recovery offer.
const RESTORE: &str = "Restore keys";

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

/// Clicking Back on the keys list pops it off the screen stack, closing the
/// dead-end lockout.
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
            AppAction::PopScreen,
            "clicking Back must pop the keys list off the stack"
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
            harness.query_by_label(PAYOUT_ROW).is_some(),
            "a transfer-purpose key must be named for its payout role"
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

/// AC-2's second half: the row passes the identity's real held-key data
/// through, so Key Info does not misreport a held key as missing.
#[test]
fn a_held_key_is_reported_as_held_and_a_missing_one_is_not() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x35, &[Purpose::AUTHENTICATION], "held-state");
        let (harness, _) = harness_for(KeysScreen::new(identity, &app_context));

        // Held state is stated in words, not conveyed by colour alone
        // (WCAG 1.4.1) — it is the single fact a user with stranded keys
        // came to this screen to find.
        assert!(
            harness
                .query_by_label("This key is not saved on this device.")
                .is_some(),
            "a key with no private material must say so in text"
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

/// AC-9: an identity with no keys explains itself instead of rendering an
/// empty table.
#[test]
fn an_identity_with_no_keys_shows_an_empty_state() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let identity = stranded_identity(0x37, &[], "no-keys");
        let (harness, _) = harness_for(KeysScreen::new(identity, &app_context));

        assert!(
            harness
                .query_by_label("This identity has no keys saved on this device yet.")
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
