use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context, Ui};
use egui::{Color32, RichText};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::get_selected_wallet;
use super::keys::add_key_screen::AddKeyScreen;
use super::keys::key_info_screen::KeyInfoScreen;

/// Fee in credits for the withdrawal transaction
const WITHDRAWAL_FEE_IN_CREDITS: Credits = 1_000_000_000;

#[derive(PartialEq)]
pub enum WithdrawFromIdentityStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct WithdrawalScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    withdrawal_address: String,
    withdrawal_amount: Option<Amount>,
    withdrawal_amount_input: Option<AmountInput>,
    max_amount_credits: Credits,
    pub app_context: Arc<AppContext>,
    confirmation_popup: bool,
    withdraw_from_identity_status: WithdrawFromIdentityStatus,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl WithdrawalScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let max_amount = identity.identity.balance();
        let identity_clone = identity.identity.clone();
        let selected_key = identity_clone.get_first_public_key_matching(
            Purpose::TRANSFER,
            SecurityLevel::full_range().into(),
            KeyType::all_key_types().into(),
            false,
        );
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key, &mut error_message);
        Self {
            identity,
            selected_key: selected_key.cloned(),
            withdrawal_address: String::new(),
            withdrawal_amount: None,
            withdrawal_amount_input: None,
            max_amount_credits: max_amount,
            app_context: app_context.clone(),
            confirmation_popup: false,
            withdraw_from_identity_status: WithdrawFromIdentityStatus::NotStarted,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message,
        }
    }

    fn render_key_selection(&mut self, ui: &mut Ui) {
        let mut selected_identity = Some(self.identity.clone());
        add_identity_key_chooser(
            ui,
            &self.app_context,
            std::iter::once(&self.identity),
            &mut selected_identity,
            &mut self.selected_key,
            TransactionType::Withdraw,
        );
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        let max_amount_credits = self
            .max_amount_credits
            .saturating_sub(WITHDRAWAL_FEE_IN_CREDITS);

        // Lazy initialization with basic configuration
        let amount_input = self.withdrawal_amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::new_dash(0.0))
                .with_label("Amount:")
                .with_max_button(true)
        });

        // Check if input should be disabled when operation is in progress
        let enabled = match self.withdraw_from_identity_status {
            WithdrawFromIdentityStatus::WaitingForResult(_)
            | WithdrawFromIdentityStatus::Complete => false,

            WithdrawFromIdentityStatus::NotStarted
            | WithdrawFromIdentityStatus::ErrorMessage(_) => {
                amount_input.set_max_amount(Some(max_amount_credits));
                true
            }
        };

        let response = ui.add_enabled_ui(enabled, |ui| amount_input.show(ui)).inner;

        response.inner.update(&mut self.withdrawal_amount);
        if let Some(error) = &response.inner.error_message {
            ui.colored_label(egui::Color32::DARK_RED, error);
        }
    }

    fn render_address_input(&mut self, ui: &mut Ui) {
        let can_have_withdrawal_address = if let Some(key) = self.selected_key.as_ref() {
            key.purpose() != Purpose::OWNER
        } else {
            true
        };
        if can_have_withdrawal_address || self.app_context.is_developer_mode() {
            ui.horizontal(|ui| {
                ui.label("Address:");

                ui.text_edit_singleline(&mut self.withdrawal_address);
            });
        } else {
            ui.label(format!(
                "Masternode payout address: {}",
                match self
                    .identity
                    .masternode_payout_address(self.app_context.network)
                {
                    Some(address) => address.to_string(),
                    None => "No masternode payout address".to_string(),
                }
            ));
        }
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Withdrawal")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                let address = if self.withdrawal_address.is_empty() {
                    None
                } else {
                    match Address::from_str(&self.withdrawal_address) {
                        Ok(address) => Some(address.assume_checked()),
                        Err(_) => {
                            self.withdraw_from_identity_status =
                                WithdrawFromIdentityStatus::ErrorMessage(
                                    "Invalid withdrawal address".to_string(),
                                );
                            None
                        }
                    }
                };

                let message_address = if address.is_some() {
                    self.withdrawal_address.clone()
                } else if let Some(payout_address) = self
                    .identity
                    .masternode_payout_address(self.app_context.network)
                {
                    format!("masternode payout address {}", payout_address)
                } else if !self.app_context.is_developer_mode() {
                    self.withdraw_from_identity_status = WithdrawFromIdentityStatus::ErrorMessage(
                        "No masternode payout address".to_string(),
                    );
                    return;
                } else {
                    "to default address".to_string()
                };

                let Some(selected_key) = self.selected_key.as_ref() else {
                    self.withdraw_from_identity_status =
                        WithdrawFromIdentityStatus::ErrorMessage("No selected key".to_string());
                    return;
                };

                ui.label(format!(
                    "Are you sure you want to withdraw {} to {}",
                    self.withdrawal_amount
                        .as_ref()
                        .expect("Withdrawal amount should be present"),
                    message_address
                ));

                // Use the amount directly from the stored amount
                let credits = self
                    .withdrawal_amount
                    .as_ref()
                    .expect("Withdrawal amount should be present")
                    .value() as u128;

                if ui.button("Confirm").clicked() {
                    self.confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.withdraw_from_identity_status =
                        WithdrawFromIdentityStatus::WaitingForResult(now);
                    app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::WithdrawFromIdentity(
                            self.identity.clone(),
                            address,
                            credits as Credits,
                            Some(selected_key.id()),
                        ),
                    ));
                }
                if ui.button("Cancel").clicked() {
                    self.confirmation_popup = false;
                }
            });
        if !is_open {
            self.confirmation_popup = false;
        }
        app_action
    }

    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully withdrew from identity");

            ui.add_space(20.0);

            // Display the "Back to Identities" button
            if ui.button("Back to Identities").clicked() {
                // Handle navigation back to the identities screen
                action = AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenLike for WithdrawalScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "Successfully withdrew from identity" {
                    self.withdraw_from_identity_status = WithdrawFromIdentityStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.withdraw_from_identity_status =
                    WithdrawFromIdentityStatus::ErrorMessage(message.to_string());
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
        self.max_amount_credits = self.identity.identity.balance();
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Withdraw", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            // Show the success screen if the withdrawal was successful
            if self.withdraw_from_identity_status == WithdrawFromIdentityStatus::Complete {
                inner_action |= self.show_success(ui);
                return inner_action;
            }

            ui.heading("Withdraw Funds");
            ui.add_space(10.0);

            let has_keys = if self.app_context.is_developer_mode() {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_withdrawal_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    format!("You do not have any withdrawal keys loaded for this {} identity. Note that TRANSFER or OWNER keys are used for withdrawals.", self.identity.identity_type));
                ui.add_space(10.0);

                if self.identity.identity_type != IdentityType::User {
                    ui.label("An evonode can withdraw with the payout address private key or the owner key.".to_string());
                    ui.label("If the owner key is used you can only withdraw to the Dash Core payout address (where you get your Core rewards).".to_string());
                    ui.add_space(10.0);
                }

                let owner_key = self.identity.identity.get_first_public_key_matching(
                    Purpose::OWNER,
                    SecurityLevel::full_range().into(),
                    KeyType::all_key_types().into(),
                    false,
                );
                let transfer_key = self.identity.identity.get_first_public_key_matching(
                    Purpose::TRANSFER,
                    SecurityLevel::full_range().into(),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(owner_key) = owner_key {
                    if ui.button("Check Owner Key").clicked() {
                        inner_action |=
                            AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                                self.identity.clone(),
                                owner_key.clone(),
                                None,
                                &self.app_context,
                            )));
                    }
                    ui.add_space(5.0);
                }

                if let Some(transfer_key) = transfer_key {
                    let key_type_name = match self.identity.identity_type {
                        IdentityType::User => "Transfer",
                        IdentityType::Masternode => "Payout",
                        IdentityType::Evonode => "Payout",
                    };
                    if ui
                        .button(format!("Check {} Address Key", key_type_name))
                        .clicked()
                    {
                        inner_action |=
                            AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                                self.identity.clone(),
                                transfer_key.clone(),
                                None,
                                &self.app_context,
                            )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    inner_action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Select the key to sign with
                ui.heading("1. Select the key to sign with");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    self.render_key_selection(ui);
                    ui.add_space(5.0);
                    let identity_id_string =
                        self.identity.identity.id().to_string(Encoding::Base58);
                    let identity_display = self
                        .identity
                        .alias
                        .as_deref()
                        .unwrap_or_else(|| &identity_id_string);
                    ui.label(format!("Identity: {}", identity_display));
                });

                // Render wallet unlock component if needed
                if let Some(selected_key) = self.selected_key.as_ref() {
                    // If there is an associated wallet then render the wallet unlock component for it if its locked
                    if let Some((
                        _,
                        PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                    )) = self.identity.private_keys.private_keys.get(&(
                        PrivateKeyTarget::PrivateKeyOnMainIdentity,
                        selected_key.id(),
                    )) {
                        self.selected_wallet = self
                            .identity
                            .associated_wallets
                            .get(&wallet_derivation_path.wallet_seed_hash)
                            .cloned();

                        let (needed_unlock, just_unlocked) =
                            self.render_wallet_unlock_if_needed(ui);

                        if needed_unlock && !just_unlocked {
                            return inner_action;
                        }
                    }
                } else {
                    return inner_action;
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the amount to transfer
                ui.heading("2. Input the amount to withdraw");
                ui.add_space(5.0);
                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the ID of the identity to transfer to
                ui.heading("3. Dash address to withdraw to");
                ui.add_space(5.0);
                self.render_address_input(ui);

                ui.add_space(10.0);

                // Withdraw button

                let button = egui::Button::new(RichText::new("Withdraw").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0)
                    .min_size(egui::vec2(60.0, 30.0));

                let ready = self.withdrawal_amount.as_ref().is_some();

                if ui
                    .add_enabled(ready, button)
                    .on_disabled_hover_text("Please enter a valid amount to withdraw")
                    .clicked()
                {
                    self.confirmation_popup = true;
                }

                if self.confirmation_popup {
                    inner_action |= self.show_confirmation_popup(ui);
                }

                ui.add_space(10.0);

                // Handle withdrawal status messages
                match &self.withdraw_from_identity_status {
                    WithdrawFromIdentityStatus::NotStarted => {
                        // Do nothing
                    }
                    WithdrawFromIdentityStatus::WaitingForResult(start_time) => {
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
                            "Withdrawing... Time taken so far: {}",
                            display_time
                        ));
                    }
                    WithdrawFromIdentityStatus::ErrorMessage(msg) => {
                        ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                    }
                    WithdrawFromIdentityStatus::Complete => {
                        ui.colored_label(
                            egui::Color32::DARK_GREEN,
                            "Successfully withdrew from identity".to_string(),
                        );
                    }
                }

                if let WithdrawFromIdentityStatus::ErrorMessage(ref error_message) =
                    self.withdraw_from_identity_status
                {
                    ui.label(format!("Error: {}", error_message));
                }
            }

            inner_action
        });
        action
    }
}

impl ScreenWithWalletUnlock for WithdrawalScreen {
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
