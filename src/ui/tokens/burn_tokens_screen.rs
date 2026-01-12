use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::model::fee_estimation::{PlatformFeeEstimator, format_credits_as_dash};
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::helpers::{TransactionType, add_identity_key_chooser, render_group_action_text};
use crate::ui::theme::DashColors;
use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use eframe::egui::{Frame, Margin};
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};

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
    pub amount: Option<Amount>,
    pub amount_input: Option<AmountInput>,
    pub max_amount: Option<u64>, // Maximum amount the user can burn based on their balance
    pub public_note: Option<String>,

    status: BurnTokensStatus,
    error_message: Option<String>,

    // Basic references
    pub app_context: Arc<AppContext>,

    // Confirmation popup
    confirmation_dialog: Option<ConfirmationDialog>,

    // For password-based wallet unlocking, if needed
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
}

impl BurnTokensScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let token_balance = match app_context.identity_token_balances() {
            Ok(identity_token_balances) => {
                let itb = identity_token_balances;
                let key = IdentityTokenIdentifier {
                    identity_id: identity_token_info.identity.identity.id(),
                    token_id: identity_token_info.token_id,
                };
                itb.get(&key).map(|itb| itb.balance)
            }
            Err(_) => None,
        };

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
                    != identity_token_info.identity.identity.id()
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
        if group.is_some()
            && let Some((_, group)) = group.clone()
        {
            let your_power = group
                .members()
                .get(&identity_token_info.identity.identity.id());

            if let Some(your_power) = your_power
                && your_power >= &group.required_power()
            {
                is_unilateral_group_member = true;
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
            amount: None,
            amount_input: None,
            max_amount: token_balance,
            public_note: None,
            status: BurnTokensStatus::NotStarted,
            error_message,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
        }
    }

    /// Renders a text input for the user to specify an amount to burn
    fn render_amount_input(&mut self, ui: &mut egui::Ui) {
        let amount_input = self.amount_input.get_or_insert_with(|| {
            let token_amount = Amount::from_token(&self.identity_token_info, 0);
            let mut input = AmountInput::new(token_amount).with_label("Amount:");

            if self.max_amount.is_some() {
                input.set_show_max_button(self.max_amount.is_some());
                input.set_max_amount(self.max_amount);
            }

            input
        });

        let amount_response = amount_input.show(ui).inner;
        // Update the amount based on user input
        amount_response.update(&mut self.amount);
        // errors are handled inside AmountInput
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let amount = match self.amount.as_ref() {
            Some(amount) if amount.value() > 0 => amount,
            _ => {
                self.error_message = Some("Please enter a valid amount greater than 0.".into());
                self.status = BurnTokensStatus::ErrorMessage("Invalid amount".into());
                self.confirmation_dialog = None;
                return AppAction::None;
            }
        };

        let dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new(
                "Confirm Burn".to_string(),
                format!("Are you sure you want to burn {}?", amount),
            )
            .danger_mode(true) // Burning tokens is destructive
        });

        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.confirmation_dialog = None;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.status = BurnTokensStatus::WaitingForResult(now);

                // Grab the data contract for this token from the app context
                let data_contract =
                    Arc::new(self.identity_token_info.data_contract.contract.clone());

                let group_info = if self.group_action_id.is_some() {
                    self.group.as_ref().map(|(pos, _)| {
                        GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                            GroupStateTransitionInfo {
                                group_contract_position: *pos,
                                action_id: self.group_action_id.unwrap(),
                                action_is_proposer: false,
                            },
                        )
                    })
                } else {
                    self.group.as_ref().map(|(pos, _)| {
                        GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
                    })
                };

                // Dispatch the actual backend burn action
                AppAction::BackendTasks(
                    vec![
                        BackendTask::TokenTask(Box::new(TokenTask::BurnTokens {
                            owner_identity: self.identity_token_info.identity.clone(),
                            data_contract,
                            token_position: self.identity_token_info.token_position,
                            signing_key: self.selected_key.clone().expect("Expected a key"),
                            public_note: if self.group_action_id.is_some() {
                                None
                            } else {
                                self.public_note.clone()
                            },
                            amount: amount.value(),
                            group_info,
                        })),
                        BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                    ],
                    BackendTasksExecutionMode::Sequential,
                )
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let fee_info = self.completed_fee_result.as_ref().map(|fee_result| {
            let fee_str = format!(
                "Estimated: {}  •  Actual: {}",
                format_credits_as_dash(fee_result.estimated_fee),
                format_credits_as_dash(fee_result.actual_fee)
            );
            ("Transaction Fee", fee_str)
        });

        let fee_ref = fee_info
            .as_ref()
            .map(|(title, desc)| (*title, desc.as_str()));

        crate::ui::helpers::show_group_token_success_screen_with_fee(
            ui,
            "Burn",
            self.group_action_id.is_some(),
            self.is_unilateral_group_member,
            self.group.is_some(),
            &self.app_context,
            fee_ref,
        )
    }
}

