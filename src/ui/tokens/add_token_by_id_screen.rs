use std::sync::Arc;

use chrono::Utc;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::DataContract;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Color32, Context, RichText, Ui};

use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTaskSuccessResult;
use crate::database::contracts::InsertTokensToo;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::tokens::tokens_screen::TokenInfo;
use crate::{
    app::AppAction,
    backend_task::{tokens::TokenTask, BackendTask},
    context::AppContext,
    ui::{components::top_panel::add_top_panel, MessageType, ScreenLike},
};

/// UI state during the add-token flow.
#[derive(PartialEq, Clone)]
enum AddTokenStatus {
    Idle,
    Searching(u32),
    FoundSingle(TokenInfo),
    FoundMultiple(Vec<TokenInfo>),
    Error(String),
    Complete,
}

pub struct AddTokenByIdScreen {
    pub app_context: Arc<AppContext>,
    contract_id_input: String,

    fetched_contract: Option<DataContract>,

    status: AddTokenStatus,
    selected_token: Option<TokenInfo>,

    error_message: Option<String>,
}

impl AddTokenByIdScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            contract_id_input: String::default(),
            fetched_contract: None,
            status: AddTokenStatus::Idle,
            selected_token: None,
            error_message: None,
        }
    }

    fn render_search_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.horizontal(|ui| {
            ui.label("Contract Identifier:");
            ui.text_edit_singleline(&mut self.contract_id_input);
        });

        ui.add_space(10.0);
        if ui
            .add_enabled(
                !self.contract_id_input.is_empty(),
                egui::Button::new("Search"),
            )
            .clicked()
        {
            let now = Utc::now().timestamp() as u32;
            self.status = AddTokenStatus::Searching(now);
            self.error_message = None;

            if !self.contract_id_input.is_empty() {
                // Search by contract
                if let Ok(contract_id) =
                    Identifier::from_string(&self.contract_id_input, Encoding::Base58)
                {
                    action = AppAction::BackendTask(BackendTask::TokenTask(
                        TokenTask::FetchTokenByContractId(contract_id),
                    ));
                } else {
                    self.status =
                        AddTokenStatus::Error("Invalid contract identifier format".into());
                }
            }
        }

        action
    }

    fn render_search_results(&mut self, ui: &mut Ui) {
        match self.status.clone() {
            // clone → no borrow
            AddTokenStatus::FoundSingle(token) => {
                ui.label(format!("Found token: {}", token.token_name));
                self.selected_token = Some(token);
            }
            AddTokenStatus::FoundMultiple(tokens) => {
                ui.label("Multiple tokens found, select one:");
                ui.add_space(5.0);

                for tok in &tokens {
                    if ui
                        .selectable_value(
                            &mut self.selected_token,
                            Some(tok.clone()),
                            format!("{} ({})", tok.token_name, tok.token_id),
                        )
                        .clicked()
                    {
                        self.status = AddTokenStatus::FoundSingle(tok.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn render_add_button(&mut self, ui: &mut Ui) -> AppAction {
        if let (Some(contract), Some(tok)) = (&self.fetched_contract, &self.selected_token) {
            if ui
                .add(
                    egui::Button::new(RichText::new("Add Token").color(Color32::WHITE))
                        .fill(Color32::from_rgb(0, 120, 0)),
                )
                .clicked()
            {
                let insert_mode =
                    InsertTokensToo::SomeTokensShouldBeAdded(vec![tok.token_position]);

                // None for alias; change if you allow user alias input
                return AppAction::BackendTasks(
                    vec![
                        BackendTask::ContractTask(ContractTask::SaveDataContract(
                            contract.clone(),
                            None,
                            insert_mode,
                        )),
                        BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
                    ],
                    crate::app::BackendTasksExecutionMode::Sequential,
                );
            }
        }
        AppAction::None
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Token Added Successfully!");

            ui.add_space(20.0);
            if ui.button("Add another token").clicked() {
                self.status = AddTokenStatus::Idle;
                self.contract_id_input.clear();
                self.fetched_contract = None;
                self.selected_token = None;
            }

            if ui.button("Back to Tokens screen").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }
}

impl ScreenLike for AddTokenByIdScreen {
    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
        match msg_type {
            MessageType::Success => {
                if msg.contains("DataContract successfully saved") {
                    self.status = AddTokenStatus::Complete;
                } else if msg.contains("Contract not found") {
                    self.status = AddTokenStatus::Error("Contract not found".into());
                } else if msg.contains("Error fetching contracts") {
                    self.status = AddTokenStatus::Error(msg.to_owned());
                }
            }
            MessageType::Error => {
                self.status = AddTokenStatus::Error(msg.to_owned());
                self.error_message = Some(msg.to_owned());
            }
            MessageType::Info => {}
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::FetchedContract(contract) = backend_task_success_result {
            // 1. Bail out if the contract has no tokens
            if contract.tokens().is_empty() {
                self.status = AddTokenStatus::Error("Contract has no token definitions".into());
                return;
            }

            // 2. Convert each token definition into TokenInfo
            let mut token_infos: Vec<TokenInfo> = contract
                .tokens()
                .iter()
                .map(|(pos, cfg)| {
                    let token_name = cfg
                        .conventions()
                        .singular_form_by_language_code_or_default("en")
                        .to_string();

                    TokenInfo {
                        token_id: contract
                            .token_id(*pos)
                            .expect("token_id must exist for position"),
                        token_name,
                        data_contract_id: contract.id(),
                        token_position: *pos as u16,
                        token_configuration: cfg.clone(),
                        description: cfg.description().clone(),
                    }
                })
                .collect();

            // 3. Decide which status to show
            if token_infos.len() == 1 {
                self.status = AddTokenStatus::FoundSingle(token_infos.remove(0));
            } else {
                // Optionally keep list sorted by name
                token_infos.sort_by(|a, b| a.token_name.cmp(&b.token_name));
                self.status = AddTokenStatus::FoundMultiple(token_infos);
            }

            // 4. Store the contract so we can save it later
            self.fetched_contract = Some(contract);
        }
    }

    fn refresh(&mut self) {
        // nothing to refresh
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                ("Add Token", AppAction::None),
            ],
            vec![],
        );

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        egui::CentralPanel::default().show(ctx, |ui| {
            // If we are in the "Complete" status, just show success screen
            if self.status == AddTokenStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Add Token");
            ui.add_space(10.0);

            // Input and search
            action |= self.render_search_inputs(ui);

            if let AddTokenStatus::Searching(start_time) = self.status {
                ui.add_space(10.0);
                let elapsed_seconds = Utc::now().timestamp() as u32 - start_time;
                ui.label(format!("Searching... {} seconds elapsed", elapsed_seconds));
            }

            ui.add_space(10.0);
            self.render_search_results(ui);

            if let AddTokenStatus::Error(err) = &self.status {
                ui.colored_label(Color32::DARK_RED, err);
            }

            ui.add_space(10.0);
            action |= self.render_add_button(ui);
        });

        action
    }
}
