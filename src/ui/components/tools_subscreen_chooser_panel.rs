use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::components::subscreen_chooser_panel::{
    SubscreenNavItem, add_subscreen_chooser_panel,
};
use crate::ui::tools::ToolsSubscreen;
use egui::Ui;

pub fn add_tools_subscreen_chooser_panel(ui: &mut Ui, app_context: &AppContext) -> AppAction {
    let active = match app_context.get_app_settings().root_screen_type {
        RootScreenType::RootScreenToolsPlatformInfoScreen => ToolsSubscreen::PlatformInfo,
        RootScreenType::RootScreenToolsAddressBalanceScreen => ToolsSubscreen::AddressBalance,
        RootScreenType::RootScreenToolsTransitionVisualizerScreen => {
            ToolsSubscreen::TransactionViewer
        }
        RootScreenType::RootScreenToolsProofVisualizerScreen => ToolsSubscreen::ProofViewer,
        RootScreenType::RootScreenToolsDocumentVisualizerScreen => ToolsSubscreen::DocumentViewer,
        RootScreenType::RootScreenToolsContractVisualizerScreen => ToolsSubscreen::ContractViewer,
        RootScreenType::RootScreenToolsGroveSTARKScreen => ToolsSubscreen::GroveSTARK,
        RootScreenType::RootScreenDPNSActiveContests
        | RootScreenType::RootScreenDPNSPastContests
        | RootScreenType::RootScreenDPNSOwnedNames
        | RootScreenType::RootScreenDPNSScheduledVotes => ToolsSubscreen::DPNS,
        _ => ToolsSubscreen::PlatformInfo,
    };

    let items = visible_tools_nav_items()
        .into_iter()
        .map(|(subscreen, target)| {
            SubscreenNavItem::new(
                subscreen.display_name(),
                subscreen == active,
                AppAction::SetMainScreen(target),
            )
        })
        .collect();

    add_subscreen_chooser_panel(ui, "tools_subscreen_chooser_panel", false, true, items)
}

/// The Tools tabs shown in the left-hand chooser panel, in display order.
///
/// GroveSTARK ("ZK Proofs") is intentionally omitted here so it does not appear
/// in the menu, but its screen and `RootScreenToolsGroveSTARKScreen` route stay
/// live — it remains reachable through other entry points and keeps working.
fn visible_tools_nav_items() -> Vec<(ToolsSubscreen, RootScreenType)> {
    vec![
        (
            ToolsSubscreen::PlatformInfo,
            RootScreenType::RootScreenToolsPlatformInfoScreen,
        ),
        (
            ToolsSubscreen::AddressBalance,
            RootScreenType::RootScreenToolsAddressBalanceScreen,
        ),
        (
            ToolsSubscreen::ProofViewer,
            RootScreenType::RootScreenToolsProofVisualizerScreen,
        ),
        (
            ToolsSubscreen::TransactionViewer,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        ),
        (
            ToolsSubscreen::DocumentViewer,
            RootScreenType::RootScreenToolsDocumentVisualizerScreen,
        ),
        (
            ToolsSubscreen::ContractViewer,
            RootScreenType::RootScreenToolsContractVisualizerScreen,
        ),
        (
            ToolsSubscreen::DPNS,
            RootScreenType::RootScreenDPNSActiveContests,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zk_proofs_hidden_from_tools_menu() {
        let names: Vec<&str> = visible_tools_nav_items()
            .iter()
            .map(|(subscreen, _)| subscreen.display_name())
            .collect();

        assert!(
            !names.contains(&ToolsSubscreen::GroveSTARK.display_name()),
            "ZK Proofs must not appear in the Tools menu"
        );
    }

    #[test]
    fn other_tools_still_listed() {
        let names: Vec<&str> = visible_tools_nav_items()
            .iter()
            .map(|(subscreen, _)| subscreen.display_name())
            .collect();

        for expected in [
            ToolsSubscreen::PlatformInfo,
            ToolsSubscreen::AddressBalance,
            ToolsSubscreen::ProofViewer,
            ToolsSubscreen::TransactionViewer,
            ToolsSubscreen::DocumentViewer,
            ToolsSubscreen::ContractViewer,
            ToolsSubscreen::DPNS,
        ] {
            assert!(
                names.contains(&expected.display_name()),
                "Tools menu should still list {}",
                expected.display_name()
            );
        }
    }
}
