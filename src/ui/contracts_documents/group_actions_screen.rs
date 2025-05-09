//! The intent of this screen is to allow the user to select a contract,
//! then select an identity, then view the active group actions for that
//! contract that the identity is involved in.
//!
//! For example, if another member of a group started a Mint Action, the user should see that
//! they are being waited on to sign off on the Mint Action. We should
//! route them to the corresponding screen from there. Info should also
//! be displayed like who has already signed off, who is being waited on,
//! and details about the action like mint amount, transfer amount, mint recipient ID, and public note.

use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::add_contract_chooser_pre_filtered;
use crate::ui::helpers::render_identity_selector;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::group::action_event::GroupActionEvent;
use dash_sdk::dpp::group::group_action::GroupAction;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::Identifier;
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Color32, Context, RichText, Ui};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

// Status of the fetch group actions task
enum FetchGroupActionsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    Complete(IndexMap<Identifier, GroupAction>),
    ErrorMessage(String),
}

/// The screen
pub struct GroupActionsScreen {
    // Contract and identity selectors
    selected_contract: Option<QualifiedContract>,
    contracts_with_group_actions: BTreeMap<
        Identifier,
        (
            QualifiedContract,
            BTreeMap<TokenContractPosition, BTreeMap<String, ChangeControlRules>>,
        ),
    >,
    contract_search: String,
    qualified_identities: Vec<QualifiedIdentity>,
    selected_identity: Option<QualifiedIdentity>,

    // Backend task status
    fetch_group_actions_status: FetchGroupActionsStatus,

    // App Context
    pub app_context: Arc<AppContext>,
}

impl GroupActionsScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let qualified_identities = app_context
            .load_local_qualified_identities()
            .expect("Failed to load identities");

        let contracts_with_group_actions = app_context.db.get_contracts(app_context, None, None).unwrap_or_default().into_iter().filter_map(|qualified_contract| {
            let tokens = qualified_contract.contract.tokens().clone().into_iter().filter_map(|(pos, token_config)| {
                let change_control_rules = token_config.all_change_control_rules().into_iter().filter_map(|(name, change_control_rules)| {
                    match change_control_rules.authorized_to_make_change_action_takers() {
                        AuthorizedActionTakers::MainGroup | AuthorizedActionTakers::Group(_) => {
                            return Some((name.to_string(), change_control_rules.clone()))
                        }
                        _ => {}
                    }

                    match change_control_rules.admin_action_takers() {
                        AuthorizedActionTakers::MainGroup | AuthorizedActionTakers::Group(_) => {
                            return Some((name.to_string(), change_control_rules.clone()))
                        }
                        _ => {}
                    }
                    None
                }).collect::<BTreeMap<String, ChangeControlRules>>();
                if change_control_rules.is_empty() {
                    None
                } else {
                    Some((pos, change_control_rules))
                }
            }).collect::<BTreeMap<TokenContractPosition, BTreeMap<String, ChangeControlRules>>>();
            if tokens.is_empty() {
                None
            } else {
                Some((qualified_contract.contract.id(), (qualified_contract, tokens)))
            }
        }).collect();

        Self {
            // Contract and identity selectors
            selected_contract: None,
            contracts_with_group_actions,
            contract_search: String::new(),
            qualified_identities,
            selected_identity: None,

            // Backend task status
            fetch_group_actions_status: FetchGroupActionsStatus::NotStarted,

            // App Context
            app_context: app_context.clone(),
        }
    }

    fn render_group_actions(
        &self,
        ui: &mut Ui,
        group_actions: &IndexMap<Identifier, GroupAction>,
    ) -> AppAction {
        ui.add_space(10.0);
        ui.label("Active Group Actions:");
        for action in group_actions {
            let action_id = action.0;
            let action = action.1;
            match action {
                GroupAction::V0(group_action_v0) => {
                    let event = group_action_v0.event.clone();
                    match event {
                        GroupActionEvent::TokenEvent(token_event) => {
                            ui.label(format!(" - Action ID: {}/nAction: {:?}", action_id, action));
                        }
                    }
                }
            }
        }
        AppAction::None
    }
}

