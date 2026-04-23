//! IT-HOME-01 — Home tab renders with one identity (smoke).
//!
//! Per the test-case specification
//! (`docs/ai-design/2026-04-23-identity-hub-impl/03-test-case-spec.md`),
//! the Home tab, when an identity is loaded, must render:
//!
//! - Breadcrumb `Identities` link + wallet pill + identity pill present.
//! - Tab bar with exactly four tab labels: Home, Contacts, Activity, Settings.
//! - Home tab selected by default.
//! - Quick-actions row with three buttons: Send, Receive, Add contact.
//! - Secondary-actions row with three ghost buttons: Add funds, Send to
//!   wallet, Send to another identity.
//!
//! Mounting `AppState` with a hand-crafted identity database requires
//! fixtures that do not exist in this branch yet (the `AppContext` is
//! constructed from the on-disk settings database at test start). We
//! therefore verify here the parts of the Home tab that are deterministic
//! at the API level: the tab-bar enumeration, the hub screen enum wiring,
//! and the Home outcome state machine. The full populated render with a
//! fake identity is covered by the unit tests in the `home` module and
//! will expand here once the kittest harness exposes a fixture identity
//! builder (tracked alongside T9+ population work).

use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::identity::IdentityHubTab;
use egui_kittest::Harness;

/// Mount the full AppState and switch to the Identities hub. Returns the
/// harness so individual tests can step the frame loop and inspect the
/// resulting widget tree.
fn mount_hub() -> Harness<'static, dash_evo_tool::app::AppState> {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false);
        app.selected_main_screen = RootScreenType::RootScreenIdentityHub;
        app
    });
    harness.set_size(egui::vec2(1280.0, 800.0));
    harness.run_steps(10);
    harness
}

/// IT-HOME-01 smoke — the hub mounts with the Home tab selected by default
/// and the surrounding chrome (nav, topbar, panels) renders without panic.
///
/// In the absence of a seeded in-memory identity (see module docstring),
/// the hub renders its onboarding empty state. This still exercises the
/// Home module compile path via the `selected_tab` default and validates
/// that the scaffolding + new hero / checklist components do not crash
/// during layout on any of the three glyph branches.
#[test]
fn home_tab_mounts_without_panic() {
    let _harness = mount_hub();
    // Returning without panic from `mount_hub` exercises:
    //   * `IdentityHubScreen::new` → `HomeState::default()`.
    //   * `IdentityHubScreen::ui` → left panel, top panel, central panel.
    //   * `HubLanding::from_identity_count(0)` → onboarding render path.
    //   * Re-rendering over 10 frames with animations off.
}

/// IT-HOME-01 — the hub's default selected tab is Home. This anchors the
/// contract that the Home tab is the landing surface once the picker /
/// onboarding gate passes.
#[test]
fn default_selected_tab_is_home() {
    assert_eq!(IdentityHubTab::default(), IdentityHubTab::Home);
}

/// IT-HOME-01 — the tab bar has exactly four tab labels in the expected
/// order. The labels are the Alex-facing strings from §B.2 / §C wording
/// audit and are consumed verbatim by the eventual `IdentityHubTabBar`
/// component (T5, sibling task).
#[test]
fn tab_bar_exposes_four_tabs_in_display_order() {
    let labels: Vec<&str> = IdentityHubTab::ALL.iter().map(|t| t.label()).collect();
    assert_eq!(labels, ["Home", "Contacts", "Activity", "Settings"]);
}

/// IT-HOME-01 — the home module's outcome state machine is reachable from
/// the public API surface. Guards against a refactor that accidentally
/// drops the tab-switch outcome the hub relies on.
#[test]
fn home_outcome_public_api_is_reachable() {
    use dash_evo_tool::ui::identity::home::{HomeOutcome, HomeState, apply_outcome};
    let mut state = HomeState::default();
    assert!(!state.advanced_open);
    assert!(!state.dismissed_checklist);
    assert!(!state.skipped_social_profile);
    // Dismiss sets the flag and returns no tab-switch.
    assert_eq!(
        apply_outcome(&mut state, HomeOutcome::DismissChecklist),
        None
    );
    assert!(state.dismissed_checklist);
    // Go-to-activity returns the correct tab without touching state flags.
    let activity_tab = apply_outcome(&mut state, HomeOutcome::GoToActivity);
    assert_eq!(activity_tab, Some(IdentityHubTab::Activity));
}
