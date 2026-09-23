use crate::support::with_isolated_data_dir;
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::RootScreenType;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

#[test]
fn welcome_role_cards_expose_radio_accessibility_state() {
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

        for (label, toggled) in [
            ("Default view", Toggled::False),
            ("Expert view", Toggled::True),
            ("Developer view", Toggled::False),
        ] {
            let card = harness.get_by_role_and_label(Role::RadioButton, label);
            assert_eq!(card.accesskit_node().toggled(), Some(toggled));
        }

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            harness.get_by_label(role.description());
        }
    });
}

#[test]
fn welcome_role_cards_paint_keyboard_focus() {
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

        let unfocused_shapes = harness.output().shapes.clone();
        harness
            .get_by_role_and_label(Role::RadioButton, "Expert view")
            .focus();
        harness.step();

        assert!(
            harness.output().shapes != unfocused_shapes,
            "the selected role card must still paint a distinct focus indicator"
        );
        let selected_focus_shapes = harness.output().shapes.clone();

        harness
            .get_by_role_and_label(Role::RadioButton, "Default view")
            .focus();
        harness.step();

        assert!(
            harness
                .get_by_role_and_label(Role::RadioButton, "Default view")
                .is_focused(),
            "keyboard focus must move to the requested role card"
        );
        assert!(
            harness.output().shapes != selected_focus_shapes,
            "a focused, unselected role card must paint a visible focus indicator"
        );
    });
}

#[test]
fn welcome_role_cards_fit_a_narrow_window() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });
        let window_width = 480.0;
        harness.set_size(egui::vec2(window_width, 1000.0));
        crate::support::wait_for_screens(&mut harness);

        for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
            let card = harness.get_by_role_and_label(Role::RadioButton, role.label());
            let rect = card.rect();
            assert!(
                rect.left() >= 0.0 && rect.right() <= window_width,
                "{} card must remain fully visible at narrow widths; got {rect:?}",
                role.label()
            );
            card.click();
            harness.step();
        }
    });
}

#[test]
fn welcome_role_card_selection_is_painted_in_the_click_frame() {
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

        harness
            .get_by_role_and_label(Role::RadioButton, "Developer view")
            .click();
        harness.step();
        let click_frame_shapes = harness.output().shapes.clone();

        harness.step();
        assert!(
            harness.output().shapes == click_frame_shapes,
            "the click frame must already paint the final selected-card state"
        );
    });
}

/// The onboarding welcome row sets the app-global role and persists it, sharing
/// the same vocabulary as the Settings selector so a role picked here is
/// findable by name there.
#[test]
fn welcome_role_selector_sets_and_persists_role() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        // A fresh data dir leaves onboarding incomplete, so the welcome screen
        // (not a main screen) renders on first frame.
        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });
        harness.set_size(egui::vec2(1024.0, 768.0));
        crate::support::wait_for_screens(&mut harness);

        let app_context = harness.state().current_app_context().clone();
        assert_eq!(
            app_context.user_role(),
            UserRole::WHEN_UNSET,
            "an account that never chose a role starts at the unset default"
        );

        // Deliberately a role the account does not already hold: picking the
        // starting role would leave the selector idle, and the persisted `None`
        // would still *read back* as the default — a green assertion proving
        // nothing. A downgrade can only be observed if it was really written.
        harness
            .get_by_role_and_label(Role::RadioButton, "Default view")
            .click();
        harness.run_steps(3);

        assert_eq!(
            app_context.user_role(),
            UserRole::Everyday,
            "selecting 'Default view' on the welcome row must set the Everyday role"
        );
        assert_eq!(
            app_context.get_app_settings().user_role,
            Some(UserRole::Everyday),
            "the onboarding role choice must be persisted to AppSettings"
        );
    });
}

/// The "Just Explore" onboarding path must land on the Identities hub — the
/// single user-facing identity entry in the left nav. It previously landed on
/// the DashPay profile screen, which the nav now hides, leaving the user with
/// no way to navigate back to their landing screen.
#[test]
fn just_explore_lands_on_identities_hub() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        // A fresh data dir leaves onboarding incomplete, so the welcome screen
        // renders on the first frame.
        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });
        harness.set_size(egui::vec2(1024.0, 768.0));
        crate::support::wait_for_screens(&mut harness);

        assert!(
            harness.state().show_welcome_screen,
            "a fresh data dir must open on the onboarding welcome screen"
        );

        harness.get_by_label("Just Explore").click();
        harness.run_steps(5);

        assert!(
            !harness.state().show_welcome_screen,
            "choosing an onboarding path must dismiss the welcome screen"
        );
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenIdentityHub,
            "the 'Just Explore' path must land on the Identities hub, not the hidden DashPay profile"
        );
    });
}

/// A cold start opens on the Welcome screen while the connection state is still
/// its default `Disconnected` — no wallet exists and nothing has asked SPV to
/// sync, so this is "hasn't started yet", not a real failure. The alarming red
/// "Disconnected — check your internet connection" banner must be suppressed
/// until onboarding completes; otherwise it reads as a connectivity problem the
/// user cannot act on.
#[test]
fn welcome_screen_suppresses_disconnected_banner() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        // A fresh data dir leaves onboarding incomplete, so the welcome screen
        // renders on the first frame while the connection state is still the
        // default `Disconnected` (no sync has been attempted).
        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });
        harness.set_size(egui::vec2(1024.0, 768.0));
        crate::support::wait_for_screens(&mut harness);

        assert!(
            harness.state().show_welcome_screen,
            "a fresh data dir must open on the onboarding welcome screen"
        );
        assert!(
            harness
                .query_by_label_contains("check your internet connection")
                .is_none(),
            "the onboarding screen must not show the red 'Disconnected' connection banner"
        );
    });
}
