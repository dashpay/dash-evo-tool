//! IT-SWITCH — breadcrumb switcher integration.
//!
//! The multi-wallet / multi-identity cases (IT-SWITCH-01/02/04) require a seeded
//! identity-database fixture that does not exist on this branch yet (see the
//! other `identity_hub_*` kittests' fixture note); the switching logic is
//! covered by the unit tests in `breadcrumb_switcher` + `hub_selection` +
//! `model::selected_identity`. IT-SWITCH-03 (the onboarding placeholders) runs
//! on the default first-run database (0 wallets, 0 identities).

use crate::support::with_isolated_data_dir;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// IT-SWITCH-03 — onboarding (0 wallets, 0 identities): the breadcrumb shows
/// `(no wallet yet)` and `(no identity yet)` placeholders, and the onboarding
/// CTAs still render beneath (switcher coexists on every landing).
#[test]
fn it_switch_03_onboarding_shows_placeholder_segments() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false);
            app.show_welcome_screen = false;
            app.welcome_screen = None;
            app.selected_main_screen = RootScreenType::RootScreenIdentityHub;
            app
        });
        harness.set_size(egui::vec2(1280.0, 800.0));
        harness.run_steps(10);

        assert!(
            harness.query_by_label("(no wallet yet)").is_some(),
            "breadcrumb must show the no-wallet placeholder on onboarding"
        );
        assert!(
            harness.query_by_label("(no identity yet)").is_some(),
            "breadcrumb must show the no-identity placeholder on onboarding"
        );
        // The onboarding CTA still renders beneath the switcher.
        assert!(
            harness.query_by_label("Create my first identity").is_some(),
            "onboarding CTA must coexist with the breadcrumb switcher"
        );
    });
}
