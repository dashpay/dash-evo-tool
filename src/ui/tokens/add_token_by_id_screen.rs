use std::sync::Arc;

use chrono::Utc;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::DataContract;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Ui};

use crate::ui::theme::ComponentStyles;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::contract::ContractTask;
use crate::model::qualified_contract::InsertTokensToo;
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

/// Which lookup is in flight, so `display_task_result` can disambiguate a
/// `ContractNotFound`: fall back to a token-ID search vs. report a genuine
/// missing-contract error.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SearchKind {
    ByContractId,
    ByTokenId,
    Saving,
}

/// UI state during the add-token flow.
#[derive(Debug, PartialEq, Clone)]
enum AddTokenStatus {
    Idle,
    Searching(SearchKind, u32),
    FoundSingle(Box<TokenInfo>),
    FoundMultiple(Vec<TokenInfo>),
    Error,
    Complete,
}

pub struct AddTokenByIdScreen {
    pub app_context: Arc<AppContext>,
    contract_or_token_id_input: String,

    fetched_contract: Option<DataContract>,

    status: AddTokenStatus,
    selected_token: Option<TokenInfo>,

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
            self.status = AddTokenStatus::Searching(SearchKind::ByContractId, now);

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
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Invalid identifier format",
                        MessageType::Error,
                    );
                    self.status = AddTokenStatus::Error;
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
        if let (Some(contract), Some(tok)) = (&self.fetched_contract, &self.selected_token)
            && ComponentStyles::add_primary_button(ui, "Import Token").clicked()
        {
            let insert_mode = InsertTokensToo::SomeTokensShouldBeAdded(vec![tok.token_position]);

            // Set status to show we're processing
            self.status =
                AddTokenStatus::Searching(SearchKind::Saving, Utc::now().timestamp() as u32);

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
            return AppAction::None;
        }

        action
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
    fn display_message(&mut self, _msg: &str, msg_type: MessageType) {
        // Status transitions are driven by typed results in `display_task_result`.
        // React only to the message *type* here, never to its text.
        match msg_type {
            MessageType::Error | MessageType::Warning => {
                self.status = AddTokenStatus::Error;
            }
            MessageType::Success | MessageType::Info => {}
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
                if matches!(
                    self.status,
                    AddTokenStatus::Searching(SearchKind::ByContractId, _)
                ) {
                    // The input was not a contract ID; try it as a token ID next
                    // frame (the search is dispatched from `ui()`).
                    self.try_token_id_next = true;
                } else {
                    // A token was found but its contract is missing — a genuine
                    // data inconsistency, so we do not loop back into a search.
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "This token was found, but its contract could not be loaded right now. Please try again in a little while.",
                        MessageType::Error,
                    );
                    self.status = AddTokenStatus::Error;
                }
            }
            BackendTaskSuccessResult::TokenNotFound => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "No token or contract was found for that ID. Double-check the ID and try again.",
                    MessageType::Error,
                );
                self.status = AddTokenStatus::Error;
            }
            BackendTaskSuccessResult::SavedContract => {
                self.status = AddTokenStatus::Complete;
            }
            _ => {}
        }
    }

    fn refresh(&mut self) {
        // nothing to refresh
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                ("Import Token", AppAction::None),
            ],
            vec![],
        );

        // Left panel
        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ui, &self.app_context);

        action |= island_central_panel(ui, |ui| {
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
                    self.status = AddTokenStatus::Searching(SearchKind::ByTokenId, now);
                    inner_action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                        TokenTask::FetchTokenByTokenId(identifier),
                    )));
                }
            }

            // Input and search
            let search_action = self.render_search_inputs(ui);
            inner_action |= search_action;

            if let AddTokenStatus::Searching(kind, start_time) = self.status {
                ui.add_space(10.0);
                let elapsed_seconds = Utc::now().timestamp() as u32 - start_time;

                if matches!(kind, SearchKind::Saving) {
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
    use super::*;
    use crate::app::AppState;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn data_dir_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Runs `f` in a unique temp data dir with a Tokio runtime in context, so
    /// `AppState::new()` neither touches the real user data dir nor races other
    /// test threads on `DASH_EVO_DATA_DIR`.
    fn with_isolated_dir<R>(f: impl FnOnce() -> R) -> R {
        let lock = data_dir_lock();
        let tmp = tempfile::tempdir().expect("create temp data dir");
        let prior = std::env::var("DASH_EVO_DATA_DIR").ok();
        // Safety: serialized by `lock`; env var is restored below before it drops.
        unsafe {
            std::env::set_var("DASH_EVO_DATA_DIR", tmp.path());
        }
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();
        let result = f();
        drop(_guard);
        drop(rt);
        // Safety: serialized by `lock`.
        unsafe {
            match &prior {
                Some(v) => std::env::set_var("DASH_EVO_DATA_DIR", v),
                None => std::env::remove_var("DASH_EVO_DATA_DIR"),
            }
        }
        drop(lock);
        drop(tmp);
        result
    }

    fn make_ctx() -> Arc<AppContext> {
        let app = AppState::new(egui::Context::default()).expect("AppState builds");
        app.current_app_context().clone()
    }

    fn screen_with(ctx: &Arc<AppContext>, status: AddTokenStatus) -> AddTokenByIdScreen {
        let mut screen = AddTokenByIdScreen::new(ctx);
        screen.status = status;
        screen
    }

    #[test]
    fn display_task_result_drives_status_from_typed_variants() {
        with_isolated_dir(|| {
            let ctx = make_ctx();

            // Saving the contract completes the import.
            let mut screen = screen_with(&ctx, AddTokenStatus::Searching(SearchKind::Saving, 0));
            screen.display_task_result(BackendTaskSuccessResult::SavedContract);
            assert_eq!(screen.status, AddTokenStatus::Complete);

            // A missing contract during a contract-ID lookup retries as a token ID.
            let mut screen =
                screen_with(&ctx, AddTokenStatus::Searching(SearchKind::ByContractId, 0));
            screen.display_task_result(BackendTaskSuccessResult::ContractNotFound);
            assert!(
                screen.try_token_id_next,
                "contract-ID miss must fall back to a token-ID search"
            );
            assert!(
                matches!(
                    screen.status,
                    AddTokenStatus::Searching(SearchKind::ByContractId, _)
                ),
                "the fallback keeps searching; it must not flip to Error"
            );

            // A missing contract behind a found token is a genuine error, not a retry.
            let mut screen = screen_with(&ctx, AddTokenStatus::Searching(SearchKind::ByTokenId, 0));
            screen.display_task_result(BackendTaskSuccessResult::ContractNotFound);
            assert_eq!(screen.status, AddTokenStatus::Error);
            assert!(
                !screen.try_token_id_next,
                "a found token's missing contract must not loop back into a search"
            );

            // Nothing matched the ID at all.
            let mut screen = screen_with(&ctx, AddTokenStatus::Searching(SearchKind::ByTokenId, 0));
            screen.display_task_result(BackendTaskSuccessResult::TokenNotFound);
            assert_eq!(screen.status, AddTokenStatus::Error);

            // A background balance refresh must not disturb a completed import.
            let mut screen = screen_with(&ctx, AddTokenStatus::Complete);
            screen.display_task_result(BackendTaskSuccessResult::FetchedTokenBalances);
            assert_eq!(screen.status, AddTokenStatus::Complete);
        });
    }

    #[test]
    fn display_message_reacts_to_type_not_text() {
        with_isolated_dir(|| {
            let ctx = make_ctx();

            // Any error banner puts the screen into the Error state.
            let mut screen =
                screen_with(&ctx, AddTokenStatus::Searching(SearchKind::ByContractId, 0));
            screen.display_message("anything at all", MessageType::Error);
            assert_eq!(screen.status, AddTokenStatus::Error);

            // The old code flipped to Complete on this exact success text; it must not now.
            let mut screen =
                screen_with(&ctx, AddTokenStatus::Searching(SearchKind::ByContractId, 0));
            screen.display_message("DataContract successfully saved", MessageType::Success);
            assert!(
                matches!(
                    screen.status,
                    AddTokenStatus::Searching(SearchKind::ByContractId, _)
                ),
                "success banner text must no longer drive status transitions"
            );
        });
    }
}
