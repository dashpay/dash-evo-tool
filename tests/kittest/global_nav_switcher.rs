//! IT-GLOBAL-NAV — the page-aware generalized global-nav switcher (A2).
//!
//! Renders `global_nav_switcher::render` directly with a custom `PageNavSpec`
//! (no root screen consumes it until A3), exercising the parts that differ from
//! the hub: a page-driven segment-1 label and a page-scoped object pill.

use crate::support::{fresh_app_context, mount_app, with_isolated_data_dir};
use dash_evo_tool::app::AppAction;
use dash_evo_tool::model::wallet_association::WalletAssociation;
use dash_evo_tool::ui::RootScreenType;
use dash_evo_tool::ui::components::global_nav_switcher::{self, GlobalNavEffect};
use dash_evo_tool::ui::components::top_panel::apply_global_nav_effect;
use dash_evo_tool::ui::state::global_nav::{
    IdentityPillScope, PageNavSpec, PageObjectItem, PillConsumption,
};
use dash_evo_tool::ui::state::hub_selection::HubSelection;
use dash_evo_tool::ui::state::masternodes_view::masternodes_page_nav_spec;
use dash_sdk::platform::Identifier;
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
                                "Switch between your loaded masternodes and evonodes.",
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

/// TC-NAV-04 — the interactive page-scoped pill names the object in view, not
/// its placeholder, and offers the page's objects.
#[test]
fn page_scoped_pill_names_the_selected_object() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 200.0))
            .build_ui(move |ui| {
                let mut selection = HubSelection::default();
                let items = vec![
                    PageObjectItem {
                        id: Identifier::new([1; 32]),
                        label: "mn-east-01".to_string(),
                        icon: Some("🖥".to_string()),
                    },
                    PageObjectItem {
                        id: Identifier::new([2; 32]),
                        label: "evo-west-02".to_string(),
                        icon: Some("◆".to_string()),
                    },
                ];
                let spec =
                    PageNavSpec::new("Masternodes", RootScreenType::RootScreenWalletsBalances)
                        .with_wallet_pill(PillConsumption::Consumed)
                        .with_identity_pill(
                            IdentityPillScope::page_scoped_object(
                                "(choose a masternode)",
                                "Switch between your loaded masternodes and evonodes.",
                                items,
                                Some(Identifier::new([2; 32])),
                            ),
                            PillConsumption::Consumed,
                        );
                global_nav_switcher::render(ui, &app_context, &spec, &mut selection);
            });
        harness.run();

        assert!(
            harness.query_by_label_contains("evo-west-02").is_some(),
            "the pill must name the object in view"
        );
        assert!(
            harness
                .query_by_label_contains("(choose a masternode)")
                .is_none(),
            "a resolved selection must replace the placeholder"
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

/// A masternode is wallet-less: its breadcrumb wallet segment shows the
/// read-only "Not in a wallet" indicator, never an interactive wallet pill or
/// the app's active-wallet placeholder — so a node never appears to belong to an
/// arbitrary loaded wallet.
#[test]
fn masternode_wallet_segment_shows_not_in_a_wallet_indicator() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 200.0))
            .build_ui(move |ui| {
                let mut selection = HubSelection::default();
                let items = vec![PageObjectItem {
                    id: Identifier::new([1; 32]),
                    label: "mn-east-01".to_string(),
                    icon: Some("🖥".to_string()),
                }];
                let spec = masternodes_page_nav_spec(
                    items,
                    Some(Identifier::new([1; 32])),
                    WalletAssociation::NotInWallet,
                );
                global_nav_switcher::render(ui, &app_context, &spec, &mut selection);
            });
        harness.run();

        assert!(
            harness.query_by_label_contains("Not in a wallet").is_some(),
            "the masternode wallet segment must show the read-only indicator"
        );
        assert!(
            harness.query_by_label_contains("(no wallet yet)").is_none(),
            "a wallet-less node must not show the app's active-wallet placeholder"
        );
    });
}

