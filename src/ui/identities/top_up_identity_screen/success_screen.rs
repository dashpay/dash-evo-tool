use crate::app::AppAction;
use crate::ui::identities::add_new_identity_screen::TopUpIdentityScreen;
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameScreen;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use crate::ui::{RootScreenType, Screen};
use egui::Ui;

impl TopUpIdentityScreen {
    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Success!");

            ui.add_space(20.0);

            // Display the "Back to Identities" button
            if ui.button("Back to Identities").clicked() {
                // Handle navigation back to the identities screen
                action = AppAction::PopScreenAndRefresh;
            }

            // Display the "Register Name" button
            if ui.button("Register Name").clicked() {
                let mut screen = RegisterDpnsNameScreen::new(&self.app_context);
                if let Some(identity_id) = self.successful_qualified_identity_id {
                    screen.select_identity(identity_id);
                    screen.show_identity_selector = false;
                }
                // Handle the registration of a new name
                action = AppAction::PopThenAddScreenToMainScreen(
                    RootScreenType::RootScreenDPNSOwnedNames,
                    Screen::RegisterDpnsNameScreen(screen),
                );
            }
        });

        action
    }
}
