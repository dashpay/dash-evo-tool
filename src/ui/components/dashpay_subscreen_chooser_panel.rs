use crate::app::AppAction;
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::ui::RootScreenType;
use crate::ui::components::subscreen_chooser_panel::{
    SubscreenNavItem, add_subscreen_chooser_panel,
};
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use egui::Ui;
use std::sync::Arc;

pub fn add_dashpay_subscreen_chooser_panel(
    ui: &mut Ui,
    app_context: &Arc<AppContext>,
    current_subscreen: DashPaySubscreen,
) -> AppAction {
    // Payment History is experimental (developer mode only).
    let mut subscreens = vec![DashPaySubscreen::Profile, DashPaySubscreen::Contacts];
    if FeatureGate::DeveloperMode.is_available(app_context) {
        subscreens.push(DashPaySubscreen::Payments);
    }
    subscreens.push(DashPaySubscreen::ProfileSearch);

    let items = subscreens
        .into_iter()
        .map(|subscreen| {
            let (label, target) = match subscreen {
                DashPaySubscreen::Contacts => {
                    ("Contacts", RootScreenType::RootScreenDashPayContacts)
                }
                DashPaySubscreen::Profile => {
                    ("My Profile", RootScreenType::RootScreenDashPayProfile)
                }
                DashPaySubscreen::Payments => {
                    ("Payment History", RootScreenType::RootScreenDashPayPayments)
                }
                DashPaySubscreen::ProfileSearch => (
                    "Search Profiles",
                    RootScreenType::RootScreenDashPayProfileSearch,
                ),
            };
            SubscreenNavItem::new(
                label,
                subscreen == current_subscreen,
                AppAction::SetMainScreen(target),
            )
        })
        .collect();

    add_subscreen_chooser_panel(ui, "dashpay_subscreen_chooser_panel", true, false, items)
}
