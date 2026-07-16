//! Kittest coverage for the Manage Keys detail screen (`KeysScreen`).
//!
//! Regression guard for the dead-end lockout: the read-only key list is pushed
//! onto the screen stack, so it must offer a Back control that pops itself off
//! (`AppAction::PopScreen`). Without it the user is trapped on the screen with no
//! way back to the identity view.

use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::identities::keys::keys_screen::KeysScreen;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Identifier;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::cell::RefCell;
use std::rc::Rc;

/// Clicking Back on the Manage Keys screen pops it off the screen stack,
/// closing the dead-end lockout.
#[test]
fn manage_keys_back_button_pops_the_screen() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();

        let identity = Identity::create_basic_identity(
            Identifier::from([0x33u8; 32]),
            PlatformVersion::latest(),
        )
        .expect("basic identity");
        let mut screen = KeysScreen::new(identity, &app_context);

        // Capture the action the screen returns on the frame Back is clicked.
        let action = Rc::new(RefCell::new(AppAction::None));
        let capture = action.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui(move |ui| {
                let act = screen.ui(ui);
                if act != AppAction::None {
                    *capture.borrow_mut() = act;
                }
            });

        harness.run();
        assert!(
            harness.query_by_label("Back").is_some(),
            "the Manage Keys screen must render a Back control"
        );

        harness.get_by_label("Back").click();
        harness.run();

        assert_eq!(
            *action.borrow(),
            AppAction::PopScreen,
            "clicking Back must pop the Manage Keys screen off the stack"
        );
    });
}
