use std::sync::Arc;

use chrono::Utc;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::DataContract;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Context, Ui};

use crate::ui::theme::ComponentStyles;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::contract::ContractTask;
use crate::database::contracts::InsertTokensToo;
use crate::ui::components::MessageBanner;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::tokens::tokens_screen::TokenInfo;
use crate::{
    app::AppAction,
    backend_task::{BackendTask, tokens::TokenTask},
    context::AppContext,
    ui::{MessageType, ScreenLike, components::top_panel::add_top_panel},
};

/// UI state during the add-token flow.
#[derive(Debug, PartialEq, Clone)]
enum AddTokenStatus {
    Idle,
    Searching(u32),
    FoundSingle(Box<TokenInfo>),
    FoundMultiple(Vec<TokenInfo>),
    Error,
    Complete,
}

#[derive(Debug, PartialEq, Eq)]
enum ContractNotFoundResolution {
    RetryWithTokenIdLookup,
    Error(&'static str),
}

const NO_CONTRACT_OR_TOKEN_FOUND_MESSAGE: &str = "No contract or token found for the given identifier. Verify the ID, check the selected network, and try again.";
const TOKEN_CONTRACT_MISSING_MESSAGE: &str = "Token was found, but its data contract could not be fetched. Check the selected network and try again.";

fn resolve_contract_not_found(
    input: &str,
    tried_token_id_lookup: bool,
) -> ContractNotFoundResolution {
    if tried_token_id_lookup {
        ContractNotFoundResolution::Error(TOKEN_CONTRACT_MISSING_MESSAGE)
    } else if Identifier::from_string(input, Encoding::Base58).is_ok() {
        ContractNotFoundResolution::RetryWithTokenIdLookup
    } else {
        ContractNotFoundResolution::Error(NO_CONTRACT_OR_TOKEN_FOUND_MESSAGE)
    }
}

fn token_not_found_error_message() -> &'static str {
    NO_CONTRACT_OR_TOKEN_FOUND_MESSAGE
}

pub struct AddTokenByIdScreen {
    pub app_context: Arc<AppContext>,
    contract_or_token_id_input: String,

    fetched_contract: Option<DataContract>,

    status: AddTokenStatus,
    selected_token: Option<TokenInfo>,

    try_token_id_next: bool,
    tried_token_id_lookup: bool,
}

