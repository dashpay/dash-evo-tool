use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::theme::DashColors;
use crate::ui::wallets::send_utils::{
    AddressType, detect_address_type, format_credits, format_dash, parse_amount_to_credits,
    parse_amount_to_duffs,
};
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::identity::core_script::CoreScript;
use eframe::egui::{self, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use super::{SendStatus, WalletSendScreen};

/// Fee strategy for platform transfers
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PlatformFeeStrategy {
    /// Deduct fee from first input
    #[default]
    DeductFromFirstInput,
    /// Deduct fee from last input
    DeductFromLastInput,
    /// Reduce first output by fee amount
    ReduceFirstOutput,
    /// Reduce last output by fee amount
    ReduceLastOutput,
}

impl std::fmt::Display for PlatformFeeStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeductFromFirstInput => write!(f, "Deduct from first input"),
            Self::DeductFromLastInput => write!(f, "Deduct from last input"),
            Self::ReduceFirstOutput => write!(f, "Reduce first output"),
            Self::ReduceLastOutput => write!(f, "Reduce last output"),
        }
    }
}

/// Source type for advanced mode - Core or Platform
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvancedSourceType {
    Core,
    Platform,
}

impl std::fmt::Display for AdvancedSourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core => write!(f, "Core Wallet"),
            Self::Platform => write!(f, "Platform Addresses"),
        }
    }
}

/// A Core address input for advanced mode
#[derive(Debug, Clone)]
pub struct CoreAddressInput {
    /// The core address
    pub address: Address,
    /// Amount to send from this address (as string for input field)
    pub amount: String,
}

/// A Platform address input for advanced mode
#[derive(Debug, Clone)]
pub struct PlatformAddressInput {
    /// The platform address
    pub platform_address: PlatformAddress,
    /// The corresponding core address (for lookup/display)
    #[allow(dead_code)]
    pub core_address: Address,
    /// Amount to send from this address (as string for input field)
    pub amount: String,
}

/// An output for advanced mode (destination + amount)
#[derive(Debug, Clone)]
pub struct AdvancedOutput {
    /// Destination address string
    pub address: String,
    /// Amount to send to this address (as string for input field)
    pub amount: String,
}

impl WalletSendScreen {
    /// Render the advanced send UI with multiple inputs/outputs
    pub(super) fn render_advanced_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Wallet info
        self.render_wallet_info(ui);

        // Wallet unlock if needed
        if !self.render_unlock_gate(ui) {
            return AppAction::None;
        }

        ui.add_space(10.0);

        // ========== SOURCE TYPE SELECTION ==========
        ui.label(
            RichText::new("Source Type")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(16.0),
        );
        ui.add_space(5.0);
        ui.label(
            RichText::new("Select whether to send from Core wallet or Platform addresses")
                .color(DashColors::text_secondary(dark_mode))
                .size(12.0),
        );
        ui.add_space(8.0);

        // Source type radio buttons
        let platform_addresses = self.get_platform_addresses();
        let has_platform_addresses = !platform_addresses.is_empty();

