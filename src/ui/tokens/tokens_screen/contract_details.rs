use crate::app::AppAction;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::tokens::tokens_screen::TokensScreen;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::Ui;

impl TokensScreen {
    /// Renders details for the selected_contract_id.
    pub(super) fn render_contract_details(
        &mut self,
        ui: &mut Ui,
        contract_id: &Identifier,
    ) -> AppAction {
        let mut action = AppAction::None;

        // Show loading spinner if data is being fetched
        if self.contract_details_loading {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.heading("Loading contract details...");
                ui.add_space(20.0);
                ui.add(egui::widgets::Spinner::default().size(50.0));
            });
            return action;
        }

        if let Some(description) = &self.selected_contract_description {
            ui.heading("Contract Description:");
            ui.add_space(10.0);
            ui.label(description.description.clone());
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.heading("Tokens:");
        let token_infos = self
            .selected_token_infos
            .iter()
            .filter(|token| token.data_contract_id == *contract_id)
            .cloned()
            .collect::<Vec<_>>();
        for token in token_infos {
            if token.data_contract_id == *contract_id {
                ui.add_space(10.0);
                ui.heading(format!("• {}", token.token_name.clone()));
                ui.add_space(10.0);
                ui.label(format!(
                    "ID: {}",
                    token.token_id.to_string(Encoding::Base58)
                ));
                ui.label(format!(
                    "Description: {}",
                    token
                        .description
                        .clone()
                        .unwrap_or("No description".to_string())
                ));
            }

            ui.add_space(10.0);

            // Add button to add token to my tokens
            ui.horizontal(|ui| {
                if ui.button("Add to My Tokens").clicked() {
                    match self.add_token_to_tracked_tokens(token.clone()) {
                        Ok(internal_action) => {
                            // Add token to my tokens
                            action |= internal_action;
                        }
                        Err(e) => {
                            self.set_error_message(Some(e));
                        }
                    }
                }
                if ui.button("View schema").clicked() {
                    // Show a popup window with the schema
                    match serde_json::to_string_pretty(&token.token_configuration) {
                        Ok(schema) => {
                            self.show_json_popup = true;
                            self.json_popup_text = schema;
                        }
                        Err(e) => {
                            self.set_error_message(Some(e.to_string()));
                        }
                    }
                }
            });

            ui.add_space(20.0);
        }

        action
    }
}
