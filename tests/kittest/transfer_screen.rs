//! Kittest coverage for the transfer screen's no-transfer-key branch.
//!
//! The branch exists to say "you cannot send from this identity". It used to
//! offer only "Add key" — typing a private key by hand — which is the wrong
//! remedy for an identity whose keys are merely stranded in the previous
//! version's saved data. It now also signposts the keys list, where those keys
//! can be brought back. The gate deciding the branch is deliberately untouched:
//! it still correctly reports that the identity cannot send.

use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::model::qualified_identity::encrypted_key_storage::KeyStorage;
use dash_evo_tool::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use dash_evo_tool::ui::identity::transfer_screen::TransferScreen;
use dash_evo_tool::ui::{Screen, ScreenLike};
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// An identity with no transfer key loaded, which is what puts the transfer
/// screen into the branch under test.
fn identity_without_transfer_keys() -> QualifiedIdentity {
    let identity =
        Identity::create_basic_identity(Identifier::from([0x51u8; 32]), PlatformVersion::latest())
            .expect("basic identity");
    QualifiedIdentity {
        identity,
        associated_voter_identity: None,
        associated_operator_identity: None,
        associated_owner_key_id: None,
        identity_type: IdentityType::User,
        alias: Some("no-transfer-key".to_string()),
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

/// The no-key branch signposts the keys list instead of dead-ending on "Add
/// key", and clicking it opens that list.
#[test]
fn the_no_transfer_key_branch_routes_to_the_keys_list() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let mut screen = TransferScreen::new(identity_without_transfer_keys(), &app_context);

        let action = Rc::new(RefCell::new(AppAction::None));
        let capture = action.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui(move |ui| {
                let act = screen.ui(ui);
                if act != AppAction::None {
                    *capture.borrow_mut() = act;
                }
            });
        harness.run_steps(3);

        // The premise: this identity cannot send, so the branch is on screen.
        assert!(
            harness.query_by_label("Add key").is_some(),
            "an identity with no transfer key must be in the no-key branch"
        );

        assert!(
            harness.query_by_label("Manage keys").is_some(),
            "the no-key branch must offer a way to see this identity's keys"
        );

        harness.get_by_label("Manage keys").click();
        harness.run_steps(2);

        assert!(
            matches!(
                &*action.borrow(),
                AppAction::AddScreen(Screen::KeysScreen(_))
            ),
            "the signpost must open the keys list, where a stranded key can be restored"
        );
    });
}
