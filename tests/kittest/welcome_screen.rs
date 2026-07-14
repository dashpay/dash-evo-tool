use crate::support::with_isolated_data_dir;
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

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
        harness.run_steps(10);

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
        harness.get_by_label("Default view").click();
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
        harness.run_steps(10);

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
