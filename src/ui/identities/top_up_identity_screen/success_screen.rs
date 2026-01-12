use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::identities::top_up_identity_screen::TopUpIdentityScreen;
use egui::Ui;

impl TopUpIdentityScreen {
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

        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Identity Topped Up Successfully!".to_string(),
            vec![(
                "Back to Identities".to_string(),
                AppAction::PopScreenAndRefresh,
            )],
            fee_ref,
        )
    }
}
