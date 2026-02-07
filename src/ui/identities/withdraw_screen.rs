use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use crate::model::qualified_identity::{IdentityType, PrivateKeyTarget, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context, Frame, Margin, Ui};
use egui::{Color32, RichText};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::get_selected_wallet;
use super::keys::add_key_screen::AddKeyScreen;
use super::keys::key_info_screen::KeyInfoScreen;

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
    withdrawal_address_error: Option<String>,
    withdrawal_amount: Option<Amount>,
    withdrawal_amount_input: Option<AmountInput>,
    max_amount: u64,
    pub app_context: Arc<AppContext>,
    confirmation_dialog: Option<ConfirmationDialog>,
    withdraw_from_identity_status: WithdrawFromIdentityStatus,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    show_advanced_options: bool,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
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
            withdrawal_address_error: None,
            withdrawal_amount: None,
            withdrawal_amount_input: None,
            max_amount,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            withdraw_from_identity_status: WithdrawFromIdentityStatus::NotStarted,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message,
            show_advanced_options: false,
            completed_fee_result: None,
        }
    }

    fn render_key_selection(&mut self, ui: &mut Ui) -> AppAction {
        add_key_chooser(
            ui,
            &self.app_context,
            &self.identity,
            &mut self.selected_key,
            TransactionType::Withdraw,
        )
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        let max_amount_minus_fee = (self.max_amount as f64 / 100_000_000_000.0 - 0.005).max(0.0);
        let max_amount_credits = (max_amount_minus_fee * 100_000_000_000.0) as u64;

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
        // errors are handled inside AmountInput
    }

    fn render_address_input(&mut self, ui: &mut Ui) {
        let is_owner_key = self
            .selected_key
            .as_ref()
            .map(|key| key.purpose() == Purpose::OWNER)
            .unwrap_or(false);
        let can_have_withdrawal_address = !is_owner_key;

        if can_have_withdrawal_address || self.app_context.is_developer_mode() {
            ui.horizontal(|ui| {
                ui.label("Address:");

                let response = ui.text_edit_singleline(&mut self.withdrawal_address);

                // Validate address when it changes
                if response.changed() {
                    if self.withdrawal_address.is_empty() {
                        self.withdrawal_address_error = None;
                    } else {
                        match Address::from_str(&self.withdrawal_address) {
                            Ok(_) => {
                                self.withdrawal_address_error = None;
                            }
                            Err(_) => {
                                self.withdrawal_address_error = Some("Invalid address".to_string());
                            }
                        }
                    }
                }

                // Show error next to input
                if let Some(error) = &self.withdrawal_address_error {
                    ui.colored_label(Color32::from_rgb(255, 100, 100), error);
                }
            });

            // In dev mode with OWNER key, show hint about auto-selected payout address
            if self.app_context.is_developer_mode()
                && is_owner_key
                && let Some(payout_address) = self
                    .identity
                    .masternode_payout_address(self.app_context.network)
            {
                ui.label(
                    RichText::new(format!(
                        "Leave empty to use masternode payout address: {}",
                        payout_address
                    ))
                    .italics()
                    .color(Color32::GRAY),
                );
            }
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
        let address = if self.withdrawal_address.is_empty() {
            None
        } else {
            match Address::from_str(&self.withdrawal_address) {
                Ok(address) => Some(address.assume_checked()),
                Err(_) => {
                    // Error is already shown next to the input field
                    self.withdrawal_address_error = Some("Invalid address".to_string());
                    self.confirmation_dialog = None;
                    return AppAction::None;
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
            self.confirmation_dialog = None;
            return AppAction::None;
        } else {
            "to default address".to_string()
        };

        let Some(selected_key) = self.selected_key.as_ref() else {
            self.withdraw_from_identity_status =
                WithdrawFromIdentityStatus::ErrorMessage("No selected key".to_string());
            self.confirmation_dialog = None;
            return AppAction::None;
        };

        let dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new(
                "Confirm Withdrawal".to_string(),
                format!(
                    "Are you sure you want to withdraw {} to {}",
                    self.withdrawal_amount
                        .as_ref()
                        .expect("Withdrawal amount should be present"),
                    message_address
                ),
            )
            .danger_mode(true) // Withdrawal is a destructive operation
        });

        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.confirmation_dialog = None;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.withdraw_from_identity_status =
                    WithdrawFromIdentityStatus::WaitingForResult(now);

                // Use the amount directly from the stored amount
                let credits = self
                    .withdrawal_amount
                    .as_ref()
                    .expect("Withdrawal amount should be present")
                    .value() as u128;

                AppAction::BackendTask(BackendTask::IdentityTask(
                    IdentityTask::WithdrawFromIdentity(
                        self.identity.clone(),
                        address,
                        credits as Credits,
                        Some(selected_key.id()),
                    ),
                ))
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Withdrawal Successful!\n\nNote: It may take a few minutes for funds to appear on the Core chain.".to_string(),
            vec![(
                "Back to Identities".to_string(),
                AppAction::PopScreenAndRefresh,
            )],
            None,
        )
    }
}

