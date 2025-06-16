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
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::tokens::tokens_screen::TokenInfo;
use crate::{
    app::AppAction,
    backend_task::{tokens::TokenTask, BackendTask},
    context::AppContext,
    ui::{components::top_panel::add_top_panel, theme::DashColors, MessageType, ScreenLike},
};

/// UI state during the add-token flow.
#[derive(PartialEq, Clone)]
enum AddTokenStatus {
    Idle,
    Searching(u32),
    FoundSingle(Box<TokenInfo>),
    FoundMultiple(Vec<TokenInfo>),
    Error(String),
    Complete,
}

pub struct AddTokenByIdScreen {
    pub app_context: Arc<AppContext>,
    contract_or_token_id_input: String,

    fetched_contract: Option<DataContract>,

    status: AddTokenStatus,
    selected_token: Option<TokenInfo>,

    error_message: Option<String>,
    try_token_id_next: bool,
}

impl AddTokenByIdScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            contract_or_token_id_input: String::default(),
            fetched_contract: None,
            status: AddTokenStatus::Idle,
            selected_token: None,
            error_message: None,
            try_token_id_next: false,
        }
    }

    fn render_search_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.horizontal(|ui| {
            ui.label("Contract or Token ID:");
            ui.text_edit_singleline(&mut self.contract_or_token_id_input);
        });

        ui.add_space(10.0);
        if ui
            .add_enabled(
                !self.contract_or_token_id_input.is_empty(),
                egui::Button::new("Search"),
            )
            .clicked()
        {
            let now = Utc::now().timestamp() as u32;
            self.status = AddTokenStatus::Searching(now);
            self.error_message = None;

            if !self.contract_or_token_id_input.is_empty() {
                // Try to parse as identifier
                if let Ok(identifier) =
                    Identifier::from_string(&self.contract_or_token_id_input, Encoding::Base58)
                {
                    // First try as contract ID
                    action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                        TokenTask::FetchTokenByContractId(identifier),
                    )));
                } else {
                    self.status = AddTokenStatus::Error("Invalid identifier format".into());
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
                self.selected_token = Some(*token);
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
                        self.status = AddTokenStatus::FoundSingle(Box::new(tok.clone()));
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

                // Set status to show we're processing
                self.status = AddTokenStatus::Searching(chrono::Utc::now().timestamp() as u32);

                // None for alias; change if you allow user alias input
                return AppAction::BackendTasks(
                    vec![
                        BackendTask::ContractTask(Box::new(ContractTask::SaveDataContract(
                            contract.clone(),
                            None,
                            insert_mode,
                        ))),
                        BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
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
            ui.heading(
                RichText::new("Token Added Successfully")
                    .color(Color32::from_rgb(0, 150, 0))
                    .size(24.0),
            );

            ui.add_space(10.0);
            if let Some(token) = &self.selected_token {
                ui.label(format!(
                    "'{}' has been added to your tokens.",
                    token.token_name
                ));
            }

            ui.add_space(20.0);
            if ui.button("Add another token").clicked() {
                self.status = AddTokenStatus::Idle;
                self.contract_or_token_id_input.clear();
                self.fetched_contract = None;
                self.selected_token = None;
                self.try_token_id_next = false;
            }

            if ui.button("Back to Tokens screen").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }

    fn handle_fetched_contract(
        &mut self,
        contract: DataContract,
        specific_token_position: Option<dash_sdk::dpp::data_contract::TokenContractPosition>,
    ) {
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
                    token_position: { *pos },
                    token_configuration: cfg.clone(),
                    description: cfg.description().clone(),
                }
            })
            .collect();

        // 3. Decide which status to show
        if let Some(position) = specific_token_position {
            // If we have a specific token position (from token ID query), find and select that token
            if let Some(token_info) = token_infos
                .into_iter()
                .find(|t| t.token_position == position)
            {
                self.status = AddTokenStatus::FoundSingle(Box::new(token_info));
            } else {
                self.status = AddTokenStatus::Error("Token position not found in contract".into());
                return;
            }
        } else if token_infos.len() == 1 {
            self.status = AddTokenStatus::FoundSingle(Box::new(token_infos.remove(0)));
        } else {
            // Optionally keep list sorted by name
            token_infos.sort_by(|a, b| a.token_name.cmp(&b.token_name));
            self.status = AddTokenStatus::FoundMultiple(token_infos);
        }

        // 4. Store the contract so we can save it later
        self.fetched_contract = Some(contract);
    }
}

