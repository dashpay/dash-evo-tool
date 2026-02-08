//! Single Key Wallet Send Screen

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::components::fee_confirmation_dialog::{
    FeeConfirmationDialog, FeeConfirmationResponse, parse_min_relay_fee_error,
};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::wallets::send_utils::{format_dash, parse_amount_to_duffs};
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeLevel;
use eframe::egui::{self, Context, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

/// A single recipient entry with address and amount
#[derive(Debug, Clone)]
pub struct SendRecipient {
    pub id: usize,
    pub address: String,
    pub amount: String,
    pub error: Option<String>,
}

impl SendRecipient {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            address: String::new(),
            amount: String::new(),
            error: None,
        }
    }
}

pub struct SingleKeyWalletSendScreen {
    pub app_context: Arc<AppContext>,
    pub selected_wallet: Option<Arc<RwLock<SingleKeyWallet>>>,

    // Recipients (support multiple)
    recipients: Vec<SendRecipient>,
    next_recipient_id: usize,

    // Common options
    subtract_fee: bool,
    memo: String,

    // State
    sending: bool,
    message: Option<(String, MessageType, DateTime<Utc>)>,

    // Wallet unlock
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,

    // Fee confirmation dialog
    fee_dialog: FeeConfirmationDialog,
    pending_request: Option<WalletPaymentRequest>,

    // Advanced options toggle
    show_advanced_options: bool,
}

impl SingleKeyWalletSendScreen {
    pub fn new(app_context: &Arc<AppContext>, wallet: Arc<RwLock<SingleKeyWallet>>) -> Self {
        Self {
            app_context: app_context.clone(),
            selected_wallet: Some(wallet),
            recipients: vec![SendRecipient::new(0)],
            next_recipient_id: 1,
            subtract_fee: false,
            memo: String::new(),
            sending: false,
            message: None,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
            fee_dialog: FeeConfirmationDialog::default(),
            pending_request: None,
            show_advanced_options: false,
        }
    }

    fn add_recipient(&mut self) {
        let id = self.next_recipient_id;
        self.next_recipient_id += 1;
        self.recipients.push(SendRecipient::new(id));
    }

    fn remove_recipient(&mut self, id: usize) {
        if self.recipients.len() > 1 {
            self.recipients.retain(|r| r.id != id);
        }
    }

    /// Estimate transaction size for P2PKH transactions
    fn estimate_p2pkh_tx_size(inputs: usize, outputs: usize) -> usize {
        fn varint_size(value: usize) -> usize {
            match value {
                0..=0xfc => 1,
                0xfd..=0xffff => 3,
                0x1_0000..=0xffff_ffff => 5,
                _ => 9,
            }
        }
        let mut size = 8; // version/type/lock_time
        size += varint_size(inputs);
        size += varint_size(outputs);
        size += inputs * 148; // P2PKH input size
        size += outputs * 34; // P2PKH output size
        size
    }

    /// Calculate estimated fee based on UTXO selection for the send amount
    fn estimate_fee(&self) -> Option<(u64, usize, usize)> {
        let wallet = self.selected_wallet.as_ref()?;
        let wallet_guard = wallet.read().ok()?;

        if wallet_guard.utxos.is_empty() {
            return None;
        }

        // Calculate total amount to send
        let total_output: u64 = self
            .recipients
            .iter()
            .filter_map(|r| parse_amount_to_duffs(&r.amount).ok())
            .sum();

        if total_output == 0 {
            // No valid amounts entered yet, show estimate for minimum tx
            let output_count = self.recipients.len().max(1) + 1;
            let estimated_size = Self::estimate_p2pkh_tx_size(1, output_count);
            let fee = FeeLevel::Normal.fee_rate().calculate_fee(estimated_size);
            return Some((fee, 1, estimated_size));
        }

        // Sort UTXOs by value descending to estimate how many we'd need
        let mut utxo_values: Vec<u64> = wallet_guard.utxos.values().map(|tx| tx.value).collect();
        utxo_values.sort_by(|a, b| b.cmp(a));

        let output_count = self.recipients.len() + 1; // +1 for change

        // Select UTXOs until we have enough (simulating the backend logic)
        let mut selected_count = 0;
        let mut selected_total: u64 = 0;

        for value in utxo_values {
            selected_count += 1;
            selected_total += value;

            // Recalculate fee with current input count
            let current_size = Self::estimate_p2pkh_tx_size(selected_count, output_count);
            let current_fee = FeeLevel::Normal.fee_rate().calculate_fee(current_size);

            if selected_total >= total_output + current_fee {
                return Some((current_fee, selected_count, current_size));
            }
        }

        // Not enough funds - show what we'd need with all UTXOs
        let estimated_size = Self::estimate_p2pkh_tx_size(selected_count, output_count);
        let fee = FeeLevel::Normal.fee_rate().calculate_fee(estimated_size);
        Some((fee, selected_count, estimated_size))
    }

