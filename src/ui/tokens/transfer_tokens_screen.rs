use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Ui};
use egui::{Color32, RichText};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ui::identities::get_selected_wallet;

use super::tokens_screen::IdentityTokenBalance;

#[derive(PartialEq)]
pub enum TransferTokensStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct TransferTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_balance: IdentityTokenBalance,
    known_identities: Vec<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    pub public_note: Option<String>,
    pub receiver_identity_id: String,
    pub amount: Option<Amount>,
    pub amount_input: Option<AmountInput>,
    transfer_tokens_status: TransferTokensStatus,
    max_amount: Amount,
    pub app_context: Arc<AppContext>,
    confirmation_dialog: Option<ConfirmationDialog>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl TransferTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let known_identities = app_context
            .load_local_qualified_identities()
            .expect("Identities not loaded");

        let identity = known_identities
            .iter()
            .find(|identity| identity.identity.id() == identity_token_balance.identity_id)
            .expect("Identity not found")
            .clone();
        let max_amount = Amount::from(&identity_token_balance);
        let identity_clone = identity.identity.clone();
        let selected_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL]),
            KeyType::all_key_types().into(),
            false,
        );
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key, &mut error_message);

        let amount = Some(Amount::from(&identity_token_balance).with_value(0));

        Self {
            identity,
            identity_token_balance,
            known_identities,
            selected_key: selected_key.cloned(),
            public_note: None,
            receiver_identity_id: String::new(),
            amount,
            amount_input: None,
            transfer_tokens_status: TransferTokensStatus::NotStarted,
            max_amount,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.label(format!("Available balance: {}", self.max_amount));
        ui.add_space(5.0);

        // Lazy initialization with proper decimal places
        let amount_input = match self.amount_input.as_mut() {
            Some(input) => input,
            _ => {
                self.amount_input = Some(
                    AmountInput::new(
                        self.amount
                            .as_ref()
                            .unwrap_or(&Amount::from(&self.identity_token_balance)),
                    )
                    .with_label("Amount:")
                    .with_max_button(true),
                );

                self.amount_input
                    .as_mut()
                    .expect("AmountInput should be initialized above")
            }
        };

        // Check if input should be disabled when operation is in progress
        let enabled = match self.transfer_tokens_status {
            TransferTokensStatus::WaitingForResult(_) | TransferTokensStatus::Complete => false,
            TransferTokensStatus::NotStarted | TransferTokensStatus::ErrorMessage(_) => {
                amount_input.set_max_amount(Some(self.max_amount.value()));
                true
            }
        };

        let response = ui.add_enabled_ui(enabled, |ui| amount_input.show(ui)).inner;

        response.inner.update(&mut self.amount);
        // errors are handled inside AmountInput
    }

    fn render_to_identity_input(&mut self, ui: &mut Ui) {
        let _response = ui.add(
            IdentitySelector::new(
                "transfer_recipient_selector",
                &mut self.receiver_identity_id,
                &self.known_identities,
            )
            .width(300.0)
            .label("Recipient:")
            .exclude(&[self.identity.identity.id()]),
        );
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let msg = format!(
            "Are you sure you want to transfer {} tokens to {}?",
            self.amount.clone().unwrap_or(Amount::new(0, 0)),
            self.receiver_identity_id
        );

        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm Transfer", msg)
                .confirm_text(Some("Transfer"))
                .cancel_text(Some("Cancel"))
        });

        let response = confirmation_dialog.show(ui);
        match response.inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.confirmation_dialog = None;
                self.confirmation_ok()
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    fn confirmation_ok(&mut self) -> AppAction {
        if self.amount.is_none() || self.amount == Some(Amount::new(0, 0)) {
            self.transfer_tokens_status =
                TransferTokensStatus::ErrorMessage("Invalid amount".into());
            return AppAction::None;
        }

        let parsed_receiver_id = Identifier::from_string_try_encodings(
            &self.receiver_identity_id,
            &[
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex,
            ],
        );

        if parsed_receiver_id.is_err() {
            self.transfer_tokens_status =
                TransferTokensStatus::ErrorMessage("Invalid receiver".into());
            return AppAction::None;
        }

        let receiver_id = parsed_receiver_id.unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.transfer_tokens_status = TransferTokensStatus::WaitingForResult(now);

        let data_contract = Arc::new(
            self.app_context
                .get_unqualified_contract_by_id(&self.identity_token_balance.data_contract_id)
                .expect("Failed to get data contract")
                .expect("Data contract not found"),
        );

        AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::TransferTokens {
                sending_identity: self.identity.clone(),
                recipient_id: receiver_id,
                amount: self.amount.clone().unwrap_or(Amount::new(0, 0)).value(),
                data_contract,
                token_position: self.identity_token_balance.token_position,
                signing_key: self.selected_key.clone().expect("No key selected"),
                public_note: self.public_note.clone(),
            },
        )))
    }
    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Success!");

            ui.add_space(20.0);

            // Display the "Back to Identities" button
            if ui.button("Back to Tokens").clicked() {
                // Handle navigation back to the identities screen
                action |= AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenLike for TransferTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "TransferTokens" {
                    self.transfer_tokens_status = TransferTokensStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.transfer_tokens_status =
                    TransferTokensStatus::ErrorMessage(message.to_string());
            }
        }
    }

    fn refresh(&mut self) {
        // Refresh the identity because there might be new keys
        self.identity = self
            .app_context
            .load_local_qualified_identities()
            .unwrap()
            .into_iter()
            .find(|identity| identity.identity.id() == self.identity.identity.id())
            .unwrap();
        let token_balances = self
            .app_context
            .db
            .get_identity_token_balances(&self.app_context)
            .expect("Token balances not loaded");
        self.max_amount = token_balances
            .values()
            .find(|balance| balance.identity_id == self.identity.identity.id())
            .map(Amount::from)
            .unwrap_or_default();
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_balance.token_alias,
                    AppAction::PopScreen,
                ),
                ("Transfer", AppAction::None),
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

        let central_panel_action = island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // Show the success screen if the transfer was successful
            if self.transfer_tokens_status == TransferTokensStatus::Complete {
                return self.show_success(ui);
            }

            ui.heading(format!(
                "Transfer {}",
                self.identity_token_balance.token_alias
            ));
            ui.add_space(10.0);

            let has_keys = if self.app_context.is_developer_mode() {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self
                    .identity
                    .available_authentication_keys_with_critical_security_level()
                    .is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    format!(
                        "You do not have any authentication keys with CRITICAL security level loaded for this {} identity.",
                        self.identity.identity_type
                    ),
                );
                ui.add_space(10.0);

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Keys").clicked() {
                        return AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    return AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return AppAction::None;
                    }
                }

                // Select the key to sign with
                ui.heading("1. Select the key to sign the transaction with");
                ui.add_space(10.0);

                let mut selected_identity = Some(self.identity.clone());
                add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    std::iter::once(&self.identity),
                    &mut selected_identity,
                    &mut self.selected_key,
                    TransactionType::TokenTransfer,
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the amount to transfer
                ui.heading("2. Input the amount to transfer");
                ui.add_space(5.0);

                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the ID of the identity to transfer to
                ui.heading("3. ID of the identity to transfer to");
                ui.add_space(5.0);
                self.render_to_identity_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("4. Public note (optional)");
                ui.add_space(5.0);
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
                        self.public_note = Some(txt);
                    }
                });
                ui.add_space(10.0);

                // Transfer button

                let ready = self.amount.is_some()
                    && !self.receiver_identity_id.is_empty()
                    && self.selected_key.is_some();
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button = egui::Button::new(RichText::new("Transfer").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);
                if ui
                    .add_enabled(ready, button)
                    .on_disabled_hover_text("Please ensure all fields are filled correctly")
                    .clicked()
                {
                    // Use the amount value directly since it's already parsed
                    if self.amount.as_ref().is_some_and(|v| v > &self.max_amount) {
                        self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(
                            "Amount exceeds available balance".to_string(),
                        );
                    } else if self.amount.as_ref().is_none_or(|a| a.value() == 0) {
                        self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(
                            "Amount must be greater than zero".to_string(),
                        );
                    } else {
                        let msg = format!(
                            "Are you sure you want to transfer {} tokens to {}?",
                            self.amount.clone().unwrap_or(Amount::new(0, 0)),
                            self.receiver_identity_id
                        );
                        self.confirmation_dialog = Some(
                            ConfirmationDialog::new("Confirm Transfer", msg)
                                .confirm_text(Some("Transfer"))
                                .cancel_text(Some("Cancel")),
                        );
                    }
                }

                if self.confirmation_dialog.is_some() {
                    return self.show_confirmation_popup(ui);
                }

                // Handle transfer status messages
                ui.add_space(5.0);
                match &self.transfer_tokens_status {
                    TransferTokensStatus::NotStarted => {
                        // Do nothing
                    }
                    TransferTokensStatus::WaitingForResult(start_time) => {
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

                        ui.label(format!(
                            "Transferring... Time taken so far: {}",
                            display_time
                        ));
                    }
                    TransferTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(
                            DashColors::error_color(dark_mode),
                            format!("Error: {}", msg),
                        );
                    }
                    TransferTokensStatus::Complete => {
                        // Handled above
                    }
                }
            }

            AppAction::None
        });
        action |= central_panel_action;
        action
    }
}

impl ScreenWithWalletUnlock for TransferTokensScreen {
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
        if let Some(error_message) = error_message {
            self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(error_message);
        }
    }

    fn error_message(&self) -> Option<&String> {
        if let TransferTokensStatus::ErrorMessage(error_message) = &self.transfer_tokens_status {
            Some(error_message)
        } else {
            None
        }
    }
}