/// TC-NAV-06 (A3) — applying a wallet switch is silent (no forced navigation)
/// and reconciles the app-global identity as a documented side effect. With no
/// identities loaded, reconciliation resolves to `None` (keep-if-owned → first
/// → None). The FR-6 MN/Evonode exclusion is enforced at the resolution layer
/// in B1; here we assert the silent reconciliation exists and never navigates.
#[test]
fn switch_wallet_is_silent_and_reconciles_identity() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let action =
            apply_global_nav_effect(&app_context, GlobalNavEffect::SwitchWallet([0x11; 32]));
        assert_eq!(
            action,
            AppAction::None,
            "switching wallet must not navigate"
        );
        assert_eq!(app_context.selected_wallet_hash(), Some([0x11; 32]));
        assert_eq!(
            app_context.selected_identity_id(),
            None,
            "reconciliation with no owned identities resolves to None"
        );
    });
}

/// Segment-1 activation navigates to the page root via `SetMainScreen`.
#[test]
fn navigate_to_root_sets_main_screen() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        let action = apply_global_nav_effect(
            &app_context,
            GlobalNavEffect::NavigateToRoot(RootScreenType::RootScreenDocumentQuery),
        );
        assert_eq!(
            action,
            AppAction::SetMainScreen(RootScreenType::RootScreenDocumentQuery)
        );
    });
}

/// TC-FR6-07 (A3) — a page-scoped object selection never writes the app-global
/// identity, even at the shared applier: the FR-6 boundary holds end-to-end.
#[test]
fn select_page_object_never_writes_app_global_identity() {
    with_isolated_data_dir(|| {
        let (_rt, app_context) = fresh_app_context();
        assert_eq!(app_context.selected_identity_id(), None);
        let action = apply_global_nav_effect(
            &app_context,
            GlobalNavEffect::SelectPageObject(Identifier::new([7; 32])),
        );
        assert_eq!(action, AppAction::None);
        assert_eq!(
            app_context.selected_identity_id(),
            None,
            "a page-scoped object must not touch the app-global identity (FR-6)"
        );
    });
}

/// The global switcher renders on non-Hub root screens. Contracts (everyday)
/// shows both the wallet and identity placeholders; Wallets (wallet-only,
/// TC-NAV-15) shows only the wallet placeholder, no identity segment.
#[test]
fn switcher_present_on_non_hub_everyday_root() {
    with_isolated_data_dir(|| {
        let harness = mount_app(RootScreenType::RootScreenDocumentQuery);
        assert!(
            harness.query_by_label_contains("(no wallet yet)").is_some(),
            "the global switcher's wallet placeholder must render on Contracts"
        );
        assert!(
            harness
                .query_by_label_contains("(no identity yet)")
                .is_some(),
            "the everyday spec must render the identity placeholder on Contracts"
        );
    });
}

/// TC-NAV-15 — the Wallets page composes a wallet-only switcher: the wallet
/// placeholder renders, and there is no identity segment at all.
#[test]
fn switcher_wallet_only_on_wallets_root() {
    with_isolated_data_dir(|| {
        let harness = mount_app(RootScreenType::RootScreenWalletsBalances);
        assert!(
            harness.query_by_label_contains("(no wallet yet)").is_some(),
            "the global switcher's wallet placeholder must render on Wallets"
        );
        assert!(
            harness
                .query_by_label_contains("(no identity yet)")
                .is_none(),
            "the wallet-only spec must render no identity segment (composition)"
        );
    });
}

fn assert_everyday_switcher_on_root(root_screen: RootScreenType, page_name: &str) {
    with_isolated_data_dir(|| {
        let harness = mount_app(root_screen);
        assert!(
            harness.query_by_label_contains("(no wallet yet)").is_some(),
            "the global switcher's wallet placeholder must render on {page_name}"
        );
        assert!(
            harness
                .query_by_label_contains("(no identity yet)")
                .is_some(),
            "the everyday spec must render the identity placeholder on {page_name}"
        );
    });
}

#[test]
fn switcher_present_on_contracts_root() {
    assert_everyday_switcher_on_root(RootScreenType::RootScreenDocumentQuery, "Contracts");
}

#[test]
fn switcher_present_on_tokens_root() {
    assert_everyday_switcher_on_root(RootScreenType::RootScreenMyTokenBalances, "Tokens");
}

#[test]
fn switcher_present_on_tools_root() {
    assert_everyday_switcher_on_root(
        RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        "Tools",
    );
}

#[test]
fn switcher_present_on_network_chooser_root() {
    assert_everyday_switcher_on_root(RootScreenType::RootScreenNetworkChooser, "Network Chooser");
}
