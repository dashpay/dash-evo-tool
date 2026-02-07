use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Frame, Margin, Ui};
use egui::{Color32, RichText};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::get_selected_wallet;
use super::keys::add_key_screen::AddKeyScreen;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::theme::DashColors;

/// Transfer destination type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferDestinationType {
    #[default]
    Identity,
    PlatformAddress,
}

#[derive(PartialEq)]
pub enum TransferCreditsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct TransferScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    known_identities: Vec<QualifiedIdentity>,
    receiver_identity_id: String,
    amount: Option<Amount>,
    amount_input: Option<AmountInput>,
    transfer_credits_status: TransferCreditsStatus,
    error_message: Option<String>,
    max_amount: u64,
    pub app_context: Arc<AppContext>,
    confirmation_popup: bool,
    confirmation_dialog: Option<ConfirmationDialog>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    // Platform address transfer fields
    destination_type: TransferDestinationType,
    platform_address_input: String,
    show_advanced_options: bool,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
}

impl TransferScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let known_identities = app_context
            .load_local_qualified_identities()
            .expect("Identities not loaded");

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
            known_identities,
            receiver_identity_id: String::new(),
            amount: Some(Amount::new_dash(0.0)),
            amount_input: None,
            transfer_credits_status: TransferCreditsStatus::NotStarted,
            error_message: None,
            max_amount,
            app_context: app_context.clone(),
            confirmation_popup: false,
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            destination_type: TransferDestinationType::Identity,
            platform_address_input: String::new(),
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
            TransactionType::Transfer,
        )
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        // Show available balance
        let balance_in_dash = self.max_amount as f64 / 100_000_000_000.0;
        ui.label(format!("Available balance: {:.8} DASH", balance_in_dash));
        ui.add_space(5.0);

        // Calculate max amount minus fee for the "Max" button
        let max_amount_minus_fee = (self.max_amount as f64 / 100_000_000_000.0 - 0.0002).max(0.0);
        let max_amount_credits = (max_amount_minus_fee * 100_000_000_000.0) as u64;

        let amount_input = self.amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::new_dash(0.0))
                .with_label("Amount:")
                .with_max_button(true)
                .with_max_amount(Some(max_amount_credits))
        });

        // Check if input should be disabled when operation is in progress
        let enabled = match self.transfer_credits_status {
            TransferCreditsStatus::WaitingForResult(_) | TransferCreditsStatus::Complete => false,
            TransferCreditsStatus::NotStarted | TransferCreditsStatus::ErrorMessage(_) => {
                amount_input.set_max_amount(Some(max_amount_credits));
                true
            }
        };

        let response = ui.add_enabled_ui(enabled, |ui| amount_input.show(ui)).inner;

        response.inner.update(&mut self.amount);
        // errors are handled inside AmountInput
    }

    fn render_to_identity_input(&mut self, ui: &mut Ui) {
        ui.add(
            IdentitySelector::new(
                "transfer_recipient_selector",
                &mut self.receiver_identity_id,
                &self.known_identities,
            )
            .width(300.0)
            .label("Receiver Identity ID:")
            .exclude(&[self.identity.identity.id()]),
        );
    }

    fn render_destination_type_selector(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Colors for selected/unselected states
        let selected_fill = DashColors::DASH_BLUE;
        let selected_text = Color32::WHITE;
        let unselected_fill = if dark_mode {
            Color32::from_rgb(60, 60, 60)
        } else {
            Color32::from_rgb(220, 220, 220)
        };
        let unselected_text = DashColors::text_primary(dark_mode);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.label("Transfer to:");
            });
            ui.add_space(10.0);

            // Identity button
            let identity_selected = self.destination_type == TransferDestinationType::Identity;
            let identity_button = egui::Button::new(
                RichText::new("Identity")
                    .color(if identity_selected {
                        selected_text
                    } else {
                        unselected_text
                    })
                    .strong(),
            )
            .fill(if identity_selected {
                selected_fill
            } else {
                unselected_fill
            })
            .min_size(egui::vec2(120.0, 28.0));

            if ui.add(identity_button).clicked() {
                self.destination_type = TransferDestinationType::Identity;
            }

            ui.add_space(5.0);

            // Platform Address button
            let platform_selected =
                self.destination_type == TransferDestinationType::PlatformAddress;
            let platform_button = egui::Button::new(
                RichText::new("Platform Address")
                    .color(if platform_selected {
                        selected_text
                    } else {
                        unselected_text
                    })
                    .strong(),
            )
            .fill(if platform_selected {
                selected_fill
            } else {
                unselected_fill
            })
            .min_size(egui::vec2(140.0, 28.0));

            if ui.add(platform_button).clicked() {
                self.destination_type = TransferDestinationType::PlatformAddress;
            }
        });
    }

    fn render_platform_address_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Platform Address:");
            ui.add_space(5.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.platform_address_input)
                    .hint_text("Enter Platform address (y...)")
                    .desired_width(400.0),
            );
        });
    }

    /// Validate and parse the Platform address
    fn validate_platform_address(&self) -> Result<PlatformAddress, String> {
        if self.platform_address_input.is_empty() {
            return Err("Platform address is required".to_string());
        }

        let input = self.platform_address_input.trim();

        // Try to parse as Bech32m Platform address first (evo1.../tevo1...)
        if input.starts_with("evo1") || input.starts_with("tevo1") {
            let (addr, _network) = PlatformAddress::from_bech32m_string(input)
                .map_err(|e| format!("Invalid Bech32m address: {}", e))?;
            return Ok(addr);
        }

        // Fall back to base58 parsing for backwards compatibility
        let unchecked_addr: Address<NetworkUnchecked> = input
            .parse()
            .map_err(|e| format!("Invalid address format: {}", e))?;

        // Platform addresses use the same version byte (0x5a / prefix 'd') for
        // testnet, devnet, and regtest per DIP-18. We use assume_checked() here
        // because require_network() would fail on regtest (address parses as testnet).
        let address = unchecked_addr.assume_checked();

        PlatformAddress::try_from(address).map_err(|e| format!("Invalid Platform address: {}", e))
    }

    /// Handle the confirmation action for Platform address transfer
    fn confirmation_ok_platform_address(&mut self) -> AppAction {
        self.confirmation_popup = false;
        self.confirmation_dialog = None;

        // Validate Platform address
        let platform_address = match self.validate_platform_address() {
            Ok(addr) => addr,
            Err(error) => {
                self.set_error_state(error);
                return AppAction::None;
            }
        };

        // Validate selected key
        let selected_key = match self.selected_key.as_ref() {
            Some(key) => key,
            None => {
                self.set_error_state("No selected key".to_string());
                return AppAction::None;
            }
        };

        // Get the amount
        let credits = self.amount.as_ref().map(|v| v.value()).unwrap_or_default() as u128;
        if credits == 0 {
            self.error_message = Some("Amount must be greater than 0".to_string());
            self.transfer_credits_status =
                TransferCreditsStatus::ErrorMessage("Amount must be greater than 0".to_string());
            return AppAction::None;
        }

        // Set waiting state
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.transfer_credits_status = TransferCreditsStatus::WaitingForResult(now);

        // Build outputs
        let mut outputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        outputs.insert(platform_address, credits as Credits);

        AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TransferToAddresses {
                identity: self.identity.clone(),
                outputs,
                key_id: Some(selected_key.id()),
            },
        ))
    }

    /// Handle the confirmation action when user clicks OK
    fn confirmation_ok(&mut self) -> AppAction {
        self.confirmation_popup = false;
        self.confirmation_dialog = None; // Reset the dialog for next use

        // Validate identifier
        let identifier = match self.validate_receiver_identifier() {
            Ok(id) => id,
            Err(error) => {
                self.set_error_state(error);
                return AppAction::None;
            }
        };

        // Validate selected key
        let selected_key = match self.selected_key.as_ref() {
            Some(key) => key,
            None => {
                self.set_error_state("No selected key".to_string());
                return AppAction::None;
            }
        };

        // Use the amount directly since it's already an Amount struct
        let credits = self.amount.as_ref().map(|v| v.value()).unwrap_or_default() as u128;
        if credits == 0 {
            self.error_message = Some("Amount must be greater than 0".to_string());
            self.transfer_credits_status =
                TransferCreditsStatus::ErrorMessage("Amount must be greater than 0".to_string());
            self.confirmation_popup = false;
            return AppAction::None;
        }

        // Set waiting state and create backend task
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.transfer_credits_status = TransferCreditsStatus::WaitingForResult(now);

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::Transfer(
            self.identity.clone(),
            identifier,
            credits as Credits,
            Some(selected_key.id()),
        )))
    }

    /// Handle the cancel action when user clicks Cancel or closes dialog
    fn confirmation_cancel(&mut self) -> AppAction {
        self.confirmation_popup = false;
        self.confirmation_dialog = None; // Reset the dialog for next use
        AppAction::None
    }

    /// Validate the receiver identity identifier
    fn validate_receiver_identifier(&self) -> Result<Identifier, String> {
        if self.receiver_identity_id.is_empty() {
            return Err("Invalid identifier".to_string());
        }

        Identifier::from_string_try_encodings(
            &self.receiver_identity_id,
            &[Encoding::Base58, Encoding::Hex],
        )
        .map_err(|_| "Invalid identifier".to_string())
    }

    /// Set error state with the given message
    fn set_error_state(&mut self, error: String) {
        self.error_message = Some(error.clone());
        self.transfer_credits_status = TransferCreditsStatus::ErrorMessage(error);
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        // Prepare values before borrowing
        let Some(amount) = &self.amount else {
            self.set_error_state("Incorrect or empty amount".to_string());
            return AppAction::None;
        };

        let receiver_id = self.receiver_identity_id.clone();

        let msg = format!(
            "Are you sure you want to transfer {} to {}?",
            amount, receiver_id
        );

        // Lazy initialization of the confirmation dialog
        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm Transfer", msg)
                .confirm_text(Some("Confirm"))
                .cancel_text(Some("Cancel"))
        });

        let response = confirmation_dialog.show(ui);

        // Handle the response using the Component pattern
        match response.inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => self.confirmation_ok(),
            Some(ConfirmationStatus::Canceled) => self.confirmation_cancel(),
            None => AppAction::None,
        }
    }

    fn show_platform_address_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        // Prepare values before borrowing
        let Some(amount) = &self.amount else {
            self.set_error_state("Incorrect or empty amount".to_string());
            return AppAction::None;
        };

        let platform_address = self.platform_address_input.clone();

        let msg = format!(
            "Are you sure you want to transfer {} to Platform address {}?",
            amount, platform_address
        );

        // Lazy initialization of the confirmation dialog
        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm Transfer to Platform Address", msg)
                .confirm_text(Some("Confirm"))
                .cancel_text(Some("Cancel"))
        });

        let response = confirmation_dialog.show(ui);

        // Handle the response using the Component pattern
        match response.inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => self.confirmation_ok_platform_address(),
            Some(ConfirmationStatus::Canceled) => self.confirmation_cancel(),
            None => AppAction::None,
        }
    }

    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Transfer Successful!".to_string(),
            vec![(
                "Back to Identities".to_string(),
                AppAction::PopScreenAndRefresh,
            )],
            None,
        )
    }
}