impl AddTokenByIdScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            contract_or_token_id_input: String::default(),
            fetched_contract: None,
            status: AddTokenStatus::Idle,
            selected_token: None,
            try_token_id_next: false,
            tried_token_id_lookup: false,
        }
    }

    fn begin_lookup(&mut self, start_time: u32) {
        self.status = AddTokenStatus::Searching(start_time);
        self.fetched_contract = None;
        self.selected_token = None;
        self.try_token_id_next = false;
        self.tried_token_id_lookup = false;
    }

    fn render_search_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let is_searching = matches!(self.status, AddTokenStatus::Searching(_));

        ui.horizontal(|ui| {
            ui.label("Contract or Token ID:");
            ui.add_enabled(
                !is_searching,
                egui::TextEdit::singleline(&mut self.contract_or_token_id_input),
            );
        });

        ui.add_space(10.0);
        if ui
            .add_enabled(
                !self.contract_or_token_id_input.is_empty() && !is_searching,
                egui::Button::new("Search"),
            )
            .clicked()
        {
            let now = Utc::now().timestamp() as u32;
            self.begin_lookup(now);

            // Try to parse as identifier
            if let Ok(identifier) =
                Identifier::from_string(&self.contract_or_token_id_input, Encoding::Base58)
            {
                // First try as contract ID
                action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                    TokenTask::FetchTokenByContractId(identifier),
                )));
            } else {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Invalid identifier format",
                    MessageType::Error,
                );
                self.status = AddTokenStatus::Error;
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
        if let (Some(contract), Some(tok)) = (&self.fetched_contract, &self.selected_token)
            && ComponentStyles::add_primary_button(ui, "Import Token").clicked()
        {
            let insert_mode = InsertTokensToo::SomeTokensShouldBeAdded(vec![tok.token_position]);

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
        AppAction::None
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let action = crate::ui::helpers::show_success_screen(
            ui,
            "Token Added Successfully".to_string(),
            vec![
                (
                    "Add another token".to_string(),
                    AppAction::Custom("add_another".to_string()),
                ),
                (
                    "Back to Tokens screen".to_string(),
                    AppAction::PopScreenAndRefresh,
                ),
            ],
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "add_another"
        {
            self.status = AddTokenStatus::Idle;
            self.contract_or_token_id_input.clear();
            self.fetched_contract = None;
            self.selected_token = None;
            self.try_token_id_next = false;
            self.tried_token_id_lookup = false;
            return AppAction::None;
        }

        action
    }

    /// Handles contract-not-found by attempting a one-time fallback to token ID lookup.
    /// Used by both `display_message` and `display_task_result`.
    fn handle_contract_not_found(&mut self) {
        match resolve_contract_not_found(
            &self.contract_or_token_id_input,
            self.tried_token_id_lookup,
        ) {
            ContractNotFoundResolution::RetryWithTokenIdLookup => {
                self.try_token_id_next = true;
                self.tried_token_id_lookup = true;
            }
            ContractNotFoundResolution::Error(message) => {
                self.try_token_id_next = false;
                MessageBanner::set_global(self.app_context.egui_ctx(), message, MessageType::Error);
                self.status = AddTokenStatus::Error;
            }
        }
    }

    fn handle_fetched_contract(
        &mut self,
        contract: DataContract,
        specific_token_position: Option<dash_sdk::dpp::data_contract::TokenContractPosition>,
    ) {
        // 1. Bail out if the contract has no tokens
        if contract.tokens().is_empty() {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Contract has no token definitions",
                MessageType::Error,
            );
            self.status = AddTokenStatus::Error;
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
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Token position not found in contract",
                    MessageType::Error,
                );
                self.status = AddTokenStatus::Error;
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
        // Banner display is handled globally by AppState; this is only for side-effects.
        match msg_type {
            MessageType::Success => {
                if msg.contains("DataContract successfully saved") {
                    self.status = AddTokenStatus::Complete;
                } else if msg.contains("Error fetching contracts") {
                    self.status = AddTokenStatus::Error;
                }
            }
            MessageType::Error | MessageType::Warning => {
                self.status = AddTokenStatus::Error;
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
            BackendTaskSuccessResult::ContractNotFound => {
                self.handle_contract_not_found();
            }
            BackendTaskSuccessResult::TokenNotFound => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    token_not_found_error_message(),
                    MessageType::Error,
                );
                self.status = AddTokenStatus::Error;
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
                ("Import Token", AppAction::None),
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
            // If we are in the "Complete" status, just show success screen
            if self.status == AddTokenStatus::Complete {
                return self.show_success_screen(ui);
            }

            ui.heading("Import Token");
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

            ui.add_space(10.0);
            inner_action |= self.render_add_button(ui);

            inner_action
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Once};

    use dash_sdk::dpp::dashcore::Network;

    use crate::app_dir::copy_env_file_if_not_exists;
    use crate::context::AppContext;
    use crate::database::Database;
    use crate::ui::ScreenLike;

    use super::{
        AddTokenByIdScreen, AddTokenStatus, ContractNotFoundResolution,
        NO_CONTRACT_OR_TOKEN_FOUND_MESSAGE, TOKEN_CONTRACT_MISSING_MESSAGE,
        resolve_contract_not_found, token_not_found_error_message,
    };
    use crate::backend_task::BackendTaskSuccessResult;

    fn ensure_test_env() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            copy_env_file_if_not_exists();

            unsafe {
                std::env::set_var("MAINNET_dapi_addresses", "http://127.0.0.1:1443");
                std::env::set_var("MAINNET_core_host", "127.0.0.1");
                std::env::set_var("MAINNET_core_rpc_port", "9998");
                std::env::set_var("MAINNET_core_rpc_user", "dashrpc");
                std::env::set_var("MAINNET_core_rpc_password", "password");

                std::env::set_var("LOCAL_dapi_addresses", "http://127.0.0.1:2443");
                std::env::set_var("LOCAL_core_host", "127.0.0.1");
                std::env::set_var("LOCAL_core_rpc_port", "20302");
                std::env::set_var("LOCAL_core_rpc_user", "dashmate");
                std::env::set_var("LOCAL_core_rpc_password", "password");
            }
        });
    }

    fn test_screen(test_name: &str) -> AddTokenByIdScreen {
        ensure_test_env();

        let db_file_path = format!("test_db_add_token_by_id_screen_{test_name}");
        let _ = std::fs::remove_file(&db_file_path);
        let db = Arc::new(Database::new(&db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        let app_context = AppContext::new(
            crate::app_dir::app_user_data_dir_path().unwrap(),
            Network::Regtest,
            db,
            None,
            Default::default(),
            Default::default(),
            egui::Context::default(),
        )
        .expect("Expected to create AppContext");
        let screen = AddTokenByIdScreen::new(&app_context);

        let _ = std::fs::remove_file(&db_file_path);
        screen
    }

    #[test]
    fn first_contract_not_found_retries_with_token_id_lookup() {
        let resolution =
            resolve_contract_not_found("7guyd1YceqT5S8xqA4UY9fZPpQn2J39GQJ4j3Nf7wV6H", false);

        assert_eq!(
            resolution,
            ContractNotFoundResolution::RetryWithTokenIdLookup
        );
    }

    #[test]
    fn second_contract_not_found_becomes_error() {
        let resolution =
            resolve_contract_not_found("7guyd1YceqT5S8xqA4UY9fZPpQn2J39GQJ4j3Nf7wV6H", true);

        assert_eq!(
            resolution,
            ContractNotFoundResolution::Error(TOKEN_CONTRACT_MISSING_MESSAGE)
        );
    }

    #[test]
    fn contract_not_found_retries_with_token_id_when_base58_is_valid() {
        let mut screen = test_screen("contract_not_found_retries");
        screen.contract_or_token_id_input =
            "7guyd1YceqT5S8xqA4UY9fZPpQn2J39GQJ4j3Nf7wV6H".to_string();
        screen.status = AddTokenStatus::Searching(42);
        screen.tried_token_id_lookup = false;

        screen.display_task_result(BackendTaskSuccessResult::ContractNotFound);

        assert_eq!(screen.status, AddTokenStatus::Searching(42));
        assert!(screen.try_token_id_next);
        assert!(screen.tried_token_id_lookup);
    }

    #[test]
    fn contract_not_found_errors_when_base58_is_invalid() {
        let mut screen = test_screen("contract_not_found_invalid");
        screen.contract_or_token_id_input = "not-base58".to_string();
        screen.status = AddTokenStatus::Searching(42);
        screen.tried_token_id_lookup = false;

        screen.display_task_result(BackendTaskSuccessResult::ContractNotFound);

        assert_eq!(screen.status, AddTokenStatus::Error);
        assert!(!screen.try_token_id_next);
        assert!(!screen.tried_token_id_lookup);
    }

    #[test]
    fn contract_not_found_errors_after_token_id_lookup_was_tried() {
        let mut screen = test_screen("contract_not_found_second_pass");
        screen.contract_or_token_id_input =
            "7guyd1YceqT5S8xqA4UY9fZPpQn2J39GQJ4j3Nf7wV6H".to_string();
        screen.status = AddTokenStatus::Searching(42);
        screen.tried_token_id_lookup = true;

        screen.display_task_result(BackendTaskSuccessResult::ContractNotFound);

        assert_eq!(screen.status, AddTokenStatus::Error);
        assert!(!screen.try_token_id_next);
        assert!(screen.tried_token_id_lookup);
    }

    #[test]
    fn token_not_found_while_searching_sets_error_status() {
        let mut screen = test_screen("token_not_found");
        screen.status = AddTokenStatus::Searching(42);

        screen.display_task_result(BackendTaskSuccessResult::TokenNotFound);

        assert_eq!(screen.status, AddTokenStatus::Error);
        assert!(!screen.try_token_id_next);
        assert!(!screen.tried_token_id_lookup);
    }

    #[test]
    fn token_not_found_maps_to_error_message() {
        assert_eq!(
            token_not_found_error_message(),
            NO_CONTRACT_OR_TOKEN_FOUND_MESSAGE
        );
    }
}
