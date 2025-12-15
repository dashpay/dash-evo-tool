use crate::ui::tokens::tokens_screen::TokensScreen;
use crate::{app::AppAction, ui::theme::DashColors};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use egui::{Frame, Margin, Ui};

impl TokensScreen {
    /// Renders details for the selected_contract_id.
    pub(super) fn render_contract_details(
        &mut self,
        ui: &mut Ui,
        contract_id: &Identifier,
    ) -> AppAction {
        let mut action = AppAction::None;

        let mut go_back = false;
        ui.horizontal(|ui| {
            if ui.button("Back to Search Results").clicked() {
                go_back = true;
            }
        });

        if go_back {
            self.selected_contract_id = None;
            self.contract_details_loading = false;
            self.selected_contract_description = None;
            self.selected_token_infos.clear();
            return action;
        }

        ui.add_space(10.0);

        // Show loading spinner if data is being fetched
        if self.contract_details_loading {
            ui.horizontal(|ui| {
                ui.label("Loading contract details...");
                ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
            });
            return action;
        }

        if let Some(description) = &self.selected_contract_description {
            ui.heading("Contract Description:");
            ui.add_space(10.0);
            ui.label(description.description.clone());
            ui.add_space(10.0);
            ui.separator();
        }

        ui.add_space(10.0);

        ui.heading("Tokens:");
        let token_infos = self
            .selected_token_infos
            .iter()
            .filter(|token| token.data_contract_id == *contract_id)
            .cloned()
            .collect::<Vec<_>>();
        let visuals = ui.visuals().clone();
        for token in token_infos {
            ui.add_space(10.0);
            Frame::group(ui.style())
                .stroke(visuals.widgets.noninteractive.bg_stroke)
                .fill(visuals.extreme_bg_color)
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.heading(token.token_name.clone());
                    ui.add_space(6.0);
                    ui.label(format!(
                        "ID: {}",
                        token.token_id.to_string(Encoding::Base58)
                    ));
                    let description = token
                        .description
                        .clone()
                        .unwrap_or_else(|| "No description".to_string());
                    ui.label(format!("Description: {}", description));

                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button("Add to My Tokens").clicked() {
                            match self.add_token_to_tracked_tokens(token.clone()) {
                                Ok(internal_action) => {
                                    action |= internal_action;
                                }
                                Err(e) => {
                                    self.token_creator_error_message = Some(e);
                                }
                            }
                        }
                        if ui.button("View schema").clicked() {
                            match serde_json::to_string_pretty(&token.token_configuration) {
                                Ok(schema) => {
                                    self.show_json_popup = true;
                                    self.json_popup_text = schema;
                                }
                                Err(e) => {
                                    self.token_creator_error_message = Some(e.to_string());
                                }
                            }
                        }
                    });
                });
        }

        action
    }
}