impl ScreenLike for BurnTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.status = BurnTokensStatus::ErrorMessage(message.to_string());
            self.error_message = Some(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::BurnedTokens(fee_result) = backend_task_success_result {
            self.completed_fee_result = Some(fee_result);
            self.status = BurnTokensStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        // If you need to reload local identity data or re-check keys
        if let Ok(all_identities) = self.app_context.load_local_user_identities()
            && let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity_token_info.identity.identity.id())
        {
            self.identity_token_info.identity = updated_identity;
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

        let central_panel_action = island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // If we are in the "Complete" status, just show success screen
            if self.status == BurnTokensStatus::Complete {
                return self.show_success_screen(ui);
            }

            ui.heading("Burn Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.is_developer_mode() {
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
                    DashColors::error_color(dark_mode),
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
                        HashSet::from([SecurityLevel::CRITICAL]),
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
                if let Some(wallet) = &self.selected_wallet {
                    if let Err(e) = try_open_wallet_no_password(wallet) {
                        self.error_message = Some(e);
                    }
                    if wallet_needs_unlock(wallet) {
                        ui.add_space(10.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to continue.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                        return AppAction::None;
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
                        self.amount
                            .as_ref()
                            .map(|a| a.to_string())
                            .unwrap_or_default()
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
                            self.public_note = if !txt.is_empty() { Some(txt) } else { None };
                        }
                    });
                }

                // Fee estimation display
                let fee_estimator = PlatformFeeEstimator::new();
                let estimated_fee = fee_estimator.estimate_document_batch(1); // Token operations are document batch transitions

                Frame::new()
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated fee:")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(14.0),
                            );
                            ui.label(
                                RichText::new(format_credits_as_dash(estimated_fee))
                                    .color(DashColors::text_primary(dark_mode))
                                    .size(14.0),
                            );
                        });
                    });

                let button_text = render_group_action_text(
                    ui,
                    &self.group,
                    &self.identity_token_info,
                    "Burn",
                    &self.group_action_id,
                );

                // Display estimated fee before action button
                let estimated_fee = PlatformFeeEstimator::new().estimate_token_transition();
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label("Estimated Fee:");
                    ui.label(RichText::new(format_credits_as_dash(estimated_fee)).strong());
                });

                // Burn button
                if self.app_context.is_developer_mode() || !button_text.contains("Test") {
                    ui.add_space(10.0);
                    let button =
                        egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .corner_radius(3.0);

                    if ui.add(button).clicked() {
                        // Create confirmation dialog on button click
                        if self.confirmation_dialog.is_none() {
                            let amount = match self.amount.as_ref() {
                                Some(amount) if amount.value() > 0 => amount,
                                _ => return AppAction::None,
                            };

                            self.confirmation_dialog = Some(
                                ConfirmationDialog::new(
                                    "Confirm Burn".to_string(),
                                    format!("Are you sure you want to burn {}?", amount),
                                )
                                .danger_mode(true),
                            );
                        }
                    }
                }

                // Show confirmation dialog if it exists
                if self.confirmation_dialog.is_some() {
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
                        ui.colored_label(
                            DashColors::error_color(dark_mode),
                            format!("Error: {}", msg),
                        );
                    }
                    BurnTokensStatus::Complete => {
                        // handled above
                    }
                }
            }

            AppAction::None
        });

        action |= central_panel_action;

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        action
    }
}
