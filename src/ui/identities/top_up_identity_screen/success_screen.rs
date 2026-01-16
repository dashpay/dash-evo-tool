use crate::app::AppAction;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use egui::Ui;

impl TopUpIdentityScreen {
    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Identity Topped Up Successfully!".to_string(),
            vec![(
                "Back to Identities".to_string(),
                AppAction::PopScreenAndRefresh,
            )],
            None,
        )
    }
}