impl ScreenLike for GroupActionsScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                // Not used
            }
            MessageType::Error => {
                self.fetch_group_actions_status =
                    FetchGroupActionsStatus::ErrorMessage(message.to_string());
            }
            MessageType::Info => {
                // Not used
            }
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::ActiveGroupActions(actions_map) => {
                self.fetch_group_actions_status =
                    FetchGroupActionsStatus::Complete(actions_map.clone());
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        // Add top panel, set action
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Group Actions", AppAction::None),
            ],
            vec![],
        );

        // Add left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        // Central panel
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Active Group Actions");

            match &self.fetch_group_actions_status {
                // Fetch not started, show contract and identity selector and fetch button
                // If there is an error message, show it at the top
                FetchGroupActionsStatus::NotStarted | FetchGroupActionsStatus::ErrorMessage(_) => {
                    if let FetchGroupActionsStatus::ErrorMessage(msg) =
                        &self.fetch_group_actions_status
                    {
                        ui.add_space(10.0);
                        ui.colored_label(Color32::RED, format!("Error: {}", msg));
                    }

                    // First a contract selector
                    ui.add_space(10.0);
                    ui.heading("1. Select a contract:");
                    let contract_search_clone = self.contract_search.clone();
                    add_contract_chooser_pre_filtered(
                        ui,
                        &mut self.contract_search,
                        self.contracts_with_group_actions
                            .values()
                            .filter_map(|(contract, _)| {
                                if contract_search_clone.is_empty() {
                                    return Some(contract);
                                }
                                if contract
                                    .alias
                                    .as_ref()
                                    .map(|alias| {
                                        alias.contains(&contract_search_clone)
                                            || alias.to_lowercase().contains(&contract_search_clone)
                                    })
                                    .unwrap_or_default()
                                    || contract
                                        .contract
                                        .id()
                                        .to_string(Encoding::Base58)
                                        .contains(&contract_search_clone)
                                {
                                    Some(contract)
                                } else {
                                    None
                                }
                            }),
                        &mut self.selected_contract,
                    );

                    // Then an identity selector
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.heading("2. Select an identity:");

                    ui.add_space(10.0);
                    self.selected_identity = render_identity_selector(
                        ui,
                        &self.qualified_identities,
                        &self.selected_identity,
                    );

                    // Fetch Button
                    if let Some(selected_contract) = &self.selected_contract {
                        if let Some(selected_identity) = &self.selected_identity {
                            let button = egui::Button::new(
                                RichText::new("Fetch Group Actions").color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(0, 128, 255))
                            .frame(true)
                            .corner_radius(3.0);
                            ui.add_space(10.0);
                            if ui.add(button).clicked() {
                                action |= AppAction::BackendTask(BackendTask::ContractTask(
                                    ContractTask::FetchActiveGroupActions(
                                        selected_contract.clone(),
                                        selected_identity.clone(),
                                    ),
                                ));
                            }
                        }
                    }
                }

                // Actively fetching, display a loading message
                FetchGroupActionsStatus::WaitingForResult(start_time) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    let elapsed_seconds = now - start_time;

                    let display_time = if elapsed_seconds < 60 {
                        format!(
                            "{} second{}",
                            elapsed_seconds,
                            if elapsed_seconds == 1 { "" } else { "s" }
                        )
                    } else {
                        let minutes = elapsed_seconds / 60;
                        let seconds = elapsed_seconds % 60;
                        format!(
                            "{} minute{} and {} second{}",
                            minutes,
                            if minutes == 1 { "" } else { "s" },
                            seconds,
                            if seconds == 1 { "" } else { "s" }
                        )
                    };

                    ui.add_space(10.0);
                    ui.label(format!(
                        "Fetching group actions... Time taken so far: {}",
                        display_time
                    ));
                }

                // Fetch complete, display the active group actions
                FetchGroupActionsStatus::Complete(group_actions) => {
                    action |= self.render_group_actions(ui, group_actions);
                }
            }
        });

        action
    }
}