    fn validate_and_send(&mut self) -> Result<AppAction, String> {
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or_else(|| "No wallet selected".to_string())?;

        // Check wallet is open
        {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            if !wallet_guard.is_open() {
                return Err("Wallet must be unlocked first".to_string());
            }
        }

        // Validate recipients
        if self.recipients.is_empty() {
            return Err("At least one recipient is required".to_string());
        }

        // Validate all recipients and build PaymentRecipient list
        let mut payment_recipients: Vec<PaymentRecipient> =
            Vec::with_capacity(self.recipients.len());
        let mut total_amount: u64 = 0;

        for (index, recipient) in self.recipients.iter().enumerate() {
            if recipient.address.trim().is_empty() {
                return Err(format!("Recipient {} has an empty address", index + 1));
            }
            let amount = parse_amount_to_duffs(&recipient.amount)
                .map_err(|e| format!("Recipient {}: {}", index + 1, e))?;
            if amount == 0 {
                return Err(format!("Recipient {} has zero amount", index + 1));
            }
            total_amount = total_amount.saturating_add(amount);

            payment_recipients.push(PaymentRecipient {
                address: recipient.address.trim().to_string(),
                amount_duffs: amount,
            });
        }

        // Check balance
        {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            if total_amount > wallet_guard.total_balance {
                return Err(format!(
                    "Insufficient balance. Need {} but only have {}",
                    format_dash(total_amount),
                    format_dash(wallet_guard.total_balance)
                ));
            }
        }

        let memo = self.memo.trim();
        let request = WalletPaymentRequest {
            recipients: payment_recipients,
            subtract_fee_from_amount: self.subtract_fee,
            memo: if memo.is_empty() {
                None
            } else {
                Some(memo.to_string())
            },
            override_fee: None,
        };

        // Store the request for potential retry if min relay fee is too low
        self.pending_request = Some(request.clone());

        self.sending = true;
        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendSingleKeyWalletPayment {
                wallet: wallet.clone(),
                request,
            },
        )))
    }

    fn render_recipients(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.add_space(15.0);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Recipients")
                    .color(DashColors::text_primary(dark_mode))
                    .strong()
                    .size(16.0),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new("+ Add Recipient").color(DashColors::DASH_BLUE))
                    .clicked()
                {
                    self.add_recipient();
                }
            });
        });

        ui.add_space(10.0);

        // Collect IDs to remove after the loop
        let mut to_remove: Option<usize> = None;
        let recipient_count = self.recipients.len();
        let show_remove = recipient_count > 1;

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                for i in 0..recipient_count {
                    let recipient_id = self.recipients[i].id;

                    // Address field
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Address {}:", i + 1))
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.add_space(5.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.recipients[i].address)
                                .hint_text(
                                    RichText::new("Enter Dash address (e.g., y...)")
                                        .color(Color32::GRAY),
                                )
                                .desired_width(600.0),
                        );

                        ui.add_space(5.0);

                        // Amount field
                        ui.label(
                            RichText::new(format!("Amount {} (DASH):", i + 1))
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.add_space(5.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.recipients[i].amount)
                                .hint_text(RichText::new("0.01").color(Color32::GRAY))
                                .desired_width(150.0),
                        );

                        ui.add_space(5.0);

                        if show_remove {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(
                                            RichText::new("Remove").color(DashColors::ERROR),
                                        )
                                        .clicked()
                                    {
                                        to_remove = Some(recipient_id);
                                    }
                                },
                            );
                        }
                    });

                    if let Some(error) = &self.recipients[i].error {
                        ui.add_space(5.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(error).color(DashColors::ERROR).size(12.0),
                            )
                            .wrap(),
                        );
                    }
                }
            });

        // Remove recipient if requested
        if let Some(id) = to_remove {
            self.remove_recipient(id);
        }
    }

    fn render_options(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.add_space(15.0);

        ui.label(
            RichText::new("Options")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(16.0),
        );

        ui.add_space(10.0);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                // Memo field
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Memo (optional):")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.add_space(5.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.memo)
                            .hint_text("Add a note...")
                            .desired_width(300.0),
                    );
                });

                ui.add_space(10.0);

                // Subtract fee checkbox
                ui.checkbox(
                    &mut self.subtract_fee,
                    RichText::new("Subtract fee from amount")
                        .color(DashColors::text_primary(dark_mode)),
                );

                // Fee estimation display
                if let Some((estimated_fee, utxo_count, tx_size)) = self.estimate_fee() {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Estimated fee:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} ({:.8} DASH)",
                                estimated_fee,
                                estimated_fee as f64 * 1e-8
                            ))
                            .color(DashColors::text_primary(dark_mode))
                            .size(14.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Transaction details:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(12.0),
                        );
                        ui.label(
                            RichText::new(format!("{} inputs, ~{} bytes", utxo_count, tx_size))
                                .color(DashColors::text_secondary(dark_mode))
                                .size(12.0),
                        );
                    });

                    if utxo_count > 100 {
                        ui.add_space(5.0);
                        ui.label(
                            RichText::new(
                                "Note: Large number of inputs may require higher network fee",
                            )
                            .color(DashColors::WARNING)
                            .size(12.0),
                        );
                    }
                }
            });
    }

    /// Render the simple (beginner) send UI - single recipient, minimal options
    fn render_simple_send(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.add_space(15.0);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                // Address field
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("To:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.add_space(5.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.recipients[0].address)
                            .hint_text(RichText::new("Enter Dash address").color(Color32::GRAY))
                            .desired_width(500.0),
                    );
                });

                ui.add_space(10.0);

                // Amount field
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Amount:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.add_space(5.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.recipients[0].amount)
                            .hint_text(RichText::new("0.00").color(Color32::GRAY))
                            .desired_width(150.0),
                    );
                    ui.label(
                        RichText::new("DASH")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                });

                // Simple fee display
                if let Some((estimated_fee, _, _)) = self.estimate_fee() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Fee:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(format!("~{:.8} DASH", estimated_fee as f64 * 1e-8))
                                .color(DashColors::text_primary(dark_mode))
                                .size(14.0),
                        );
                    });
                }
            });
    }

    fn render_wallet_info(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        if let Some(wallet_arc) = &self.selected_wallet
            && let Ok(wallet) = wallet_arc.read()
        {
            let alias = wallet
                .alias
                .clone()
                .unwrap_or_else(|| "Unnamed Wallet".to_string());
            let balance = wallet.total_balance;

            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Sending from:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(&alias)
                                .color(DashColors::text_primary(dark_mode))
                                .strong()
                                .size(14.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Address:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(wallet.address.to_string())
                                .color(DashColors::text_primary(dark_mode))
                                .size(14.0),
                        );
                    });

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Available balance:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(format_dash(balance))
                                .color(DashColors::SUCCESS)
                                .strong()
                                .size(14.0),
                        );
                    });
                });
        }
    }

    fn render_wallet_unlock(&mut self, ui: &mut Ui) -> AppAction {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Unlock Wallet")
                        .color(DashColors::text_primary(dark_mode))
                        .strong()
                        .size(14.0),
                );

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Password:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.add_space(5.0);

                    let password_field = if self.show_password {
                        egui::TextEdit::singleline(&mut self.wallet_password)
                    } else {
                        egui::TextEdit::singleline(&mut self.wallet_password).password(true)
                    };
                    ui.add(password_field.desired_width(200.0));

                    ui.checkbox(&mut self.show_password, "Show");

                    ui.add_space(10.0);

                    if ui.button("Unlock").clicked()
                        && let Some(wallet) = &self.selected_wallet
                    {
                        match wallet.write() {
                            Ok(mut wallet_guard) => {
                                match wallet_guard.open(&self.wallet_password) {
                                    Ok(_) => {
                                        self.error_message = None;
                                        self.wallet_password.zeroize();
                                    }
                                    Err(e) => {
                                        self.error_message =
                                            Some(format!("Failed to unlock: {}", e));
                                    }
                                }
                            }
                            Err(_) => {
                                self.error_message =
                                    Some("Wallet lock error, please try again".to_string());
                            }
                        }
                    }
                });

                if let Some(error) = &self.error_message {
                    ui.add_space(5.0);
                    ui.add(
                        egui::Label::new(RichText::new(error).color(DashColors::ERROR).size(12.0))
                            .wrap(),
                    );
                }
            });

        AppAction::None
    }

    fn render_send_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.add_space(20.0);

        ui.horizontal(|ui| {
            // Back button
            if ui.button("Cancel").clicked() {
                action = AppAction::PopScreen;
            }

            ui.add_space(20.0);

            // Send button
            let wallet_is_open = self
                .selected_wallet
                .as_ref()
                .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

            let send_button = egui::Button::new(
                RichText::new(if self.sending { "Sending..." } else { "Send" })
                    .color(Color32::WHITE)
                    .strong(),
            )
            .fill(if wallet_is_open && !self.sending {
                DashColors::DASH_BLUE
            } else {
                DashColors::DASH_BLUE.gamma_multiply(0.5)
            })
            .min_size(egui::vec2(120.0, 36.0));

            let button_enabled = wallet_is_open && !self.sending;
            if ui.add_enabled(button_enabled, send_button).clicked() {
                match self.validate_and_send() {
                    Ok(send_action) => {
                        action = send_action;
                    }
                    Err(e) => {
                        self.display_message(&e, MessageType::Error);
                    }
                }
            }
        });

        action
    }

    fn dismiss_message(&mut self) {
        self.message = None;
    }
}

