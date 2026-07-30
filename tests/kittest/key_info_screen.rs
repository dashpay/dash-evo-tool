//! Kittest coverage for `KeyInfoScreen`'s arrival refresh.
//!
//! The screen persists the whole identity clone it was opened with on every key
//! add and remove, so a write that lands while it sits on the stack has to be
//! picked up — otherwise the next ordinary key edit puts the pre-write record
//! back, silently and with no error to show for it.
//!
//! That only holds if the re-read hangs off a hook `AppState` actually
//! dispatches to a *pushed* screen. `refresh_on_arrival` is not one:
//! `app.rs` sends it only to root screens, and `KeyInfoScreen` is always pushed
//! onto the screen stack. This drives the dispatch the running app really makes.

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::app::TaskResult;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
use dash_evo_tool::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_evo_tool::model::qualified_identity::{
    IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use dash_evo_tool::ui::{RootScreenType, Screen};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Identity, KeyID, Purpose};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;

const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
/// The row control the keys list renders for an authentication key.
const AUTH_ROW: &str = "Authentication key ›";

/// A key with a chosen id and purpose. Deterministic data, because the row
/// label this test navigates by is derived from the purpose.
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

/// A plain user identity carrying `purposes` as public keys and no private key
/// material — the state that makes every key openable from the keys list.
fn identity_with(id_byte: u8, purposes: &[Purpose], alias: &str) -> QualifiedIdentity {
    let mut identity =
        Identity::create_basic_identity(Identifier::from([id_byte; 32]), PlatformVersion::latest())
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

/// Walk the route a user takes to one key's own page: Identities → Settings →
/// Advanced → Manage keys → the key.
fn open_key_info(harness: &mut egui_kittest::Harness<'static, dash_evo_tool::app::AppState>) {
    // The left nav carries its own "Settings" entry. Both are buttons; only the
    // nav one is a toggle, so that is what tells the hub tab apart.
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
    harness.get_by_label(AUTH_ROW).click();
    harness.run_steps(3);
}

/// A restore that lands while this screen is open never reaches its
/// `display_task_result` — results go only to the screen that is visible, and a
/// restore run from elsewhere is not this screen's. Its clone must still be
/// re-read when the app refreshes it, or the next ordinary key edit writes that
/// clone back over the restored key.
///
/// Drives the refresh the running app actually performs: `AppState::update`
/// drains `task_result_sender` every frame, and `TaskResult::Refresh` calls
/// `refresh()` on the *visible* screen — the pushed `KeyInfoScreen`. A screen
/// that puts its re-read on `refresh_on_arrival` instead is never asked, since
/// `AppState` sends that hook only to root screens.
#[test]
fn a_write_that_lands_while_key_info_is_open_survives_the_next_key_edit() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);
        // The Settings tab stacks two columns above the Advanced expander; a
        // short window puts the keys controls below the fold, where a simulated
        // click lands outside the widget.
        harness.set_size(egui::vec2(1280.0, 1800.0));
        let app_context = harness.state().current_app_context().clone();
        let identity = identity_with(0x51, &[Purpose::AUTHENTICATION], "arrival-refresh");
        let identity_id = identity.identity.id();
        app_context
            .insert_local_qualified_identity(&identity, &None)
            .expect("seed the identity");
        harness.run_steps(5);

        open_key_info(&mut harness);
        assert!(
            matches!(
                harness.state().screen_stack.last(),
                Some(Screen::KeyInfoScreen(_))
            ),
            "the premise: the key's own page is the visible screen"
        );

        // The restore lands behind this screen's back: the record gains a key
        // this screen's clone knows nothing about.
        let restored = key(1, Purpose::TRANSFER);
        let mut record = app_context
            .get_local_qualified_identity(&identity_id)
            .expect("read the record")
            .expect("record stored");
        record.identity.add_public_key(restored.clone());
        record.private_keys.private_keys.insert(
            (MAIN, restored.id()),
            (
                QualifiedIdentityPublicKey::from(restored.clone()),
                PrivateKeyData::Clear([0x22; 32]),
            ),
        );
        app_context
            .update_local_qualified_identity(&record)
            .expect("the other writer's write");

        harness
            .state()
            .task_result_sender
            .try_send(TaskResult::Refresh)
            .expect("queue the refresh the app dispatches");
        harness.run_steps(3);

        // What every key add and remove on this screen does with its clone.
        let Some(Screen::KeyInfoScreen(screen)) = harness.state().screen_stack.last() else {
            panic!("Key Info must still be the open screen");
        };
        app_context
            .update_local_qualified_identity(&screen.identity)
            .expect("the next key edit's write");

        assert!(
            app_context
                .get_local_qualified_identity(&identity_id)
                .expect("read back")
                .expect("still stored")
                .private_keys
                .private_keys
                .contains_key(&(MAIN, restored.id())),
            "a key edit on this screen must not erase a key written while it was open",
        );
    });
}
