use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::identities::identities_screen::IdentitiesScreen;
use dash_evo_tool::ui::{Screen, ScreenLike};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0 as _;
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Test that the identities screen can be rendered
#[test]
fn test_identities_screen_renders() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));
        crate::support::wait_for_screens(&mut harness);
    });
}

/// Test that the app renders correctly at minimum size
#[test]
fn test_minimum_window_size() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Test with a small window size
        harness.set_size(egui::vec2(400.0, 300.0));
        crate::support::wait_for_screens(&mut harness);
    });
}

/// Test that the app handles resize gracefully
#[test]
fn test_window_resize() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Start small
        harness.set_size(egui::vec2(640.0, 480.0));
        harness.run_steps(5);

        // Resize larger
        harness.set_size(egui::vec2(1280.0, 720.0));
        harness.run_steps(5);

        // Resize smaller again
        harness.set_size(egui::vec2(800.0, 600.0));
        harness.run_steps(5);
    });
}

/// A voting-purpose key with deterministic material, since the popup's row
/// label is derived from id, purpose and security level.
fn voting_key(id: KeyID) -> IdentityPublicKey {
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
    use dash_sdk::dpp::platform_value::BinaryData;
    IdentityPublicKeyV0 {
        id,
        key_type: KeyType::ECDSA_HASH160,
        purpose: Purpose::VOTING,
        security_level: SecurityLevel::CRITICAL,
        read_only: false,
        data: BinaryData::new(vec![id as u8; 20]),
        disabled_at: None,
        contract_bounds: None,
    }
    .into()
}

/// A masternode whose keys are filed where an older build put them — each
/// under the *other* identity's store. The main identity publishes `main_key`,
/// held under the voter store; the voter identity publishes `voter_key`, held
/// under the main store. Both are held, whichever store an old install chose.
fn masternode_with_legacy_filed_keys(
    main_key: &IdentityPublicKey,
    voter_key: &IdentityPublicKey,
) -> QualifiedIdentity {
    let pv = PlatformVersion::latest();
    let build = |id_byte: u8, key: &IdentityPublicKey| {
        Identity::new_with_id_and_keys(
            Identifier::from([id_byte; 32]),
            BTreeMap::from([(key.id(), key.clone())]),
            pv,
        )
        .expect("identity publishing one key")
    };
    QualifiedIdentity {
        identity: build(0x51, main_key),
        associated_voter_identity: Some((build(0x52, voter_key), voter_key.clone())),
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::Masternode,
        alias: Some("legacy-filed".to_string()),
        private_keys: KeyStorage::from(BTreeMap::from([
            (
                (PrivateKeyTarget::PrivateKeyOnVoterIdentity, main_key.id()),
                (
                    QualifiedIdentityPublicKey::from(main_key.clone()),
                    [0x11u8; 32],
                ),
            ),
            (
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, voter_key.id()),
                (
                    QualifiedIdentityPublicKey::from(voter_key.clone()),
                    [0x22u8; 32],
                ),
            ),
        ])),
        dpns_names: vec![],
        associated_wallets: BTreeMap::new(),
        secret_access: None,
        wallet_index: None,
        top_ups: BTreeMap::new(),
        status: IdentityStatus::Active,
        network: dash_sdk::dpp::dashcore::Network::Testnet,
    }
}

/// Drive `screen` in a bare harness, capturing the last non-`None` action.
fn harness_for(mut screen: IdentitiesScreen) -> (Harness<'static>, Rc<RefCell<AppAction>>) {
    let action = Rc::new(RefCell::new(AppAction::None));
    let capture = action.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 800.0))
        .build_ui(move |ui| {
            let act = screen.ui(ui);
            if act != AppAction::None {
                *capture.borrow_mut() = act;
            }
        });
    harness.run_steps(3);
    (harness, action)
}

/// Open the Keys popup, click the row labelled `key_label`, and return the
/// Key Info screen that click opened.
fn open_key_from_popup(
    harness: &mut Harness<'_>,
    action: &Rc<RefCell<AppAction>>,
    key_label: &str,
) -> dash_evo_tool::ui::identities::keys::key_info_screen::KeyInfoScreen {
    harness.get_by_label("Keys").click();
    harness.run_steps(2);
    harness.get_by_label(key_label).click();
    harness.run_steps(2);
    let opened = std::mem::replace(&mut *action.borrow_mut(), AppAction::None);
    let AppAction::AddScreen(Screen::KeyInfoScreen(key_info)) = opened else {
        panic!("clicking the key row must open Key Info, got a different action");
    };
    key_info
}

/// The identities list's Keys popup must see a key as held wherever its
/// private half is filed. A main-identity voting key an older build filed
/// under the voter store is a documented on-disk shape; probing only the
/// store matching the list being walked misses it, so the popup calls a held
/// key unsaved and opens its Key Info page in the wrong state.
#[test]
fn the_keys_popup_finds_a_main_key_an_older_build_filed_under_voter() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let main_key = voting_key(3);
        let identity = masternode_with_legacy_filed_keys(&main_key, &voting_key(0));
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("store the identity for the list to load");

        let (mut harness, action) = harness_for(IdentitiesScreen::new(&app_context));
        let key_info = open_key_from_popup(&mut harness, &action, "3 - V - Critical");
        assert!(
            key_info.private_key_data.is_some(),
            "a held main-identity key filed under the voter store must open as held",
        );
    });
}

/// The voter-list mirror of the same defect: a voter identity's key misfiled
/// under the main store must still be seen as held by the voter rows.
#[test]
fn the_keys_popup_finds_a_voter_key_an_older_build_filed_under_main() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let voter_key = voting_key(0);
        let identity = masternode_with_legacy_filed_keys(&voting_key(3), &voter_key);
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("store the identity for the list to load");

        let (mut harness, action) = harness_for(IdentitiesScreen::new(&app_context));
        let key_info = open_key_from_popup(&mut harness, &action, "0 - V - Critical");
        assert!(
            key_info.private_key_data.is_some(),
            "a held voter-identity key filed under the main store must open as held",
        );
    });
}

/// Test multiple frame batches
#[test]
fn test_frame_batch_processing() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(150).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));

        // Process frames in batches
        for batch in 0..10 {
            crate::support::wait_for_screens(&mut harness);
            // Just ensure we can run multiple batches without error
            let _ = batch;
        }
    });
}
