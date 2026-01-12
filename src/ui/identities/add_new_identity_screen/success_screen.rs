use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreen;
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameScreen;
use crate::ui::{RootScreenType, Screen};
use egui::Ui;

impl AddNewIdentityScreen {
    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        // Prepare fee info for display
        let fee_info = self.completed_fee_result.as_ref().map(|fee_result| {
            let fee_str = format!(
                "Estimated: {}  •  Actual: {}",
                format_credits_as_dash(fee_result.estimated_fee),
                format_credits_as_dash(fee_result.actual_fee)
            );
            ("Transaction Fee".to_string(), fee_str)
        });
        let fee_ref = fee_info.as_ref().map(|(title, desc)| (title.as_str(), desc.as_str()));

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
            fee_ref,
        );

        // Handle the custom action to navigate to DPNS registration
        if let AppAction::Custom(ref s) = action
            && s == "register_dpns"
        {
            let mut screen = RegisterDpnsNameScreen::new(&self.app_context);
            if let Some(identity_id) = self.successful_qualified_identity_id {
                screen.select_identity(identity_id);
                screen.show_identity_selector = false;
            }
            return AppAction::PopThenAddScreenToMainScreen(
                RootScreenType::RootScreenDPNSOwnedNames,
                Screen::RegisterDpnsNameScreen(screen),
            );
        }

        action
    }
}