impl Drop for SingleKeyWalletSendScreen {
    fn drop(&mut self) {
        self.wallet_password.zeroize();
    }
}

impl ScreenLike for SingleKeyWalletSendScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Wallets", AppAction::PopScreen), ("Send", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // Display messages at the top
            let mut should_dismiss = false;
            if let Some((message, message_type, _)) = &self.message {
                let message = message.clone();
                let message_color = match message_type {
                    MessageType::Error => Color32::from_rgb(255, 100, 100),
                    MessageType::Info => DashColors::text_primary(dark_mode),
                    MessageType::Success => Color32::DARK_GREEN,
                };

                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(message_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, message_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&message).color(message_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    should_dismiss = true;
                                }
                            });
                        });
                });
                ui.add_space(10.0);
            }
            if should_dismiss {
                self.dismiss_message();
            }

            egui::ScrollArea::vertical()
                .auto_shrink([true; 2])
                .show(ui, |ui| {
                    // Heading with Advanced Options checkbox
                    ui.horizontal(|ui| {
                        ui.heading(
                            RichText::new("Send Dash")
                                .color(DashColors::text_primary(dark_mode))
                                .size(24.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                        });
                    });

                    ui.add_space(15.0);

                    // Wallet info
                    self.render_wallet_info(ui);

                    ui.add_space(10.0);

                    // Wallet unlock if needed
                    let wallet_is_open = self
                        .selected_wallet
                        .as_ref()
                        .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

                    if !wallet_is_open {
                        inner_action |= self.render_wallet_unlock(ui);
                        ui.add_space(10.0);
                    }

                    if self.show_advanced_options {
                        // Advanced mode: multiple recipients, memo, subtract fee, detailed info
                        self.render_recipients(ui);
                        self.render_options(ui);
                    } else {
                        // Simple mode: single recipient, minimal UI
                        self.render_simple_send(ui);
                    }

                    // Send button
                    inner_action |= self.render_send_button(ui);
                });

            inner_action
        });

        // Render fee confirmation dialog (modal, on top of everything)
        if let Some(response) = self.fee_dialog.show(ctx) {
            match response {
                FeeConfirmationResponse::Confirmed { override_fee } => {
                    if let Some(mut request) = self.pending_request.take() {
                        request.override_fee = Some(override_fee);
                        if let Some(wallet) = &self.selected_wallet {
                            action |= AppAction::BackendTask(BackendTask::CoreTask(
                                CoreTask::SendSingleKeyWalletPayment {
                                    wallet: wallet.clone(),
                                    request,
                                },
                            ));
                        }
                    }
                }
                FeeConfirmationResponse::Canceled => {
                    self.pending_request = None;
                    self.sending = false;
                }
            }
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Check for success messages to reset sending state
        if message.contains("Sent") || message.contains("TxID") {
            self.sending = false;
            self.pending_request = None;
        }

        // Check for min relay fee error and show confirmation dialog
        if message_type == MessageType::Error
            && let Some(required_fee) = parse_min_relay_fee_error(message)
        {
            // Show the fee confirmation dialog instead of the error message
            let estimated_fee = self.estimate_fee().map(|(fee, _, _)| fee).unwrap_or(0);
            self.fee_dialog.open(estimated_fee, required_fee);
            // Keep sending state true until user confirms or cancels
            return;
        }

        self.message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::backend_task::BackendTaskSuccessResult,
    ) {
        self.sending = false;

        match backend_task_success_result {
            crate::backend_task::BackendTaskSuccessResult::WalletPayment {
                txid,
                recipients,
                total_amount,
            } => {
                let msg = if recipients.len() == 1 {
                    let (address, amount) = &recipients[0];
                    format!(
                        "Sent {} to {}\nTxID: {}",
                        format_dash(*amount),
                        address,
                        txid
                    )
                } else {
                    let recipient_list: String = recipients
                        .iter()
                        .map(|(addr, amt)| format!("  {} to {}", format_dash(*amt), addr))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Sent {} total to {} recipients:\n{}\nTxID: {}",
                        format_dash(total_amount),
                        recipients.len(),
                        recipient_list,
                        txid
                    )
                };
                self.display_message(&msg, MessageType::Success);

                // Clear the form after successful send
                self.recipients = vec![SendRecipient::new(0)];
                self.next_recipient_id = 1;
                self.memo.clear();
                self.subtract_fee = false;
            }
            _ => {
                // Ignore other results
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