impl ScreenLike for AddTokenByIdScreen {
    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
        match msg_type {
            MessageType::Success => {
                if msg.contains("DataContract successfully saved") {
                    self.status = AddTokenStatus::Complete;
                } else if msg.contains("Contract not found") {
                    // Contract not found, try as token ID
                    if let Ok(_identifier) =
                        Identifier::from_string(&self.contract_or_token_id_input, Encoding::Base58)
                    {
                        // We'll initiate a token ID search
                        self.try_token_id_next = true;
                    } else {
                        self.status = AddTokenStatus::Error("Contract not found".into());
                    }
                } else if msg.contains("Token not found") {
                    self.status = AddTokenStatus::Error("Token not found".into());
                } else if msg.contains("Error fetching contracts") {
                    self.status = AddTokenStatus::Error(msg.to_owned());
                }
            }
            MessageType::Error => {
                // Handle any error during the add token process
                if msg.contains("Error inserting contract into the database") {
                    self.status = AddTokenStatus::Error("Failed to add token to database".into());
                } else {
                    self.status = AddTokenStatus::Error(msg.to_owned());
                }
                self.error_message = Some(msg.to_owned());
            }
            MessageType::Info => {}
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::FetchedContract(contract) => {
                self.handle_fetched_contract(contract, None);
            }
            BackendTaskSuccessResult::FetchedContractWithTokenPosition(
                contract,
                token_position,
            ) => {
                self.handle_fetched_contract(contract, Some(token_position));
            }
            _ => {}
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

        action |= island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // If we are in the "Complete" status, just show success screen
            if self.status == AddTokenStatus::Complete {
                return self.show_success_screen(ui);
            }

            ui.heading("Add Token");
            ui.add_space(10.0);

            ui.label("Enter either a Contract ID or Token ID to search for tokens.");
            ui.add_space(5.0);

            let mut inner_action = AppAction::None;

            // Check if we need to try token ID search
            if self.try_token_id_next {
                self.try_token_id_next = false;
                if let Ok(identifier) =
                    Identifier::from_string(&self.contract_or_token_id_input, Encoding::Base58)
                {
                    let now = Utc::now().timestamp() as u32;
                    self.status = AddTokenStatus::Searching(now);
                    inner_action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                        TokenTask::FetchTokenByTokenId(identifier),
                    )));
                }
            }

            // Input and search
            let search_action = self.render_search_inputs(ui);
            inner_action |= search_action;

            if let AddTokenStatus::Searching(start_time) = self.status {
                ui.add_space(10.0);
                let elapsed_seconds = Utc::now().timestamp() as u32 - start_time;

                // Show different messages based on whether we have a token selected
                if self.selected_token.is_some() {
                    ui.label(format!(
                        "Adding token... {} seconds elapsed",
                        elapsed_seconds
                    ));
                } else {
                    ui.label(format!("Searching... {} seconds elapsed", elapsed_seconds));
                }
            }

            ui.add_space(10.0);
            self.render_search_results(ui);

            if let AddTokenStatus::Error(err) = &self.status {
                ui.add_space(10.0);
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    format!("Error: {}", err),
                );
            }

            ui.add_space(10.0);
            inner_action |= self.render_add_button(ui);

            // Show any additional error messages
            if let Some(error_msg) = &self.error_message {
                ui.add_space(5.0);
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    format!("Details: {}", error_msg),
                );
            }

            inner_action
        });

        action
    }
}