impl ScreenLike for TransferScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.transfer_credits_status = TransferCreditsStatus::ErrorMessage(message.to_string());
            self.error_message = Some(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::TransferredCredits(fee_result) =
            backend_task_success_result
        {
            self.completed_fee_result = Some(fee_result);
            self.transfer_credits_status = TransferCreditsStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        // Refresh the identity because there might be new keys
        let identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default();
        if let Some(refreshed) = identities
            .iter()
            .find(|identity| identity.identity.id() == self.identity.identity.id())
        {
            self.identity = refreshed.clone();
            self.max_amount = self.identity.identity.balance();
        }
        self.known_identities = identities;
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Transfer", AppAction::None),
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

            // Show the success screen if the transfer was successful
            if self.transfer_credits_status == TransferCreditsStatus::Complete {
                inner_action |= self.show_success(ui);
                return inner_action;
            }

            let has_keys = if self.app_context.is_developer_mode() {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_transfer_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    format!(
                        "You do not have any transfer keys loaded for this {} identity.",
                        self.identity.identity_type
                    ),
                );
                ui.add_space(10.0);

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::TRANSFER,
                    SecurityLevel::full_range().into(),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Transfer Key").clicked() {
                        inner_action |=
                            AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                                self.identity.clone(),
                                key.clone(),
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
                if self.selected_wallet.is_some()
                    && let Some(wallet) = &self.selected_wallet
                {
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

                // Heading with checkbox on the same line
                ui.horizontal(|ui| {
                    ui.heading("Transfer Funds");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Show Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Input the amount to transfer
                ui.heading("1. Input the amount to transfer");
                ui.add_space(5.0);

                // Show identity info
                let identity_id_string = self.identity.identity.id().to_string(Encoding::Base58);
                let identity_label = if let Some(alias) = &self.identity.alias {
                    format!("From: {} ({})", alias, identity_id_string)
                } else {
                    format!("From: {}", identity_id_string)
                };
                ui.label(identity_label);
                ui.add_space(5.0);

                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Destination type selector
                ui.heading("2. Select transfer destination type");
                ui.add_space(5.0);
                self.render_destination_type_selector(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the destination based on type
                match self.destination_type {
                    TransferDestinationType::Identity => {
                        ui.heading("3. ID of the identity to transfer to");
                        ui.add_space(5.0);
                        self.render_to_identity_input(ui);
                    }
                    TransferDestinationType::PlatformAddress => {
                        ui.heading("3. Platform address to transfer to");
                        ui.add_space(5.0);
                        self.render_platform_address_input(ui);
                    }
                }

                // Select the key to sign with (only in advanced mode)
                if self.show_advanced_options {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading("4. Select the key to sign the transaction with");
                    ui.add_space(10.0);
                    inner_action |= self.render_key_selection(ui);
                }

                ui.add_space(10.0);

                // Fee estimation
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = match self.destination_type {
                    TransferDestinationType::Identity => fee_estimator.estimate_credit_transfer(),
                    TransferDestinationType::PlatformAddress => {
                        // Platform address transfer has output cost
                        fee_estimator.estimate_credit_transfer_to_addresses(1)
                    }
                };

                // Display estimated fee
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                Frame::group(ui.style())
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

                // Transfer button - check readiness based on destination type
                let has_enough_balance = self.identity.identity.balance() > estimated_fee;

                let ready = self.amount.is_some()
                    && self.selected_key.is_some()
                    && has_enough_balance
                    && !matches!(
                        self.transfer_credits_status,
                        TransferCreditsStatus::WaitingForResult(_),
                    )
                    && match self.destination_type {
                        TransferDestinationType::Identity => !self.receiver_identity_id.is_empty(),
                        TransferDestinationType::PlatformAddress => {
                            !self.platform_address_input.is_empty()
                        }
                    };
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button = egui::Button::new(RichText::new("Transfer").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);

                let hover_text = if !has_enough_balance {
                    format!(
                        "Insufficient balance for transfer fee (need at least {})",
                        format_credits_as_dash(estimated_fee)
                    )
                } else if ready {
                    "Transfer credits to another identity or Platform address".to_string()
                } else {
                    "Please ensure all fields are filled correctly".to_string()
                };

                if ui
                    .add_enabled(ready, button)
                    .on_hover_text(hover_text)
                    .clicked()
                {
                    self.confirmation_popup = true;
                }

                if self.confirmation_popup {
                    inner_action |= match self.destination_type {
                        TransferDestinationType::Identity => self.show_confirmation_popup(ui),
                        TransferDestinationType::PlatformAddress => {
                            self.show_platform_address_confirmation_popup(ui)
                        }
                    };
                }

                // Handle transfer status messages
                ui.add_space(5.0);
                match &self.transfer_credits_status {
                    TransferCreditsStatus::NotStarted => {
                        // Do nothing
                    }
                    TransferCreditsStatus::WaitingForResult(start_time) => {
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
                    TransferCreditsStatus::ErrorMessage(msg) => {
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
                                        self.transfer_credits_status =
                                            TransferCreditsStatus::NotStarted;
                                    }
                                });
                            });
                    }
                    TransferCreditsStatus::Complete => {
                        // Handled above
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
