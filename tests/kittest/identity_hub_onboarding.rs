//! IT-ONBOARD-01 — Onboarding empty state renders.
//!
//! See `docs/ai-design/2026-04-23-identity-hub-impl/03-test-case-spec.md`.
//!
//! Mounts the real [`AppState`] on a fresh in-memory database (zero loaded
//! identities), switches the root screen to `RootScreenIdentityHub`, runs a
//! frame, and asserts on the three elements the spec mandates:
//!
//! 1. Heading `Welcome to Identities.`
//! 2. Primary CTA `Create my first identity`
//! 3. Secondary CTA `I already have an identity — load it`
//!
//! It also asserts the Developer-tools footer is absent at the Everyday role
//! (Alex persona).
//!
//! The rendering path exercised here is
//! `IdentityHubScreen::ui` → `HubLanding::Onboarding` → `onboarding::render`.
//! Developer Mode toggling is covered separately by unit tests in the
//! `onboarding` module and by UI polish tests that will land alongside the
//! identity picker work.

use crate::support::{fresh_app_context, mount_app, with_isolated_data_dir};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::components::styled::island_central_panel;
use dash_evo_tool::ui::identity::onboarding;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, Mutex};

/// IT-ONBOARD-01: the onboarding empty state renders all required copy and
/// CTAs when no identities are loaded, and hides the Developer-tools footer at
/// the Everyday role.
#[test]
fn it_onboard_01_renders_heading_and_both_ctas() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentityHub);

        // Pin the role: the footer's gate is what's under test here, not
        // whichever role an account with no recorded choice starts at.
        harness
            .state()
            .current_app_context()
            .set_user_role(UserRole::Everyday);
        harness.run_steps(2);

        // Heading — design-spec §B.1.
        assert!(
            harness.query_by_label("Welcome to Identities.").is_some(),
            "onboarding must render the 'Welcome to Identities.' heading"
        );

        // Primary CTA.
        assert!(
            harness.query_by_label("Create my first identity").is_some(),
            "onboarding must render the 'Create my first identity' primary button"
        );

        // Secondary CTA — exact text per the design spec (em-dash included).
        assert!(
            harness
                .query_by_label("I already have an identity — load it")
                .is_some(),
            "onboarding must render the 'I already have an identity — load it' secondary button"
        );

        // The label `Developer tools:` is rendered only at the Power role or
        // above (`user_role().at_least(UserRole::Power)`).
        assert!(
            harness.query_by_label("Developer tools:").is_none(),
            "Developer-tools footer must be hidden at the Everyday role"
        );
    });
}

/// IT-ONBOARD-02 (regression) — the onboarding island fills the panel width.
///
/// User report: on "Welcome to Identities." the bordered island does not reach
/// the window edges — it is pinned narrow with dead space outside its border.
/// `island_central_panel` draws the island as a Frame that shrink-wraps to its
/// content, and `onboarding::render` centers a 640px-capped readable column, so
/// without an explicit full-width claim the island collapsed to ~640px. The fix
/// makes `onboarding::render` claim the full available width (the readable
/// column stays centered at 640px). This renders the real onboarding inside the
/// real `island_central_panel` at a wide window and asserts the island content
/// fills its panel rather than shrinking to the inner column.
#[test]
fn onboarding_island_fills_panel_width() {
    with_isolated_data_dir(|| {
        let (rt, app_context) = fresh_app_context();
        let _guard = rt.enter();

        // (width handed to the island content, width the content occupied)
        let measured = Arc::new(Mutex::new((0.0f32, 0.0f32)));
        let probe = measured.clone();
        let ctx = app_context.clone();

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .build_ui(move |ui| {
                island_central_panel(ui, |ui| {
                    let available = ui.available_width();
                    let action = onboarding::render(ui, &ctx);
                    let occupied = ui.min_rect().width();
                    *probe.lock().unwrap() = (available, occupied);
                    action
                });
            });
        harness.run();

        let (available, occupied) = *measured.lock().unwrap();
        assert!(
            available > 800.0,
            "test precondition: the wide window must hand the island a wide panel \
             (got available={available})"
        );
        assert!(
            occupied >= available - 2.0,
            "onboarding island must fill its panel: occupied={occupied}, \
             available={available} — a large gap means the bordered island is \
             pinned narrow with dead space outside its border"
        );
    });
}
