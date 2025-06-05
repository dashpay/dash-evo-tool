use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::contracts_documents::group_actions_screen::GroupActionsScreen;
use crate::ui::helpers::{add_identity_key_chooser, render_group_action_text, TransactionType};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};

use super::tokens_screen::IdentityTokenInfo;

/// Internal states for the burn process.
#[derive(PartialEq)]
pub enum BurnTokensStatus {
    NotStarted,
    WaitingForResult(u64),
    ErrorMessage(String),
    Complete,
}

pub struct BurnTokensScreen {
    pub identity_token_info: IdentityTokenInfo,
    selected_key: Option<IdentityPublicKey>,
    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,

    // The user chooses how many tokens to burn
    pub amount_to_burn: String,
    pub public_note: Option<String>,

    status: BurnTokensStatus,
    error_message: Option<String>,

    // Basic references
    pub app_context: Arc<AppContext>,

    // Confirmation popup
    show_confirmation_popup: bool,

    // For password-based wallet unlocking, if needed
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl BurnTokensScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let possible_key = identity_token_info
            .identity
            .identity
            .get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            )
            .cloned();

        let mut error_message = None;

        let group = match identity_token_info
            .token_config
            .manual_burning_rules()
            .authorized_to_make_change_action_takers()
        {
            AuthorizedActionTakers::NoOne => {
                error_message = Some("Burning is not allowed on this token".to_string());
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != &identity_token_info.identity.identity.id()
                {
                    error_message = Some(
                        "You are not allowed to burn this token. Only the contract owner is."
                            .to_string(),
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    error_message = Some("You are not allowed to burn this token".to_string());
                }
                None
            }
            AuthorizedActionTakers::MainGroup => {
                match identity_token_info.token_config.main_control_group() {
                    None => {
                        error_message = Some(
                            "Invalid contract: No main control group, though one should exist"
                                .to_string(),
                        );
                        None
                    }
                    Some(group_pos) => {
                        match identity_token_info
                            .data_contract
                            .contract
                            .expected_group(group_pos)
                        {
                            Ok(group) => Some((group_pos, group.clone())),
                            Err(e) => {
                                error_message = Some(format!("Invalid contract: {}", e));
                                None
                            }
                        }
                    }
                }
            }
            AuthorizedActionTakers::Group(group_pos) => {
                match identity_token_info
                    .data_contract
                    .contract
                    .expected_group(*group_pos)
                {
                    Ok(group) => Some((*group_pos, group.clone())),
                    Err(e) => {
                        error_message = Some(format!("Invalid contract: {}", e));
                        None
                    }
                }
            }
        };

        let mut is_unilateral_group_member = false;
        if group.is_some() {
            if let Some((_, group)) = group.clone() {
                let your_power = group
                    .members()
                    .get(&identity_token_info.identity.identity.id());

                if let Some(your_power) = your_power {
                    if your_power >= &group.required_power() {
                        is_unilateral_group_member = true;
                    }
                }
            }
        };

        // Attempt to get an unlocked wallet reference
        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key.as_ref(),
            &mut error_message,
        );

        Self {
            identity_token_info,
            selected_key: possible_key,
            group,
            is_unilateral_group_member,
            group_action_id: None,
            amount_to_burn: String::new(),
            public_note: None,
            status: BurnTokensStatus::NotStarted,
            error_message,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    /// Renders a text input for the user to specify an amount to burn
    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Amount to Burn:");
            ui.text_edit_singleline(&mut self.amount_to_burn);
        });
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Burn")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Validate user input
                let amount_ok = self.amount_to_burn.parse::<u64>().ok();
                if amount_ok.is_none() {
                    self.error_message = Some("Please enter a valid integer amount.".into());
                    self.status = BurnTokensStatus::ErrorMessage("Invalid amount".into());
                    self.show_confirmation_popup = false;
                    return;
                }

                ui.label(format!(
                    "Are you sure you want to burn {} tokens?",
                    self.amount_to_burn
                ));

                ui.add_space(10.0);

                // Confirm button
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = BurnTokensStatus::WaitingForResult(now);

                    // Grab the data contract for this token from the app context
                    let data_contract = self.identity_token_info.data_contract.contract.clone();

                    let group_info;
                    if self.group_action_id.is_some() {
                        group_info = self.group.as_ref().map(|(pos, _)| {
                            GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                                GroupStateTransitionInfo {
                                    group_contract_position: *pos,
                                    action_id: self.group_action_id.unwrap(),
                                    action_is_proposer: false,
                                },
                            )
                        });
                    } else {
                        group_info = self.group.as_ref().map(|(pos, _)| {
                            GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
                        });
                    }

                    // Dispatch the actual backend burn action
                    action = AppAction::BackendTasks(
                        vec![
                            BackendTask::TokenTask(TokenTask::BurnTokens {
                                owner_identity: self.identity_token_info.identity.clone(),
                                data_contract,
                                token_position: self.identity_token_info.token_position,
                                signing_key: self.selected_key.clone().expect("Expected a key"),
                                public_note: if self.group_action_id.is_some() {
                                    None
                                } else {
                                    self.public_note.clone()
                                },
                                amount: amount_ok.unwrap(),
                                group_info,
                            }),
                            BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
                        ],
                        BackendTasksExecutionMode::Sequential,
                    );
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.show_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_confirmation_popup = false;
        }
        action
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            if self.group_action_id.is_some() {
                // This burn is already initiated by the group, we are just signing it
                ui.heading("Group Burn Signing Successful.");
            } else {
                if !self.is_unilateral_group_member && self.group.is_some() {
                    ui.heading("Group Burn Initiated.");
                } else {
                    ui.heading("Burn Successful.");
                }
            }

            ui.add_space(20.0);

            if self.group_action_id.is_some() {
                if ui.button("Back to Group Actions").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenMyTokenBalances,
                    );
                }
            } else {
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }

                if !self.is_unilateral_group_member {
                    if ui.button("Go to Group Actions").clicked() {
                        action = AppAction::PopThenAddScreenToMainScreen(
                            RootScreenType::RootScreenDocumentQuery,
                            Screen::GroupActionsScreen(GroupActionsScreen::new(
                                &self.app_context.clone(),
                            )),
                        );
                    }
                }
            }
        });
        action
    }
}

