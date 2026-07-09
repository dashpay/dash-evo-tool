//! IT-MN-TAB — Masternodes root tab: Expert-Mode nav gate + live de-gating (B2).

use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::kittest::Queryable;

/// TC-FR1-01…04 — the Masternodes nav entry is absent when Expert Mode is off
/// and present when it is on. Toggling `enable_developer_mode` flips the gate;
/// the nav rail re-evaluates the per-entry `FeatureGate::DeveloperMode` skip
/// each frame. Counted with `query_all_by_label` because the nav button exposes
/// both a Button and an inner Label node for the same text.
#[test]
fn nav_gated_by_expert_mode() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        app_context.enable_developer_mode(false);
        harness.run_steps(3);
        assert_eq!(
            harness.query_all_by_label("Masternodes").count(),
            0,
            "the Masternodes nav entry must be absent when Expert Mode is off"
        );

        app_context.enable_developer_mode(true);
        harness.run_steps(3);
        assert!(
            harness.query_all_by_label("Masternodes").count() >= 1,
            "the Masternodes nav entry must appear when Expert Mode is on"
        );
    });
}

/// TC-EDGE-05/06 (§10.11) — live de-gating: with the Masternodes tab active,
/// flipping Expert Mode off falls the active tab back to the neutral Identities
/// tab (the gated screen is never shown without its gate). Drives the guard in
/// `active_root_screen_mut` directly by selecting the tab, then revoking the
/// gate.
#[test]
fn de_gating_falls_back_to_identities() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = mount_app(RootScreenType::RootScreenIdentities);
        let app_context = harness.state().current_app_context().clone();

        // Expert Mode on, select the Masternodes tab — it stays selected.
        app_context.enable_developer_mode(true);
        harness.state_mut().selected_main_screen = RootScreenType::RootScreenMasternodes;
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenMasternodes,
            "the Masternodes tab stays active while Expert Mode is on"
        );

        // Flip Expert Mode off — the active tab must fall back to Identities.
        app_context.enable_developer_mode(false);
        harness.run_steps(3);
        assert_eq!(
            harness.state().selected_main_screen,
            RootScreenType::RootScreenIdentities,
            "de-gating must fall the active tab back to Identities"
        );
    });
}
