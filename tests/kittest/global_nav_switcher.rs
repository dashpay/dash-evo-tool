//! IT-GLOBAL-NAV — the page-aware generalized global-nav switcher (A2).
//!
//! Renders `global_nav_switcher::render` directly with a custom `PageNavSpec`
//! (no root screen consumes it until A3), exercising the parts that differ from
//! the hub: a page-driven segment-1 label and a page-scoped object pill.

use crate::support::{fresh_app_context, with_isolated_data_dir};
use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::components::global_nav_switcher;
use dash_evo_tool::ui::state::global_nav::{IdentityPillScope, PageNavSpec, PillConsumption};
use dash_evo_tool::ui::state::hub_selection::HubSelection;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// TC-NAV-01 foundation — segment-1 is page-driven: a spec labelled
/// `Masternodes` renders that label, not the hub's literal `Identities`.
#[test]
fn segment1_label_is_page_driven() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 200.0))
            .build_ui(move |ui| {
                let mut selection = HubSelection::default();
                let spec =
                    PageNavSpec::new("Masternodes", RootScreenType::RootScreenWalletsBalances)
                        .with_wallet_pill(PillConsumption::Consumed);
                global_nav_switcher::render(ui, &app_context, &spec, &mut selection);
            });
        harness.run();

        assert!(
            harness.query_by_label("Masternodes").is_some(),
            "segment-1 must render the page-driven label"
        );
        assert!(
            harness.query_by_label("Identities").is_none(),
            "segment-1 must NOT hardcode the hub's Identities label"
        );
    });
}

/// TC-NAV-16 foundation — the page-scoped object pill renders its placeholder
/// when nothing is selected (never the app-global identity).
#[test]
fn page_scoped_pill_renders_placeholder_when_empty() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 200.0))
            .build_ui(move |ui| {
                let mut selection = HubSelection::default();
                let spec =
                    PageNavSpec::new("Masternodes", RootScreenType::RootScreenWalletsBalances)
                        .with_wallet_pill(PillConsumption::Consumed)
                        .with_identity_pill(
                            IdentityPillScope::page_scoped_object(
                                "(no masternode yet)",
                                vec![],
                                None,
                            ),
                            PillConsumption::Consumed,
                        );
                global_nav_switcher::render(ui, &app_context, &spec, &mut selection);
            });
        harness.run();

        assert!(
            harness
                .query_by_label_contains("(no masternode yet)")
                .is_some(),
            "the page-scoped pill must show its placeholder when empty"
        );
    });
}

/// TC-NAV-13 — an unwired pill renders subdued (non-interactive): the wallet
/// placeholder still shows, but with no dropdown wiring. Here it renders on a
/// spec whose wallet pill is unwired; the value/placeholder is visible.
#[test]
fn unwired_wallet_pill_renders_placeholder() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 200.0))
            .build_ui(move |ui| {
                let mut selection = HubSelection::default();
                let spec = PageNavSpec::new("Contracts", RootScreenType::RootScreenWalletsBalances)
                    .with_wallet_pill(PillConsumption::Unwired {
                        tooltip: "Change the active wallet from the Wallets tab.".to_string(),
                    });
                global_nav_switcher::render(ui, &app_context, &spec, &mut selection);
            });
        harness.run();

        // No wallets loaded in the fresh context → the unwired wallet pill shows
        // the no-wallet placeholder.
        assert!(
            harness.query_by_label_contains("(no wallet yet)").is_some(),
            "the unwired wallet pill must still show its placeholder value"
        );
    });
}
