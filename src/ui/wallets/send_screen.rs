use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use eframe::egui::{self, Context, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Represents which mode the user is sending from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendMode {
    #[default]
    Core,
    Platform,
}

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

/// A single Platform recipient entry with address and amount
#[derive(Debug, Clone)]
pub struct PlatformSendRecipient {
    pub id: usize,
    pub address: String,
    pub amount: String,
    pub error: Option<String>,
}

impl PlatformSendRecipient {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            address: String::new(),
            amount: String::new(),
            error: None,
        }
    }
}

pub struct WalletSendScreen {
    pub app_context: Arc<AppContext>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    #[allow(dead_code)]
    selected_wallet_seed_hash: Option<WalletSeedHash>,

    // Mode selection
    send_mode: SendMode,

    // Recipients (for Core mode, we support multiple)
    recipients: Vec<SendRecipient>,
    next_recipient_id: usize,

    // Platform mode fields
    platform_source_addresses: Vec<(Address, PlatformAddress, Credits, String)>, // Added: manual amount input
    platform_recipients: Vec<PlatformSendRecipient>,
    next_platform_recipient_id: usize,
    platform_advanced_inputs: bool, // Show manual input amount configuration

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
}

impl WalletSendScreen {
    pub fn new(app_context: &Arc<AppContext>, wallet: Arc<RwLock<Wallet>>) -> Self {
        let seed_hash = wallet.read().ok().map(|w| w.seed_hash());
        Self {
            app_context: app_context.clone(),
            selected_wallet: Some(wallet),
            selected_wallet_seed_hash: seed_hash,
            send_mode: SendMode::Core,
            recipients: vec![SendRecipient::new(0)],
            next_recipient_id: 1,
            platform_source_addresses: Vec::new(),
            platform_recipients: vec![PlatformSendRecipient::new(0)],
            next_platform_recipient_id: 1,
            platform_advanced_inputs: false,
            subtract_fee: false,
            memo: String::new(),
            sending: false,
            message: None,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
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

    fn add_platform_recipient(&mut self) {
        let id = self.next_platform_recipient_id;
        self.next_platform_recipient_id += 1;
        self.platform_recipients.push(PlatformSendRecipient::new(id));
    }

    fn remove_platform_recipient(&mut self, id: usize) {
        if self.platform_recipients.len() > 1 {
            self.platform_recipients.retain(|r| r.id != id);
        }
    }

    fn format_dash(amount_duffs: u64) -> String {
        Amount::dash_from_duffs(amount_duffs).to_string()
    }

    fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
        let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
        amount.dash_to_duffs()
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
            let amount = Self::parse_amount_to_duffs(&recipient.amount)
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
            if total_amount > wallet_guard.confirmed_balance_duffs() {
                return Err(format!(
                    "Insufficient confirmed balance. Need {} but only have {}",
                    Self::format_dash(total_amount),
                    Self::format_dash(wallet_guard.confirmed_balance_duffs())
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
        };

        self.sending = true;
        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet: wallet.clone(),
                request,
            },
        )))
    }

    fn render_mode_selector(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Send from:")
                    .color(DashColors::text_primary(dark_mode))
                    .strong(),
            );

            ui.add_space(10.0);

            // Core button
            let core_selected = self.send_mode == SendMode::Core;
            let core_button = egui::Button::new(
                RichText::new("Core")
                    .color(if core_selected {
                        Color32::WHITE
                    } else {
                        DashColors::text_primary(dark_mode)
                    })
                    .strong(),
            )
            .fill(if core_selected {
                DashColors::DASH_BLUE
            } else {
                DashColors::surface(dark_mode)
            })
            .min_size(egui::vec2(80.0, 32.0));

            if ui.add(core_button).clicked() {
                self.send_mode = SendMode::Core;
            }

            ui.add_space(5.0);

            // Platform button
            let platform_selected = self.send_mode == SendMode::Platform;
            let platform_button =
                egui::Button::new(RichText::new("Platform").color(if platform_selected {
                    Color32::WHITE
                } else {
                    DashColors::text_secondary(dark_mode)
                }))
                .fill(if platform_selected {
                    DashColors::DASH_BLUE
                } else {
                    DashColors::surface(dark_mode)
                })
                .min_size(egui::vec2(80.0, 32.0));

            if ui.add(platform_button).clicked() {
                self.send_mode = SendMode::Platform;
            }
        });
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
                                .hint_text("Enter Dash address (e.g., y...)")
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
                                .hint_text("0.01")
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
            let balance = wallet.confirmed_balance_duffs();

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
                            RichText::new("Available balance:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(Self::format_dash(balance))
                                .color(DashColors::SUCCESS)
                                .strong()
                                .size(14.0),
                        );
                    });

                    if !wallet.is_open() {
                        ui.add_space(5.0);
                        ui.label(
                            RichText::new("Wallet is locked. Unlock below to send.")
                                .color(DashColors::text_secondary(dark_mode))
                                .italics()
                                .size(12.0),
                        );
                    }
                });
        }
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

    /// Get the list of Platform addresses with balances for the selected wallet
    fn get_platform_addresses(&self) -> Vec<(Address, PlatformAddress, Credits)> {
        let Some(wallet_arc) = &self.selected_wallet else {
            return vec![];
        };
        let Ok(wallet) = wallet_arc.read() else {
            return vec![];
        };

        let network = self.app_context.network;
        wallet
            .platform_addresses(network)
            .into_iter()
            .map(|(core_addr, platform_addr)| {
                let balance = wallet
                    .platform_address_info
                    .get(&core_addr)
                    .map(|info| info.balance)
                    .unwrap_or(0);
                (core_addr, platform_addr, balance)
            })
            .filter(|(_, _, balance)| *balance > 0)
            .collect()
    }

    /// Format credits as DASH equivalent
    fn format_credits(credits: Credits) -> String {
        // Credits are in a specific unit; 1 DASH = 100_000_000 duffs, 1 duff = 1000 credits
        let dash_equivalent = credits as f64 / 1000.0 / 100_000_000.0;
        format!("{:.8} DASH", dash_equivalent)
    }

    /// Parse amount string to credits
    fn parse_amount_to_credits(input: &str) -> Result<Credits, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Amount is required".to_string());
        }

        // Parse as DASH and convert to credits
        let dash_amount: f64 = trimmed
            .parse()
            .map_err(|_| "Invalid amount format".to_string())?;

        if dash_amount <= 0.0 {
            return Err("Amount must be positive".to_string());
        }

        // Convert DASH to credits: 1 DASH = 100_000_000 duffs, 1 duff = 1000 credits
        let credits = (dash_amount * 100_000_000.0 * 1000.0) as Credits;
        Ok(credits)
    }

    fn validate_and_send_platform(&mut self) -> Result<AppAction, String> {
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or_else(|| "No wallet selected".to_string())?;

        // Check wallet is open
        let seed_hash = {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            if !wallet_guard.is_open() {
                return Err("Wallet must be unlocked first".to_string());
            }
            wallet_guard.seed_hash()
        };

        // Validate source addresses
        if self.platform_source_addresses.is_empty() {
            return Err("Please select at least one source address".to_string());
        }

        // Calculate total available from selected sources
        let total_available: Credits = self
            .platform_source_addresses
            .iter()
            .map(|(_, _, balance, _)| *balance)
            .sum();

        // If advanced mode, calculate total input from manual amounts
        let total_manual_input: Option<Credits> = if self.platform_advanced_inputs {
            let mut total: Credits = 0;
            for (index, (_, _, balance, amount_str)) in
                self.platform_source_addresses.iter().enumerate()
            {
                if amount_str.trim().is_empty() {
                    continue; // Skip empty amounts (will use 0)
                }
                let amount = Self::parse_amount_to_credits(amount_str)
                    .map_err(|e| format!("Source {}: {}", index + 1, e))?;
                if amount > *balance {
                    return Err(format!(
                        "Source {}: Amount {} exceeds available balance {}",
                        index + 1,
                        Self::format_credits(amount),
                        Self::format_credits(*balance)
                    ));
                }
                total = total.saturating_add(amount);
            }
            Some(total)
        } else {
            None
        };

        // Validate recipients
        if self.platform_recipients.is_empty() {
            return Err("At least one recipient is required".to_string());
        }

        // Build outputs from recipients
        let mut outputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        let mut total_output: Credits = 0;

        for (index, recipient) in self.platform_recipients.iter().enumerate() {
            let dest_addr_str = recipient.address.trim();
            if dest_addr_str.is_empty() {
                return Err(format!("Recipient {} has an empty address", index + 1));
            }

            // Try to parse as Bech32m Platform address first (DIP-18 format: dashevo1.../tdashevo1...)
            // Fall back to standard base58 Dash address for backwards compatibility
            let dest_platform_addr = if dest_addr_str.starts_with("dashevo1")
                || dest_addr_str.starts_with("tdashevo1")
            {
                let (addr, _network) = PlatformAddress::from_bech32m_string(dest_addr_str)
                    .map_err(|e| format!("Recipient {}: Invalid Bech32m address: {}", index + 1, e))?;
                addr
            } else {
                // Parse as a standard Dash address, then convert to PlatformAddress
                let unchecked_addr: Address<NetworkUnchecked> = dest_addr_str
                    .parse()
                    .map_err(|e| format!("Recipient {}: Invalid address format: {}", index + 1, e))?;
                let dest_address = unchecked_addr.assume_checked();
                PlatformAddress::try_from(dest_address)
                    .map_err(|e| format!("Recipient {}: Invalid Platform address: {}", index + 1, e))?
            };

            // Parse amount
            let amount = Self::parse_amount_to_credits(&recipient.amount)
                .map_err(|e| format!("Recipient {}: {}", index + 1, e))?;

            if amount == 0 {
                return Err(format!("Recipient {} has zero amount", index + 1));
            }

            total_output = total_output.saturating_add(amount);

            // Add to outputs (if same address exists, amounts are combined)
            *outputs.entry(dest_platform_addr).or_insert(0) += amount;
        }

        // Build inputs from selected source addresses
        let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();

        if self.platform_advanced_inputs {
            // Advanced mode: use manual input amounts
            let total_input = total_manual_input.unwrap_or(0);
            if total_input < total_output {
                return Err(format!(
                    "Total input ({}) is less than total output ({})",
                    Self::format_credits(total_input),
                    Self::format_credits(total_output)
                ));
            }

            for (_, platform_addr, _, amount_str) in &self.platform_source_addresses {
                if amount_str.trim().is_empty() {
                    continue;
                }
                if let Ok(amount) = Self::parse_amount_to_credits(amount_str) {
                    if amount > 0 {
                        inputs.insert(*platform_addr, amount);
                    }
                }
            }
        } else {
            // Simple mode: auto-distribute output amount across sources
            if total_output > total_available {
                return Err(format!(
                    "Insufficient balance. Available: {}, Total requested: {}",
                    Self::format_credits(total_available),
                    Self::format_credits(total_output)
                ));
            }

            let mut remaining = total_output;
            for (_, platform_addr, balance, _) in &self.platform_source_addresses {
                if remaining == 0 {
                    break;
                }
                let amount_from_this_source = remaining.min(*balance);
                inputs.insert(*platform_addr, amount_from_this_source);
                remaining = remaining.saturating_sub(amount_from_this_source);
            }
        }

        self.sending = true;

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::TransferPlatformCredits {
                seed_hash,
                inputs,
                outputs,
            },
        )))
    }

    fn render_platform_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Wallet info
        self.render_wallet_info(ui);

        ui.add_space(10.0);

        // Wallet unlock if needed
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        if !wallet_is_open {
            self.render_wallet_unlock_if_needed(ui);
            ui.add_space(10.0);
        }

        // Platform addresses list
        let platform_addresses = self.get_platform_addresses();

        ui.add_space(15.0);

        // Calculate total selected balance
        let total_selected: Credits = self
            .platform_source_addresses
            .iter()
            .map(|(_, _, b, _)| *b)
            .sum();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Source Platform Addresses")
                    .color(DashColors::text_primary(dark_mode))
                    .strong()
                    .size(16.0),
            );
            if !self.platform_source_addresses.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "({} selected, Total: {})",
                        self.platform_source_addresses.len(),
                        Self::format_credits(total_selected)
                    ))
                    .color(DashColors::text_secondary(dark_mode))
                    .size(12.0),
                );
            }
        });
        ui.add_space(5.0);

        // Advanced mode checkbox
        ui.checkbox(
            &mut self.platform_advanced_inputs,
            RichText::new("Advanced: Configure input amounts manually")
                .color(DashColors::text_secondary(dark_mode))
                .size(12.0),
        );
        ui.add_space(5.0);

        if platform_addresses.is_empty() {
            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("No Platform addresses with balance found.")
                            .color(DashColors::text_secondary(dark_mode))
                            .italics(),
                    );
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Fund a Platform address first to send credits.")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                });
        } else {
            let advanced_mode = self.platform_advanced_inputs;
            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    let network = self.app_context.network;
                    for (core_addr, platform_addr, balance) in &platform_addresses {
                        let selected_idx = self
                            .platform_source_addresses
                            .iter()
                            .position(|(_, p, _, _)| p == platform_addr);
                        let is_selected = selected_idx.is_some();

                        // Display in DIP-18 Bech32m format
                        let addr_display = platform_addr.to_bech32m_string(network);

                        ui.horizontal(|ui| {
                            let label =
                                format!("{} - {}", addr_display, Self::format_credits(*balance));

                            let mut checked = is_selected;
                            if ui.checkbox(&mut checked, label).changed() {
                                if checked {
                                    // Add to selected with empty amount
                                    self.platform_source_addresses.push((
                                        core_addr.clone(),
                                        *platform_addr,
                                        *balance,
                                        String::new(),
                                    ));
                                } else {
                                    // Remove from selected
                                    self.platform_source_addresses
                                        .retain(|(_, p, _, _)| p != platform_addr);
                                }
                            }

                            // Show amount input in advanced mode for selected addresses
                            // Re-check the index after potential modification above
                            if advanced_mode {
                                if let Some(current_idx) = self
                                    .platform_source_addresses
                                    .iter()
                                    .position(|(_, p, _, _)| p == platform_addr)
                                {
                                    ui.add_space(10.0);
                                    ui.label(
                                        RichText::new("Amount:")
                                            .color(DashColors::text_secondary(dark_mode))
                                            .size(12.0),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.platform_source_addresses[current_idx].3,
                                        )
                                        .hint_text("DASH")
                                        .desired_width(100.0),
                                    );
                                    if ui.small_button("Max").clicked() {
                                        let max_dash = *balance as f64 / 1000.0 / 100_000_000.0;
                                        self.platform_source_addresses[current_idx].3 =
                                            format!("{:.8}", max_dash);
                                    }
                                }
                            }
                        });
                    }

                    // Show total input in advanced mode
                    if advanced_mode && !self.platform_source_addresses.is_empty() {
                        ui.add_space(10.0);
                        let mut total_input: Credits = 0;
                        for (_, _, _, amount_str) in &self.platform_source_addresses {
                            if let Ok(amount) = Self::parse_amount_to_credits(amount_str) {
                                total_input = total_input.saturating_add(amount);
                            }
                        }
                        ui.label(
                            RichText::new(format!(
                                "Total input: {}",
                                Self::format_credits(total_input)
                            ))
                            .color(DashColors::text_primary(dark_mode))
                            .strong()
                            .size(12.0),
                        );
                    }
                });
        }

        ui.add_space(15.0);

        // Recipients section (similar to Core mode)
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
                    self.add_platform_recipient();
                }
            });
        });

        ui.add_space(10.0);

        // Show available balance from selected sources
        if !self.platform_source_addresses.is_empty() {
            let total_available: Credits = self
                .platform_source_addresses
                .iter()
                .map(|(_, _, b, _)| *b)
                .sum();
            ui.label(
                RichText::new(format!("Available from selected sources: {}", Self::format_credits(total_available)))
                    .color(DashColors::text_secondary(dark_mode))
                    .size(12.0),
            );
            ui.add_space(5.0);
        }

        // Collect IDs to remove after the loop
        let mut to_remove: Option<usize> = None;
        let recipient_count = self.platform_recipients.len();
        let show_remove = recipient_count > 1;

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                for i in 0..recipient_count {
                    let recipient_id = self.platform_recipients[i].id;

                    // Address field
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Address {}:", i + 1))
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.add_space(5.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.platform_recipients[i].address)
                                .hint_text("Enter Platform address (dashevo1... or tdashevo1...)")
                                .desired_width(450.0),
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
                            egui::TextEdit::singleline(&mut self.platform_recipients[i].amount)
                                .hint_text("0.01")
                                .desired_width(120.0),
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

                    if let Some(error) = &self.platform_recipients[i].error {
                        ui.add_space(5.0);
                        ui.label(RichText::new(error).color(DashColors::ERROR).size(12.0));
                    }

                    if i < recipient_count - 1 {
                        ui.add_space(5.0);
                    }
                }
            });

        // Remove recipient if requested
        if let Some(id) = to_remove {
            self.remove_platform_recipient(id);
        }

        // Send button
        ui.add_space(20.0);

        ui.horizontal(|ui| {
            // Back button
            if ui.button("Cancel").clicked() {
                action = AppAction::PopScreen;
            }

            ui.add_space(20.0);

            // Check if any recipient has data
            let has_recipient_data = self
                .platform_recipients
                .iter()
                .any(|r| !r.address.is_empty() && !r.amount.is_empty());

            // Send button
            let can_send = wallet_is_open
                && !self.sending
                && !self.platform_source_addresses.is_empty()
                && has_recipient_data;

            let send_button = egui::Button::new(
                RichText::new(if self.sending {
                    "Sending..."
                } else {
                    "Send Credits"
                })
                .color(Color32::WHITE)
                .strong(),
            )
            .fill(if can_send {
                DashColors::DASH_BLUE
            } else {
                DashColors::DASH_BLUE.gamma_multiply(0.5)
            })
            .min_size(egui::vec2(120.0, 36.0));

            if ui.add_enabled(can_send, send_button).clicked() {
                match self.validate_and_send_platform() {
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

    fn render_core_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Wallet info
        self.render_wallet_info(ui);

        ui.add_space(10.0);

        // Wallet unlock if needed
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        if !wallet_is_open {
            self.render_wallet_unlock_if_needed(ui);
            ui.add_space(10.0);
        }

        // Recipients
        self.render_recipients(ui);

        // Options (memo, subtract fee)
        self.render_options(ui);

        // Send button
        action |= self.render_send_button(ui);

        action
    }

    fn dismiss_message(&mut self) {
        self.message = None;
    }
}

impl ScreenLike for WalletSendScreen {
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
                    ui.heading(
                        RichText::new("Send Dash")
                            .color(DashColors::text_primary(dark_mode))
                            .size(24.0),
                    );

                    ui.add_space(15.0);

                    // Mode selector (Core vs Platform)
                    self.render_mode_selector(ui);

                    ui.add_space(10.0);
                    ui.separator();

                    // Render based on mode
                    match self.send_mode {
                        SendMode::Core => {
                            inner_action |= self.render_core_send(ui);
                        }
                        SendMode::Platform => {
                            inner_action |= self.render_platform_send(ui);
                        }
                    }
                });

            inner_action
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Check for success messages to reset sending state
        if message.contains("Sent") || message.contains("TxID") {
            self.sending = false;
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
                        Self::format_dash(*amount),
                        address,
                        txid
                    )
                } else {
                    let recipient_list: String = recipients
                        .iter()
                        .map(|(addr, amt)| format!("  {} to {}", Self::format_dash(*amt), addr))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "Sent {} total to {} recipients:\n{}\nTxID: {}",
                        Self::format_dash(total_amount),
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
            crate::backend_task::BackendTaskSuccessResult::PlatformCreditsTransferred {
                ..
            } => {
                self.display_message(
                    "Platform credits transferred successfully!",
                    MessageType::Success,
                );

                // Clear the Platform form
                self.platform_source_addresses.clear();
                self.platform_recipients = vec![PlatformSendRecipient::new(0)];
                self.next_platform_recipient_id = 1;
                self.platform_advanced_inputs = false;
            }
            _ => {
                self.display_message("Operation completed", MessageType::Success);
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}

impl ScreenWithWalletUnlock for WalletSendScreen {
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

    fn app_context(&self) -> Arc<AppContext> {
        self.app_context.clone()
    }
}
