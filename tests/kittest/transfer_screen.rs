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
                if !crate::support::prepare_app_theme(ui) {
                    return;
                }
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

/// With a transfer key loaded, a Platform Address destination and the advanced
/// options open, the Transfer action scrolls into view in an 800×600 window.
#[test]
fn the_transfer_action_is_reachable_in_a_small_window() {
    use dash_sdk::dpp::identity::Purpose;
    use egui::accesskit::Role;
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let transfer = crate::withdraw_screen::key(1, Purpose::TRANSFER);
        let identity = crate::withdraw_screen::build_identity(
            &app_context,
            IdentityType::User,
            vec![transfer.clone()],
            vec![transfer],
        );
        let mut screen = TransferScreen::new(identity, &app_context);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                if !crate::support::prepare_app_theme(ui) {
                    return;
                }
                screen.ui(ui);
            });
        harness.run_steps(3);
        harness.get_by_label("Show Advanced Options").click();
        harness.run_steps(3);
        harness
            .get_by_role_and_label(Role::Button, "Platform Address")
            .click();
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("4. Select the key to sign the transaction with")
                .is_some(),
            "precondition: the populated form with advanced options is shown"
        );

        harness.hover_at(harness.get_by_label("Transfer Funds").rect().center());
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Unknown,
            delta: egui::vec2(0.0, -2000.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(12);
        // The breadcrumb is also a "Transfer" button; the action is the lower one.
        let action = harness
            .query_all_by_role_and_label(Role::Button, "Transfer")
            .map(|node| node.rect())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("the Transfer action");
        assert!(
            action.bottom() <= 600.0,
            "the Transfer button ({action:?}) must scroll into the 800×600 window"
        );
    });
}
