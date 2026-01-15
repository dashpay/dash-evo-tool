use crate::app::AppAction;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::identities::register_dpns_name_screen::{RegisterDpnsNameScreen, RegisterDpnsNameSource};
use crate::ui::{RootScreenType, Screen};
use egui::Ui;

impl AddNewIdentityScreen {
    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        let action = crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Identity Registered Successfully!".to_string(),
            vec![
                (
                    "Back to Identities".to_string(),
                    AppAction::PopScreenAndRefresh,
                ),
                (
                    "Register DPNS Name".to_string(),
                    AppAction::Custom("register_dpns".to_string()),
                ),
            ],
            None,
        );

        // Handle the custom action to navigate to DPNS registration
        if let AppAction::Custom(ref s) = action
            && s == "register_dpns"
        {
            // Use Identities source since we came from the Add New Identity flow
            let mut screen = RegisterDpnsNameScreen::new(&self.app_context, RegisterDpnsNameSource::Identities);
            if let Some(identity_id) = self.successful_qualified_identity_id {
                screen.select_identity(identity_id);
                screen.show_identity_selector = false;
            }
            return AppAction::PopThenAddScreenToMainScreen(
                RootScreenType::RootScreenIdentities,
                Screen::RegisterDpnsNameScreen(screen),
            );
        }

        action
    }
}
