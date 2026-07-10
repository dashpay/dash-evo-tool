use crate::support::with_isolated_data_dir;
use dash_evo_tool::model::user_role::UserRole;
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
        assert_eq!(app_context.user_role(), UserRole::Everyday);

        harness.get_by_label("Detailed view").click();
        harness.run_steps(3);

        assert_eq!(
            app_context.user_role(),
            UserRole::Power,
            "selecting 'Detailed view' on the welcome row must set the Power role"
        );
        assert_eq!(
            app_context.get_app_settings().user_role,
            Some(UserRole::Power),
            "the onboarding role choice must be persisted to AppSettings"
        );
    });
}
