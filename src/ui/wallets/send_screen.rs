use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::fee_estimation::format_credits_as_dash;
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

/// Maximum number of platform address inputs allowed per state transition
const MAX_PLATFORM_INPUTS: usize = 16;

use crate::model::fee_estimation::PlatformFeeEstimator;

/// Estimated serialized bytes per input (address + signature/witness data)
const ESTIMATED_BYTES_PER_INPUT: usize = 225;

/// Calculate the estimated fee for a platform address funds transfer.
///
/// Uses PlatformFeeEstimator for base costs (input/output fees) plus storage fees.
fn estimate_platform_fee(input_count: usize) -> u64 {
    let estimator = PlatformFeeEstimator::new();
    let inputs = input_count.max(1);

    // Base fee from Platform's min fee structure
    // - 500,000 credits per input (address_funds_transfer_input_cost)
    // - 6,000,000 credits per output (address_funds_transfer_output_cost)
    let base_fee = estimator.estimate_address_funds_transfer(inputs, 1);

    // Add storage fees for serialized input bytes only
    // (outputs don't add significant serialization overhead)
    let estimated_bytes = inputs * ESTIMATED_BYTES_PER_INPUT;
    let storage_fee = estimator.estimate_storage_based_fee(estimated_bytes, inputs);

    // Total with 20% safety buffer
    let total = base_fee.saturating_add(storage_fee);
    total.saturating_add(total / 5)
}

/// Result of allocating platform addresses for a transfer.
#[derive(Debug, Clone)]
struct AddressAllocationResult {
    /// Map of platform address to amount to transfer from each
    inputs: BTreeMap<PlatformAddress, u64>,
    /// Index of the fee payer in BTreeMap iteration order
    fee_payer_index: u16,
    /// Estimated fee for this transaction
    estimated_fee: u64,
    /// Amount that couldn't be covered (0 if fully covered)
    shortfall: u64,
    /// Addresses sorted by balance descending (for UI display)
    sorted_addresses: Vec<(PlatformAddress, Address, u64)>,
}

