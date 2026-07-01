//! IT-ACTIVITY-01 — Activity tab shell renders.
//!
//! Verifies the default (feature `identity-hub-activity-feed` off) path of
//! the Activity tab:
//!
//! - Filter chips `All`, `Payments`, and `Funding` are present.
//! - The gated empty-state message `Unified activity is coming soon.` is
//!   present.
//!
//! The full hub renders the Onboarding empty state when no identities are
//! loaded, which would hide the Activity tab entirely. To keep this test
//! reliable on a fresh first-run database we:
//!
//! 1. build a real `AppContext` via the same `AppState::new` factory the
//!    other kittest files use, then
//! 2. call `activity::render` directly inside a fresh `build_ui` harness.
//!
//! This exercises the component stack, theme, and egui storage while keeping
//! the test scoped to the Activity tab's own contract.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::ui::identity::activity;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::Arc;

fn fresh_app_context() -> (tokio::runtime::Runtime, Arc<AppContext>) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let guard = rt.enter();
    let mut bootstrap = Harness::builder().with_max_steps(20).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });
    bootstrap.run_steps(5);
    let app_context = bootstrap.state().current_app_context().clone();
    drop(bootstrap);
    drop(guard);
    (rt, app_context)
}

/// IT-ACTIVITY-01
#[test]
fn activity_tab_shell_renders_filter_chips_and_gated_message() {
    with_isolated_data_dir(|| {
        let (rt, app_context) = fresh_app_context();
        let _guard = rt.enter();

        let ctx_for_render = app_context.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui(move |ui| {
                let _ = activity::render(ui, &ctx_for_render);
            });
        harness.run();

        // Filter chips — called out verbatim in the test-case spec.
        assert!(
            harness.query_by_label("All").is_some(),
            "Activity tab must render the `All` filter chip"
        );
        assert!(
            harness.query_by_label("Payments").is_some(),
            "Activity tab must render the `Payments` filter chip"
        );
        assert!(
            harness.query_by_label("Funding").is_some(),
            "Activity tab must render the `Funding` filter chip"
        );

        // Gated empty-state message — exact string required by the spec when
        // the `identity-hub-activity-feed` feature is off (default). We use
        // `query_by_label_contains` so trailing punctuation / whitespace
        // variations in the accessibility tree do not make the test brittle.
        #[cfg(not(feature = "identity-hub-activity-feed"))]
        assert!(
            harness
                .query_by_label_contains("Unified activity is coming soon")
                .is_some(),
            "Default (feature off) path must show the gated empty-state message"
        );

        // When the feature is on, the placeholder copy is different — we still
        // assert the shell renders something meaningful in that configuration.
        #[cfg(feature = "identity-hub-activity-feed")]
        assert!(
            harness
                .query_by_label_contains("Unified activity feed coming soon")
                .is_some(),
            "Feature-on path must show the aggregator placeholder"
        );
    });
}
