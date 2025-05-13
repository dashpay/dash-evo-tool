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
use crate::ui::tokens::burn_tokens_screen::BurnTokensScreen;
use crate::ui::tokens::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use crate::ui::tokens::freeze_tokens_screen::FreezeTokensScreen;
use crate::ui::tokens::mint_tokens_screen::MintTokensScreen;
use crate::ui::tokens::pause_tokens_screen::PauseTokensScreen;
use crate::ui::tokens::resume_tokens_screen::ResumeTokensScreen;
use crate::ui::tokens::tokens_screen::{
    IdentityTokenBalance, IdentityTokenIdentifier, IdentityTokenInfo,
};
use crate::ui::tokens::unfreeze_tokens_screen::UnfreezeTokensScreen;
use crate::ui::tokens::update_token_config::UpdateTokenConfigScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::TokenContractPosition;
use dash_sdk::dpp::group::action_event::GroupActionEvent;
use dash_sdk::dpp::group::group_action::{GroupAction, GroupActionAccessors};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
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

        let identity_token_balances = app_context
            .identity_token_balances()
            .expect("Failed to load identity token balances");

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
        &self,
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

        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
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
                                    ui.label(RichText::new(title).strong());
                                    ui.add_space(30.0);
                                });
                            });
                        }
                        header.col(|ui| {
                            ui.label(""); // No header for button
                        });
                    })
                    .body(|mut body| {
                        for (id, group_action) in group_actions {
                            let (typ, info, note): (&str, String, String) = match group_action {
                                GroupAction::V0(action_v0) => match &action_v0.event {
                                    GroupActionEvent::TokenEvent(token_event) => {
                                        match token_event {
                                            TokenEvent::Mint(amount, identifier, note_opt) => (
                                                "Mint",
                                                format!("{} to {}", amount, identifier),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::Burn(amount, note_opt) => (
                                                "Burn",
                                                format!("{}", amount),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::Freeze(identifier, note_opt) => (
                                                "Freeze",
                                                format!("{}", identifier),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::Unfreeze(identifier, note_opt) => (
                                                "Unfreeze",
                                                format!("{}", identifier),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::DestroyFrozenFunds(
                                                identifier,
                                                amount,
                                                note_opt,
                                            ) => (
                                                "DestroyFrozenFunds",
                                                format!("{} from {}", amount, identifier),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::Transfer(
                                                identifier,
                                                public_note,
                                                _,
                                                _,
                                                amount,
                                            ) => (
                                                "Transfer",
                                                format!("{} to {}", amount, identifier),
                                                format!(
                                                    "{}",
                                                    public_note.clone().unwrap_or_default()
                                                ),
                                            ),
                                            TokenEvent::Claim(dist_type, amount, note_opt) => (
                                                "Claim",
                                                format!("{} via {:?}", amount, dist_type),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::EmergencyAction(action, note_opt) => (
                                                "Emergency",
                                                format!("{:?}", action),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::ConfigUpdate(change_item, note_opt) => (
                                                "ConfigUpdate",
                                                format!("{:?}", change_item),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::ChangePriceForDirectPurchase(
                                                schedule,
                                                note_opt,
                                            ) => (
                                                "ChangePrice",
                                                format!("{:?}", schedule),
                                                format!("{}", note_opt.clone().unwrap_or_default()),
                                            ),
                                            TokenEvent::DirectPurchase(amount, credits) => (
                                                "DirectPurchase",
                                                format!("{} for {} credits", amount, credits),
                                                "".to_string(),
                                            ),
                                        }
                                    }
                                },
                            };

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
                                        ui.label(typ);
                                        ui.add_space(30.0);
                                    });
                                });
                                row.col(|ui| {
                                    let info_clone = info.clone();
                                    ui.horizontal(|ui| {
                                        ui.label(info_clone);
                                        ui.add_space(30.0);
                                    });
                                });
                                row.col(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(note);
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
                                        let token_contract_position = match group_action {
                                            GroupAction::V0(action_v0) => action_v0
                                                .token_contract_position()
                                                .clone(),
                                        };
                                        let token_id = self.selected_contract.clone().expect("No contract selected").contract.token_id(token_contract_position).expect("No token ID found");
                                        let identity_token_balance = self
                                            .identity_token_balances
                                            .get(&IdentityTokenIdentifier {
                                                token_id,
                                                identity_id: self.selected_identity
                                                    .as_ref()
                                                    .unwrap()
                                                    .identity
                                                    .id()
                                            })
                                            .cloned()
                                            .expect("Failed to get identity token balance");
                                        let identity_token_info = IdentityTokenInfo::try_from_identity_token_balance_with_lookup(&identity_token_balance, &self.app_context).expect("Failed to convert identity token balance");
                                        match typ {
                                            "Mint" => {
                                                let mut mint_screen =
                                                    MintTokensScreen::new(identity_token_info, &self.app_context);
                                                mint_screen.group_action_id = Some(*id);
                                                mint_screen.amount_to_mint = info
                                                    .split_whitespace()
                                                    .next()
                                                    .unwrap_or("0")
                                                    .to_string();
                                                mint_screen.recipient_identity_id = info
                                                    .split_whitespace()
                                                    .nth(2)
                                                    .unwrap_or("")
                                                    .to_string();

                                                action |= AppAction::AddScreen(
                                                    Screen::MintTokensScreen(mint_screen),
                                                );
                                            }
                                            "Burn" => {
                                                let mut burn_screen =
                                                    BurnTokensScreen::new(identity_token_info, &self.app_context);
                                                burn_screen.group_action_id = Some(*id);
                                                burn_screen.amount_to_burn = info
                                                    .split_whitespace()
                                                    .next()
                                                    .unwrap_or("0")
                                                    .to_string();
                                                action |= AppAction::AddScreen(
                                                    Screen::BurnTokensScreen(burn_screen),
                                                );
                                            }
                                            "Freeze" => {
                                                let mut freeze_screen = FreezeTokensScreen::new(
                                                    identity_token_info, &self.app_context,
                                                );
                                                freeze_screen.group_action_id = Some(*id);
                                                freeze_screen.freeze_identity_id = info
                                                    .split_whitespace()
                                                    .next()
                                                    .unwrap_or("")
                                                    .to_string();
                                                action |= AppAction::AddScreen(
                                                    Screen::FreezeTokensScreen(freeze_screen),
                                                );
                                            }
                                            "Unfreeze" => {
                                                let mut unfreeze_screen = UnfreezeTokensScreen::new(
                                                    identity_token_info, &self.app_context,
                                                );
                                                unfreeze_screen.group_action_id = Some(*id);
                                                unfreeze_screen.unfreeze_identity_id = info
                                                    .split_whitespace()
                                                    .next()
                                                    .unwrap_or("")
                                                    .to_string();
                                                action |= AppAction::AddScreen(
                                                    Screen::UnfreezeTokensScreen(unfreeze_screen),
                                                );
                                            }
                                            "DestroyFrozenFunds" => {
                                                let mut destroy_screen =
                                                    DestroyFrozenFundsScreen::new(
                                                        identity_token_info, &self.app_context,
                                                    );
                                                destroy_screen.group_action_id = Some(*id);
                                                destroy_screen.frozen_identity_id = info
                                                    .split_whitespace()
                                                    .nth(2)
                                                    .unwrap_or("")
                                                    .to_string();
                                                action |= AppAction::AddScreen(
                                                    Screen::DestroyFrozenFundsScreen(
                                                        destroy_screen,
                                                    ),
                                                );
                                            }
                                            "Emergency" => match info.split_whitespace().nth(1) {
                                                Some("Pause") => {
                                                    let mut pause_screen = PauseTokensScreen::new(
                                                        identity_token_info, &self.app_context,
                                                    );
                                                    pause_screen.group_action_id = Some(*id);
                                                    action |= AppAction::AddScreen(
                                                        Screen::PauseTokensScreen(pause_screen),
                                                    );
                                                }
                                                Some("Resume") => {
                                                    let mut resume_screen = ResumeTokensScreen::new(
                                                        identity_token_info, &self.app_context,
                                                    );
                                                    resume_screen.group_action_id = Some(*id);
                                                    action |= AppAction::AddScreen(
                                                        Screen::ResumeTokensScreen(resume_screen),
                                                    );
                                                }
                                                _ => {}
                                            },
                                            "ConfigUpdate" => {
                                                let mut update_screen =
                                                    UpdateTokenConfigScreen::new(
                                                        identity_token_info, &self.app_context,
                                                    );
                                                update_screen.group_action_id = Some(*id);
                                                action |= AppAction::AddScreen(
                                                    Screen::UpdateTokenConfigScreen(update_screen),
                                                );
                                            }

                                            // To do: Change price and direct purchase
                                            _ => {
                                                action |= AppAction::None;
                                            }
                                        }
                                    }
                                });
                            });
                        }
                    });
            });

        action
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
        match backend_task_success_result {
            BackendTaskSuccessResult::ActiveGroupActions(actions_map) => {
                self.fetch_group_actions_status =
                    FetchGroupActionsStatus::Complete(actions_map.clone());
            }
            _ => {}
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

        egui::CentralPanel::default().show(ctx, |ui| {
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
                        if contract_search_clone.is_empty() {
                            Some(contract)
                        } else if contract
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
                    action |= AppAction::BackendTask(BackendTask::ContractTask(
                        ContractTask::FetchActiveGroupActions(contract, identity),
                    ));
                }
            }

            if let FetchGroupActionsStatus::Complete(group_actions) =
                &self.fetch_group_actions_status
            {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                action |= self.render_group_actions(ui, group_actions);
            }
        });

        action
    }
}