impl ScreenLike for WithdrawalScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.withdraw_from_identity_status =
                WithdrawFromIdentityStatus::ErrorMessage(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) =
            backend_task_success_result
        {
            self.completed_fee_result = Some(fee_result);
            self.withdraw_from_identity_status = WithdrawFromIdentityStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        // Refresh the identity because there might be new keys
        if let Some(refreshed) = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|identity| identity.identity.id() == self.identity.identity.id())
        {
            self.identity = refreshed;
            self.max_amount = self.identity.identity.balance();
        }
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

            // Heading with checkbox on the same line
            ui.horizontal(|ui| {
                ui.heading("Withdraw Funds");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.show_advanced_options, "Show Advanced Options");
                });
            });
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
                                return inner_action;
                            }
                        }
                    }
                } else {
                    return inner_action;
                }

                // Input the amount to withdraw
                ui.heading("1. Amount to withdraw (Dash)");
                ui.add_space(5.0);

                // Show identity info
                let identity_id_string = self.identity.identity.id().to_string(Encoding::Base58);
                let identity_label = if let Some(alias) = &self.identity.alias {
                    format!("From: {} ({})", alias, identity_id_string)
                } else {
                    format!("From: {}", identity_id_string)
                };
                ui.label(identity_label);

                // Display available balance
                let balance_dash = self.max_amount as f64 / 100_000_000_000.0;
                ui.horizontal(|ui| {
                    ui.label("Available Balance:");
                    ui.label(RichText::new(format!("{:.4} Dash", balance_dash)));
                });
                ui.add_space(5.0);

                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the address to withdraw to
                ui.heading("2. Dash address to withdraw to");
                ui.add_space(5.0);
                self.render_address_input(ui);

                // Only show key selection in advanced mode
                if self.show_advanced_options {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading("3. Select the key to sign with");
                    inner_action |= self.render_key_selection(ui);
                }

                ui.add_space(10.0);

                // Fee estimation display
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_credit_withdrawal();

                let dark_mode = ui.ctx().style().visuals.dark_mode;
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

                ui.add_space(10.0);

                // Withdraw button

                let button = egui::Button::new(RichText::new("Withdraw").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0)
                    .min_size(egui::vec2(60.0, 30.0));

                let has_valid_amount = self.withdrawal_amount.is_some();
                let has_address_error = self.withdrawal_address_error.is_some();
                let has_enough_balance = self.max_amount > estimated_fee;
                let ready = has_valid_amount && !has_address_error && has_enough_balance;

                let hover_text = if !has_valid_amount {
                    "Please enter a valid amount to withdraw".to_string()
                } else if has_address_error {
                    "Please enter a valid withdrawal address".to_string()
                } else if !has_enough_balance {
                    format!(
                        "Insufficient balance for withdrawal fee (need at least {})",
                        format_credits_as_dash(estimated_fee)
                    )
                } else {
                    String::new()
                };

                if ui
                    .add_enabled(ready, button)
                    .on_disabled_hover_text(&hover_text)
                    .clicked()
                    && self.confirmation_dialog.is_none()
                {
                    // Create dialog directly in show_confirmation_popup with correct message
                    inner_action |= self.show_confirmation_popup(ui);
                }

                if self.confirmation_dialog.is_some() {
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
                            .unwrap_or_default()
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
                        let error_color = Color32::from_rgb(255, 100, 100);
                        let msg = msg.clone();
                        Frame::new()
                            .fill(error_color.gamma_multiply(0.1))
                            .inner_margin(Margin::symmetric(10, 8))
                            .corner_radius(5.0)
                            .stroke(egui::Stroke::new(1.0, error_color))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Error: {}", msg)).color(error_color),
                                    );
                                    ui.add_space(10.0);
                                    if ui.small_button("Dismiss").clicked() {
                                        self.withdraw_from_identity_status =
                                            WithdrawFromIdentityStatus::NotStarted;
                                    }
                                });
                            });
                    }
                    WithdrawFromIdentityStatus::Complete => {
                        ui.colored_label(
                            egui::Color32::DARK_GREEN,
                            "Successfully withdrew from identity".to_string(),
                        );
                    }
                }
            }

            inner_action
        });

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