impl ScreenLike for BurnTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Successfully burned tokens") || message == "BurnTokens" {
                    self.status = BurnTokensStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = BurnTokensStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {
        // If you need to reload local identity data or re-check keys
        if let Ok(all_identities) = self.app_context.load_local_user_identities() {
            if let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity_token_info.identity.identity.id())
            {
                self.identity_token_info.identity = updated_identity;
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action;

        // Build a top panel
        if self.group_action_id.is_some() {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Contracts", AppAction::GoToMainScreen),
                    ("Group Actions", AppAction::PopScreen),
                    ("Burn", AppAction::None),
                ],
                vec![],
            );
        } else {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Tokens", AppAction::GoToMainScreen),
                    (&self.identity_token_info.token_alias, AppAction::PopScreen),
                    ("Burn", AppAction::None),
                ],
                vec![],
            );
        }

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
            if self.status == BurnTokensStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Burn Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode.load(Ordering::Relaxed) {
                !self
                    .identity_token_info
                    .identity
                    .identity
                    .public_keys()
                    .is_empty()
            } else {
                !self
                    .identity_token_info
                    .identity
                    .available_authentication_keys()
                    .is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    Color32::DARK_RED,
                    format!(
                        "No authentication keys found for this {} identity.",
                        self.identity_token_info.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                // Show "Add key" or "Check keys" option
                let first_key = self
                    .identity_token_info
                    .identity
                    .identity
                    .get_first_public_key_matching(
                        Purpose::AUTHENTICATION,
                        HashSet::from([
                            SecurityLevel::CRITICAL,
                        ]),
                        KeyType::all_key_types().into(),
                        false,
                    );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity_token_info.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity_token_info.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Possibly handle locked wallet scenario
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        // Must unlock before we can proceed
                        return;
                    }
                }

                // 1) Key selection
                ui.heading("1. Select the key to sign the Burn transaction");
                ui.add_space(10.0);

                let mut selected_identity = Some(self.identity_token_info.identity.clone());
                add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    std::iter::once(&self.identity_token_info.identity),
                    &mut selected_identity,
                    &mut self.selected_key,
                    TransactionType::TokenAction,
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // 2) Amount to burn
                ui.heading("2. Amount to burn");
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group Burn so you are not allowed to choose the amount.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!(
                        "Amount: {}",
                        self.amount_to_burn
                    ));
                } else {
                    self.render_amount_input(ui);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("3. Public note (optional)");
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group Burn so you are not allowed to put a note.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!(
                        "Note: {}",
                        self.public_note.clone().unwrap_or("None".to_string())
                    ));
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Public note (optional):");
                        ui.add_space(10.0);
                        let mut txt = self.public_note.clone().unwrap_or_default();
                        if ui
                            .text_edit_singleline(&mut txt)
                            .on_hover_text(
                                "A note about the transaction that can be seen by the public.",
                            )
                            .changed()
                        {
                            self.public_note = if txt.len() > 0 {
                                Some(txt)
                            } else {
                                None
                            };
                        }
                    });
                }

                let button_text =
                    render_group_action_text(ui, &self.group, &self.identity_token_info, "Burn", &self.group_action_id);

                // Burn button
                if self.app_context.developer_mode.load(Ordering::Relaxed) || !button_text.contains("Test") {
                    ui.add_space(10.0);
                    let button =
                        egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .corner_radius(3.0);

                    if ui.add(button).clicked() {
                        self.show_confirmation_popup = true;
                    }
                }

                // If user pressed "Burn," show a popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    BurnTokensStatus::NotStarted => {
                        // no-op
                    }
                    BurnTokensStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Burning... elapsed: {} seconds", elapsed));
                    }
                    BurnTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
                    }
                    BurnTokensStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for BurnTokensScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}