/// Allocates platform addresses for a transfer, selecting which addresses to use
/// and how much from each.
///
/// Algorithm:
/// 1. Filters out the destination address (can't be both input and output)
/// 2. Sorts addresses by balance descending (largest first)
/// 3. The highest-balance address pays the fee
/// 4. Iteratively allocates until fee estimate converges
/// 5. Fee payer is always included in inputs (even with 0 contribution) so fee can be deducted
///
/// Returns the allocation result with inputs, fee payer index, and any shortfall.
fn allocate_platform_addresses(
    addresses: &[(PlatformAddress, Address, u64)],
    amount_credits: u64,
    destination: Option<&PlatformAddress>,
) -> AddressAllocationResult {
    // Filter out the destination address if provided (protocol doesn't allow same address as input and output)
    let filtered: Vec<_> = addresses
        .iter()
        .filter(|(platform_addr, _, _)| destination != Some(platform_addr))
        .cloned()
        .collect();

    // Sort addresses by balance descending so the largest balance is used first
    let mut sorted_addresses = filtered;
    sorted_addresses.sort_by(|a, b| b.2.cmp(&a.2));

    // The highest-balance address (first in sorted order) will pay the fee
    let fee_payer_addr = sorted_addresses.first().map(|(addr, _, _)| *addr);

    // Calculate fee based on expected number of inputs (use worst-case for safety)
    // This matches what the Max button calculation uses
    let max_inputs = sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
    let estimated_fee = estimate_platform_fee(max_inputs.max(1));

    // Allocate inputs = outputs (protocol requires equality).
    // Fee payer must reserve enough remaining balance to pay the fee separately.
    let mut inputs = BTreeMap::new();
    let mut remaining = amount_credits;

    for (idx, (platform_addr, _, balance)) in sorted_addresses.iter().enumerate() {
        if remaining == 0 || inputs.len() >= MAX_PLATFORM_INPUTS {
            break;
        }
        // Fee payer (idx 0, highest balance) must keep fee in reserve
        let is_fee_payer = idx == 0;
        let available = if is_fee_payer {
            balance.saturating_sub(estimated_fee)
        } else {
            *balance
        };
        let use_amount = remaining.min(available);
        // Fee payer must always be in inputs so fee can be deducted from their balance
        if use_amount > 0 || is_fee_payer {
            inputs.insert(*platform_addr, use_amount);
            remaining = remaining.saturating_sub(use_amount);
        }
    }

    // Calculate shortfall
    let total_allocated: u64 = inputs.values().sum();
    let shortfall = amount_credits.saturating_sub(total_allocated);

    // Find the index of the fee payer in BTreeMap order (required by backend)
    let fee_payer_index = fee_payer_addr
        .and_then(|payer| {
            inputs
                .keys()
                .enumerate()
                .find(|(_, addr)| **addr == payer)
                .map(|(idx, _)| idx as u16)
        })
        .unwrap_or(0);

    AddressAllocationResult {
        inputs,
        fee_payer_index,
        estimated_fee,
        shortfall,
        sorted_addresses,
    }
}

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
        if trimmed.starts_with("evo1") || trimmed.starts_with("tevo1") {
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
        let dest_type = self.detect_address_type(&self.destination_address);
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
        let dest_type = self.detect_address_type(&self.destination_address);
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

        // Calculate total balance across all platform addresses
        let total_balance: u64 = addresses.iter().map(|(_, _, balance)| *balance).sum();

        tracing::debug!(
            "Platform transfer: {} requested, {} total balance across {} addresses",
            Self::format_credits(amount_credits),
            Self::format_credits(total_balance),
            addresses.len()
        );

        if amount_credits > total_balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                Self::format_credits(amount_credits),
                Self::format_credits(total_balance)
            ));
        }

        // Parse destination platform address
        let address_str = self.destination_address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map(|(addr, _)| addr)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        // Allocate addresses using the helper function
        let allocation = allocate_platform_addresses(&addresses, amount_credits, Some(&destination));

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
                Self::format_credits(amount_credits),
                Self::format_credits(available_balance)
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
            let max_fee = estimate_platform_fee(MAX_PLATFORM_INPUTS);
            let max_sendable = max_balance.saturating_sub(max_fee);

            return Err(format!(
                "Requested amount {} exceeds maximum {} for a single transaction.\n\n\
                 Details:\n\
                 • You have {} addresses with a combined balance of {}\n\
                 • Protocol limit: {} input addresses per transaction\n\
                 • Estimated fee: {} (for {} inputs)\n\
                 • Shortfall: {}\n\n\
                 Try reducing the amount slightly to account for fees.",
                Self::format_credits(amount_credits),
                Self::format_credits(max_sendable),
                addresses_available,
                Self::format_credits(max_balance),
                MAX_PLATFORM_INPUTS,
                Self::format_credits(allocation.estimated_fee),
                allocation.inputs.len(),
                Self::format_credits(allocation.shortfall)
            ));
        }

        let mut outputs = BTreeMap::new();
        outputs.insert(destination, amount_credits);

        // Log transfer summary
        let total_input: u64 = allocation.inputs.values().sum();
        tracing::debug!(
            "Platform transfer: {} inputs totaling {}, output {}, fee {} (payer idx {})",
            allocation.inputs.len(),
            Self::format_credits(total_input),
            Self::format_credits(amount_credits),
            Self::format_credits(allocation.estimated_fee),
            allocation.fee_payer_index
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

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
            Self::format_credits(amount_credits),
            Self::format_credits(total_balance),
            addresses.len()
        );

        if amount_credits > total_balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                Self::format_credits(amount_credits),
                Self::format_credits(total_balance)
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

        // Allocate addresses using the helper function (no destination filter for withdrawals)
        let allocation = allocate_platform_addresses(&addresses, amount_credits, None);

        if allocation.shortfall > 0 {
            // Calculate the max we can send with MAX_PLATFORM_INPUTS addresses (minus fees)
            let addresses_available = allocation.sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
            let max_balance: u64 = allocation
                .sorted_addresses
                .iter()
                .take(MAX_PLATFORM_INPUTS)
                .map(|(_, _, b)| *b)
                .sum();
            let max_fee = estimate_platform_fee(MAX_PLATFORM_INPUTS);
            let max_sendable = max_balance.saturating_sub(max_fee);

            return Err(format!(
                "Requested withdrawal {} exceeds maximum {} for a single transaction.\n\n\
                 Details:\n\
                 • You have {} Platform addresses with a combined balance of {}\n\
                 • Protocol limit: {} input addresses per transaction\n\
                 • Estimated fee: {} (for {} inputs)\n\
                 • Shortfall: {}\n\n\
                 Try reducing the amount slightly to account for fees.",
                Self::format_credits(amount_credits),
                Self::format_credits(max_sendable),
                addresses_available,
                Self::format_credits(max_balance),
                MAX_PLATFORM_INPUTS,
                Self::format_credits(allocation.estimated_fee),
                allocation.inputs.len(),
                Self::format_credits(allocation.shortfall)
            ));
        }

        // Log withdrawal summary
        let total_input: u64 = allocation.inputs.values().sum();
        tracing::debug!(
            "Platform withdrawal: {} inputs totaling {}, withdraw {}, fee {} (payer idx {})",
            allocation.inputs.len(),
            Self::format_credits(total_input),
            Self::format_credits(amount_credits),
            Self::format_credits(allocation.estimated_fee),
            allocation.fee_payer_index
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

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
                            RichText::new(Self::format_dash(core_balance))
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
                                RichText::new(Self::format_credits(total_platform_balance))
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
                        .map(|wallet| wallet.total_balance_duffs() * 1000) // duffs to credits
                });
                (max, None)
            }
            Some(SourceSelection::PlatformAddresses(addresses)) => {
                // Sort by balance descending to use the largest balances first (same as send logic).
                let mut sorted_addresses = addresses.clone();
                sorted_addresses.sort_by(|a, b| b.2.cmp(&a.2));

                // Sum balances from top addresses, limited by MAX_PLATFORM_INPUTS.
                let usable_count = sorted_addresses.len().min(MAX_PLATFORM_INPUTS);
                let total: u64 = sorted_addresses
                    .iter()
                    .take(MAX_PLATFORM_INPUTS)
                    .map(|(_, _, balance)| *balance)
                    .sum();
                let max_fee = estimate_platform_fee(usable_count);

                // Build hint explaining the limit
                let hint = if addresses.len() > MAX_PLATFORM_INPUTS {
                    format!(
                        "Limited to {} input addresses per transaction, ~{} reserved for fees",
                        MAX_PLATFORM_INPUTS,
                        Self::format_credits(max_fee)
                    )
                } else {
                    format!("~{} reserved for fees", Self::format_credits(max_fee))
                };
                (Some(total.saturating_sub(max_fee)), Some(hint))
            }
            None => (None, None),
        };

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

                // Update max amount and hint dynamically
                amount_input.set_max_amount(max_amount_credits);
                amount_input.set_max_exceeded_hint(max_hint);

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
        let dest_type = self.detect_address_type(&self.destination_address);
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

    /// Renders a breakdown of which platform addresses will be used and how much from each
    /// Renders a breakdown of which platform addresses will be used and how much from each.
    /// Uses the same allocation algorithm as the actual send logic.
    fn render_platform_source_breakdown(&self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let network = self.app_context.network;

        // Only show for platform address sources with a valid amount
        let addresses = match &self.selected_source {
            Some(SourceSelection::PlatformAddresses(addrs)) if !addrs.is_empty() => addrs,
            _ => return,
        };

        let amount_credits = match self.amount.as_ref() {
            Some(a) if a.value() > 0 => a.value(),
            _ => return,
        };

        // Use the same allocation algorithm as the send logic (no destination filter for preview)
        let allocation = allocate_platform_addresses(addresses, amount_credits, None);

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
                    let short_addr =
                        format!("{}...{}", &addr_str[..12], &addr_str[addr_str.len() - 6..]);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&short_addr)
                                .monospace()
                                .color(DashColors::text_primary(dark_mode))
                                .size(11.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(Self::format_credits(*use_amount))
                                    .color(DashColors::SUCCESS)
                                    .size(11.0),
                            );
                        });
                    });
                }

                ui.add_space(4.0);

                if hit_limit {
                    ui.label(
                        RichText::new(format!(
                            "Warning: Amount requires more than {} addresses. \
                             Reduce amount or use multiple transactions.",
                            MAX_PLATFORM_INPUTS
                        ))
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

        let dest_type = self.detect_address_type(&self.destination_address);
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

    /// Render the advanced send UI with multiple inputs/outputs
    fn render_advanced_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

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
            let addr_type = Self::detect_address_type_static(&o.address);
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
                                RichText::new(format!("({})", Self::format_dash(balance)))
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
                            Self::format_dash(*balance)
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
                                RichText::new(format!("({})", Self::format_credits(balance)))
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
                            Self::format_credits(*balance)
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
            .map(|o| Self::detect_address_type_static(&o.address))
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
                                        ("Platform", Color32::from_rgb(130, 80, 220))
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

    /// Static version of detect_address_type that doesn't need self
    fn detect_address_type_static(address: &str) -> AddressType {
        let trimmed = address.trim();
        if trimmed.is_empty() {
            return AddressType::Unknown;
        }

        // Check for Platform address (Bech32m format)
        if trimmed.starts_with("evo1") || trimmed.starts_with("tevo1") {
            return AddressType::Platform;
        }

        // Try to parse as Core address
        if trimmed.parse::<Address<NetworkUnchecked>>().is_ok() {
            return AddressType::Core;
        }

        AddressType::Unknown
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
            .map(|o| Self::detect_address_type_static(&o.address))
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
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or("No wallet selected")?
            .clone();

        // Parse inputs to get total available
        let mut total_input = 0u64;
        for input in &self.core_inputs {
            let amount_duffs = Self::parse_amount_to_duffs(&input.amount)?;
            total_input = total_input.saturating_add(amount_duffs);
        }

        if total_input == 0 {
            return Err("Please specify amounts for the input addresses".to_string());
        }

        // Parse outputs
        let mut recipients = Vec::new();
        let mut total_output = 0u64;

        for output in &self.advanced_outputs {
            let amount_duffs = Self::parse_amount_to_duffs(&output.amount)?;
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
                Self::format_dash(total_output),
                Self::format_dash(total_input)
            ));
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.send_status = SendStatus::WaitingForResult(now);

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
            let amount_duffs = Self::parse_amount_to_duffs(&input.amount)?;
            total_input = total_input.saturating_add(amount_duffs);
        }

        let output = &self.advanced_outputs[0];
        let amount_duffs = Self::parse_amount_to_duffs(&output.amount)?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        if amount_duffs > total_input {
            return Err(format!(
                "Insufficient input amount. Output is {} but inputs only {}",
                Self::format_dash(amount_duffs),
                Self::format_dash(total_input)
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
            let credits = Self::parse_amount_to_credits(&input.amount)?;
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
            let credits = Self::parse_amount_to_credits(&output.amount)?;
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
            let credits = Self::parse_amount_to_credits(&input.amount)?;
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
                fee_payer_index,
            },
        )))
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
            crate::backend_task::BackendTaskSuccessResult::WalletPayment {
                txid: _,
                recipients,
                total_amount,
            } => {
                let msg = if recipients.len() == 1 {
                    let (address, amount) = &recipients[0];
                    format!("Sent {} to {}", Self::format_dash(*amount), address,)
                } else {
                    format!(
                        "Sent {} to {} recipients",
                        Self::format_dash(total_amount),
                        recipients.len(),
                    )
                };
                self.send_status = SendStatus::Complete(msg);
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
            crate::backend_task::BackendTaskSuccessResult::PlatformAddressFunded { .. } => {
                self.send_status =
                    SendStatus::Complete("Platform address funded successfully!".to_string());
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformAddressWithdrawal { .. } => {
                self.send_status =
                    SendStatus::Complete("Withdrawal initiated successfully!\n\nNote: It may take a few minutes for funds to appear on the Core chain.".to_string());
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformCreditsTransferred {
                ..
            } => {
                self.send_status =
                    SendStatus::Complete("Platform credits transferred successfully!".to_string());
            }
            _ => {
                // Ignore other results
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