        ui.horizontal(|ui| {
            if ui
                .radio_value(
                    &mut self.advanced_source_type,
                    AdvancedSourceType::Core,
                    "Core Wallet",
                )
                .changed()
            {
                // Clear inputs when switching to Core
                self.core_inputs.clear();
                self.platform_inputs.clear();
            }

            ui.add_enabled_ui(has_platform_addresses, |ui| {
                if ui
                    .radio_value(
                        &mut self.advanced_source_type,
                        AdvancedSourceType::Platform,
                        "Platform Addresses",
                    )
                    .changed()
                {
                    // Clear inputs when switching to Platform
                    self.core_inputs.clear();
                    self.platform_inputs.clear();
                }
            });

            if !has_platform_addresses {
                ui.label(
                    RichText::new("(no Platform addresses with balance)")
                        .color(DashColors::text_secondary(dark_mode))
                        .size(12.0)
                        .italics(),
                );
            }
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // ========== INPUTS SECTION ==========
        match self.advanced_source_type {
            AdvancedSourceType::Core => {
                self.render_core_inputs(ui);
            }
            AdvancedSourceType::Platform => {
                self.render_platform_inputs(ui);
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // ========== OUTPUTS SECTION ==========
        ui.label(
            RichText::new("Outputs (Send To)")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(16.0),
        );
        ui.add_space(5.0);

        // Show hint based on source type
        let hint = match self.advanced_source_type {
            AdvancedSourceType::Core => "Add Core or Platform destination addresses",
            AdvancedSourceType::Platform => "Add Platform or Core destination addresses",
        };
        ui.label(
            RichText::new(hint)
                .color(DashColors::text_secondary(dark_mode))
                .size(12.0),
        );
        ui.add_space(8.0);

        self.render_advanced_outputs(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // ========== FEE STRATEGY SECTION ==========
        // Only show for platform source or platform outputs
        let has_platform_output = self.advanced_outputs.iter().any(|o| {
            let addr_type = detect_address_type(&o.address);
            addr_type == AddressType::Platform
        });

        if self.advanced_source_type == AdvancedSourceType::Platform || has_platform_output {
            ui.label(
                RichText::new("Fee Strategy")
                    .color(DashColors::text_primary(dark_mode))
                    .strong()
                    .size(14.0),
            );
            ui.add_space(8.0);

            egui::ComboBox::from_id_salt("fee_strategy")
                .selected_text(format!("{}", self.fee_strategy))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.fee_strategy,
                        PlatformFeeStrategy::DeductFromFirstInput,
                        "Deduct from first input",
                    );
                    ui.selectable_value(
                        &mut self.fee_strategy,
                        PlatformFeeStrategy::DeductFromLastInput,
                        "Deduct from last input",
                    );
                    ui.selectable_value(
                        &mut self.fee_strategy,
                        PlatformFeeStrategy::ReduceFirstOutput,
                        "Reduce first output",
                    );
                    ui.selectable_value(
                        &mut self.fee_strategy,
                        PlatformFeeStrategy::ReduceLastOutput,
                        "Reduce last output",
                    );
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
        }

        // ========== SEND BUTTON ==========
        action |= self.render_advanced_send_button(ui);

        action
    }

    /// Render Core address inputs for advanced mode
    fn render_core_inputs(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut inputs_to_remove = Vec::new();

        ui.label(
            RichText::new("Core Address Inputs")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(14.0),
        );
        ui.add_space(5.0);
        ui.label(
            RichText::new("Select core addresses and amounts to send from each")
                .color(DashColors::text_secondary(dark_mode))
                .size(12.0),
        );
        ui.add_space(8.0);

        // Get available core addresses
        let core_addresses = self.get_core_addresses();

        // Collect already-used addresses
        let used_addresses: std::collections::HashSet<_> =
            self.core_inputs.iter().map(|i| i.address.clone()).collect();

        let num_inputs = self.core_inputs.len();
        for idx in 0..num_inputs {
            let input = &self.core_inputs[idx];
            let addr_str = input.address.to_string();

            // Find balance for this address
            let balance = core_addresses
                .iter()
                .find(|(a, _)| *a == input.address)
                .map(|(_, b)| *b)
                .unwrap_or(0);

            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&addr_str)
                                    .color(DashColors::text_primary(dark_mode))
                                    .monospace(),
                            );
                            ui.label(
                                RichText::new(format!("({})", format_dash(balance)))
                                    .color(DashColors::SUCCESS)
                                    .size(12.0),
                            );

                            // Remove button
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("x").clicked() {
                                        inputs_to_remove.push(idx);
                                    }
                                },
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label("Amount:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.core_inputs[idx].amount)
                                    .hint_text(RichText::new("0.0").color(Color32::GRAY))
                                    .desired_width(100.0),
                            );
                            ui.label(
                                RichText::new("DASH")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(12.0),
                            );
                        });
                    });
                });
            ui.add_space(5.0);
        }

        // Remove marked inputs
        for idx in inputs_to_remove.into_iter().rev() {
            self.core_inputs.remove(idx);
        }

        // Add input dropdown - only show addresses not already added
        let available_addresses: Vec<_> = core_addresses
            .iter()
            .filter(|(a, _)| !used_addresses.contains(a))
            .collect();

        if !available_addresses.is_empty() {
            egui::ComboBox::from_id_salt("add_core_input")
                .selected_text("+ Add Core Address")
                .show_ui(ui, |ui| {
                    for (address, balance) in available_addresses {
                        let addr_str = address.to_string();
                        let display = format!(
                            "{}... ({})",
                            &addr_str[..12.min(addr_str.len())],
                            format_dash(*balance)
                        );
                        if ui.selectable_label(false, display).clicked() {
                            self.core_inputs.push(CoreAddressInput {
                                address: address.clone(),
                                amount: String::new(),
                            });
                        }
                    }
                });
        } else if self.core_inputs.is_empty() {
            ui.label(
                RichText::new("No core addresses with balance available")
                    .color(DashColors::text_secondary(dark_mode))
                    .italics(),
            );
        }
    }

    /// Render Platform address inputs for advanced mode
    fn render_platform_inputs(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut inputs_to_remove = Vec::new();

        ui.label(
            RichText::new("Platform Address Inputs")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(14.0),
        );
        ui.add_space(5.0);
        ui.label(
            RichText::new("Select platform addresses and amounts to send from each")
                .color(DashColors::text_secondary(dark_mode))
                .size(12.0),
        );
        ui.add_space(8.0);

        // Get available platform addresses
        let platform_addresses = self.get_platform_addresses();
        let network = self.app_context.network;

        // Collect already-used addresses
        let used_addresses: std::collections::HashSet<_> = self
            .platform_inputs
            .iter()
            .map(|i| i.platform_address)
            .collect();

        let num_inputs = self.platform_inputs.len();
        for idx in 0..num_inputs {
            let input = &self.platform_inputs[idx];
            let addr_str = input.platform_address.to_bech32m_string(network);

            // Find balance for this address
            let balance = platform_addresses
                .iter()
                .find(|(_, pa, _)| *pa == input.platform_address)
                .map(|(_, _, b)| *b)
                .unwrap_or(0);

            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&addr_str)
                                    .color(DashColors::text_primary(dark_mode))
                                    .monospace(),
                            );
                            ui.label(
                                RichText::new(format!("({})", format_credits(balance)))
                                    .color(DashColors::SUCCESS)
                                    .size(12.0),
                            );

                            // Remove button
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("x").clicked() {
                                        inputs_to_remove.push(idx);
                                    }
                                },
                            );
                        });

                        ui.horizontal(|ui| {
                            ui.label("Amount:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.platform_inputs[idx].amount)
                                    .hint_text(RichText::new("0.0").color(Color32::GRAY))
                                    .desired_width(100.0),
                            );
                            ui.label(
                                RichText::new("DASH")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(12.0),
                            );
                        });
                    });
                });
            ui.add_space(5.0);
        }

        // Remove marked inputs
        for idx in inputs_to_remove.into_iter().rev() {
            self.platform_inputs.remove(idx);
        }

        // Add input dropdown - only show addresses not already added
        let available_addresses: Vec<_> = platform_addresses
            .iter()
            .filter(|(_, pa, _)| !used_addresses.contains(pa))
            .collect();

        if !available_addresses.is_empty() {
            egui::ComboBox::from_id_salt("add_platform_input")
                .selected_text("+ Add Platform Address")
                .show_ui(ui, |ui| {
                    for (core_addr, platform_addr, balance) in available_addresses {
                        let addr_str = platform_addr.to_bech32m_string(network);
                        let display = format!(
                            "{}... ({})",
                            &addr_str[..20.min(addr_str.len())],
                            format_credits(*balance)
                        );
                        if ui.selectable_label(false, display).clicked() {
                            self.platform_inputs.push(PlatformAddressInput {
                                platform_address: *platform_addr,
                                core_address: core_addr.clone(),
                                amount: String::new(),
                            });
                        }
                    }
                });
        } else if self.platform_inputs.is_empty() {
            ui.label(
                RichText::new("No platform addresses with balance available")
                    .color(DashColors::text_secondary(dark_mode))
                    .italics(),
            );
        }
    }

    /// Render the outputs section for advanced mode
    fn render_advanced_outputs(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut outputs_to_remove = Vec::new();
        let num_outputs = self.advanced_outputs.len();

        // Pre-compute address types to avoid borrow issues
        let addr_types: Vec<AddressType> = self
            .advanced_outputs
            .iter()
            .map(|o| detect_address_type(&o.address))
            .collect();

        for (idx, &addr_type) in addr_types.iter().enumerate() {
            Frame::group(ui.style())
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(12, 10))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("To:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.advanced_outputs[idx].address)
                                    .hint_text("Enter address (X.../y.../evo1.../tevo1...)")
                                    .desired_width(350.0),
                            );

                            // Show detected type
                            if addr_type != AddressType::Unknown {
                                let (type_text, type_color) = match addr_type {
                                    AddressType::Core => ("Core", DashColors::DASH_BLUE),
                                    AddressType::Platform => {
                                        ("Platform", DashColors::PLATFORM_PURPLE)
                                    }
                                    AddressType::Unknown => ("", Color32::GRAY),
                                };
                                ui.label(
                                    RichText::new(format!("({})", type_text))
                                        .color(type_color)
                                        .size(12.0),
                                );
                            }

                            ui.label("Amount:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.advanced_outputs[idx].amount)
                                    .hint_text(RichText::new("0.0").color(Color32::GRAY))
                                    .desired_width(100.0),
                            );
                            ui.label(
                                RichText::new("DASH")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(12.0),
                            );

                            // Remove button (only if more than one output)
                            if num_outputs > 1 {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.button("x").clicked() {
                                            outputs_to_remove.push(idx);
                                        }
                                    },
                                );
                            }
                        });
                    });
                });
            ui.add_space(5.0);
        }

        // Remove marked outputs
        for idx in outputs_to_remove.into_iter().rev() {
            self.advanced_outputs.remove(idx);
        }

        // Add output button
        if ui.button("+ Add Output").clicked() {
            self.advanced_outputs.push(AdvancedOutput {
                address: String::new(),
                amount: String::new(),
            });
        }
    }

    /// Render the send button for advanced mode
    fn render_advanced_send_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let wallet_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        let is_sending = matches!(self.send_status, SendStatus::WaitingForResult(_));

        // Check if we have valid inputs based on source type
        let has_valid_inputs = match self.advanced_source_type {
            AdvancedSourceType::Core => {
                !self.core_inputs.is_empty()
                    && self.core_inputs.iter().any(|i| !i.amount.trim().is_empty())
            }
            AdvancedSourceType::Platform => {
                !self.platform_inputs.is_empty()
                    && self
                        .platform_inputs
                        .iter()
                        .any(|i| !i.amount.trim().is_empty())
            }
        };

        let has_outputs = self
            .advanced_outputs
            .iter()
            .any(|o| !o.address.trim().is_empty() && !o.amount.trim().is_empty());

        let can_send = wallet_open && !is_sending && has_valid_inputs && has_outputs;

        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                action = AppAction::PopScreen;
            }

            ui.add_space(10.0);

            let button_text = if is_sending { "Sending..." } else { "Send" };

            let send_button =
                egui::Button::new(RichText::new(button_text).color(Color32::WHITE).strong())
                    .fill(if can_send {
                        DashColors::DASH_BLUE
                    } else {
                        DashColors::DASH_BLUE.gamma_multiply(0.5)
                    })
                    .min_size(egui::vec2(160.0, 36.0));

            if ui.add_enabled(can_send, send_button).clicked() {
                match self.validate_and_send_advanced() {
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

    /// Validate and execute advanced send
    fn validate_and_send_advanced(&mut self) -> Result<AppAction, String> {
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet selected")?;
        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

        if !wallet_guard.is_open() {
            return Err("Wallet must be unlocked first".to_string());
        }

        let seed_hash = wallet_guard.seed_hash();
        let network = self.app_context.network;

        // Validate outputs
        if self.advanced_outputs.is_empty() {
            return Err("Please add at least one output".to_string());
        }

        // Determine output types
        let output_types: Vec<AddressType> = self
            .advanced_outputs
            .iter()
            .map(|o| detect_address_type(&o.address))
            .collect();

        let has_core_output = output_types.contains(&AddressType::Core);
        let has_platform_output = output_types.contains(&AddressType::Platform);

        // Validate that we don't mix output types
        if has_core_output && has_platform_output {
            return Err(
                "Cannot mix Core and Platform address outputs in the same transaction".to_string(),
            );
        }

        drop(wallet_guard);

        // Route to appropriate handler based on source type and output type
        match self.advanced_source_type {
            AdvancedSourceType::Core => {
                if self.core_inputs.is_empty() {
                    return Err("Please add at least one Core address input".to_string());
                }

                if has_core_output {
                    self.send_advanced_core_to_core()
                } else if has_platform_output {
                    self.send_advanced_core_to_platform(seed_hash)
                } else {
                    Err("Invalid output address".to_string())
                }
            }
            AdvancedSourceType::Platform => {
                if self.platform_inputs.is_empty() {
                    return Err("Please add at least one Platform address input".to_string());
                }

                if has_platform_output {
                    self.send_advanced_platform_to_platform(seed_hash)
                } else if has_core_output {
                    self.send_advanced_platform_to_core(seed_hash, network)
                } else {
                    Err("Invalid output address".to_string())
                }
            }
        }
    }

    /// Advanced Core to Core send (multiple outputs)
    fn send_advanced_core_to_core(&mut self) -> Result<AppAction, String> {
        let wallet: Arc<RwLock<Wallet>> = self
            .selected_wallet
            .as_ref()
            .ok_or("No wallet selected")?
            .clone();

        // Parse inputs to get total available
        let mut total_input = 0u64;
        for input in &self.core_inputs {
            let amount_duffs = parse_amount_to_duffs(&input.amount)?;
            total_input = total_input.saturating_add(amount_duffs);
        }

        if total_input == 0 {
            return Err("Please specify amounts for the input addresses".to_string());
        }

        // Parse outputs
        let mut recipients = Vec::new();
        let mut total_output = 0u64;

        for output in &self.advanced_outputs {
            let amount_duffs = parse_amount_to_duffs(&output.amount)?;
            if amount_duffs == 0 {
                continue;
            }
            total_output = total_output.saturating_add(amount_duffs);
            recipients.push(PaymentRecipient {
                address: output.address.trim().to_string(),
                amount_duffs,
            });
        }

        if recipients.is_empty() {
            return Err("No valid outputs specified".to_string());
        }

        // Check that inputs cover outputs (with some margin for fees)
        if total_output > total_input {
            return Err(format!(
                "Insufficient input amount. Outputs total {} but inputs only {}",
                format_dash(total_output),
                format_dash(total_input)
            ));
        }

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet,
                request: WalletPaymentRequest {
                    recipients,
                    subtract_fee_from_amount: self.subtract_fee,
                    memo: None,
                    override_fee: None,
                },
            },
        )))
    }

    /// Advanced Core to Platform send
    fn send_advanced_core_to_platform(
        &mut self,
        seed_hash: WalletSeedHash,
    ) -> Result<AppAction, String> {
        // For now, only support single output for Core to Platform
        // The SDK's FundPlatformAddressFromWalletUtxos only supports a single destination
        if self.advanced_outputs.len() != 1 {
            return Err(
                "Core to Platform currently only supports a single destination".to_string(),
            );
        }

        // Validate core inputs have enough
        let mut total_input = 0u64;
        for input in &self.core_inputs {
            let amount_duffs = parse_amount_to_duffs(&input.amount)?;
            total_input = total_input.saturating_add(amount_duffs);
        }

        let output = &self.advanced_outputs[0];
        let amount_duffs = parse_amount_to_duffs(&output.amount)?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        if amount_duffs > total_input {
            return Err(format!(
                "Insufficient input amount. Output is {} but inputs only {}",
                format_dash(amount_duffs),
                format_dash(total_input)
            ));
        }

        // Parse platform address
        let address_str = output.address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map(|(addr, _)| addr)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        // Determine fee strategy based on user selection
        // DeductFromInput variants mean fees are paid from wallet (recipient gets exact amount)
        // ReduceOutput variants mean fees are deducted from output (recipient gets less)
        let fee_deduct_from_output = matches!(
            self.fee_strategy,
            PlatformFeeStrategy::ReduceFirstOutput | PlatformFeeStrategy::ReduceLastOutput
        );

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromWalletUtxos {
                seed_hash,
                amount: amount_duffs,
                destination,
                fee_deduct_from_output,
            },
        )))
    }

    /// Advanced Platform to Platform send
    fn send_advanced_platform_to_platform(
        &mut self,
        seed_hash: WalletSeedHash,
    ) -> Result<AppAction, String> {
        // Build inputs map from platform_inputs
        let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        for input in &self.platform_inputs {
            let credits = parse_amount_to_credits(&input.amount)?;
            if credits > 0 {
                *inputs.entry(input.platform_address).or_insert(0) += credits;
            }
        }

        if inputs.is_empty() {
            return Err("No valid Platform inputs specified".to_string());
        }

        // Build outputs map
        let mut outputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        for output in &self.advanced_outputs {
            let destination = PlatformAddress::from_bech32m_string(output.address.trim())
                .map(|(addr, _)| addr)
                .map_err(|e| format!("Invalid platform address: {}", e))?;
            let credits = parse_amount_to_credits(&output.amount)?;
            if credits > 0 {
                *outputs.entry(destination).or_insert(0) += credits;
            }
        }

        if outputs.is_empty() {
            return Err("No valid Platform outputs specified".to_string());
        }

        // Find the input with the highest amount to be the fee payer.
        // In advanced mode, user specifies amounts (we don't know balances), so we pick
        // the input with the largest contribution as fee payer.
        let fee_payer_index = inputs
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, amount))| *amount)
            .map(|(idx, _)| idx as u16)
            .unwrap_or(0);

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::TransferPlatformCredits {
                seed_hash,
                inputs,
                outputs,
                fee_payer_index,
            },
        )))
    }

    /// Advanced Platform to Core send (withdrawal)
    fn send_advanced_platform_to_core(
        &mut self,
        seed_hash: WalletSeedHash,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> Result<AppAction, String> {
        // For withdrawal, we only support a single Core output
        if self.advanced_outputs.len() != 1 {
            return Err("Withdrawal currently only supports a single Core destination".to_string());
        }

        // Build inputs map from platform_inputs
        let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        for input in &self.platform_inputs {
            let credits = parse_amount_to_credits(&input.amount)?;
            if credits > 0 {
                *inputs.entry(input.platform_address).or_insert(0) += credits;
            }
        }

        if inputs.is_empty() {
            return Err("No valid Platform inputs specified".to_string());
        }

        // Parse Core destination
        let output = &self.advanced_outputs[0];
        let address_str = output.address.trim();
        let dest_address: Address<NetworkUnchecked> = address_str
            .parse()
            .map_err(|e| format!("Invalid Core address: {}", e))?;
        let dest_address = dest_address
            .require_network(network)
            .map_err(|e| format!("Address network mismatch: {}", e))?;

        let output_script = CoreScript::new(dest_address.script_pubkey());

        // Find the input with the highest amount to be the fee payer.
        // In advanced mode, user specifies amounts (we don't know balances), so we pick
        // the input with the largest contribution as fee payer.
        let fee_payer_index = inputs
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, amount))| *amount)
            .map(|(idx, _)| idx as u16)
            .unwrap_or(0);

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::WithdrawFromPlatformAddress {
                seed_hash,
                inputs,
                output_script,
                core_fee_per_byte: 1,
                fee_payer_index,
            },
        )))
    }
}
