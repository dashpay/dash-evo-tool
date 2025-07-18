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
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::add_contract_chooser_pre_filtered;
use crate::ui::helpers::render_identity_selector;
use crate::ui::tokens::burn_tokens_screen::BurnTokensScreen;
use crate::ui::tokens::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use crate::ui::tokens::freeze_tokens_screen::FreezeTokensScreen;
use crate::ui::tokens::mint_tokens_screen::MintTokensScreen;
use crate::ui::tokens::pause_tokens_screen::PauseTokensScreen;
use crate::ui::tokens::resume_tokens_screen::ResumeTokensScreen;
use crate::ui::tokens::set_token_price_screen::SetTokenPriceScreen;
use crate::ui::tokens::tokens_screen::{
    IdentityTokenBalance, IdentityTokenIdentifier, IdentityTokenInfo,
};
use crate::ui::tokens::unfreeze_tokens_screen::UnfreezeTokensScreen;
use crate::ui::tokens::update_token_config::UpdateTokenConfigScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::group::action_event::GroupActionEvent;
use dash_sdk::dpp::group::group_action::{GroupAction, GroupActionAccessors};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::dpp::tokens::emergency_action::TokenEmergencyAction;
use dash_sdk::dpp::tokens::token_event::TokenEvent;
use dash_sdk::platform::Identifier;
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Color32, Context, RichText};
use egui::{ScrollArea, TextStyle};
use egui_extras::{Column, TableBuilder};
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
    #[allow(clippy::type_complexity)]
    contracts_with_group_actions: BTreeMap<
        Identifier,
        (
            QualifiedContract,
            BTreeMap<TokenContractPosition, BTreeMap<String, ChangeControlRules>>,
        ),
    >,
    contract_search: String,
    qualified_identities: Vec<QualifiedIdentity>,
    identity_token_balances: IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>,
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
            .unwrap_or_else(|_| {
                tracing::info!("Failed to load local qualified identities");
                vec![]
            });

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

        let identity_token_balances = match app_context.identity_token_balances() {
            Ok(identity_token_balances) => identity_token_balances,
            Err(e) => {
                tracing::error!("Failed to load identity token balances: {}", e);
                IndexMap::new()
            }
        };

        Self {
            // Contract and identity selectors
            selected_contract: None,
            contracts_with_group_actions,
            contract_search: String::new(),
            qualified_identities,
            identity_token_balances,
            selected_identity: None,

            // Backend task status
            fetch_group_actions_status: FetchGroupActionsStatus::NotStarted,

            // App Context
            app_context: app_context.clone(),
        }
    }

    fn render_group_actions(
        &mut self,
        ui: &mut egui::Ui,
        group_actions: &IndexMap<Identifier, GroupAction>,
    ) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("Active Group Actions:");
        ui.add_space(10.0);

        if group_actions.is_empty() {
            ui.label("No active group actions found.");
            return action;
        }

        let text_style = TextStyle::Body;
        let row_height = ui.text_style_height(&text_style) + 8.0;

        ScrollArea::both()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(false)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::auto().resizable(true)) // Action ID
                    .column(Column::auto().resizable(true)) // Type
                    .column(Column::auto().resizable(true)) // Info
                    .column(Column::auto().resizable(true)) // Note
                    .column(Column::auto()) // Button
                    .min_scrolled_height(0.0)
                    .header(row_height, |mut header| {
                        for title in ["Action ID", "Type", "Info", "Note"] {
                            header.col(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(title).strong().color(ui.visuals().text_color()).underline());
                                    ui.add_space(30.0);
                                });
                            });
                        }
                        header.col(|ui| {
                            ui.label("");
                        });
                    })
                    .body(|mut body| {
                        for (id, group_action) in group_actions {
                            let GroupActionEvent::TokenEvent(token_event) = group_action.event();

                            body.row(row_height, |mut row| {
                                row.col(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(
                                            id.to_string(Encoding::Base58)
                                                .chars()
                                                .take(16)
                                                .collect::<String>(),
                                        ));
                                        ui.add_space(30.0);
                                    });
                                });
                                row.col(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(self.get_token_event_type(token_event));
                                        ui.add_space(30.0);
                                    });
                                });
                                row.col(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(self.get_token_event_info(token_event));
                                        ui.add_space(30.0);
                                    });
                                });
                                row.col(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(self.get_token_event_note(token_event));
                                        ui.add_space(30.0);
                                    });
                                });
                                row.col(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Take Action").color(Color32::WHITE),
                                            )
                                            .fill(Color32::from_rgb(0, 128, 255))
                                            .frame(true),
                                        )
                                        .clicked()
                                    {
                                        let token_contract_position = group_action.token_contract_position();
                                        let token_id = self.selected_contract.clone().expect("No contract selected").contract.token_id(token_contract_position).expect("No token ID found at the given position");
                                        let identity_token_balance = match self
                                            .identity_token_balances
                                            .get(&IdentityTokenIdentifier {
                                                token_id,
                                                identity_id: self.selected_identity
                                                    .as_ref()
                                                    .unwrap()
                                                    .identity
                                                    .id()
                                            })
                                            .cloned() {
                                            Some(identity_token_balance) => identity_token_balance,
                                            None => {
                                                self.fetch_group_actions_status =
                                                    FetchGroupActionsStatus::ErrorMessage(
                                                        "No identity token balance found".to_string(),
                                                    );
                                                return;
                                            }
                                        };
                                        let identity_token_info = match IdentityTokenInfo::try_from_identity_token_balance_with_lookup(&identity_token_balance, &self.app_context) {
                                            Ok(identity_token_info) => identity_token_info,
                                            Err(e) => {
                                                self.fetch_group_actions_status =
                                                    FetchGroupActionsStatus::ErrorMessage(
                                                        format!("Failed to get identity token info: {}", e),
                                                    );
                                                return;
                                            }
                                        };

                                        self.handle_token_event_action(token_event, *id, identity_token_info, &mut action);
                                    }
                                });
                            });
                        }
                    });
            });

        action
    }

    fn get_token_event_type(&self, token_event: &TokenEvent) -> &'static str {
        match token_event {
            TokenEvent::Mint(..) => "Mint",
            TokenEvent::Burn(..) => "Burn",
            TokenEvent::Freeze(..) => "Freeze",
            TokenEvent::Unfreeze(..) => "Unfreeze",
            TokenEvent::DestroyFrozenFunds(..) => "DestroyFrozenFunds",
            TokenEvent::Transfer(..) => "Transfer",
            TokenEvent::Claim(..) => "Claim",
            TokenEvent::EmergencyAction(..) => "Emergency",
            TokenEvent::ConfigUpdate(..) => "ConfigUpdate",
            TokenEvent::ChangePriceForDirectPurchase(..) => "ChangePrice",
            TokenEvent::DirectPurchase(..) => "DirectPurchase",
        }
    }

    fn get_token_event_info(&self, token_event: &TokenEvent) -> String {
        match token_event {
            TokenEvent::Mint(amount, identifier, _) => format!("{} to {}", amount, identifier),
            TokenEvent::Burn(amount, burn_from, _) => format!("{} from {}", amount, burn_from),
            TokenEvent::Freeze(identifier, _) => format!("{}", identifier),
            TokenEvent::Unfreeze(identifier, _) => format!("{}", identifier),
            TokenEvent::DestroyFrozenFunds(identifier, amount, _) => {
                format!("{} from {}", amount, identifier)
            }
            TokenEvent::Transfer(identifier, _, _, _, amount) => {
                format!("{} to {}", amount, identifier)
            }
            TokenEvent::Claim(dist_type, amount, _) => format!("{} via {:?}", amount, dist_type),
            TokenEvent::EmergencyAction(action, _) => format!("{:?}", action),
            TokenEvent::ConfigUpdate(change_item, _) => format!("{:?}", change_item),
            TokenEvent::ChangePriceForDirectPurchase(schedule, _) => format!("{:?}", schedule),
            TokenEvent::DirectPurchase(amount, credits) => {
                format!("{} for {} credits", amount, credits)
            }
        }
    }

    fn get_token_event_note(&self, token_event: &TokenEvent) -> String {
        match token_event {
            TokenEvent::Mint(_, _, note_opt)
            | TokenEvent::Burn(_, _, note_opt)
            | TokenEvent::Freeze(_, note_opt)
            | TokenEvent::Unfreeze(_, note_opt)
            | TokenEvent::DestroyFrozenFunds(_, _, note_opt)
            | TokenEvent::Claim(_, _, note_opt)
            | TokenEvent::EmergencyAction(_, note_opt)
            | TokenEvent::ConfigUpdate(_, note_opt)
            | TokenEvent::ChangePriceForDirectPurchase(_, note_opt) => {
                note_opt.clone().unwrap_or_default().to_string()
            }
            TokenEvent::Transfer(_, public_note, _, _, _) => {
                public_note.clone().unwrap_or_default().to_string()
            }
            TokenEvent::DirectPurchase(_, _) => String::new(),
        }
    }

    /// On click of the "Take Action" button
    fn handle_token_event_action(
        &mut self,
        token_event: &TokenEvent,
        action_id: Identifier,
        identity_token_info: IdentityTokenInfo,
        action: &mut AppAction,
    ) {
        match token_event {
            TokenEvent::Mint(amount, _identifier, note_opt) => {
                let mut mint_screen = MintTokensScreen::new(identity_token_info, &self.app_context);
                mint_screen.group_action_id = Some(action_id);
                mint_screen.amount_to_mint = amount.to_string();
                mint_screen.public_note = note_opt.clone();
                *action |= AppAction::AddScreen(Screen::MintTokensScreen(mint_screen));
            }
            TokenEvent::Burn(amount, _burn_from, note_opt) => {
                let mut burn_screen = BurnTokensScreen::new(identity_token_info, &self.app_context);
                burn_screen.group_action_id = Some(action_id);
                burn_screen.amount_to_burn = amount.to_string();
                burn_screen.public_note = note_opt.clone();
                *action |= AppAction::AddScreen(Screen::BurnTokensScreen(burn_screen));
            }
            TokenEvent::Freeze(identifier, note_opt) => {
                let mut freeze_screen =
                    FreezeTokensScreen::new(identity_token_info, &self.app_context);
                freeze_screen.group_action_id = Some(action_id);
                freeze_screen.freeze_identity_id = identifier.to_string(Encoding::Base58);
                freeze_screen.public_note = note_opt.clone();
                *action |= AppAction::AddScreen(Screen::FreezeTokensScreen(freeze_screen));
            }
            TokenEvent::Unfreeze(identifier, note_opt) => {
                let mut unfreeze_screen =
                    UnfreezeTokensScreen::new(identity_token_info, &self.app_context);
                unfreeze_screen.group_action_id = Some(action_id);
                unfreeze_screen.unfreeze_identity_id = identifier.to_string(Encoding::Base58);
                unfreeze_screen.public_note = note_opt.clone();
                *action |= AppAction::AddScreen(Screen::UnfreezeTokensScreen(unfreeze_screen));
            }
            TokenEvent::DestroyFrozenFunds(identifier, _amount, note_opt) => {
                let mut destroy_screen =
                    DestroyFrozenFundsScreen::new(identity_token_info, &self.app_context);
                destroy_screen.group_action_id = Some(action_id);
                destroy_screen.frozen_identity_id = identifier.to_string(Encoding::Base58);
                destroy_screen.public_note = note_opt.clone();
                *action |= AppAction::AddScreen(Screen::DestroyFrozenFundsScreen(destroy_screen));
            }
            TokenEvent::EmergencyAction(emergency_action, note_opt) => {
                // Match against the debug representation to handle Pause/Resume
                match emergency_action {
                    TokenEmergencyAction::Pause => {
                        let mut pause_screen =
                            PauseTokensScreen::new(identity_token_info, &self.app_context);
                        pause_screen.group_action_id = Some(action_id);
                        pause_screen.public_note = note_opt.clone();
                        *action |= AppAction::AddScreen(Screen::PauseTokensScreen(pause_screen));
                    }
                    TokenEmergencyAction::Resume => {
                        let mut resume_screen =
                            ResumeTokensScreen::new(identity_token_info, &self.app_context);
                        resume_screen.group_action_id = Some(action_id);
                        resume_screen.public_note = note_opt.clone();
                        *action |= AppAction::AddScreen(Screen::ResumeTokensScreen(resume_screen));
                    }
                }
            }
            TokenEvent::ConfigUpdate(change_item, note_opt) => {
                let mut update_screen =
                    UpdateTokenConfigScreen::new(identity_token_info, &self.app_context);
                update_screen.group_action_id = Some(action_id);
                update_screen.public_note = note_opt.clone();
                update_screen.change_item = change_item.clone();
                *action |=
                    AppAction::AddScreen(Screen::UpdateTokenConfigScreen(Box::new(update_screen)));
            }
            TokenEvent::ChangePriceForDirectPurchase(schedule, note_opt) => {
                let mut change_price_screen =
                    SetTokenPriceScreen::new(identity_token_info, &self.app_context);
                change_price_screen.group_action_id = Some(action_id);
                change_price_screen.token_pricing_schedule = format!("{:?}", schedule);
                change_price_screen.public_note = note_opt.clone();
                *action |= AppAction::AddScreen(Screen::SetTokenPriceScreen(change_price_screen));
            }
            _ => {
                // For other token events that don't have specific screens
                *action |= AppAction::None;
            }
        }
    }
}

