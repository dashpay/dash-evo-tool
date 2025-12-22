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
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use eframe::egui::{self, Context, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Detected address type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressType {
    Core,
    Platform,
    Unknown,
}

/// Source selection for sending
#[derive(Debug, Clone, PartialEq)]
pub enum SourceSelection {
    /// Use Core wallet UTXOs
    CoreWallet,
    /// Use a specific Platform address (stores both platform address and original core address for lookup)
    PlatformAddress(PlatformAddress, Address),
}

/// Status of the send operation
#[derive(Debug, Clone, PartialEq)]
pub enum SendStatus {
    NotStarted,
    /// Waiting for result, stores the start time in seconds since epoch
    WaitingForResult(u64),
    /// Successfully completed with a success message
    Complete(String),
    /// Error occurred
    Error(String),
}

pub struct WalletSendScreen {
    pub app_context: Arc<AppContext>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    #[allow(dead_code)]
    selected_wallet_seed_hash: Option<WalletSeedHash>,

    // Unified send fields
    selected_source: Option<SourceSelection>,
    destination_address: String,
    amount: String,

    // Common options
    subtract_fee: bool,

    // State
    send_status: SendStatus,

    // Wallet unlock
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
}

impl WalletSendScreen {
    pub fn new(app_context: &Arc<AppContext>, wallet: Arc<RwLock<Wallet>>) -> Self {
        let seed_hash = wallet.read().ok().map(|w| w.seed_hash());
        Self {
            app_context: app_context.clone(),
            selected_wallet: Some(wallet),
            selected_wallet_seed_hash: seed_hash,
            selected_source: Some(SourceSelection::CoreWallet),
            destination_address: String::new(),
            amount: String::new(),
            subtract_fee: false,
            send_status: SendStatus::NotStarted,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message: None,
        }
    }

    fn reset_form(&mut self) {
        self.destination_address.clear();
        self.amount.clear();
        self.selected_source = Some(SourceSelection::CoreWallet);
        self.send_status = SendStatus::NotStarted;
    }

    fn format_dash(amount_duffs: u64) -> String {
        Amount::dash_from_duffs(amount_duffs).to_string()
    }

    fn format_credits(credits: Credits) -> String {
        let dash = credits as f64 / 1000.0 / 100_000_000.0;
        format!("{:.8} DASH", dash)
    }

    fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
        let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
        amount.dash_to_duffs()
    }

    fn parse_amount_to_credits(input: &str) -> Result<Credits, String> {
        let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
        let duffs = amount.dash_to_duffs()?;
        Ok(duffs as Credits * 1000)
    }

    /// Detect address type from the address string
    fn detect_address_type(&self, address: &str) -> AddressType {
        let trimmed = address.trim();
        if trimmed.is_empty() {
            return AddressType::Unknown;
        }

        // Check for Platform address (Bech32m format)
        if trimmed.starts_with("dashevo1") || trimmed.starts_with("tdashevo1") {
            return AddressType::Platform;
        }

        // Try to parse as Core address
        if trimmed.parse::<Address<NetworkUnchecked>>().is_ok() {
            return AddressType::Core;
        }

        AddressType::Unknown
    }

    /// Get available Platform addresses with balances
    /// Deduplicates addresses based on their canonical Bech32m string representation,
    /// preferring the entry with the highest nonce (most recent update)
    fn get_platform_addresses(&self) -> Vec<(Address, PlatformAddress, Credits)> {
        use std::collections::HashMap;

        let Some(wallet_arc) = &self.selected_wallet else {
            return vec![];
        };
        let Ok(wallet) = wallet_arc.read() else {
            return vec![];
        };

        let network = self.app_context.network;
        // Use HashMap to deduplicate by canonical address string
        // Store (core_addr, platform_addr, balance, nonce) and prefer higher nonce
        let mut address_map: HashMap<String, (Address, PlatformAddress, Credits, u32)> =
            HashMap::new();

        for (addr, info) in wallet.platform_address_info.iter() {
            if let Ok(platform_addr) = PlatformAddress::try_from(addr.clone()) {
                let canonical_str = platform_addr.to_bech32m_string(network);

                // Check if we already have this address
                let should_update = match address_map.get(&canonical_str) {
                    Some((_, _, _, existing_nonce)) => {
                        // Prefer the entry with higher nonce (more recent)
                        info.nonce >= *existing_nonce
                    }
                    None => true,
                };

                if should_update {
                    address_map.insert(
                        canonical_str,
                        (addr.clone(), platform_addr, info.balance, info.nonce),
                    );
                }
            }
        }

        // Filter to only addresses with positive balance and return
        address_map
            .into_values()
            .filter(|(_, _, balance, _)| *balance > 0)
            .map(|(addr, platform_addr, balance, _)| (addr, platform_addr, balance))
            .collect()
    }

    /// Get Core wallet balance
    fn get_core_balance(&self) -> u64 {
        self.selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok())
            .map(|w| w.confirmed_balance_duffs())
            .unwrap_or(0)
    }

    /// Get description of transaction type based on source and destination
    fn get_transaction_type_description(&self) -> &'static str {
        let dest_type = self.detect_address_type(&self.destination_address);
        match (&self.selected_source, dest_type) {
            (Some(SourceSelection::CoreWallet), AddressType::Core) => "Core Transaction",
            (Some(SourceSelection::CoreWallet), AddressType::Platform) => "Fund Platform Address",
            (Some(SourceSelection::PlatformAddress(_, _)), AddressType::Platform) => {
                "Platform Transfer"
            }
            (Some(SourceSelection::PlatformAddress(_, _)), AddressType::Core) => {
                "Withdraw to Core"
            }
            _ => "Send",
        }
    }

    /// Validate and execute the send based on detected types
    fn validate_and_send(&mut self) -> Result<AppAction, String> {
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or("No wallet selected")?;

        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

        if !wallet_guard.is_open() {
            return Err("Wallet must be unlocked first".to_string());
        }

        let seed_hash = wallet_guard.seed_hash();
        let network = self.app_context.network;

        // Validate source
        let source = self
            .selected_source
            .as_ref()
            .ok_or("Please select a source")?;

        // Validate destination
        let dest_type = self.detect_address_type(&self.destination_address);
        if dest_type == AddressType::Unknown {
            return Err(
                "Invalid destination address. Use a Dash address (X.../y...) or Platform address (dashevo1.../tdashevo1...)"
                    .to_string(),
            );
        }

        // Validate amount
        let amount_str = self.amount.trim();
        if amount_str.is_empty() {
            return Err("Please enter an amount".to_string());
        }

        drop(wallet_guard);

        // Route to appropriate handler based on source and destination types
        match (source.clone(), dest_type) {
            (SourceSelection::CoreWallet, AddressType::Core) => self.send_core_to_core(),
            (SourceSelection::CoreWallet, AddressType::Platform) => {
                self.send_core_to_platform(seed_hash)
            }
            (SourceSelection::PlatformAddress(platform_addr, core_addr), AddressType::Platform) => {
                self.send_platform_to_platform(seed_hash, platform_addr, core_addr)
            }
            (SourceSelection::PlatformAddress(platform_addr, core_addr), AddressType::Core) => {
                self.send_platform_to_core(seed_hash, platform_addr, core_addr, network)
            }
            _ => Err("Invalid source/destination combination".to_string()),
        }
    }

    fn send_core_to_core(&mut self) -> Result<AppAction, String> {
        let amount_duffs = Self::parse_amount_to_duffs(&self.amount)?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Check balance
        let balance = self.get_core_balance();
        if amount_duffs > balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                Self::format_dash(amount_duffs),
                Self::format_dash(balance)
            ));
        }

        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or("No wallet selected")?
            .clone();

        let recipient = PaymentRecipient {
            address: self.destination_address.trim().to_string(),
            amount_duffs,
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet,
                request: WalletPaymentRequest {
                    recipients: vec![recipient],
                    subtract_fee_from_amount: self.subtract_fee,
                    memo: None,
                },
            },
        )))
    }

    fn send_core_to_platform(&mut self, seed_hash: WalletSeedHash) -> Result<AppAction, String> {
        let amount_duffs = Self::parse_amount_to_duffs(&self.amount)?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Check balance (include fee for asset lock)
        let required = amount_duffs.saturating_add(3000);
        let balance = self.get_core_balance();
        if required > balance {
            return Err(format!(
                "Insufficient balance. Need {} (including fee) but have {}",
                Self::format_dash(required),
                Self::format_dash(balance)
            ));
        }

        // Parse platform address
        let address_str = self.destination_address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map(|(addr, _)| addr)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromWalletUtxos {
                seed_hash,
                amount: amount_duffs,
                destination,
            },
        )))
    }

    fn send_platform_to_platform(
        &mut self,
        seed_hash: WalletSeedHash,
        source_addr: PlatformAddress,
        source_core_addr: Address,
    ) -> Result<AppAction, String> {
        let amount_credits = Self::parse_amount_to_credits(&self.amount)?;
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Check balance using the original core address
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet")?;
        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

        let balance = wallet_guard
            .platform_address_info
            .get(&source_core_addr)
            .map(|info| info.balance)
            .unwrap_or(0);

        if amount_credits > balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                Self::format_credits(amount_credits),
                Self::format_credits(balance)
            ));
        }
        drop(wallet_guard);

        // Parse destination platform address
        let address_str = self.destination_address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map(|(addr, _)| addr)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        let mut inputs = BTreeMap::new();
        inputs.insert(source_addr, amount_credits);

        let mut outputs = BTreeMap::new();
        outputs.insert(destination, amount_credits);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::TransferPlatformCredits {
                seed_hash,
                inputs,
                outputs,
            },
        )))
    }

    fn send_platform_to_core(
        &mut self,
        seed_hash: WalletSeedHash,
        source_addr: PlatformAddress,
        source_core_addr: Address,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> Result<AppAction, String> {
        let amount_credits = Self::parse_amount_to_credits(&self.amount)?;
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Check balance using the original core address
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet")?;
        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

        let balance = wallet_guard
            .platform_address_info
            .get(&source_core_addr)
            .map(|info| info.balance)
            .unwrap_or(0);

        if amount_credits > balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                Self::format_credits(amount_credits),
                Self::format_credits(balance)
            ));
        }
        drop(wallet_guard);

        // Parse destination Core address
        let address_str = self.destination_address.trim();
        let dest_address: Address<NetworkUnchecked> = address_str
            .parse()
            .map_err(|e| format!("Invalid Core address: {}", e))?;
        let dest_address = dest_address
            .require_network(network)
            .map_err(|e| format!("Address network mismatch: {}", e))?;

        let output_script = CoreScript::new(dest_address.script_pubkey());

        let mut inputs = BTreeMap::new();
        inputs.insert(source_addr, amount_credits);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::WithdrawFromPlatformAddress {
                seed_hash,
                inputs,
                output_script,
                core_fee_per_byte: 1,
            },
        )))
    }

    fn render_unified_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Wallet info
        self.render_wallet_info(ui);

        // Wallet unlock if needed
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        if !wallet_is_open && let Some(wallet) = &self.selected_wallet {
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
                ui.add_space(10.0);
                return AppAction::None;
            }
        }

        ui.add_space(10.0);

        // Source selection
        self.render_source_selection(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Destination address
        self.render_destination_input(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Amount
        self.render_amount_input(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Send button
        action |= self.render_send_button(ui);

        action
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

            egui::Grid::new("wallet_info_grid")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Wallet:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.label(
                        RichText::new(&alias)
                            .color(DashColors::text_primary(dark_mode))
                            .strong()
                            .size(14.0),
                    );
                    ui.end_row();
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

            ui.add_space(10.0);
            ui.separator();
        }
    }

    fn render_source_selection(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.label(
            RichText::new("Send from")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(14.0),
        );

        ui.add_space(8.0);

        // Core wallet option
        let core_balance = self.get_core_balance();
        let is_core_selected = matches!(self.selected_source, Some(SourceSelection::CoreWallet));

        Frame::group(ui.style())
            .fill(if is_core_selected {
                DashColors::DASH_BLUE.gamma_multiply(0.1)
            } else {
                DashColors::surface(dark_mode)
            })
            .stroke(if is_core_selected {
                egui::Stroke::new(2.0, DashColors::DASH_BLUE)
            } else {
                egui::Stroke::new(1.0, DashColors::border_light(dark_mode))
            })
            .inner_margin(Margin::symmetric(12, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut selected = is_core_selected;
                    if ui.radio_value(&mut selected, true, "").changed() && selected {
                        self.selected_source = Some(SourceSelection::CoreWallet);
                    }
                    ui.label(
                        RichText::new("Core Wallet")
                            .color(DashColors::text_primary(dark_mode))
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(Self::format_dash(core_balance))
                                .color(DashColors::SUCCESS)
                                .strong(),
                        );
                    });
                });
            });

        // Platform address options
        let platform_addresses = self.get_platform_addresses();
        if !platform_addresses.is_empty() {
            ui.add_space(5.0);

            for (core_addr, platform_addr, balance) in &platform_addresses {
                let is_selected = matches!(
                    &self.selected_source,
                    Some(SourceSelection::PlatformAddress(addr, _)) if addr == platform_addr
                );

                let addr_str = platform_addr.to_bech32m_string(self.app_context.network);

                Frame::group(ui.style())
                    .fill(if is_selected {
                        DashColors::DASH_BLUE.gamma_multiply(0.1)
                    } else {
                        DashColors::surface(dark_mode)
                    })
                    .stroke(if is_selected {
                        egui::Stroke::new(2.0, DashColors::DASH_BLUE)
                    } else {
                        egui::Stroke::new(1.0, DashColors::border_light(dark_mode))
                    })
                    .inner_margin(Margin::symmetric(12, 8))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut selected = is_selected;
                            if ui.radio_value(&mut selected, true, "").changed() && selected {
                                self.selected_source =
                                    Some(SourceSelection::PlatformAddress(*platform_addr, core_addr.clone()));
                            }
                            ui.label(
                                RichText::new(&addr_str)
                                    .color(DashColors::text_primary(dark_mode))
                                    .monospace(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(Self::format_credits(*balance))
                                            .color(DashColors::SUCCESS)
                                            .strong(),
                                    );
                                },
                            );
                        });
                    });
            }
        }
    }

    fn render_destination_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let dest_type = self.detect_address_type(&self.destination_address);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Send to")
                    .color(DashColors::text_primary(dark_mode))
                    .strong()
                    .size(14.0),
            );

            // Show detected type
            if dest_type != AddressType::Unknown {
                ui.add_space(10.0);
                let (type_text, type_color) = match dest_type {
                    AddressType::Core => ("Core Address", DashColors::DASH_BLUE),
                    AddressType::Platform => ("Platform Address", Color32::from_rgb(130, 80, 220)),
                    AddressType::Unknown => ("", Color32::GRAY),
                };
                ui.label(
                    RichText::new(format!("({})", type_text))
                        .color(type_color)
                        .size(12.0),
                );
            }
        });

        ui.add_space(8.0);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.destination_address)
                        .hint_text("Enter address (X.../y.../dashevo1.../tdashevo1...)")
                        .desired_width(f32::INFINITY),
                );
            });

        // Show error for invalid address
        if !self.destination_address.trim().is_empty() && dest_type == AddressType::Unknown {
            ui.add_space(5.0);
            ui.label(
                RichText::new("Invalid address format")
                    .color(DashColors::ERROR)
                    .size(12.0),
            );
        }
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.label(
            RichText::new("Amount")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(14.0),
        );

        ui.add_space(8.0);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.amount)
                            .hint_text("0.0")
                            .desired_width(150.0),
                    );
                    ui.label(
                        RichText::new("DASH")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                });
            });

        // Show transaction type hint
        let tx_type = self.get_transaction_type_description();
        if tx_type != "Send" && !self.destination_address.trim().is_empty() {
            ui.add_space(5.0);
            ui.label(
                RichText::new(format!("Transaction type: {}", tx_type))
                    .color(DashColors::text_secondary(dark_mode))
                    .italics()
                    .size(12.0),
            );
        }
    }

    fn render_send_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let wallet_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        let dest_type = self.detect_address_type(&self.destination_address);
        let has_destination = dest_type != AddressType::Unknown;
        let has_amount = !self.amount.trim().is_empty();
        let has_source = self.selected_source.is_some();

        let is_sending = matches!(self.send_status, SendStatus::WaitingForResult(_));
        let can_send = wallet_open && !is_sending && has_destination && has_amount && has_source;

        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                action = AppAction::PopScreen;
            }

            ui.add_space(20.0);

            let button_text = if is_sending {
                "Sending..."
            } else {
                self.get_transaction_type_description()
            };

            let send_button = egui::Button::new(
                RichText::new(button_text)
                    .color(if can_send {
                        Color32::WHITE
                    } else {
                        Color32::WHITE.gamma_multiply(0.5)
                    })
                    .strong(),
            )
            .fill(if can_send {
                DashColors::DASH_BLUE
            } else {
                DashColors::DASH_BLUE.gamma_multiply(0.5)
            })
            .min_size(egui::vec2(160.0, 36.0));

            if ui.add_enabled(can_send, send_button).clicked() {
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

            // Handle different states - clone to avoid borrow issues
            let current_status = self.send_status.clone();
            match current_status {
                SendStatus::Complete(message) => {
                    // Show custom success screen
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.heading("🎉");
                        ui.heading(&message);
                        ui.add_space(20.0);

                        if ui.button("Send Another").clicked() {
                            self.reset_form();
                        }
                        ui.add_space(8.0);
                        if ui.button("Back to Wallet").clicked() {
                            inner_action = AppAction::PopScreenAndRefresh;
                        }

                        ui.add_space(100.0);
                    });

                    return inner_action;
                }
                SendStatus::WaitingForResult(start_time) => {
                    // Show sending spinner
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.add(egui::Spinner::new().size(40.0));
                        ui.add_space(20.0);
                        ui.heading("Sending...");

                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed_seconds = now.saturating_sub(start_time);

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
                                "{} minute{} {} second{}",
                                minutes,
                                if minutes == 1 { "" } else { "s" },
                                seconds,
                                if seconds == 1 { "" } else { "s" }
                            )
                        };

                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("Time elapsed: {}", display_time))
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        ui.add_space(100.0);
                    });
                    return inner_action;
                }
                SendStatus::Error(error_msg) => {
                    // Show error at the top
                    let mut dismiss = false;
                    ui.horizontal(|ui| {
                        Frame::new()
                            .fill(Color32::from_rgb(255, 100, 100).gamma_multiply(0.1))
                            .inner_margin(Margin::symmetric(10, 8))
                            .corner_radius(5.0)
                            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(255, 100, 100)))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&error_msg)
                                            .color(Color32::from_rgb(255, 100, 100)),
                                    );
                                    ui.add_space(10.0);
                                    if ui.small_button("Dismiss").clicked() {
                                        dismiss = true;
                                    }
                                });
                            });
                    });
                    if dismiss {
                        self.send_status = SendStatus::NotStarted;
                    }
                    ui.add_space(10.0);
                }
                SendStatus::NotStarted => {
                    // Normal flow - continue to render the form
                }
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

                    inner_action |= self.render_unified_send(ui);
                });

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

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Error => {
                self.send_status = SendStatus::Error(message.to_string());
            }
            MessageType::Success => {
                self.send_status = SendStatus::Complete(message.to_string());
            }
            MessageType::Info => {
                // Info messages don't change status
            }
        }
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::backend_task::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::backend_task::BackendTaskSuccessResult::WalletPayment {
                txid: _,
                recipients,
                total_amount,
            } => {
                let msg = if recipients.len() == 1 {
                    let (address, amount) = &recipients[0];
                    format!(
                        "Sent {} to {}",
                        Self::format_dash(*amount),
                        address,
                    )
                } else {
                    format!(
                        "Sent {} to {} recipients",
                        Self::format_dash(total_amount),
                        recipients.len(),
                    )
                };
                self.send_status = SendStatus::Complete(msg);
            }
            crate::backend_task::BackendTaskSuccessResult::TransferredCredits => {
                self.send_status = SendStatus::Complete("Credits transferred successfully!".to_string());
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformAddressFunded { .. } => {
                self.send_status = SendStatus::Complete("Platform address funded successfully!".to_string());
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformAddressWithdrawal { .. } => {
                self.send_status = SendStatus::Complete("Withdrawal initiated successfully!".to_string());
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformCreditsTransferred { .. } => {
                self.send_status = SendStatus::Complete("Platform credits transferred successfully!".to_string());
            }
            _ => {
                // Ignore other results
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
