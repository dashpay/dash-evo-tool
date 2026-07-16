use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::components::subscreen_chooser_panel::{
    SubscreenNavItem, add_subscreen_chooser_panel,
};
use crate::ui::dpns::dpns_contested_names_screen::DPNSSubscreen;
use egui::Ui;

pub fn add_dpns_subscreen_chooser_panel(ui: &mut Ui, app_context: &AppContext) -> AppAction {
    let active = match app_context.get_app_settings().root_screen_type {
        RootScreenType::RootScreenDPNSActiveContests => DPNSSubscreen::Active,
        RootScreenType::RootScreenDPNSPastContests => DPNSSubscreen::Past,
        RootScreenType::RootScreenDPNSOwnedNames => DPNSSubscreen::Owned,
        RootScreenType::RootScreenDPNSScheduledVotes => DPNSSubscreen::ScheduledVotes,
        _ => DPNSSubscreen::Active,
    };

    let items = [
        (
            DPNSSubscreen::Active,
            RootScreenType::RootScreenDPNSActiveContests,
        ),
        (
            DPNSSubscreen::Past,
            RootScreenType::RootScreenDPNSPastContests,
        ),
        (
            DPNSSubscreen::Owned,
            RootScreenType::RootScreenDPNSOwnedNames,
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

    add_subscreen_chooser_panel(ui, "dpns_subscreen_chooser_panel", true, false, items)
}