impl ScreenLike for GroupActionsScreen {
    fn refresh(&mut self) {
        self.fetch_group_actions_status = FetchGroupActionsStatus::NotStarted;
    }

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
        if let BackendTaskSuccessResult::ActiveGroupActions(actions_map) =
            backend_task_success_result
        {
            self.fetch_group_actions_status =
                FetchGroupActionsStatus::Complete(actions_map.clone());
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Group Actions", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        let central_panel_action = island_central_panel(ctx, |ui| {
            ui.heading("Active Group Actions");

            ui.add_space(10.0);
            ui.heading("1. Select a contract:");

            let contract_search_clone = self.contract_search.clone();
            ui.add_space(10.0);
            add_contract_chooser_pre_filtered(
                ui,
                &mut self.contract_search,
                self.contracts_with_group_actions
                    .values()
                    .filter_map(|(contract, _)| {
                        if contract_search_clone.is_empty()
                            || contract
                                .alias
                                .as_ref()
                                .map(|alias| {
                                    alias.contains(&contract_search_clone)
                                        || alias
                                            .to_lowercase()
                                            .contains(&contract_search_clone.to_lowercase())
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

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.heading("2. Select an identity:");

            ui.add_space(10.0);
            self.selected_identity =
                render_identity_selector(ui, &self.qualified_identities, &self.selected_identity);

            let mut fetch_clicked = false;
            if self.selected_contract.is_some() && self.selected_identity.is_some() {
                ui.add_space(10.0);
                let button =
                    egui::Button::new(RichText::new("Fetch Group Actions").color(Color32::WHITE))
                        .fill(Color32::from_rgb(0, 128, 255))
                        .frame(true)
                        .corner_radius(3.0);

                if ui.add(button).clicked() {
                    self.fetch_group_actions_status = FetchGroupActionsStatus::WaitingForResult(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs(),
                    );
                    fetch_clicked = true;
                }
            }

            match &self.fetch_group_actions_status {
                FetchGroupActionsStatus::ErrorMessage(msg) => {
                    ui.add_space(10.0);
                    ui.colored_label(Color32::RED, format!("Error: {}", msg));
                }

                FetchGroupActionsStatus::WaitingForResult(start_time) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    let elapsed = now - start_time;
                    let status = if elapsed < 60 {
                        format!("{} second{}", elapsed, if elapsed == 1 { "" } else { "s" })
                    } else {
                        format!(
                            "{} minute{} and {} second{}",
                            elapsed / 60,
                            if elapsed / 60 == 1 { "" } else { "s" },
                            elapsed % 60,
                            if elapsed % 60 == 1 { "" } else { "s" }
                        )
                    };
                    ui.add_space(10.0);
                    ui.label(format!(
                        "Fetching group actions… Time taken so far: {}",
                        status
                    ));
                }

                _ => {}
            }

            if fetch_clicked {
                if let (Some(contract), Some(identity)) = (
                    self.selected_contract.clone(),
                    self.selected_identity.clone(),
                ) {
                    action |= AppAction::BackendTask(BackendTask::ContractTask(Box::new(
                        ContractTask::FetchActiveGroupActions(contract, identity),
                    )));
                }
            }

            if let FetchGroupActionsStatus::Complete(group_actions) =
                &self.fetch_group_actions_status
            {
                let group_actions = group_actions.clone();
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                return self.render_group_actions(ui, &group_actions);
            }
            AppAction::None
        });

        action |= central_panel_action;
        action
    }
}
