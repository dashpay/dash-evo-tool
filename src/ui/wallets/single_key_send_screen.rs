//! Single Key Wallet Send Screen

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::wallets::wallets_screen::SINGLE_KEY_SEND_UNAVAILABLE;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::fee::FeeRate;
use eframe::egui::{self, Context, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::sync::{Arc, RwLock};

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

/// State for the fee confirmation dialog shown when min relay fee is higher than estimated
#[derive(Debug, Clone, Default)]
struct FeeConfirmationDialog {
    is_open: bool,
    estimated_fee: u64,
    required_fee: u64,
    pending_request: Option<WalletPaymentRequest>,
}

pub struct SingleKeyWalletSendScreen {
    pub app_context: Arc<AppContext>,
    pub selected_wallet: Option<Arc<RwLock<SingleKeyWallet>>>,

    // Recipients (support multiple)
    recipients: Vec<SendRecipient>,
    next_recipient_id: usize,

    // State
    sending: bool,

    // Wallet unlock
    password_input: PasswordInput,

    // Fee confirmation dialog
    fee_dialog: FeeConfirmationDialog,

    // Advanced options toggle
    show_advanced_options: bool,

    /// States the single-key send limitation up front. Stored on the screen
    /// (rather than constructed fresh each frame) so the underlying tracing log
    /// fires once on entry instead of every repaint.
    send_unavailable_banner: MessageBanner,
}

impl SingleKeyWalletSendScreen {
    pub fn new(app_context: &Arc<AppContext>, wallet: Arc<RwLock<SingleKeyWallet>>) -> Self {
        Self {
            app_context: app_context.clone(),
            selected_wallet: Some(wallet),
            recipients: vec![SendRecipient::new(0)],
            next_recipient_id: 1,
            sending: false,
            password_input: PasswordInput::new().with_hint_text("Enter password"),
            fee_dialog: FeeConfirmationDialog::default(),
            show_advanced_options: false,
            send_unavailable_banner: MessageBanner::new(),
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

    fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
        let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
        amount.dash_to_duffs()
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
            .filter_map(|r| Self::parse_amount_to_duffs(&r.amount).ok())
            .sum();

        if total_output == 0 {
            // No valid amounts entered yet, show estimate for minimum tx
            let output_count = self.recipients.len().max(1) + 1;
            let estimated_size = Self::estimate_p2pkh_tx_size(1, output_count);
            let fee = FeeRate::normal().calculate_fee(estimated_size);
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
            let current_fee = FeeRate::normal().calculate_fee(current_size);

            if selected_total >= total_output + current_fee {
                return Some((current_fee, selected_count, current_size));
            }
        }

        // Not enough funds - show what we'd need with all UTXOs
        let estimated_size = Self::estimate_p2pkh_tx_size(selected_count, output_count);
        let fee = FeeRate::normal().calculate_fee(estimated_size);
        Some((fee, selected_count, estimated_size))
    }

    /// Parse the required fee from a "min relay fee not met" error message
    fn parse_min_relay_fee_error(error: &str) -> Option<u64> {
        // Error format: "min relay fee not met, X < Y"
        if error.contains("min relay fee not met") || error.contains("min relay fee") {
            // Try to find the pattern "X < Y" and extract Y
            if let Some(pos) = error.find('<') {
                let after_lt = &error[pos + 1..];
                // Extract the number after '<'
                let num_str: String = after_lt
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(required_fee) = num_str.parse::<u64>() {
                    return Some(required_fee);
                }
            }
        }
        None
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
            let recipient_number = index + 1;
            if recipient.address.trim().is_empty() {
                return Err(format!("Recipient {recipient_number} has an empty address"));
            }
            let amount = Self::parse_amount_to_duffs(&recipient.amount)
                .map_err(|error| format!("Recipient {recipient_number}: {error}"))?;
            if amount == 0 {
                return Err(format!("Recipient {recipient_number} has zero amount"));
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
                    "Insufficient balance. Need {needed} but only have {available}",
                    needed = format_duffs_as_dash(total_amount),
                    available = format_duffs_as_dash(wallet_guard.total_balance)
                ));
            }
        }

        let request = WalletPaymentRequest {
            recipients: payment_recipients,
            override_fee: None,
        };

        // Store the request for potential retry if min relay fee is too low
        self.fee_dialog.pending_request = Some(request.clone());
        // Store estimated fee for display in dialog
        if let Some((estimated_fee, _, _)) = self.estimate_fee() {
            self.fee_dialog.estimated_fee = estimated_fee;
        }

        Ok(self.dispatch_send(wallet.clone(), request))
    }

    /// The single dispatch point for a send: arms the busy flag as it hands the
    /// task off, so the flag cannot get out of step with what is in flight.
    /// Every terminal result clears it again (see `display_message`).
    fn dispatch_send(
        &mut self,
        wallet: Arc<RwLock<SingleKeyWallet>>,
        request: WalletPaymentRequest,
    ) -> AppAction {
        self.sending = true;
        AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendSingleKeyWalletPayment { wallet, request },
        ))
    }

    fn render_recipients(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

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
                    let recipient_number = i + 1;

                    // Address field
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Address {recipient_number}:"))
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
                            RichText::new(format!("Amount {recipient_number} (DASH):"))
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
                        ui.label(RichText::new(error).color(DashColors::ERROR).size(12.0));
                    }
                }
            });

        // Remove recipient if requested
        if let Some(id) = to_remove {
            self.remove_recipient(id);
        }
    }

    fn render_options(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

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
                                "{estimated_fee} ({fee_dash:.8} DASH)",
                                fee_dash = estimated_fee as f64 * 1e-8
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
                            RichText::new(format!("{utxo_count} inputs, ~{tx_size} bytes"))
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
        let dark_mode = ui.style().visuals.dark_mode;

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
                            RichText::new(format!(
                                "~{fee_dash:.8} DASH",
                                fee_dash = estimated_fee as f64 * 1e-8
                            ))
                            .color(DashColors::text_primary(dark_mode))
                            .size(14.0),
                        );
                    });
                }
            });
    }

    fn render_fee_confirmation_dialog(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        if !self.fee_dialog.is_open {
            return action;
        }

        let dark_mode = ctx.global_style().visuals.dark_mode;

        egui::Window::new("Fee Confirmation Required")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(10.0);

                ui.label(
                    RichText::new("The network requires a higher fee than estimated.")
                        .color(DashColors::text_primary(dark_mode))
                        .size(14.0),
                );

                ui.add_space(15.0);

                Frame::group(ui.style())
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(12, 10))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated fee:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{estimated_fee} duffs ({fee_dash:.8} DASH)",
                                    estimated_fee = self.fee_dialog.estimated_fee,
                                    fee_dash = self.fee_dialog.estimated_fee as f64 * 1e-8
                                ))
                                .color(DashColors::text_primary(dark_mode)),
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Required fee:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{required_fee} duffs ({fee_dash:.8} DASH)",
                                    required_fee = self.fee_dialog.required_fee,
                                    fee_dash = self.fee_dialog.required_fee as f64 * 1e-8
                                ))
                                .color(DashColors::WARNING)
                                .strong(),
                            );
                        });

                        let fee_diff = self
                            .fee_dialog
                            .required_fee
                            .saturating_sub(self.fee_dialog.estimated_fee);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Additional cost:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "+{fee_diff} duffs ({fee_dash:.8} DASH)",
                                    fee_dash = fee_diff as f64 * 1e-8
                                ))
                                .color(DashColors::text_primary(dark_mode)),
                            );
                        });
                    });

                ui.add_space(15.0);

                ui.label(
                    RichText::new("Would you like to proceed with the higher fee?")
                        .color(DashColors::text_primary(dark_mode)),
                );

                ui.add_space(15.0);

                ui.horizontal(|ui| {
                    if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked() {
                        self.fee_dialog.is_open = false;
                        self.fee_dialog.pending_request = None;
                    }

                    ui.add_space(20.0);

                    if ComponentStyles::add_primary_button(ui, "Confirm & Send").clicked() {
                        if let Some(mut request) = self.fee_dialog.pending_request.take() {
                            // Update the request to use the higher fee
                            request.override_fee = Some(self.fee_dialog.required_fee);

                            if let Some(wallet) = self.selected_wallet.clone() {
                                action = self.dispatch_send(wallet, request);
                            }
                        }
                        self.fee_dialog.is_open = false;
                    }
                });

                ui.add_space(10.0);
            });

        action
    }

    fn render_wallet_info(&self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

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
                            RichText::new(format_duffs_as_dash(balance))
                                .color(DashColors::SUCCESS)
                                .strong()
                                .size(14.0),
                        );
                    });
                });
        }
    }

    fn render_wallet_unlock(&mut self, ui: &mut Ui) -> AppAction {
        let dark_mode = ui.style().visuals.dark_mode;

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

                    self.password_input.show(ui);

                    ui.add_space(10.0);

                    if ui.button("Unlock").clicked()
                        && let Some(wallet) = &self.selected_wallet
                    {
                        // Verify the passphrase against the encrypted vault
                        // without opening the shared map entry: signing
                        // decrypts just-in-time, so no plaintext is re-parked.
                        let address = wallet.read().ok().map(|w| w.address.to_string());
                        let verify_result = match address {
                            Some(addr) => self
                                .app_context
                                .verify_single_key_passphrase(&addr, self.password_input.text()),
                            None => Err(TaskError::ImportedKeyNotFound),
                        };
                        match verify_result {
                            Ok(()) => {
                                self.password_input.clear();
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    "Password confirmed. This key is ready to use.",
                                    MessageType::Success,
                                );
                            }
                            Err(e) => {
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    e.to_string(),
                                    MessageType::Error,
                                )
                                .with_details(&e);
                            }
                        }
                    }
                });
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

            // `CoreTask::SendSingleKeyWalletPayment` refuses every single-key
            // send with `TaskError::SingleKeyWalletsUnsupported`, so the button
            // stays disabled until that task can build, sign and broadcast a
            // transaction. Mirrors the disabled Send in the wallets action bar;
            // left unstyled so egui's default disabled visuals apply.
            let send_label =
                RichText::new(if self.sending { "Sending..." } else { "Send" }).strong();
            let send_button = egui::Button::new(send_label).min_size(egui::vec2(120.0, 36.0));

            let response = ui
                .add_enabled(false, send_button)
                .on_disabled_hover_text(SINGLE_KEY_SEND_UNAVAILABLE);
            if response.clicked() {
                match self.validate_and_send() {
                    Ok(send_action) => {
                        action = send_action;
                    }
                    Err(e) => {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
                    }
                }
            }
        });

        action
    }
}

