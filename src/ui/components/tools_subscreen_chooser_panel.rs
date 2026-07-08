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

    let items = [
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
            ToolsSubscreen::GroveSTARK,
            RootScreenType::RootScreenToolsGroveSTARKScreen,
        ),
        (
            ToolsSubscreen::DPNS,
            RootScreenType::RootScreenDPNSActiveContests,
        ),
    ]
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
