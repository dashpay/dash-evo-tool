mod advanced;

use advanced::{
    AdvancedOutput, AdvancedSourceType, CoreAddressInput, PlatformAddressInput, PlatformFeeStrategy,
};

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::platform_address_allocation::{
    MAX_PLATFORM_INPUTS, allocate_platform_addresses, allocate_platform_addresses_with_fee,
    estimate_address_funding_fee_from_transition, estimate_platform_fee,
    estimate_withdrawal_fee_from_transition,
};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::theme::DashColors;
use crate::ui::wallets::send_utils::{
    AddressType, detect_address_type, format_credits, format_dash,
};
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits};
use dash_sdk::dpp::identity::core_script::CoreScript;
use eframe::egui::{self, Context, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::fee_estimation::PlatformFeeEstimator;

/// Source selection for sending
#[derive(Debug, Clone, PartialEq)]
pub enum SourceSelection {
    /// Use Core wallet UTXOs
    CoreWallet,
    /// Use all Platform addresses (stores list of platform address, core address, and balance)
    PlatformAddresses(Vec<(PlatformAddress, Address, u64)>),
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

    // Unified send fields (simple mode)
    selected_source: Option<SourceSelection>,
    destination_address: String,
    amount: Option<Amount>,
    amount_input: Option<AmountInput>,

    // Advanced mode state
    show_advanced_options: bool,
    advanced_source_type: AdvancedSourceType,
    /// For Core source type: list of core address inputs
    core_inputs: Vec<CoreAddressInput>,
    /// For Platform source type: list of platform address inputs
    platform_inputs: Vec<PlatformAddressInput>,
    advanced_outputs: Vec<AdvancedOutput>,
    fee_strategy: PlatformFeeStrategy,

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
            amount: None,
            amount_input: None,
            show_advanced_options: false,
            advanced_source_type: AdvancedSourceType::Core,
            core_inputs: Vec::new(),
            platform_inputs: Vec::new(),
            advanced_outputs: vec![AdvancedOutput {
                address: String::new(),
                amount: String::new(),
            }],
            fee_strategy: PlatformFeeStrategy::default(),
            subtract_fee: false,
            send_status: SendStatus::NotStarted,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message: None,
        }
    }

    fn estimate_max_fee_for_platform_send(
        &self,
        fee_estimator: &PlatformFeeEstimator,
        addresses: &[(PlatformAddress, Address, u64)],
        destination: Option<&PlatformAddress>,
    ) -> u64 {
        let mut sorted_addresses: Vec<_> = addresses
            .iter()
            .filter(|(addr, _, _)| destination != Some(addr))
            .cloned()
            .collect();
        sorted_addresses.sort_by(|a, b| b.2.cmp(&a.2));

        let usable_count = sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
        if usable_count == 0 {
            return estimate_platform_fee(fee_estimator, 1);
        }

        let dest_type = detect_address_type(&self.destination_address);
        if dest_type == AddressType::Core {
            let output_script = self
                .destination_address
                .trim()
                .parse::<Address<NetworkUnchecked>>()
                .ok()
                .and_then(|addr| addr.require_network(self.app_context.network).ok())
                .map(|addr| CoreScript::new(addr.script_pubkey()));
            if let Some(output_script) = output_script {
                let max_fee_inputs: BTreeMap<PlatformAddress, u64> = sorted_addresses
                    .iter()
                    .take(usable_count)
                    .map(|(addr, _, _)| (*addr, 0))
                    .collect();
                return estimate_withdrawal_fee_from_transition(
                    self.app_context.platform_version(),
                    &max_fee_inputs,
                    &output_script,
                );
            }
        }

        estimate_platform_fee(fee_estimator, usable_count)
    }

    fn reset_form(&mut self) {
        self.destination_address.clear();
        self.amount = None;
        self.amount_input = None;
        self.selected_source = Some(SourceSelection::CoreWallet);
        self.advanced_source_type = AdvancedSourceType::Core;
        self.core_inputs.clear();
        self.platform_inputs.clear();
        self.advanced_outputs = vec![AdvancedOutput {
            address: String::new(),
            amount: String::new(),
        }];
        self.fee_strategy = PlatformFeeStrategy::default();
        self.send_status = SendStatus::NotStarted;
    }

    fn now_epoch_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn mark_sending(&mut self) {
        self.send_status = SendStatus::WaitingForResult(Self::now_epoch_secs());
    }

    fn min_output_amount(
        &self,
        input_type: AddressType,
        output_type: AddressType,
    ) -> Option<Credits> {
        let core_min = 5460_u64 * CREDITS_PER_DUFF;
        let platform_min = self
            .app_context
            .platform_version()
            .dpp
            .state_transitions
            .address_funds
            .min_output_amount;

        match (input_type, output_type) {
            (AddressType::Unknown, AddressType::Unknown) => None,
            (AddressType::Core, AddressType::Core) => Some(core_min),
            (AddressType::Platform, AddressType::Platform) => Some(platform_min),
            (AddressType::Core, AddressType::Platform) => Some(56000000), // needed for asset locks
            (AddressType::Platform, AddressType::Core) => Some(core_min.max(platform_min)),
            (AddressType::Unknown, AddressType::Core) => Some(core_min),
            (AddressType::Unknown, AddressType::Platform) => Some(platform_min),
            (AddressType::Core, AddressType::Unknown) => Some(core_min),
            (AddressType::Platform, AddressType::Unknown) => Some(platform_min),
        }
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

        // Filter to only addresses with positive balance, sort by canonical string, and return
        let mut result: Vec<_> = address_map
            .into_iter()
            .filter(|(_, (_, _, balance, _))| *balance > 0)
            .map(|(canonical_str, (addr, platform_addr, balance, _))| {
                (canonical_str, addr, platform_addr, balance)
            })
            .collect();

        // Sort by canonical address string for consistent ordering
        result.sort_by(|a, b| a.0.cmp(&b.0));

        result
            .into_iter()
            .map(|(_, addr, platform_addr, balance)| (addr, platform_addr, balance))
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

    /// Get Core addresses with their UTXO balances
    fn get_core_addresses(&self) -> Vec<(Address, u64)> {
        let Some(wallet_arc) = &self.selected_wallet else {
            return vec![];
        };
        let Ok(wallet) = wallet_arc.read() else {
            return vec![];
        };

        let mut addresses = wallet.utxos_by_address();
        // Sort by balance descending for better UX
        addresses.sort_by(|a, b| b.1.cmp(&a.1));
        addresses
    }

    /// Get description of transaction type based on source and destination
    fn get_transaction_type_description(&self) -> &'static str {
        let dest_type = detect_address_type(&self.destination_address);
        match (&self.selected_source, dest_type) {
            (Some(SourceSelection::CoreWallet), AddressType::Core) => "Core Transaction",
            (Some(SourceSelection::CoreWallet), AddressType::Platform) => "Fund Platform Address",
            (Some(SourceSelection::PlatformAddresses(_)), AddressType::Platform) => {
                "Platform Transfer"
            }
            (Some(SourceSelection::PlatformAddresses(_)), AddressType::Core) => "Withdraw to Core",
            _ => "Send",
        }
    }

    /// Validate and execute the send based on detected types
    fn validate_and_send(&mut self) -> Result<AppAction, String> {
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet selected")?;

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
        let dest_type = detect_address_type(&self.destination_address);
        if dest_type == AddressType::Unknown {
            return Err(
                "Invalid destination address. Use a Dash address (X.../y...) or Platform address (evo1.../tevo1...)"
                    .to_string(),
            );
        }

        // Validate amount
        let amount = self
            .amount
            .as_ref()
            .ok_or_else(|| "Please enter an amount".to_string())?;
        if amount.value() == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        drop(wallet_guard);

        // Route to appropriate handler based on source and destination types
        match (source.clone(), dest_type) {
            (SourceSelection::CoreWallet, AddressType::Core) => self.send_core_to_core(),
            (SourceSelection::CoreWallet, AddressType::Platform) => {
                self.send_core_to_platform(seed_hash)
            }
            (SourceSelection::PlatformAddresses(addresses), AddressType::Platform) => {
                self.send_platform_to_platform(seed_hash, addresses)
            }
            (SourceSelection::PlatformAddresses(addresses), AddressType::Core) => {
                self.send_platform_to_core(seed_hash, addresses, network)
            }
            _ => Err("Invalid source/destination combination".to_string()),
        }
    }

    fn send_core_to_core(&mut self) -> Result<AppAction, String> {
        let amount_duffs = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .dash_to_duffs()?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Check balance
        let balance = self.get_core_balance();
        if amount_duffs > balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_dash(amount_duffs),
                format_dash(balance)
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

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet,
                request: WalletPaymentRequest {
                    recipients: vec![recipient],
                    subtract_fee_from_amount: self.subtract_fee,
                    memo: None,
                    override_fee: None,
                },
            },
        )))
    }

    fn send_core_to_platform(&mut self, seed_hash: WalletSeedHash) -> Result<AppAction, String> {
        let amount_duffs = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .dash_to_duffs()?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Parse platform address
        let address_str = self.destination_address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map(|(addr, _)| addr)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        // Check balance; fees will be subtracted from amount
        let required = amount_duffs;
        let balance = self.get_core_balance();
        if required > balance {
            return Err(format!(
                "Insufficient balance. Need {} (including fee) but have {}",
                format_dash(required),
                format_dash(balance)
            ));
        }

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromWalletUtxos {
                seed_hash,
                amount: amount_duffs,
                destination,
                // In simple mode, default to deducting fees from output (current behavior)
                fee_deduct_from_output: true,
            },
        )))
    }

    fn send_platform_to_platform(
        &mut self,
        seed_hash: WalletSeedHash,
        addresses: Vec<(PlatformAddress, Address, u64)>,
    ) -> Result<AppAction, String> {
        // Amount in credits (Amount stores in credits for DASH with 11 decimal places)
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Get fee estimator with current network multiplier
        let fee_estimator = self.app_context.fee_estimator();

        // Calculate total balance across all platform addresses
        let total_balance: u64 = addresses.iter().map(|(_, _, balance)| *balance).sum();

        tracing::debug!(
            "Platform transfer: {} requested, {} total balance across {} addresses",
            format_credits(amount_credits),
            format_credits(total_balance),
            addresses.len()
        );

        if amount_credits > total_balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_credits(amount_credits),
                format_credits(total_balance)
            ));
        }

        // Parse destination platform address
        let address_str = self.destination_address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map(|(addr, _)| addr)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        // Allocate addresses using the helper function
        let allocation = allocate_platform_addresses(
            &fee_estimator,
            &addresses,
            amount_credits,
            Some(&destination),
        );

        if allocation.sorted_addresses.is_empty() {
            return Err(
                "Cannot send to your own address. The destination must be different from your source addresses."
                    .to_string(),
            );
        }

        // Check available balance after filtering out destination
        let available_balance: u64 = allocation.sorted_addresses.iter().map(|(_, _, b)| *b).sum();
        if amount_credits > available_balance {
            return Err(format!(
                "Insufficient balance from other addresses. Need {} but have {} (excluding destination address)",
                format_credits(amount_credits),
                format_credits(available_balance)
            ));
        }

        if allocation.shortfall > 0 {
            // Calculate the max we can send with MAX_PLATFORM_INPUTS addresses (minus fees)
            let addresses_available = allocation.sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
            let max_balance: u64 = allocation
                .sorted_addresses
                .iter()
                .take(MAX_PLATFORM_INPUTS)
                .map(|(_, _, b)| *b)
                .sum();
            let max_fee = self.estimate_max_fee_for_platform_send(
                &fee_estimator,
                &allocation.sorted_addresses,
                Some(&destination),
            );
            let max_sendable = max_balance.saturating_sub(max_fee);

            return Err(format!(
                "Requested amount {} exceeds maximum {} for a single transaction.\n\n\
                 Details:\n\
                 • You have {} addresses with a combined balance of {}\n\
                 • Protocol limit: {} input addresses per transaction\n\
                 • Estimated fee: {} (for {} inputs)\n\
                 • Shortfall: {}\n\n\
                 Try reducing the amount slightly to account for fees.",
                format_credits(amount_credits),
                format_credits(max_sendable),
                addresses_available,
                format_credits(max_balance),
                MAX_PLATFORM_INPUTS,
                format_credits(allocation.estimated_fee),
                allocation.inputs.len(),
                format_credits(allocation.shortfall)
            ));
        }

        let mut outputs = BTreeMap::new();
        outputs.insert(destination, amount_credits);

        // Log transfer summary
        let total_input: u64 = allocation.inputs.values().sum();
        tracing::debug!(
            "Platform transfer: {} inputs totaling {}, output {}, fee {} (payer idx {})",
            allocation.inputs.len(),
            format_credits(total_input),
            format_credits(amount_credits),
            format_credits(allocation.estimated_fee),
            allocation.fee_payer_index
        );

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::TransferPlatformCredits {
                seed_hash,
                inputs: allocation.inputs,
                outputs,
                fee_payer_index: allocation.fee_payer_index,
            },
        )))
    }

    fn send_platform_to_core(
        &mut self,
        seed_hash: WalletSeedHash,
        addresses: Vec<(PlatformAddress, Address, u64)>,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> Result<AppAction, String> {
        // Amount in credits
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Calculate total balance across all platform addresses
        let total_balance: u64 = addresses.iter().map(|(_, _, balance)| *balance).sum();

        tracing::debug!(
            "Platform withdrawal: {} requested, {} total balance across {} addresses",
            format_credits(amount_credits),
            format_credits(total_balance),
            addresses.len()
        );

        if amount_credits > total_balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_credits(amount_credits),
                format_credits(total_balance)
            ));
        }

        // Parse destination Core address
        let address_str = self.destination_address.trim();
        let dest_address: Address<NetworkUnchecked> = address_str
            .parse()
            .map_err(|e| format!("Invalid Core address: {}", e))?;
        let dest_address = dest_address
            .require_network(network)
            .map_err(|e| format!("Address network mismatch: {}", e))?;

        let output_script = CoreScript::new(dest_address.script_pubkey());

        let platform_version = self.app_context.platform_version();

        // Allocate addresses using state-transition-based fee estimation (no destination filter)
        let allocation =
            allocate_platform_addresses_with_fee(&addresses, amount_credits, None, |inputs| {
                estimate_withdrawal_fee_from_transition(platform_version, inputs, &output_script)
            });

        if allocation.shortfall > 0 {
            // Calculate the max we can send with MAX_PLATFORM_INPUTS addresses (minus fees)
            let addresses_available = allocation.sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
            let max_balance: u64 = allocation
                .sorted_addresses
                .iter()
                .take(MAX_PLATFORM_INPUTS)
                .map(|(_, _, b)| *b)
                .sum();
            let max_fee_inputs: BTreeMap<PlatformAddress, u64> = allocation
                .sorted_addresses
                .iter()
                .take(addresses_available)
                .map(|(addr, _, _)| (*addr, 0))
                .collect();
            let max_fee = estimate_withdrawal_fee_from_transition(
                platform_version,
                &max_fee_inputs,
                &output_script,
            );
            let max_sendable = max_balance.saturating_sub(max_fee);

            return Err(format!(
                "Requested withdrawal {} exceeds maximum {} for a single transaction.\n\n\
                 Details:\n\
                 • You have {} Platform addresses with a combined balance of {}\n\
                 • Protocol limit: {} input addresses per transaction\n\
                 • Estimated fee: {} (for {} inputs)\n\
                 • Shortfall: {}\n\n\
                 Try reducing the amount slightly to account for fees.",
                format_credits(amount_credits),
                format_credits(max_sendable),
                addresses_available,
                format_credits(max_balance),
                MAX_PLATFORM_INPUTS,
                format_credits(allocation.estimated_fee),
                allocation.inputs.len(),
                format_credits(allocation.shortfall)
            ));
        }

        // Log withdrawal summary
        let total_input: u64 = allocation.inputs.values().sum();
        tracing::debug!(
            "Platform withdrawal: {} inputs totaling {}, withdraw {}, fee {} (payer idx {})",
            allocation.inputs.len(),
            format_credits(total_input),
            format_credits(amount_credits),
            format_credits(allocation.estimated_fee),
            allocation.fee_payer_index
        );

        self.mark_sending();

        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::WithdrawFromPlatformAddress {
                seed_hash,
                inputs: allocation.inputs,
                output_script,
                core_fee_per_byte: 1,
                fee_payer_index: allocation.fee_payer_index,
            },
        )))
    }

    fn render_unlock_gate(&mut self, ui: &mut Ui) -> bool {
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        if wallet_is_open {
            return true;
        }

        let Some(wallet) = &self.selected_wallet else {
            return true;
        };

        if let Err(e) = try_open_wallet_no_password(wallet) {
            self.error_message = Some(e);
        }
        if wallet_needs_unlock(wallet) {
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Wallet is locked. Please unlock to continue.")
                        .color(egui::Color32::from_rgb(200, 150, 50)),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            if ui.button("Unlock Wallet").clicked() {
                self.wallet_unlock_popup.open();
            }
            ui.add_space(10.0);
            return false;
        }

        true
    }

    fn format_elapsed_time(start_time: u64) -> String {
        let elapsed_seconds = Self::now_epoch_secs().saturating_sub(start_time);
        if elapsed_seconds < 60 {
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
        }
    }

    fn render_send_status(&mut self, ui: &mut Ui, dark_mode: bool) -> Option<AppAction> {
        match self.send_status.clone() {
            SendStatus::Complete(message) => {
                let mut action = AppAction::None;
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
                        action = AppAction::PopScreenAndRefresh;
                    }

                    ui.add_space(100.0);
                });
                Some(action)
            }
            SendStatus::WaitingForResult(start_time) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.add(egui::Spinner::new().size(40.0));
                    ui.add_space(20.0);
                    ui.heading("Sending...");
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!(
                            "Time elapsed: {}",
                            Self::format_elapsed_time(start_time)
                        ))
                        .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.add_space(100.0);
                });
                Some(AppAction::None)
            }
            SendStatus::Error(error_msg) => {
                let mut dismiss = false;
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(Color32::from_rgb(255, 100, 100).gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(255, 100, 100)))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&error_msg)
                                            .color(Color32::from_rgb(255, 100, 100)),
                                    )
                                    .wrap(),
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
                None
            }
            SendStatus::NotStarted => None,
        }
    }

    fn render_unified_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Wallet info
        self.render_wallet_info(ui);

        // Wallet unlock if needed
        if !self.render_unlock_gate(ui) {
            return AppAction::None;
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

        // Platform source breakdown (shows which addresses will be used)
        self.render_platform_source_breakdown(ui);

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
                            RichText::new(format_dash(core_balance))
                                .color(DashColors::SUCCESS)
                                .strong(),
                        );
                    });
                });
            });

        // Platform addresses option (simplified - shows combined balance)
        let platform_addresses = self.get_platform_addresses();
        if !platform_addresses.is_empty() {
            ui.add_space(5.0);

            // Calculate total platform balance
            let total_platform_balance: u64 = platform_addresses.iter().map(|(_, _, b)| *b).sum();

            // Check if platform addresses are selected
            let is_platform_selected = matches!(
                &self.selected_source,
                Some(SourceSelection::PlatformAddresses(_))
            );

            Frame::group(ui.style())
                .fill(if is_platform_selected {
                    DashColors::DASH_BLUE.gamma_multiply(0.1)
                } else {
                    DashColors::surface(dark_mode)
                })
                .stroke(if is_platform_selected {
                    egui::Stroke::new(2.0, DashColors::DASH_BLUE)
                } else {
                    egui::Stroke::new(1.0, DashColors::border_light(dark_mode))
                })
                .inner_margin(Margin::symmetric(12, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut selected = is_platform_selected;
                        if ui.radio_value(&mut selected, true, "").changed() && selected {
                            // Select all platform addresses
                            let addresses_with_balances: Vec<_> = platform_addresses
                                .iter()
                                .map(|(core_addr, platform_addr, balance)| {
                                    (*platform_addr, core_addr.clone(), *balance)
                                })
                                .collect();
                            self.selected_source =
                                Some(SourceSelection::PlatformAddresses(addresses_with_balances));
                        }
                        ui.label(
                            RichText::new("Platform Addresses")
                                .color(DashColors::text_primary(dark_mode))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_credits(total_platform_balance))
                                    .color(DashColors::SUCCESS)
                                    .strong(),
                            );
                        });
                    });
                });
        }
    }

    fn render_destination_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let dest_type = detect_address_type(&self.destination_address);

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
                        .hint_text("Enter address (X.../y.../evo1.../tevo1...)")
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
        let fee_estimator = self.app_context.fee_estimator();

        ui.label(
            RichText::new("Amount")
                .color(DashColors::text_primary(dark_mode))
                .strong()
                .size(14.0),
        );

        ui.add_space(8.0);

        // Get max amount and hint based on source selection
        let (max_amount_credits, max_hint) = match &self.selected_source {
            Some(SourceSelection::CoreWallet) => {
                let max = self.selected_wallet.as_ref().and_then(|w| {
                    w.read()
                        .ok()
                        .map(|wallet| wallet.total_balance_duffs() * CREDITS_PER_DUFF) // duffs to credits
                });
                let dest_type = detect_address_type(&self.destination_address);
                let hint = if dest_type == AddressType::Platform {
                    let destination =
                        PlatformAddress::from_bech32m_string(self.destination_address.trim())
                            .map(|(addr, _)| addr)
                            .ok();
                    if let Some(destination) = destination {
                        let estimated_fee = estimate_address_funding_fee_from_transition(
                            self.app_context.platform_version(),
                            &destination,
                        );
                        // max = max.map(|amount| amount.saturating_sub(estimated_fee));
                        Some(format!(
                            "Estimated platform fee ~{} (deducted from amount)",
                            format_credits(estimated_fee)
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                (max, hint)
            }
            Some(SourceSelection::PlatformAddresses(addresses)) => {
                // Parse destination to exclude it from max calculation (can't send to yourself)
                let destination =
                    PlatformAddress::from_bech32m_string(self.destination_address.trim())
                        .map(|(addr, _)| addr)
                        .ok();

                // Filter out destination and sort by balance descending
                let mut sorted_addresses: Vec<_> = addresses
                    .iter()
                    .filter(|(addr, _, _)| destination.as_ref() != Some(addr))
                    .cloned()
                    .collect();
                sorted_addresses.sort_by(|a, b| b.2.cmp(&a.2));

                // Sum balances from top addresses, limited by MAX_PLATFORM_INPUTS.
                let total: u64 = sorted_addresses
                    .iter()
                    .take(MAX_PLATFORM_INPUTS)
                    .map(|(_, _, balance)| *balance)
                    .sum();
                let max_fee = self.estimate_max_fee_for_platform_send(
                    &fee_estimator,
                    &sorted_addresses,
                    destination.as_ref(),
                );

                // Build hint explaining the limit
                let hint = if sorted_addresses.len() > MAX_PLATFORM_INPUTS {
                    format!(
                        "Limited to {} input addresses per transaction, ~{} reserved for fees",
                        MAX_PLATFORM_INPUTS,
                        format_credits(max_fee)
                    )
                } else {
                    format!("~{} reserved for fees", format_credits(max_fee))
                };
                (Some(total.saturating_sub(max_fee)), Some(hint))
            }
            None => (None, None),
        };

        let input_type = match self.selected_source {
            Some(SourceSelection::CoreWallet) => AddressType::Core,
            Some(SourceSelection::PlatformAddresses(_)) => AddressType::Platform,
            None => AddressType::Unknown,
        };
        let output_type = detect_address_type(&self.destination_address);
        let min_amount = self.min_output_amount(input_type, output_type);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                let amount_input = self.amount_input.get_or_insert_with(|| {
                    AmountInput::new(Amount::new_dash(0.0))
                        .with_hint_text("Enter amount")
                        .with_max_button(true)
                        .with_desired_width(150.0)
                });

                // Update max/min amount and hint dynamically
                amount_input.set_max_amount(max_amount_credits);
                amount_input.set_max_exceeded_hint(max_hint);
                amount_input.set_min_amount(min_amount);

                let response = amount_input.show(ui);
                response.inner.update(&mut self.amount);

                // When Max is clicked for Core wallet, automatically enable subtract_fee
                // so the transaction fee is deducted from the amount instead of failing
                if response.inner.max_clicked
                    && matches!(self.selected_source, Some(SourceSelection::CoreWallet))
                {
                    self.subtract_fee = true;
                }
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

        // Show subtract fee checkbox for Core wallet to Core address transactions
        let dest_type = detect_address_type(&self.destination_address);
        if matches!(self.selected_source, Some(SourceSelection::CoreWallet))
            && dest_type == AddressType::Core
        {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.subtract_fee, "Subtract fee from amount");
                if self.subtract_fee {
                    ui.label(
                        RichText::new("(recipient receives amount minus fee)")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0)
                            .italics(),
                    );
                }
            });
        }
    }

    /// Renders a breakdown of which platform addresses will be used and how much from each.
    /// Uses the same allocation algorithm as the actual send logic.
    fn render_platform_source_breakdown(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let network = self.app_context.network;
        let fee_estimator = self.app_context.fee_estimator();

        // Only show for platform address sources with a valid amount
        let addresses = match &self.selected_source {
            Some(SourceSelection::PlatformAddresses(addrs)) if !addrs.is_empty() => addrs,
            _ => return,
        };

        let amount_credits = match self.amount.as_ref() {
            Some(a) if a.value() > 0 => a.value(),
            _ => return,
        };

        // Parse destination platform address (if valid) to exclude it from inputs
        let destination = PlatformAddress::from_bech32m_string(self.destination_address.trim())
            .map(|(addr, _)| addr)
            .ok();

        // Use the same allocation algorithm as the send logic, filtering out the destination
        let allocation = allocate_platform_addresses(
            &fee_estimator,
            addresses,
            amount_credits,
            destination.as_ref(),
        );

        if allocation.inputs.is_empty() {
            return;
        }

        let hit_limit = allocation.shortfall > 0;

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode).gamma_multiply(0.5))
            .inner_margin(Margin::symmetric(10, 8))
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Source breakdown:")
                        .color(DashColors::text_secondary(dark_mode))
                        .size(12.0),
                );
                ui.add_space(4.0);

                for (platform_addr, use_amount) in &allocation.inputs {
                    let addr_str = platform_addr.to_bech32m_string(network);
                    let short_addr = if addr_str.len() >= 18 {
                        format!("{}...{}", &addr_str[..12], &addr_str[addr_str.len() - 6..])
                    } else {
                        addr_str.clone()
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&short_addr)
                                .monospace()
                                .color(DashColors::text_primary(dark_mode))
                                .size(11.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_credits(*use_amount))
                                    .color(DashColors::SUCCESS)
                                    .size(11.0),
                            );
                        });
                    });
                }

                ui.add_space(4.0);

                if hit_limit {
                    // Determine if the shortfall is due to address limit or insufficient balance
                    let exceeds_address_limit =
                        allocation.sorted_addresses.len() > MAX_PLATFORM_INPUTS;
                    let warning_msg = if exceeds_address_limit {
                        format!(
                            "Warning: Amount requires more than {} addresses. \
                             Reduce amount or use multiple transactions.",
                            MAX_PLATFORM_INPUTS
                        )
                    } else {
                        "Warning: Amount exceeds available balance (including fees).".to_string()
                    };
                    ui.label(
                        RichText::new(warning_msg)
                            .color(DashColors::WARNING)
                            .size(10.0),
                    );
                    ui.add_space(2.0);
                }

                ui.label(
                    RichText::new(
                        "Use Advanced Options to customize which addresses to send from.",
                    )
                    .color(DashColors::text_secondary(dark_mode))
                    .italics()
                    .size(10.0),
                );
            });
    }

    fn render_send_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let wallet_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        let dest_type = detect_address_type(&self.destination_address);
        let has_destination = dest_type != AddressType::Unknown;
        let has_amount = self.amount.as_ref().map(|a| a.value() > 0).unwrap_or(false);
        let has_source = self.selected_source.is_some();

        let is_sending = matches!(self.send_status, SendStatus::WaitingForResult(_));
        let can_send = wallet_open && !is_sending && has_destination && has_amount && has_source;

        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                action = AppAction::PopScreen;
            }

            ui.add_space(10.0);

            let button_text = if is_sending {
                "Sending..."
            } else {
                self.get_transaction_type_description()
            };

            let send_button =
                egui::Button::new(RichText::new(button_text).color(Color32::WHITE).strong())
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

            if let Some(status_action) = self.render_send_status(ui, dark_mode) {
                return status_action;
            }

            egui::ScrollArea::vertical()
                .auto_shrink([true; 2])
                .show(ui, |ui| {
                    // Heading with advanced options checkbox
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

                    if self.show_advanced_options {
                        inner_action |= self.render_advanced_send(ui);
                    } else {
                        inner_action |= self.render_unified_send(ui);
                    }
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
            crate::backend_task::BackendTaskSuccessResult::Wallet(wallet_result) => {
                use crate::backend_task::wallet::WalletResult;
                match wallet_result {
                    WalletResult::Payment {
                        txid: _,
                        recipients,
                        total_amount,
                    } => {
                        let msg = if recipients.len() == 1 {
                            let (address, amount) = &recipients[0];
                            format!("Sent {} to {}", format_dash(*amount), address,)
                        } else {
                            format!(
                                "Sent {} to {} recipients",
                                format_dash(total_amount),
                                recipients.len(),
                            )
                        };
                        self.send_status = SendStatus::Complete(msg);
                    }
                    WalletResult::PlatformAddressFunded { .. } => {
                        self.send_status = SendStatus::Complete(
                            "Platform address funded successfully!".to_string(),
                        );
                    }
                    WalletResult::PlatformAddressWithdrawal { .. } => {
                        self.send_status =
                            SendStatus::Complete("Withdrawal initiated successfully!\n\nNote: It may take a few minutes for funds to appear on the Core chain.".to_string());
                    }
                    WalletResult::PlatformCreditsTransferred { .. } => {
                        self.send_status = SendStatus::Complete(
                            "Platform credits transferred successfully!".to_string(),
                        );
                    }
                    _ => {}
                }
            }
            crate::backend_task::BackendTaskSuccessResult::TransferredCredits(fee_result) => {
                let fee_info = format!(
                    "\n\nFee: Estimated {} • Actual {}",
                    format_credits_as_dash(fee_result.estimated_fee),
                    format_credits_as_dash(fee_result.actual_fee)
                );
                self.send_status =
                    SendStatus::Complete(format!("Credits transferred successfully!{}", fee_info));
            }
            _ => {
                // Ignore other results
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