impl ScreenLike for SingleKeyWalletSendScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![("Wallets", AppAction::PopScreen), ("Send", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.style().visuals.dark_mode;

            // Message display is handled by the global MessageBanner.

            egui::ScrollArea::vertical()
                .auto_shrink([true; 2])
                .show(ui, |ui| {
                    // States the limitation up front, so the disabled Send below
                    // is never a surprise.
                    if !self.send_unavailable_banner.has_message() {
                        self.send_unavailable_banner
                            .set_message(SINGLE_KEY_SEND_UNAVAILABLE, MessageType::Warning)
                            .disable_auto_dismiss();
                    }
                    self.send_unavailable_banner.show(ui);
                    ui.add_space(10.0);

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
                        // Advanced mode: multiple recipients, subtract fee, detailed info
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
        action |= self.render_fee_confirmation_dialog(ctx);

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for
        // side-effects. Always clear sending — the task that armed it is done,
        // whatever it returned. A send refused by the backend (every single-key
        // send is, today) must not strand the button on "Sending...".
        self.sending = false;

        if matches!(message_type, MessageType::Error | MessageType::Warning)
            && let Some(required_fee) = Self::parse_min_relay_fee_error(message)
        {
            // The fee is the only recoverable send error: offer the higher fee
            // instead of the raw message. Confirming re-dispatches, which arms
            // the busy flag again.
            self.fee_dialog.required_fee = required_fee;
            self.fee_dialog.is_open = true;
        } else {
            self.fee_dialog.pending_request = None;
        }
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
                        "Sent {amount} to {address}\nTxID: {txid}",
                        amount = format_duffs_as_dash(*amount)
                    )
                } else {
                    let recipient_list: String = recipients
                        .iter()
                        .map(|(address, amount)| {
                            format!(
                                "  {amount} to {address}",
                                amount = format_duffs_as_dash(*amount)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Sent {total_amount} total to {recipient_count} recipients:\n{recipient_list}\nTxID: {txid}",
                        total_amount = format_duffs_as_dash(total_amount),
                        recipient_count = recipients.len()
                    )
                };
                MessageBanner::set_global(self.app_context.egui_ctx(), &msg, MessageType::Success);
                self.fee_dialog.pending_request = None;

                // Clear the form after successful send
                self.recipients = vec![SendRecipient::new(0)];
                self.next_recipient_id = 1;
            }
            _ => {
                // Ignore other results
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;

    /// Build an offline `AppContext` (no network I/O, throwaway data dir).
    fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        use crate::app_dir::ensure_env_file;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        (ctx, temp_dir)
    }

    fn send_screen() -> (SingleKeyWalletSendScreen, tempfile::TempDir) {
        let (ctx, temp_dir) = offline_ctx();
        let wallet =
            SingleKeyWallet::new([1u8; 32], Network::Testnet, None, None).expect("single key");
        let screen = SingleKeyWalletSendScreen::new(&ctx, Arc::new(RwLock::new(wallet)));
        (screen, temp_dir)
    }

    /// The busy flag means "a send is in flight". Every single-key send is
    /// refused by the backend with `SingleKeyWalletsUnsupported`, so a flag
    /// that only clears on success-shaped text would strand the button on
    /// "Sending..." forever.
    #[test]
    fn busy_flag_clears_on_the_refusal_every_single_key_send_produces() {
        let (mut screen, _tmp) = send_screen();
        screen.sending = true;

        screen.display_message(
            &TaskError::SingleKeyWalletsUnsupported.to_string(),
            MessageType::Error,
        );

        assert!(
            !screen.sending,
            "a refused send must not leave the screen stuck busy"
        );
    }

    /// The busy flag is armed by the dispatch itself, so a new call site cannot
    /// hand a task off without it.
    #[test]
    fn dispatching_a_send_arms_the_busy_flag() {
        let (mut screen, _tmp) = send_screen();
        let wallet = screen.selected_wallet.clone().expect("wallet");
        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: "yWxJqW5Kt1bnJoLtvxDrTBcpqhFuBCVFEK".to_string(),
                amount_duffs: 100_000,
            }],
            override_fee: None,
        };

        let action = screen.dispatch_send(wallet, request);

        assert!(screen.sending, "dispatch must arm the busy flag");
        assert!(matches!(
            action,
            AppAction::BackendTask(BackendTask::CoreTask(
                CoreTask::SendSingleKeyWalletPayment { .. }
            ))
        ));
    }

    /// The min-relay-fee dialog is the one recoverable send error: it keeps the
    /// stashed request so "Confirm & Send" can re-dispatch at the higher fee,
    /// but no task is in flight while the user decides.
    #[test]
    fn min_relay_fee_error_offers_the_retry_and_ends_the_in_flight_send() {
        let (mut screen, _tmp) = send_screen();
        screen.sending = true;
        screen.fee_dialog.pending_request = Some(WalletPaymentRequest {
            recipients: vec![],
            override_fee: None,
        });

        screen.display_message("min relay fee not met, 226 < 1000", MessageType::Error);

        assert!(
            !screen.sending,
            "no task is in flight while the dialog waits"
        );
        assert!(screen.fee_dialog.is_open, "the retry dialog must open");
        assert_eq!(screen.fee_dialog.required_fee, 1000);
        assert!(
            screen.fee_dialog.pending_request.is_some(),
            "the stashed request must survive for the higher-fee retry"
        );
    }
}
