use crate::app::AppAction;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::fee_estimation::{
    MAX_PLATFORM_INPUTS, PlatformFeeEstimator, allocate_platform_addresses,
    allocate_platform_addresses_with_fee, core_max_send_amount_duffs, core_max_send_reserve_duffs,
    estimate_address_funding_fee_from_transition, estimate_core_l1_send_fee_duffs,
    estimate_platform_fee, estimate_withdrawal_fee_from_transition, format_credits_as_dash,
    format_duffs_as_dash, shield_from_balance_fee_headroom,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::user_role::UserRole;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::address_input::{AddressInput, WalletWithSnapshot};
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::Error as SdkError;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::{CREDITS_PER_DUFF, Credits};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::core_script::CoreScript;
use eframe::egui::{self, Context, RichText, Ui};
use egui::{Color32, Frame, Margin};
use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};

/// Source selection for sending
#[derive(Debug, Clone, PartialEq)]
pub enum SourceSelection {
    /// Use Core wallet UTXOs
    CoreWallet,
    /// Use all Platform addresses (stores list of platform address, core address, and balance)
    PlatformAddresses(Vec<(PlatformAddress, Address, u64)>),
    /// Use an identity's credit balance
    Identity(Box<QualifiedIdentity>),
    /// Use shielded pool balance (stores seed_hash and balance in credits)
    Shielded(WalletSeedHash, u64),
}

/// Optional preset that opens the send screen pre-configured for one of the
/// shielded flows launched from the Shielded tab.
///
/// [`SendFlow::General`] is the full free-form send screen (any source, any
/// destination). The other variants lock the source — and, for
/// [`SendFlow::Shield`], the destination — so the screen presents only the
/// controls that flow needs while reusing the unified screen's validation and
/// dispatch. This is how the three former standalone shielded screens are
/// expressed as routes into the one canonical send screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendFlow {
    /// Free-form send: any source, any destination.
    #[default]
    General,
    /// Shield into the wallet's own shielded pool (Core or Platform → Shielded).
    Shield,
    /// Private transfer within the shielded pool (Shielded → Shielded).
    ShieldedSend,
    /// Move credits out of the shielded pool (Shielded → Platform or Core).
    Unshield,
}

impl SendFlow {
    /// Whether this is a locked shielded preset (anything but `General`).
    fn is_preset(self) -> bool {
        !matches!(self, SendFlow::General)
    }

    /// Heading shown at the top of the send screen for this flow.
    fn heading(self) -> &'static str {
        match self {
            SendFlow::General => "Send Dash",
            SendFlow::Shield => "Shield",
            SendFlow::ShieldedSend => "Send (Private)",
            SendFlow::Unshield => "Unshield Credits",
        }
    }

    /// One-line description shown under the heading, if any.
    fn description(self) -> Option<&'static str> {
        match self {
            SendFlow::General => None,
            SendFlow::Shield => {
                Some("Move funds from your wallet or platform balance into the shielded pool.")
            }
            SendFlow::ShieldedSend => Some("Transfer credits privately within the shielded pool."),
            SendFlow::Unshield => Some(
                "Move credits from the shielded pool to a platform address or a core DASH address.",
            ),
        }
    }

    /// Destination address kinds accepted by a preset flow that takes a
    /// recipient address. Returns `None` for `General` (the caller derives the
    /// kinds from the selected source) and for `Shield` (the destination is the
    /// wallet's own pool, so no address input is rendered).
    fn preset_destination_kinds(self) -> Option<Vec<AddressKind>> {
        match self {
            SendFlow::General | SendFlow::Shield => None,
            SendFlow::ShieldedSend => Some(vec![AddressKind::Shielded]),
            SendFlow::Unshield => Some(vec![AddressKind::Platform, AddressKind::Core]),
        }
    }
}

/// Status of the send operation
#[derive(Debug, Clone, PartialEq)]
pub enum SendStatus {
    NotStarted,
    /// Waiting for result
    WaitingForResult,
    /// Successfully completed with a success message
    Complete(String),
    /// Error occurred (message displayed by global MessageBanner)
    Error,
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

/// A pre-send fee/total estimate for the current source, destination, and
/// amount, all expressed in credits (the unit `Amount::value()` stores).
///
/// Surfaced before the Send button so the user sees the network fee and the
/// total that will leave their balance *before* committing (SND-005). The fee
/// itself comes from `model::fee_estimation` — this type only arranges the
/// already-estimated numbers for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FeePreview {
    /// Estimated network fee, in credits.
    fee_credits: u64,
    /// Total that leaves the source balance, in credits.
    total_debit_credits: u64,
    /// What the recipient receives, in credits, when the fee is taken out of
    /// the entered amount so it differs from that amount. `None` when the
    /// recipient receives exactly the amount entered (fee paid on top).
    recipient_receives_credits: Option<u64>,
}

struct PendingSendConfirmation {
    dialog: ConfirmationDialog,
    action: Box<AppAction>,
}

impl FeePreview {
    /// The fee is paid on top of the amount: the recipient receives the full
    /// amount and the balance is debited amount + fee.
    fn on_top(amount_credits: u64, fee_credits: u64) -> Self {
        Self {
            fee_credits,
            total_debit_credits: amount_credits.saturating_add(fee_credits),
            recipient_receives_credits: None,
        }
    }

    /// The fee is deducted from the amount: the balance is debited exactly the
    /// amount and the recipient receives amount − fee.
    fn deducted_from_amount(amount_credits: u64, fee_credits: u64) -> Self {
        Self {
            fee_credits,
            total_debit_credits: amount_credits,
            recipient_receives_credits: Some(amount_credits.saturating_sub(fee_credits)),
        }
    }
}

/// Render one "label   ≈ value" row inside a fee-summary grid. `strong`
/// emphasises the value (used for the fee and total, not the derived
/// recipient-receives line).
fn fee_summary_row(ui: &mut Ui, dark_mode: bool, label: &str, value_credits: u64, strong: bool) {
    ui.label(
        RichText::new(label)
            .color(DashColors::text_secondary(dark_mode))
            .size(13.0),
    );
    let mut value = RichText::new(format!("≈ {}", format_credits_as_dash(value_credits)))
        .color(DashColors::text_primary(dark_mode))
        .size(13.0);
    if strong {
        value = value.strong();
    }
    ui.label(value);
    ui.end_row();
}

pub struct WalletSendScreen {
    pub app_context: Arc<AppContext>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    selected_wallet_seed_hash: Option<WalletSeedHash>,

    // Unified send fields (simple mode)
    selected_source: Option<SourceSelection>,
    address_input: Option<AddressInput>,
    address_input_snapshot_signature: Option<u64>,
    validated_destination: Option<ValidatedAddress>,
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

    // Identity source fields
    selected_identity: Option<QualifiedIdentity>,

    // State
    send_status: SendStatus,
    send_banner: Option<BannerHandle>,
    send_confirmation: Option<PendingSendConfirmation>,

    /// Preset flow this screen was opened for. `General` is the full send
    /// screen; the shielded presets lock source/destination for that flow.
    flow: SendFlow,

    // Wallet unlock
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
}

impl WalletSendScreen {
    pub fn new(app_context: &Arc<AppContext>, wallet: Arc<RwLock<Wallet>>) -> Self {
        let seed_hash = wallet.read().ok().map(|w| w.seed_hash());
        Self {
            app_context: app_context.clone(),
            selected_wallet: Some(wallet),
            selected_wallet_seed_hash: seed_hash,
            selected_source: Some(SourceSelection::CoreWallet),
            address_input: None,
            address_input_snapshot_signature: None,
            validated_destination: None,
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
            selected_identity: None,
            send_status: SendStatus::NotStarted,
            send_banner: None,
            send_confirmation: None,
            flow: SendFlow::General,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
        }
    }

    /// The preset flow this screen was opened for.
    pub fn flow(&self) -> SendFlow {
        self.flow
    }

    /// Open the screen pre-configured for a shielded [`SendFlow`]. The flow
    /// locks the source (and, for [`SendFlow::Shield`], the destination) so the
    /// screen shows only the controls that flow needs.
    pub fn with_flow(mut self, flow: SendFlow) -> Self {
        self.flow = flow;
        // For shielded-source presets, seed the source immediately so the first
        // frame's amount limits are correct; `sync_flow_state` keeps it fresh.
        if flow.is_preset() {
            self.selected_source = None;
        }
        self
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
            return estimate_platform_fee(fee_estimator, 1, 1);
        }

        let dest_kind = self.validated_destination.as_ref().map(|v| v.kind());
        if dest_kind == Some(AddressKind::Core) {
            let output_script = self
                .validated_destination
                .as_ref()
                .and_then(|v| v.as_core())
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

        estimate_platform_fee(fee_estimator, usable_count, 1)
    }

    /// Clear the AddressInput widget so it picks up the new network on next frame.
    pub(crate) fn invalidate_address_input(&mut self) {
        self.address_input = None;
        self.address_input_snapshot_signature = None;
        self.validated_destination = None;
    }

    /// Reset all wallet-bound state on a network switch. The old wallet, its
    /// seed hash, and any source/destination/amount selection belong to the
    /// previous network — leaving the seed hash behind would let a preset flow
    /// resurrect a stale source and show the previous network's balance. Source
    /// resets to the Core-wallet default so the free-form screen behaves as it
    /// does on first open; a preset re-derives its source once a wallet for the
    /// new network is selected.
    pub(crate) fn reset_for_network_switch(&mut self) {
        self.selected_wallet = None;
        self.selected_wallet_seed_hash = None;
        self.selected_source = Some(SourceSelection::CoreWallet);
        self.selected_identity = None;
        self.amount = None;
        self.amount_input = None;
        self.invalidate_address_input();
        self.send_confirmation = None;
    }

    fn reset_form(&mut self) {
        self.address_input = None;
        self.address_input_snapshot_signature = None;
        self.validated_destination = None;
        self.amount = None;
        self.amount_input = None;
        self.selected_source = Some(SourceSelection::CoreWallet);
        self.selected_identity = None;
        self.advanced_source_type = AdvancedSourceType::Core;
        self.core_inputs.clear();
        self.platform_inputs.clear();
        self.advanced_outputs = vec![AdvancedOutput {
            address: String::new(),
            amount: String::new(),
        }];
        self.fee_strategy = PlatformFeeStrategy::default();
        self.send_status = SendStatus::NotStarted;
        self.send_confirmation = None;
    }

    fn mark_sending(&mut self) {
        self.send_status = SendStatus::WaitingForResult;
    }

    /// Renders the fee summary appended to a completed-send message. Shows the
    /// settled fee only when the platform reported one; otherwise labels the
    /// value as an estimate.
    fn format_fee_info(fee_result: &crate::backend_task::FeeResult) -> String {
        match fee_result.actual_fee {
            Some(actual) => format!(
                "\n\nFee: Estimated {} • Actual {}",
                format_credits_as_dash(fee_result.estimated_fee),
                format_credits_as_dash(actual)
            ),
            None => format!(
                "\n\nFee: Estimated {}",
                format_credits_as_dash(fee_result.estimated_fee)
            ),
        }
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

    /// Detect address kind from the address string.
    ///
    /// Returns `None` for empty or unrecognized input.
    fn detect_address_kind(&self, address: &str) -> Option<AddressKind> {
        AddressKind::detect(address)
    }

    fn min_output_amount(
        &self,
        input_type: Option<AddressKind>,
        output_type: Option<AddressKind>,
    ) -> Option<Credits> {
        let core_min = 5460_u64 * CREDITS_PER_DUFF;
        let platform_min = self
            .app_context
            .platform_version()
            .dpp
            .state_transitions
            .address_funds
            .min_output_amount;

        use AddressKind::*;
        match (input_type, output_type) {
            (None, None) => None,
            (Some(Core), Some(Core)) => Some(core_min),
            (Some(Platform), Some(Platform)) => Some(platform_min),
            (Some(Core), Some(Platform)) => Some(56000000), // needed for asset locks
            (Some(Platform), Some(Core)) => Some(core_min.max(platform_min)),
            (None, Some(Core)) => Some(core_min),
            (None, Some(Platform)) => Some(platform_min),
            (Some(Core), None) => Some(core_min),
            (Some(Platform), None) => Some(platform_min),
            (Some(Shielded), Some(Shielded)) => Some(platform_min),
            (Some(Shielded), Some(Platform)) => Some(platform_min),
            (Some(Shielded), _) => Some(platform_min),
            (_, Some(Shielded)) => Some(platform_min),
            (Some(Identity), _) | (_, Some(Identity)) => Some(platform_min),
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

    /// Get shielded pool balance for the selected wallet from the frame-safe
    /// push snapshot (no lock-in-frame-loop, no `block_in_place`). Returns
    /// `None` when there is no positive shielded balance to spend.
    fn get_shielded_balance(&self) -> Option<(WalletSeedHash, u64)> {
        let seed_hash = self.selected_wallet_seed_hash?;
        let balance = self.app_context.shielded_balance_credits(&seed_hash);
        if balance > 0 {
            Some((seed_hash, balance))
        } else {
            None
        }
    }

    /// Get the Core wallet's **spendable** balance from the display-only
    /// `WalletBackend` snapshot (P4a). DISPLAY-ONLY — this number never feeds
    /// coin selection itself, but it must mirror what coin selection can spend
    /// so the amount checks here agree with the actual send. `spendable()` is
    /// the upstream `CoinSelector`'s set (confirmed + unconfirmed); reading
    /// `confirmed` alone would understate IS-locked funds that have not yet been
    /// flagged locally (they sit in `unconfirmed`), making "Max" exceed this
    /// check and the validations reject sends coin selection would accept.
    fn get_core_balance(&self) -> u64 {
        self.selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok())
            .map(|w| {
                self.app_context
                    .snapshot_balance(&w.seed_hash())
                    .spendable()
            })
            .unwrap_or(0)
    }

    /// Get loaded identities for the current wallet, filtered by wallet seed hash.
    fn get_loaded_identities(&self) -> Vec<QualifiedIdentity> {
        let Some(wallet_arc) = &self.selected_wallet else {
            return vec![];
        };
        let Ok(wallet) = wallet_arc.read() else {
            return vec![];
        };
        let seed_hash = wallet.seed_hash();

        let Ok(all_identities) = self.app_context.load_local_qualified_identities() else {
            return vec![];
        };

        all_identities
            .into_iter()
            .filter(|qi| qi.associated_wallets.contains_key(&seed_hash))
            .collect()
    }

    /// Get Core addresses with their UTXO balances
    fn get_core_addresses(&self) -> Vec<(Address, u64)> {
        let Some(wallet_arc) = &self.selected_wallet else {
            return vec![];
        };
        let Ok(wallet) = wallet_arc.read() else {
            return vec![];
        };
        let seed_hash = wallet.seed_hash();
        drop(wallet);

        let mut addresses: Vec<(Address, u64)> = self
            .app_context
            .snapshot_address_balances(&seed_hash)
            .into_iter()
            .filter(|(_, balance)| *balance > 0)
            .collect();
        // Sort by balance descending for better UX
        addresses.sort_by(|a, b| b.1.cmp(&a.1));
        addresses
    }

    /// Get description of transaction type based on source and destination
    fn get_transaction_type_description(&self) -> &'static str {
        let dest_kind = self.destination_kind();
        match (&self.selected_source, dest_kind) {
            // Core Wallet source
            (Some(SourceSelection::CoreWallet), Some(AddressKind::Core)) => "Send DASH",
            (Some(SourceSelection::CoreWallet), Some(AddressKind::Platform)) => {
                "Fund Platform Address"
            }
            (Some(SourceSelection::CoreWallet), Some(AddressKind::Shielded)) => "Shield DASH",
            (Some(SourceSelection::CoreWallet), Some(AddressKind::Identity)) => "Top Up Identity",
            // Platform Addresses source
            (Some(SourceSelection::PlatformAddresses(_)), Some(AddressKind::Platform)) => {
                "Transfer Credits"
            }
            (Some(SourceSelection::PlatformAddresses(_)), Some(AddressKind::Core)) => {
                "Withdraw to Wallet"
            }
            (Some(SourceSelection::PlatformAddresses(_)), Some(AddressKind::Shielded)) => {
                "Shield Credits"
            }
            (Some(SourceSelection::PlatformAddresses(_)), Some(AddressKind::Identity)) => {
                "Top Up Identity"
            }
            // Identity source
            (Some(SourceSelection::Identity(_)), Some(AddressKind::Core)) => "Withdraw Credits",
            (Some(SourceSelection::Identity(_)), Some(AddressKind::Platform)) => {
                "Transfer to Address"
            }
            (Some(SourceSelection::Identity(_)), Some(AddressKind::Identity)) => "Transfer Credits",
            // Shielded source
            (Some(SourceSelection::Shielded(..)), Some(AddressKind::Core)) => {
                "Withdraw from Shield"
            }
            (Some(SourceSelection::Shielded(..)), Some(AddressKind::Shielded)) => "Private Send",
            (Some(SourceSelection::Shielded(..)), Some(AddressKind::Platform)) => {
                "Unshield Credits"
            }
            _ => "Send",
        }
    }

    /// Returns the address kind of the current validated destination, if any.
    fn destination_kind(&self) -> Option<AddressKind> {
        self.validated_destination.as_ref().map(|v| v.kind())
    }

    /// Returns the destination address string, from the validated address if
    /// available or an empty string otherwise.
    fn destination_address_string(&self) -> String {
        self.validated_destination
            .as_ref()
            .map(|v| v.to_address_string())
            .unwrap_or_default()
    }

    /// Clear the current send banner and show a new "Sending transaction..." progress banner.
    ///
    /// Called before dispatching any send backend task so the elapsed counter always starts fresh.
    fn set_send_progress_banner(&mut self, ctx: &Context) {
        self.send_banner.take_and_clear();
        let handle = MessageBanner::set_global(ctx, "Sending transaction...", MessageType::Info);
        handle.with_elapsed();
        self.send_banner = Some(handle);
    }

    /// Validate and execute the send based on detected types
    fn validate_and_send(&mut self) -> Result<AppAction, String> {
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet selected")?;

        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

        if !wallet_guard.is_open() {
            return Err("Wallet must be unlocked first".to_string());
        }

        let seed_hash = wallet_guard.seed_hash();

        // Validate source
        let source = self
            .selected_source
            .as_ref()
            .ok_or("Please select a source")?;

        // Validate destination
        let dest_kind = self.destination_kind();
        if dest_kind.is_none() {
            return Err(
                "Invalid destination address. Use a Dash address (X.../y...) or Platform address (dash1.../tdash1...)"
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
        match (source.clone(), dest_kind) {
            // === Existing 6 combinations ===
            (SourceSelection::CoreWallet, Some(AddressKind::Core)) => self.send_core_to_core(),
            (SourceSelection::CoreWallet, Some(AddressKind::Platform)) => {
                self.send_core_to_platform(seed_hash)
            }
            (SourceSelection::PlatformAddresses(addresses), Some(AddressKind::Platform)) => {
                self.send_platform_to_platform(seed_hash, addresses)
            }
            (SourceSelection::PlatformAddresses(addresses), Some(AddressKind::Core)) => {
                self.send_platform_to_core(seed_hash, addresses)
            }
            (SourceSelection::Shielded(sh, _), Some(AddressKind::Shielded)) => {
                self.send_shielded_to_shielded(sh)
            }
            (SourceSelection::Shielded(sh, _), Some(AddressKind::Platform)) => {
                self.send_shielded_to_platform(sh)
            }
            // === New 8 combinations ===
            (SourceSelection::CoreWallet, Some(AddressKind::Shielded)) => {
                self.send_core_to_shielded(seed_hash)
            }
            (SourceSelection::CoreWallet, Some(AddressKind::Identity)) => {
                self.send_core_to_identity(seed_hash)
            }
            (SourceSelection::PlatformAddresses(addresses), Some(AddressKind::Shielded)) => {
                self.send_platform_to_shielded(seed_hash, addresses)
            }
            (SourceSelection::PlatformAddresses(addresses), Some(AddressKind::Identity)) => {
                self.send_platform_to_identity(seed_hash, addresses)
            }
            (SourceSelection::Shielded(sh, _), Some(AddressKind::Core)) => {
                self.send_shielded_to_core(sh)
            }
            (SourceSelection::Identity(qi), Some(AddressKind::Core)) => {
                self.send_identity_to_core(*qi)
            }
            (SourceSelection::Identity(qi), Some(AddressKind::Platform)) => {
                self.send_identity_to_platform(*qi)
            }
            (SourceSelection::Identity(qi), Some(AddressKind::Identity)) => {
                self.send_identity_to_identity(*qi)
            }
            // === Unsupported combinations (defer to v2) ===
            (SourceSelection::Identity(_), Some(AddressKind::Shielded)) => Err(
                "Sending from an identity to the shielded pool is not yet supported. \
                     Transfer to a Platform address first, then shield from there."
                    .to_string(),
            ),
            (SourceSelection::Shielded(..), Some(AddressKind::Identity)) => Err(
                "Sending from the shielded pool to an identity is not yet supported. \
                     Transfer to a Platform address first, then top up the identity."
                    .to_string(),
            ),
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
                format_duffs_as_dash(amount_duffs),
                format_duffs_as_dash(balance)
            ));
        }

        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or("No wallet selected")?
            .clone();

        let recipient = PaymentRecipient {
            address: self.destination_address_string(),
            amount_duffs,
        };

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet,
                request: WalletPaymentRequest {
                    recipients: vec![recipient],
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

        // Extract validated platform address
        let destination = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied())
            .ok_or_else(|| "Invalid platform address".to_string())?;

        // Check balance; fees will be subtracted from amount
        let required = amount_duffs;
        let balance = self.get_core_balance();
        if required > balance {
            return Err(format!(
                "Insufficient balance. Need {} (including fee) but have {}",
                format_duffs_as_dash(required),
                format_duffs_as_dash(balance)
            ));
        }

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
            format_credits_as_dash(amount_credits),
            format_credits_as_dash(total_balance),
            addresses.len()
        );

        if amount_credits > total_balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(total_balance)
            ));
        }

        // Extract validated platform address
        let destination = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied())
            .ok_or_else(|| "Invalid platform address".to_string())?;

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
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(available_balance)
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
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(max_sendable),
                addresses_available,
                format_credits_as_dash(max_balance),
                MAX_PLATFORM_INPUTS,
                format_credits_as_dash(allocation.estimated_fee),
                allocation.inputs.len(),
                format_credits_as_dash(allocation.shortfall)
            ));
        }

        let mut outputs = BTreeMap::new();
        outputs.insert(destination, amount_credits);

        // Log transfer summary
        let total_input: u64 = allocation.inputs.values().sum();
        tracing::debug!(
            "Platform transfer: {} inputs totaling {}, output {}, fee {} (payer idx {})",
            allocation.inputs.len(),
            format_credits_as_dash(total_input),
            format_credits_as_dash(amount_credits),
            format_credits_as_dash(allocation.estimated_fee),
            allocation.fee_payer_index
        );

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
            format_credits_as_dash(amount_credits),
            format_credits_as_dash(total_balance),
            addresses.len()
        );

        if amount_credits > total_balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(total_balance)
            ));
        }

        // Extract validated Core address
        let dest_address = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_core())
            .ok_or_else(|| "Invalid Core address".to_string())?;

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
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(max_sendable),
                addresses_available,
                format_credits_as_dash(max_balance),
                MAX_PLATFORM_INPUTS,
                format_credits_as_dash(allocation.estimated_fee),
                allocation.inputs.len(),
                format_credits_as_dash(allocation.shortfall)
            ));
        }

        // Log withdrawal summary
        let total_input: u64 = allocation.inputs.values().sum();
        tracing::debug!(
            "Platform withdrawal: {} inputs totaling {}, withdraw {}, fee {} (payer idx {})",
            allocation.inputs.len(),
            format_credits_as_dash(total_input),
            format_credits_as_dash(amount_credits),
            format_credits_as_dash(allocation.estimated_fee),
            allocation.fee_payer_index
        );

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

        if !self.wallet_open_attempted {
            if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                MessageBanner::set_global(ui.ctx(), &e, MessageType::Error).disable_auto_dismiss();
            }
            self.wallet_open_attempted = true;
        }
        if wallet_needs_unlock(wallet) {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add_space(10.0);
            ui.colored_label(
                DashColors::warning_color(dark_mode),
                "Wallet is locked. Please unlock to continue.",
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

    fn render_send_status(&mut self, ui: &mut Ui) -> Option<AppAction> {
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
            SendStatus::WaitingForResult => {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.add(egui::Spinner::new().size(40.0));
                    ui.add_space(20.0);
                    ui.heading("Sending...");
                    ui.add_space(100.0);
                });
                Some(AppAction::None)
            }
            SendStatus::Error => {
                // Error is displayed by the global MessageBanner — no extra
                // UI needed here. Reset status so the form is usable again.
                self.send_status = SendStatus::NotStarted;
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

        // Fee estimate + total, shown before the Send button (SND-005).
        self.render_fee_summary(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Send button
        action |= self.render_send_button(ui);

        action
    }

    /// Keep a preset flow's source (and, for `Shield`, the sentinel own-pool
    /// destination) in sync with current balances. Idempotent — safe to call
    /// every frame. Reads only the frame-safe push snapshot.
    fn sync_flow_state(&mut self) {
        let Some(seed_hash) = self.selected_wallet_seed_hash else {
            return;
        };
        match self.flow {
            SendFlow::ShieldedSend | SendFlow::Unshield => {
                // Source is always the wallet's shielded pool; refresh the
                // captured balance so the amount cap tracks spends.
                let balance = self.app_context.shielded_balance_credits(&seed_hash);
                self.selected_source = Some(SourceSelection::Shielded(seed_hash, balance));
            }
            SendFlow::Shield => {
                // Default to shielding the whole Core wallet; the user may switch
                // to the platform balance via the toggle.
                if !matches!(
                    self.selected_source,
                    Some(SourceSelection::CoreWallet | SourceSelection::PlatformAddresses(_))
                ) {
                    self.selected_source = Some(SourceSelection::CoreWallet);
                }
                // Shielding always targets the wallet's own pool and the dispatch
                // ignores the destination address, so satisfy the router with a
                // sentinel shielded destination instead of rendering an input.
                if !matches!(
                    self.validated_destination,
                    Some(ValidatedAddress::Shielded(_))
                ) {
                    self.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
                }
            }
            SendFlow::General => {}
        }
    }

    /// Render the source controls for a preset flow: a Core/Platform toggle for
    /// `Shield`, or a read-only shielded-balance line for the spend presets.
    fn render_flow_source(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        match self.flow {
            SendFlow::Shield => {
                let has_platform = !self.get_platform_addresses().is_empty();
                ui.label(
                    RichText::new("Shield from")
                        .color(DashColors::text_primary(dark_mode))
                        .strong()
                        .size(14.0),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let mut is_core =
                        matches!(self.selected_source, Some(SourceSelection::CoreWallet));
                    if ui
                        .radio_value(&mut is_core, true, "Core wallet (whole balance)")
                        .changed()
                        && is_core
                    {
                        self.selected_source = Some(SourceSelection::CoreWallet);
                        self.amount = None;
                        self.amount_input = None;
                    }
                    ui.add_enabled_ui(has_platform, |ui| {
                        let mut is_platform = matches!(
                            self.selected_source,
                            Some(SourceSelection::PlatformAddresses(_))
                        );
                        if ui
                            .radio_value(&mut is_platform, true, "Platform balance")
                            .changed()
                            && is_platform
                        {
                            let addresses: Vec<_> = self
                                .get_platform_addresses()
                                .into_iter()
                                .map(|(core_addr, platform_addr, balance)| {
                                    (platform_addr, core_addr, balance)
                                })
                                .collect();
                            self.selected_source =
                                Some(SourceSelection::PlatformAddresses(addresses));
                            self.amount = None;
                            self.amount_input = None;
                        }
                    });
                });
                ui.add_space(4.0);
                let balance_label = match &self.selected_source {
                    Some(SourceSelection::PlatformAddresses(addresses)) => {
                        let total: u64 = addresses.iter().map(|(_, _, b)| *b).sum();
                        format!(
                            "Available platform balance: {}",
                            format_credits_as_dash(total)
                        )
                    }
                    _ => format!(
                        "Available core wallet balance: {}",
                        format_duffs_as_dash(self.get_core_balance())
                    ),
                };
                ui.label(RichText::new(balance_label).color(DashColors::success_color(dark_mode)));
            }
            SendFlow::ShieldedSend | SendFlow::Unshield => {
                let balance = match &self.selected_source {
                    Some(SourceSelection::Shielded(_, balance)) => *balance,
                    _ => 0,
                };
                ui.label(
                    RichText::new(format!(
                        "Available shielded balance: {}",
                        format_credits_as_dash(balance)
                    ))
                    .color(DashColors::success_color(dark_mode)),
                );
            }
            SendFlow::General => {}
        }
    }

    /// Render a preset shielded flow (Shield / Send Private / Unshield): a
    /// locked source, a flow-scoped destination (or none for Shield), the shared
    /// amount input, and the shared send button.
    fn render_flow_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        self.render_wallet_info(ui);
        if !self.render_unlock_gate(ui) {
            return AppAction::None;
        }
        ui.add_space(10.0);

        self.sync_flow_state();
        self.render_flow_source(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Destination (Shield targets the own pool and renders no input).
        if let Some(kinds) = self.flow.preset_destination_kinds() {
            self.render_destination_input_with_kinds(ui, &kinds);
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
        }

        self.render_amount_input(ui);

        ui.add_space(10.0);

        // Fee estimate + total, shown before the Send button (SND-005).
        self.render_fee_summary(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        action |= self.render_send_button(ui);
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

    /// Send from shielded pool to another shielded address (private transfer).
    fn send_shielded_to_shielded(
        &mut self,
        seed_hash: WalletSeedHash,
    ) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();

        let recipient = self.destination_address_string();
        let recipient_bytes = crate::model::address::parse_shielded_recipient(&recipient)
            .ok_or_else(|| "Invalid shielded address".to_string())?;

        self.send_status = SendStatus::WaitingForResult;
        Ok(AppAction::BackendTask(
            crate::backend_task::BackendTask::ShieldedTask(
                crate::backend_task::shielded::ShieldedTask::ShieldedTransfer {
                    seed_hash,
                    amount: amount_credits,
                    recipient_address_bytes: recipient_bytes,
                },
            ),
        ))
    }

    /// Send from shielded pool to a platform address (unshield).
    fn send_shielded_to_platform(
        &mut self,
        seed_hash: WalletSeedHash,
    ) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();

        let platform_addr = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied())
            .ok_or_else(|| "Invalid platform address".to_string())?;

        self.send_status = SendStatus::WaitingForResult;
        Ok(AppAction::BackendTask(
            crate::backend_task::BackendTask::ShieldedTask(
                crate::backend_task::shielded::ShieldedTask::UnshieldCredits {
                    seed_hash,
                    amount: amount_credits,
                    to_platform_address: platform_addr,
                },
            ),
        ))
    }

    // === New send handler methods (8 combinations) ===

    /// Shield DASH from Core wallet via asset lock (Core -> Shielded).
    fn send_core_to_shielded(&mut self, seed_hash: WalletSeedHash) -> Result<AppAction, String> {
        // Shielding from Core always deposits into the wallet's own shielded pool.
        // Validate the destination is a shielded address (the address input already constrains this).
        if !matches!(
            &self.validated_destination,
            Some(ValidatedAddress::Shielded(_))
        ) {
            return Err("Please enter a valid shielded address".to_string());
        }

        let amount_duffs = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .dash_to_duffs()?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let balance = self.get_core_balance();
        if amount_duffs > balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_duffs_as_dash(amount_duffs),
                format_duffs_as_dash(balance)
            ));
        }

        Ok(AppAction::BackendTask(BackendTask::ShieldedTask(
            crate::backend_task::shielded::ShieldedTask::ShieldFromAssetLock {
                seed_hash,
                amount_duffs,
            },
        )))
    }

    /// Top up an identity from Core wallet via asset lock (Core -> Identity).
    fn send_core_to_identity(&mut self, _seed_hash: WalletSeedHash) -> Result<AppAction, String> {
        let amount_duffs = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .dash_to_duffs()?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let balance = self.get_core_balance();
        if amount_duffs > balance {
            return Err(format!(
                "Insufficient balance. Need {} but have {}",
                format_duffs_as_dash(amount_duffs),
                format_duffs_as_dash(balance)
            ));
        }

        // Resolve identity from destination
        let identity_id = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_identity_id().copied())
            .ok_or_else(|| "Invalid identity ID".to_string())?;

        let qualified_identity = self
            .app_context
            .get_identity_by_id(&identity_id)
            .map_err(|e| format!("Could not look up identity: {e}"))?
            .ok_or_else(|| {
                "No identity found with this ID. Please check the ID and try again.".to_string()
            })?;

        let identity_index = qualified_identity.wallet_index.unwrap_or(0);
        let top_up_index = qualified_identity.top_ups.len() as u32;

        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or("No wallet selected")?
            .clone();

        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TopUpIdentity(IdentityTopUpInfo {
                qualified_identity,
                wallet,
                identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                    amount_duffs,
                    identity_index,
                    top_up_index,
                ),
            }),
        )))
    }

    /// Shield credits from the wallet's Platform balance into the shielded pool
    /// (Platform -> Shielded).
    ///
    /// The upstream coordinator selects the funding platform addresses, so DET
    /// dispatches a single `ShieldFromBalance` task with the total amount rather
    /// than one task per address.
    fn send_platform_to_shielded(
        &mut self,
        seed_hash: WalletSeedHash,
        addresses: Vec<(PlatformAddress, Address, u64)>,
    ) -> Result<AppAction, String> {
        if !matches!(
            &self.validated_destination,
            Some(ValidatedAddress::Shielded(_))
        ) {
            return Err("Please enter a valid shielded address".to_string());
        }

        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        // Pre-flight: the total platform balance must cover the amount. The
        // upstream coordinator handles per-address selection and fees.
        let total_available: u64 = addresses.iter().map(|(_, _, b)| b).sum();
        if amount_credits > total_available {
            return Err(format!(
                "Insufficient platform balance. Need {} but total available is {}.",
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(total_available)
            ));
        }

        Ok(AppAction::BackendTask(BackendTask::ShieldedTask(
            crate::backend_task::shielded::ShieldedTask::ShieldFromBalance {
                seed_hash,
                amount: amount_credits,
            },
        )))
    }

    /// Top up an identity from Platform addresses (Platform -> Identity).
    fn send_platform_to_identity(
        &mut self,
        seed_hash: WalletSeedHash,
        addresses: Vec<(PlatformAddress, Address, u64)>,
    ) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let identity_id = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_identity_id().copied())
            .ok_or_else(|| "Invalid identity ID".to_string())?;

        let qualified_identity = self
            .app_context
            .get_identity_by_id(&identity_id)
            .map_err(|e| format!("Could not look up identity: {e}"))?
            .ok_or_else(|| {
                "No identity found with this ID. Please check the ID and try again.".to_string()
            })?;

        let fee_estimator = self.app_context.fee_estimator();
        let allocation =
            allocate_platform_addresses(&fee_estimator, &addresses, amount_credits, None);
        if allocation.shortfall > 0 {
            return Err(format!(
                "Insufficient platform balance. Need {} (including estimated fee of {}) but short by {}",
                format_credits_as_dash(amount_credits + allocation.estimated_fee),
                format_credits_as_dash(allocation.estimated_fee),
                format_credits_as_dash(allocation.shortfall)
            ));
        }

        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TopUpIdentityFromPlatformAddresses {
                identity: qualified_identity,
                inputs: allocation.inputs,
                wallet_seed_hash: seed_hash,
            },
        )))
    }

    /// Withdraw from shielded pool to Core address (Shielded -> Core).
    fn send_shielded_to_core(&mut self, seed_hash: WalletSeedHash) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let core_address = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_core().cloned())
            .ok_or_else(|| "Invalid Core address".to_string())?;

        Ok(AppAction::BackendTask(BackendTask::ShieldedTask(
            crate::backend_task::shielded::ShieldedTask::ShieldedWithdrawal {
                seed_hash,
                amount: amount_credits,
                to_core_address: core_address,
            },
        )))
    }

    /// Withdraw identity credits to Core address (Identity -> Core).
    fn send_identity_to_core(
        &mut self,
        qualified_identity: QualifiedIdentity,
    ) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let identity_balance = qualified_identity.identity.balance();
        if amount_credits > identity_balance {
            return Err(format!(
                "Insufficient identity balance. Need {} but have {}",
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(identity_balance)
            ));
        }

        let core_address = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_core().cloned())
            .ok_or_else(|| "Invalid Core address".to_string())?;

        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::WithdrawFromIdentity(
                qualified_identity,
                Some(core_address),
                amount_credits,
                None,
            ),
        )))
    }

    /// Transfer identity credits to Platform address (Identity -> Platform).
    fn send_identity_to_platform(
        &mut self,
        qualified_identity: QualifiedIdentity,
    ) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let identity_balance = qualified_identity.identity.balance();
        if amount_credits > identity_balance {
            return Err(format!(
                "Insufficient identity balance. Need {} but have {}",
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(identity_balance)
            ));
        }

        let platform_addr = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied())
            .ok_or_else(|| "Invalid Platform address".to_string())?;

        let mut outputs = BTreeMap::new();
        outputs.insert(platform_addr, amount_credits);

        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TransferToAddresses {
                identity: qualified_identity,
                outputs,
                key_id: None,
            },
        )))
    }

    /// Transfer identity credits to another identity (Identity -> Identity).
    fn send_identity_to_identity(
        &mut self,
        qualified_identity: QualifiedIdentity,
    ) -> Result<AppAction, String> {
        let amount_credits = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .value();
        if amount_credits == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let identity_balance = qualified_identity.identity.balance();
        if amount_credits > identity_balance {
            return Err(format!(
                "Insufficient identity balance. Need {} but have {}",
                format_credits_as_dash(amount_credits),
                format_credits_as_dash(identity_balance)
            ));
        }

        let to_identity_id = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_identity_id().copied())
            .ok_or_else(|| "Invalid identity ID".to_string())?;

        // Prevent self-send (same identity as source and destination)
        if to_identity_id == qualified_identity.identity.id() {
            return Err(
                "You cannot send credits to the same identity. Please choose a different destination."
                    .to_string(),
            );
        }

        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::Transfer(qualified_identity, to_identity_id, amount_credits, None),
        )))
    }

    fn render_source_selection(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

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
                        self.address_input = None;
                        self.validated_destination = None;
                    }
                    ui.label(
                        RichText::new("Core Wallet")
                            .color(DashColors::text_primary(dark_mode))
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format_duffs_as_dash(core_balance))
                                .color(DashColors::success_color(dark_mode))
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
                            self.address_input = None;
                            self.validated_destination = None;
                        }
                        ui.label(
                            RichText::new("Platform Addresses")
                                .color(DashColors::text_primary(dark_mode))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_credits_as_dash(total_platform_balance))
                                    .color(DashColors::success_color(dark_mode))
                                    .strong(),
                            );
                        });
                    });
                });
        }

        // Identity source option — visible when identities exist or developer mode
        let identities = self.get_loaded_identities();
        // READ-only seed (R1, W5): if no identity is pre-selected yet, try the app-scoped
        // identity — but only if it belongs to this wallet's identity list.
        // No syncing_global: the send screen is wallet-primary; K1 reconcile would fight it.
        if self.selected_identity.is_none()
            && let Some(preferred_id) = self.app_context.selected_identity_id()
            && let Some(qi) = identities.iter().find(|qi| {
                use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                qi.identity.id() == preferred_id
            })
        {
            self.selected_identity = Some(qi.clone());
        }
        if !identities.is_empty() || self.app_context.user_role().at_least(UserRole::Power) {
            ui.add_space(5.0);

            let is_identity_selected =
                matches!(&self.selected_source, Some(SourceSelection::Identity(_)));

            Frame::group(ui.style())
                .fill(if is_identity_selected {
                    DashColors::DASH_BLUE.gamma_multiply(0.1)
                } else {
                    DashColors::surface(dark_mode)
                })
                .stroke(if is_identity_selected {
                    egui::Stroke::new(2.0, DashColors::DASH_BLUE)
                } else {
                    egui::Stroke::new(1.0, DashColors::border_light(dark_mode))
                })
                .inner_margin(Margin::symmetric(12, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut selected = is_identity_selected;
                        if ui.radio_value(&mut selected, true, "").changed() && selected {
                            if let Some(identity) = self.selected_identity.clone().or_else(|| {
                                // Default to the identity with the highest balance
                                identities
                                    .iter()
                                    .max_by_key(|qi| qi.identity.balance())
                                    .cloned()
                            }) {
                                self.selected_source =
                                    Some(SourceSelection::Identity(Box::new(identity.clone())));
                                self.selected_identity = Some(identity);
                            }
                            self.address_input = None;
                            self.validated_destination = None;
                        }
                        ui.label(
                            RichText::new("Identity")
                                .color(DashColors::text_primary(dark_mode))
                                .strong(),
                        );
                        if let Some(SourceSelection::Identity(qi)) = &self.selected_source {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format_credits_as_dash(
                                            qi.identity.balance(),
                                        ))
                                        .color(DashColors::success_color(dark_mode))
                                        .strong(),
                                    );
                                },
                            );
                        } else if let Some(first) = identities.first() {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format_credits_as_dash(
                                            first.identity.balance(),
                                        ))
                                        .color(DashColors::success_color(dark_mode))
                                        .strong(),
                                    );
                                },
                            );
                        }
                    });

                    // Identity selector dropdown when multiple identities
                    if is_identity_selected && identities.len() > 1 {
                        ui.add_space(4.0);
                        let current_label = self
                            .selected_identity
                            .as_ref()
                            .map(|qi| {
                                let name = qi
                                    .dpns_names
                                    .first()
                                    .map(|n| n.name.clone())
                                    .or_else(|| qi.alias.clone())
                                    .unwrap_or_else(|| {
                                        let id_str = qi.identity.id().to_string(
                                            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                                        );
                                        format!("{}...", &id_str[..8.min(id_str.len())])
                                    });
                                format!(
                                    "{} ({})",
                                    name,
                                    format_credits_as_dash(qi.identity.balance())
                                )
                            })
                            .unwrap_or_else(|| "Select identity".to_string());

                        egui::ComboBox::from_id_salt("identity_source_selector")
                            .selected_text(&current_label)
                            .width(ui.available_width() - 20.0)
                            .show_ui(ui, |ui| {
                                for identity in &identities {
                                    let label = {
                                        let name = identity
                                            .dpns_names
                                            .first()
                                            .map(|n| n.name.clone())
                                            .or_else(|| identity.alias.clone())
                                            .unwrap_or_else(|| {
                                                let id_str = identity.identity.id().to_string(
                                                    dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                                                );
                                                format!(
                                                    "{}...",
                                                    &id_str[..8.min(id_str.len())]
                                                )
                                            });
                                        format!(
                                            "{} ({})",
                                            name,
                                            format_credits_as_dash(identity.identity.balance())
                                        )
                                    };
                                    let is_selected = self
                                        .selected_identity
                                        .as_ref()
                                        .is_some_and(|sel| {
                                            sel.identity.id() == identity.identity.id()
                                        });
                                    if ui.selectable_label(is_selected, &label).clicked() {
                                        self.selected_identity = Some(identity.clone());
                                        self.selected_source =
                                            Some(SourceSelection::Identity(Box::new(identity.clone())));
                                        self.address_input = None;
                                        self.validated_destination = None;
                                    }
                                }
                            });
                    }
                });
        }

        // Shielded balance option (experimental, and only where the network
        // defines the shielded state transitions).
        let shielded_balance = self.get_shielded_balance();
        if FeatureGate::ShieldedOperations.is_available(&self.app_context)
            && let Some((seed_hash, balance)) = shielded_balance
            && balance > 0
        {
            ui.add_space(5.0);

            let is_shielded_selected =
                matches!(&self.selected_source, Some(SourceSelection::Shielded(..)));

            Frame::group(ui.style())
                .fill(if is_shielded_selected {
                    DashColors::DASH_BLUE.gamma_multiply(0.1)
                } else {
                    DashColors::surface(dark_mode)
                })
                .stroke(if is_shielded_selected {
                    egui::Stroke::new(2.0, DashColors::DASH_BLUE)
                } else {
                    egui::Stroke::new(1.0, DashColors::border_light(dark_mode))
                })
                .inner_margin(Margin::symmetric(12, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut selected = is_shielded_selected;
                        if ui.radio_value(&mut selected, true, "").changed() && selected {
                            self.selected_source =
                                Some(SourceSelection::Shielded(seed_hash, balance));
                            self.address_input = None;
                            self.validated_destination = None;
                        }
                        ui.label(
                            RichText::new("Shielded Balance")
                                .color(DashColors::text_primary(dark_mode))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format_credits_as_dash(balance))
                                    .color(DashColors::success_color(dark_mode))
                                    .strong(),
                            );
                        });
                    });
                });
        }
    }

    /// Destination address kinds for the free-form (General) send, derived from
    /// the selected source. Shielded destinations are offered only where the
    /// network can settle them and the feature is unlocked.
    fn general_destination_kinds(&self) -> Vec<AddressKind> {
        let shielded_enabled = FeatureGate::ShieldedOperations.is_available(&self.app_context);
        match &self.selected_source {
            Some(SourceSelection::CoreWallet) => {
                let mut kinds = vec![AddressKind::Core, AddressKind::Platform];
                if shielded_enabled {
                    kinds.push(AddressKind::Shielded);
                }
                kinds.push(AddressKind::Identity);
                kinds
            }
            Some(SourceSelection::PlatformAddresses(_)) => {
                let mut kinds = vec![AddressKind::Platform, AddressKind::Core];
                if shielded_enabled {
                    kinds.push(AddressKind::Shielded);
                }
                kinds.push(AddressKind::Identity);
                kinds
            }
            Some(SourceSelection::Identity(_)) => {
                vec![
                    AddressKind::Core,
                    AddressKind::Platform,
                    AddressKind::Identity,
                ]
            }
            Some(SourceSelection::Shielded(..)) => {
                vec![
                    AddressKind::Shielded,
                    AddressKind::Platform,
                    AddressKind::Core,
                ]
            }
            None => AddressKind::ALL.to_vec(),
        }
    }

    /// Build a fresh destination [`AddressInput`] for `allowed_kinds`, wired
    /// with wallet and identity autocomplete. Returns the widget so callers can
    /// assign it without holding a mutable borrow of `self` across the build.
    fn build_address_input(
        &self,
        allowed_kinds: &[AddressKind],
        wallets: &[WalletWithSnapshot],
    ) -> AddressInput {
        // Filter out the source identity (if any) to prevent self-sends.
        let source_identity_id = if let Some(SourceSelection::Identity(qi)) = &self.selected_source
        {
            Some(qi.identity.id())
        } else {
            None
        };
        let loaded_identities: Vec<_> = self
            .get_loaded_identities()
            .into_iter()
            .filter(|qi| Some(qi.identity.id()) != source_identity_id)
            .collect();

        let mut builder = AddressInput::new(self.app_context.network)
            .with_label("Send to")
            .with_address_kinds(allowed_kinds)
            .with_exclude_change(true);

        if !wallets.is_empty() {
            builder = builder.with_wallets(wallets);
        }

        // Add identities for autocomplete (searchable by alias/DPNS name).
        if !loaded_identities.is_empty() {
            builder = builder.with_identities(&loaded_identities);
        }

        builder
    }

    fn address_input_wallets(&self) -> Vec<WalletWithSnapshot> {
        self.app_context
            .wallets
            .read()
            .map(|wallets| {
                wallets
                    .values()
                    .map(|wallet| {
                        let seed_hash = wallet
                            .read()
                            .map(|guard| guard.seed_hash())
                            .unwrap_or_default();
                        (
                            wallet.clone(),
                            self.app_context.snapshot_address_balances(&seed_hash),
                            self.app_context.snapshot_address_paths(&seed_hash),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn address_input_snapshot_signature(wallets: &[WalletWithSnapshot]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for (wallet, balances, paths) in wallets {
            let Ok(wallet) = wallet.read() else {
                continue;
            };
            wallet.seed_hash().hash(&mut hasher);
            wallet.alias.hash(&mut hasher);
            balances.hash(&mut hasher);
            for (address, path) in paths {
                address.hash(&mut hasher);
                for child in path {
                    child.hash(&mut hasher);
                }
                if let Some(info) = wallet.platform_address_info.get(address) {
                    info.balance.hash(&mut hasher);
                    info.nonce.hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    /// Render the destination address input for `allowed_kinds`. Lazily builds
    /// the widget and refreshes its entries when snapshot-backed sources change.
    fn render_destination_input_with_kinds(&mut self, ui: &mut Ui, allowed_kinds: &[AddressKind]) {
        let wallets = self.address_input_wallets();
        let signature = Self::address_input_snapshot_signature(&wallets);
        if self.address_input_snapshot_signature != Some(signature) {
            if let Some(address_input) = &mut self.address_input {
                address_input.set_wallets(&wallets);
            }
            self.address_input_snapshot_signature = Some(signature);
        }
        if self.address_input.is_none() {
            self.address_input = Some(self.build_address_input(allowed_kinds, &wallets));
        }
        let resp = self
            .address_input
            .as_mut()
            .expect("invariant: address_input set to Some immediately above")
            .show(ui);
        resp.inner.update(&mut self.validated_destination);
    }

    fn render_destination_input(&mut self, ui: &mut Ui) {
        let kinds = self.general_destination_kinds();
        self.render_destination_input_with_kinds(ui, &kinds);
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        let fee_estimator = self.app_context.fee_estimator();

        // Get max amount and hint based on source selection
        let (max_amount_credits, max_hint) = match &self.selected_source {
            Some(SourceSelection::CoreWallet) => {
                let mut max = self.selected_wallet.as_ref().and_then(|w| {
                    w.read().ok().map(|wallet| {
                        // Reserve against the spendable set (confirmed +
                        // unconfirmed), not `total` — `total` counts immature
                        // and locked funds coin selection can't spend, so a
                        // total-based Max over-shoots and the send fails.
                        self.app_context
                            .snapshot_balance(&wallet.seed_hash())
                            .spendable()
                            * CREDITS_PER_DUFF // duffs to credits
                    })
                });
                let dest_kind = self.destination_kind();
                let hint = match dest_kind {
                    Some(AddressKind::Platform) => {
                        let destination = self
                            .validated_destination
                            .as_ref()
                            .and_then(|v| v.as_platform().copied());
                        if let Some(destination) = destination {
                            let estimated_fee = estimate_address_funding_fee_from_transition(
                                self.app_context.platform_version(),
                                &destination,
                            );
                            max = max.map(|amount| amount.saturating_sub(estimated_fee));
                            let fee = format_credits_as_dash(estimated_fee);
                            Some(format!(
                                "A Platform fee of approximately {fee} is reserved from your balance and deducted from the amount."
                            ))
                        } else {
                            None
                        }
                    }
                    Some(AddressKind::Shielded) => {
                        let (platform_fee_duffs, l1_tx_fee_duffs) =
                            fee_estimator.estimate_shield_from_core_fees_duffs();
                        let total_fee_credits =
                            (platform_fee_duffs + l1_tx_fee_duffs) * CREDITS_PER_DUFF;
                        max = max.map(|amount| amount.saturating_sub(total_fee_credits));
                        let fee = format_credits_as_dash(total_fee_credits);
                        Some(format!(
                            "Shielding fees of approximately {fee} are reserved from your balance."
                        ))
                    }
                    Some(AddressKind::Core) => {
                        // Core-to-Core "Max": reserve the L1 network fee so the
                        // send leaves enough to cover it. The fee scales with
                        // the wallet's UTXO count (a Max send spends them all)
                        // into a single recipient output with no change.
                        let seed_hash = self
                            .selected_wallet
                            .as_ref()
                            .and_then(|w| w.read().ok().map(|wallet| wallet.seed_hash()));
                        if let Some(seed_hash) = seed_hash {
                            let spendable_balance_duffs =
                                self.app_context.snapshot_balance(&seed_hash).spendable();
                            let utxo_count = self.app_context.snapshot_utxo_count(&seed_hash);
                            match core_max_send_amount_duffs(spendable_balance_duffs, utxo_count, 1)
                            {
                                Some(send_amount_duffs) => {
                                    max = Some(send_amount_duffs * CREDITS_PER_DUFF);
                                    let fee_duffs = core_max_send_reserve_duffs(
                                        spendable_balance_duffs,
                                        utxo_count,
                                        1,
                                    )
                                    .unwrap_or(0);
                                    let fee = format_credits_as_dash(fee_duffs * CREDITS_PER_DUFF);
                                    Some(format!(
                                        "A network fee of approximately {fee} is reserved from your balance."
                                    ))
                                }
                                None => {
                                    // Balance does not cover the network fee.
                                    // Leave no Max value so the button cannot
                                    // produce an amount that would fail.
                                    max = None;
                                    Some(
                                        "Your balance is too low to cover the network fee."
                                            .to_string(),
                                    )
                                }
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                (max, hint)
            }
            Some(SourceSelection::PlatformAddresses(addresses))
                if self.destination_kind() == Some(AddressKind::Shielded) =>
            {
                // Shield-from-Platform: the coordinator selects the inputs and
                // the shield fee is paid from the same balance as the amount, so
                // reserve the two-action shielded-fee headroom (far larger than
                // the plain platform-transfer estimate) against the full balance.
                let total: u64 = addresses.iter().map(|(_, _, balance)| *balance).sum();
                let headroom = shield_from_balance_fee_headroom(
                    self.app_context.platform_version(),
                    self.app_context.fee_multiplier_permille(),
                );
                let fee = format_credits_as_dash(headroom);
                (
                    Some(total.saturating_sub(headroom)),
                    Some(format!(
                        "A shielding fee of approximately {fee} is reserved from your balance."
                    )),
                )
            }
            Some(SourceSelection::PlatformAddresses(addresses)) => {
                // Extract destination to exclude it from max calculation (can't send to yourself)
                let destination = self
                    .validated_destination
                    .as_ref()
                    .and_then(|v| v.as_platform().copied());

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
                let fee = format_credits_as_dash(max_fee);
                let hint = if sorted_addresses.len() > MAX_PLATFORM_INPUTS {
                    format!(
                        "This transaction is limited to {MAX_PLATFORM_INPUTS} input addresses, and fees of approximately {fee} are reserved from your balance."
                    )
                } else {
                    format!("Fees of approximately {fee} are reserved from your balance.")
                };
                (Some(total.saturating_sub(max_fee)), Some(hint))
            }
            Some(SourceSelection::Shielded(_, balance)) => (
                Some(*balance),
                Some("The maximum is based on your shielded pool balance.".to_string()),
            ),
            Some(SourceSelection::Identity(qi)) => {
                let balance = qi.identity.balance();
                let estimated_fee = fee_estimator.estimate_credit_transfer();
                let available = balance.saturating_sub(estimated_fee);
                let fee = format_credits_as_dash(estimated_fee);
                (
                    Some(available),
                    Some(format!(
                        "Fees of approximately {fee} are reserved from your balance."
                    )),
                )
            }
            None => (None, None),
        };

        let input_kind = match self.selected_source {
            Some(SourceSelection::CoreWallet) => Some(AddressKind::Core),
            Some(SourceSelection::PlatformAddresses(_)) => Some(AddressKind::Platform),
            Some(SourceSelection::Identity(_)) => Some(AddressKind::Platform), // credits like platform
            Some(SourceSelection::Shielded(_, _)) => Some(AddressKind::Shielded),
            None => None,
        };
        let output_kind = self.destination_kind();
        let min_amount = self.min_output_amount(input_kind, output_kind);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| {
                let amount_input = self.amount_input.get_or_insert_with(|| {
                    AmountInput::new(Amount::new_dash(0.0))
                        .with_label("Amount (DASH):")
                        .with_hint_text("Enter amount")
                        .with_max_button(true)
                        .with_desired_width(150.0)
                });

                // Update the amount limits and reserve caption dynamically.
                amount_input.set_max_amount(max_amount_credits);
                amount_input.set_caption(max_hint);
                amount_input.set_min_amount(min_amount);

                let response = amount_input.show(ui);
                response.inner.update(&mut self.amount);
            });

        // Show transaction type hint
        let tx_type = self.get_transaction_type_description();
        if tx_type != "Send" && self.validated_destination.is_some() {
            ui.add_space(5.0);
            ui.label(
                RichText::new(format!("Transaction type: {}", tx_type))
                    .color(DashColors::text_secondary(dark_mode))
                    .italics()
                    .size(12.0),
            );
        }
    }

    /// Renders a breakdown of which platform addresses will be used and how much from each.
    /// Uses the same allocation algorithm as the actual send logic.
    fn render_platform_source_breakdown(&self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
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

        // Extract destination platform address (if valid) to exclude it from inputs
        let destination = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied());

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
                                RichText::new(format_credits_as_dash(*use_amount))
                                    .color(DashColors::success_color(dark_mode))
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

    /// Estimate the network fee and total for the current simple-mode
    /// selection (source + destination + amount), in credits.
    ///
    /// All fee numbers come from `model::fee_estimation` — the same estimators
    /// the amount field's "Max" reserve uses — so the preview and the reserve
    /// agree. Returns `None` for a combination whose fee cannot be estimated
    /// before send (identity top-ups and shielded spends, where the fee depends
    /// on inputs the backend selects at dispatch time); the caller then shows a
    /// neutral "calculated when you send" note instead of a wrong number.
    fn current_fee_preview(&self) -> Option<FeePreview> {
        let amount_credits = self.amount.as_ref().map(|a| a.value()).filter(|v| *v > 0)?;
        let dest_kind = self.destination_kind()?;
        let fee_estimator = self.app_context.fee_estimator();

        match (self.selected_source.as_ref()?, dest_kind) {
            // Core → Core: L1 network fee, paid on top (reserved from balance).
            (SourceSelection::CoreWallet, AddressKind::Core) => {
                let seed_hash = self.selected_wallet_seed_hash?;
                let utxo_count = self.app_context.snapshot_utxo_count(&seed_hash);
                let fee_duffs = estimate_core_l1_send_fee_duffs(utxo_count.max(1), 1);
                let fee_credits = fee_duffs.saturating_mul(CREDITS_PER_DUFF);
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Core → Platform: address-funding fee, deducted from the amount.
            (SourceSelection::CoreWallet, AddressKind::Platform) => {
                let destination = self
                    .validated_destination
                    .as_ref()
                    .and_then(|v| v.as_platform().copied())?;
                let fee_credits = estimate_address_funding_fee_from_transition(
                    self.app_context.platform_version(),
                    &destination,
                );
                Some(FeePreview::deducted_from_amount(
                    amount_credits,
                    fee_credits,
                ))
            }
            // Core → Shielded: platform + L1 shield fees, paid on top.
            (SourceSelection::CoreWallet, AddressKind::Shielded) => {
                let (platform_fee_duffs, l1_tx_fee_duffs) =
                    fee_estimator.estimate_shield_from_core_fees_duffs();
                let fee_credits = platform_fee_duffs
                    .saturating_add(l1_tx_fee_duffs)
                    .saturating_mul(CREDITS_PER_DUFF);
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Platform → Platform: credit-transfer fee, paid on top.
            (SourceSelection::PlatformAddresses(addresses), AddressKind::Platform) => {
                let destination = self
                    .validated_destination
                    .as_ref()
                    .and_then(|v| v.as_platform().copied());
                let allocation = allocate_platform_addresses(
                    &fee_estimator,
                    addresses,
                    amount_credits,
                    destination.as_ref(),
                );
                Some(FeePreview::on_top(amount_credits, allocation.estimated_fee))
            }
            // Platform → Core: withdrawal fee, paid on top.
            (SourceSelection::PlatformAddresses(addresses), AddressKind::Core) => {
                let dest = self
                    .validated_destination
                    .as_ref()
                    .and_then(|v| v.as_core())?;
                let output_script = CoreScript::new(dest.script_pubkey());
                let platform_version = self.app_context.platform_version();
                let allocation = allocate_platform_addresses_with_fee(
                    addresses,
                    amount_credits,
                    None,
                    |inputs| {
                        estimate_withdrawal_fee_from_transition(
                            platform_version,
                            inputs,
                            &output_script,
                        )
                    },
                );
                Some(FeePreview::on_top(amount_credits, allocation.estimated_fee))
            }
            // Platform → Shielded: two-action shield fee headroom, paid on top.
            (SourceSelection::PlatformAddresses(_), AddressKind::Shielded) => {
                let fee_credits = shield_from_balance_fee_headroom(
                    self.app_context.platform_version(),
                    self.app_context.fee_multiplier_permille(),
                );
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Identity → Core: credit-withdrawal fee, paid on top.
            (SourceSelection::Identity(_), AddressKind::Core) => {
                let fee_credits = fee_estimator.estimate_address_credit_withdrawal();
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Identity → Platform / Identity: credit-transfer fee, paid on top.
            (SourceSelection::Identity(_), AddressKind::Platform | AddressKind::Identity) => {
                let fee_credits = fee_estimator.estimate_credit_transfer();
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Identity → Shielded, Shielded → anything, Core → Identity: the fee
            // depends on inputs the backend selects at send time.
            _ => None,
        }
    }

    /// Render the fee/total summary shown above the simple-mode Send button.
    ///
    /// Only rendered once a destination and a positive amount are set, so the
    /// numbers reflect a concrete send. Shows the estimated network fee, the
    /// total debited from the balance, and (when the fee is taken out of the
    /// amount) what the recipient actually receives.
    fn render_fee_summary(&self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

        let has_amount = self.amount.as_ref().map(|a| a.value() > 0).unwrap_or(false);
        if !has_amount || self.validated_destination.is_none() {
            return;
        }

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| match self.current_fee_preview() {
                Some(preview) => {
                    egui::Grid::new("send_fee_summary_grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            fee_summary_row(
                                ui,
                                dark_mode,
                                "Estimated network fee:",
                                preview.fee_credits,
                                true,
                            );
                            if let Some(recipient_receives) = preview.recipient_receives_credits {
                                fee_summary_row(
                                    ui,
                                    dark_mode,
                                    "Recipient receives:",
                                    recipient_receives,
                                    false,
                                );
                            }
                            fee_summary_row(
                                ui,
                                dark_mode,
                                "Total deducted:",
                                preview.total_debit_credits,
                                true,
                            );
                        });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Fees are estimated; the exact amount is confirmed when you send.",
                        )
                        .color(DashColors::text_secondary(dark_mode))
                        .italics()
                        .size(11.0),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("The network fee is calculated when you send.")
                            .color(DashColors::text_secondary(dark_mode))
                            .italics()
                            .size(12.0),
                    );
                }
            });
    }

    /// Estimate the advanced-mode network fee, in credits, from the entered
    /// input and output counts.
    ///
    /// Covers the two advanced paths whose fee scales with input/output count
    /// alone — Core → Core and Platform → Platform. Returns `None` for mixed or
    /// cross-network output sets, where the fee model differs; the caller then
    /// shows the neutral "calculated when you send" note.
    fn advanced_fee_estimate_credits(&self) -> Option<u64> {
        let row_output_count = self
            .advanced_outputs
            .iter()
            .filter(|o| !o.address.trim().is_empty())
            .count();
        if row_output_count == 0 {
            return None;
        }
        let has_core_out = self
            .advanced_outputs
            .iter()
            .any(|o| self.detect_address_kind(&o.address) == Some(AddressKind::Core));
        let has_platform_out = self
            .advanced_outputs
            .iter()
            .any(|o| self.detect_address_kind(&o.address) == Some(AddressKind::Platform));

        match self.advanced_source_type {
            AdvancedSourceType::Core if has_core_out && !has_platform_out => {
                let num_inputs = self
                    .core_inputs
                    .iter()
                    .filter(|i| !i.amount.trim().is_empty())
                    .count()
                    .max(1);
                let fee_duffs = estimate_core_l1_send_fee_duffs(num_inputs, row_output_count);
                Some(fee_duffs.saturating_mul(CREDITS_PER_DUFF))
            }
            AdvancedSourceType::Platform if has_platform_out && !has_core_out => {
                let outputs = Self::normalize_advanced_platform_outputs(&self.advanced_outputs)
                    .ok()
                    .filter(|outputs| !outputs.is_empty())?;
                let num_inputs = self
                    .platform_inputs
                    .iter()
                    .filter(|i| !i.amount.trim().is_empty())
                    .count()
                    .max(1);
                Some(estimate_platform_fee(
                    &self.app_context.fee_estimator(),
                    num_inputs,
                    outputs.len(),
                ))
            }
            _ => None,
        }
    }

    fn normalize_advanced_platform_outputs(
        advanced_outputs: &[AdvancedOutput],
    ) -> Result<BTreeMap<PlatformAddress, Credits>, TaskError> {
        let mut outputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        for output in advanced_outputs {
            let destination =
                PlatformAddress::from_bech32m_string(output.address.trim()).map_err(|source| {
                    TaskError::AdvancedPlatformOutputAddressInvalid {
                        source_error: Box::new(SdkError::Protocol(source)),
                    }
                })?;
            let credits = Self::parse_amount_to_credits(&output.amount)
                .map_err(|_| TaskError::AdvancedPlatformAmountInvalid)?;
            if credits > 0 {
                let total = outputs.entry(destination).or_insert(0);
                *total = total
                    .checked_add(credits)
                    .ok_or(TaskError::AdvancedPlatformOutputsOverflow)?;
            }
        }
        Ok(outputs)
    }

    fn normalize_advanced_platform_inputs(
        advanced_inputs: &[PlatformAddressInput],
    ) -> Result<BTreeMap<PlatformAddress, Credits>, TaskError> {
        let mut inputs: BTreeMap<PlatformAddress, Credits> = BTreeMap::new();
        for input in advanced_inputs {
            let credits = Self::parse_amount_to_credits(&input.amount)
                .map_err(|_| TaskError::AdvancedPlatformAmountInvalid)?;
            if credits > 0 {
                let total = inputs.entry(input.platform_address).or_insert(0);
                *total = total
                    .checked_add(credits)
                    .ok_or(TaskError::AdvancedPlatformInputsOverflow)?;
            }
        }
        Ok(inputs)
    }

    /// Render the estimated-fee line shown above the advanced-mode Send button.
    fn render_advanced_fee_summary(&self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(12, 10))
            .corner_radius(5.0)
            .show(ui, |ui| match self.advanced_fee_estimate_credits() {
                Some(fee_credits) => {
                    egui::Grid::new("advanced_send_fee_summary_grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            fee_summary_row(
                                ui,
                                dark_mode,
                                "Estimated network fee:",
                                fee_credits,
                                true,
                            );
                        });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Fees are estimated and added on top of your output amounts.",
                        )
                        .color(DashColors::text_secondary(dark_mode))
                        .italics()
                        .size(11.0),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("The network fee is calculated when you send.")
                            .color(DashColors::text_secondary(dark_mode))
                            .italics()
                            .size(12.0),
                    );
                }
            });
    }

    fn confirmation_fee_sentence(estimated_fee_credits: Option<u64>) -> String {
        match estimated_fee_credits {
            Some(fee) => {
                let estimated_fee = format_credits_as_dash(fee);
                format!("The estimated network fee is approximately {estimated_fee}.")
            }
            None => "The network fee will be calculated when you confirm this send.".to_string(),
        }
    }

    fn simple_send_confirmation_message(&self) -> String {
        let destination = self.destination_address_string();
        let amount = self
            .amount
            .as_ref()
            .map(|amount| format_credits_as_dash(amount.value()))
            .unwrap_or_else(|| "0 DASH".to_string());
        let fee_preview = self.current_fee_preview();
        let fee = Self::confirmation_fee_sentence(fee_preview.map(|preview| preview.fee_credits));

        if let Some(recipient_receives_credits) =
            fee_preview.and_then(|preview| preview.recipient_receives_credits)
        {
            let recipient_amount = format_credits_as_dash(recipient_receives_credits);
            return format!(
                "You are about to send an entered amount of {amount} to {destination}. The network fee will be deducted from the output amount, so the recipient will receive {recipient_amount}. {fee} Confirm this transaction only if the destination, amount, and fee are correct."
            );
        }

        format!(
            "You are about to send {amount} to {destination}. {fee} Confirm this transaction only if the destination, amount, and fee are correct."
        )
    }

    fn advanced_send_confirmation_message(&self) -> String {
        let recipients: Vec<_> = self
            .advanced_outputs
            .iter()
            .filter_map(|output| {
                let amount_credits = match self.detect_address_kind(&output.address) {
                    Some(AddressKind::Core) => Self::parse_amount_to_duffs(&output.amount)
                        .ok()
                        .map(|duffs| duffs.saturating_mul(CREDITS_PER_DUFF)),
                    Some(AddressKind::Platform) => {
                        Self::parse_amount_to_credits(&output.amount).ok()
                    }
                    _ => None,
                }?;
                (amount_credits > 0).then(|| {
                    (
                        output.address.trim().to_string(),
                        format_credits_as_dash(amount_credits),
                    )
                })
            })
            .collect();
        let destinations = recipients
            .iter()
            .enumerate()
            .map(|(index, (destination, amount))| {
                let output_number = index + 1;
                format!(
                    "Output {output_number} sends an entered amount of {amount} to {destination}."
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let fee = Self::confirmation_fee_sentence(self.advanced_fee_estimate_credits());
        let output_fee_effect = if self.advanced_source_type == AdvancedSourceType::Core
            && self.advanced_outputs.iter().any(|output| {
                self.detect_address_kind(&output.address) == Some(AddressKind::Platform)
            }) {
            if matches!(
                self.fee_strategy,
                PlatformFeeStrategy::ReduceFirstOutput | PlatformFeeStrategy::ReduceLastOutput
            ) {
                "The network fee will be deducted from the output amount, so the recipient will receive less than the entered amount."
            } else {
                "The network fee will be paid from the selected inputs, so the recipient will receive the entered amount."
            }
        } else {
            ""
        };
        let fee_details = if output_fee_effect.is_empty() {
            fee
        } else {
            format!("{output_fee_effect} {fee}")
        };

        format!(
            "You are about to send the following outputs.\n{destinations}\n{fee_details} Confirm this transaction only if every destination, amount, and fee is correct."
        )
    }

    fn open_send_confirmation(&mut self, action: AppAction, message: String) {
        self.send_confirmation = Some(PendingSendConfirmation {
            dialog: ConfirmationDialog::new("Confirm this send.", message)
                .confirm_text(Some("Confirm and Send"))
                .cancel_text(Some("Cancel"))
                .danger_mode(true)
                .blocks_input(true),
            action: Box::new(action),
        });
    }

    fn render_send_confirmation(&mut self, ui: &mut Ui) -> AppAction {
        let response = self
            .send_confirmation
            .as_mut()
            .and_then(|pending| pending.dialog.show(ui).inner.dialog_response);

        match response {
            Some(ConfirmationStatus::Confirmed) => {
                let Some(pending) = self.send_confirmation.take() else {
                    return AppAction::None;
                };
                self.mark_sending();
                self.set_send_progress_banner(ui.ctx());
                *pending.action
            }
            Some(ConfirmationStatus::Canceled) => {
                self.send_confirmation = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    fn render_send_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let wallet_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|w| w.read().map(|g| g.is_open()).unwrap_or(false));

        let has_destination = self.validated_destination.is_some();
        let has_amount = self.amount.as_ref().map(|a| a.value() > 0).unwrap_or(false);
        let has_source = self.selected_source.is_some();

        let is_sending = matches!(self.send_status, SendStatus::WaitingForResult);
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
                        let message = self.simple_send_confirmation_message();
                        self.open_send_confirmation(send_action, message);
                    }
                    Err(e) => {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
                        self.send_status = SendStatus::Error;
                    }
                }
            }
        });

        action
    }

    /// Render the advanced send UI with multiple inputs/outputs
    fn render_advanced_send(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

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
        let has_platform_output = self
            .advanced_outputs
            .iter()
            .any(|o| self.detect_address_kind(&o.address) == Some(AddressKind::Platform));

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

        // Fee estimate, shown before the Send button (SND-005).
        self.render_advanced_fee_summary(ui);

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // ========== SEND BUTTON ==========
        action |= self.render_advanced_send_button(ui);

        action
    }

    /// Render Core address inputs for advanced mode
    fn render_core_inputs(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
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
                                RichText::new(format!("({})", format_duffs_as_dash(balance)))
                                    .color(DashColors::success_color(dark_mode))
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
                            format_duffs_as_dash(*balance)
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
        let dark_mode = ui.style().visuals.dark_mode;
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
                                RichText::new(format!("({})", format_credits_as_dash(balance)))
                                    .color(DashColors::success_color(dark_mode))
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
                    for (_core_addr, platform_addr, balance) in available_addresses {
                        let addr_str = platform_addr.to_bech32m_string(network);
                        let display = format!(
                            "{}... ({})",
                            &addr_str[..20.min(addr_str.len())],
                            format_credits_as_dash(*balance)
                        );
                        if ui.selectable_label(false, display).clicked() {
                            self.platform_inputs.push(PlatformAddressInput {
                                platform_address: *platform_addr,
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
        let dark_mode = ui.style().visuals.dark_mode;
        let mut outputs_to_remove = Vec::new();
        let num_outputs = self.advanced_outputs.len();

        // Pre-compute address kinds to avoid borrow issues
        let addr_kinds: Vec<Option<AddressKind>> = self
            .advanced_outputs
            .iter()
            .map(|o| self.detect_address_kind(&o.address))
            .collect();

        for (idx, &addr_kind) in addr_kinds.iter().enumerate() {
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
                                    .hint_text("Enter address (X.../y.../dash1.../tdash1...)")
                                    .desired_width(350.0),
                            );

                            // Show detected type
                            if let Some(kind) = addr_kind {
                                let (type_text, type_color) = match kind {
                                    AddressKind::Core => ("Core", DashColors::DASH_BLUE),
                                    AddressKind::Platform => {
                                        ("Platform", DashColors::PLATFORM_PURPLE)
                                    }
                                    AddressKind::Shielded => {
                                        ("Shielded", DashColors::success_color(dark_mode))
                                    }
                                    AddressKind::Identity => {
                                        ("Identity", DashColors::PLATFORM_PURPLE)
                                    }
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

        let is_sending = matches!(self.send_status, SendStatus::WaitingForResult);

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
                        let message = self.advanced_send_confirmation_message();
                        self.open_send_confirmation(send_action, message);
                    }
                    Err(e) => {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
                        self.send_status = SendStatus::Error;
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

        // Determine output kinds
        let output_kinds: Vec<Option<AddressKind>> = self
            .advanced_outputs
            .iter()
            .map(|o| self.detect_address_kind(&o.address))
            .collect();

        if output_kinds
            .iter()
            .any(|kind| !matches!(kind, Some(AddressKind::Core | AddressKind::Platform)))
        {
            return Err(
                "Each output must use a valid Core or Platform address. Correct the invalid destination and try again."
                    .to_string(),
            );
        }

        let has_core_output = output_kinds.contains(&Some(AddressKind::Core));
        let has_platform_output = output_kinds.contains(&Some(AddressKind::Platform));

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
                format_duffs_as_dash(total_output),
                format_duffs_as_dash(total_input)
            ));
        }

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet,
                request: WalletPaymentRequest {
                    recipients,
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
                format_duffs_as_dash(amount_duffs),
                format_duffs_as_dash(total_input)
            ));
        }

        // Parse platform address
        let address_str = output.address.trim();
        let destination = PlatformAddress::from_bech32m_string(address_str)
            .map_err(|e| format!("Invalid platform address: {}", e))?;

        // Determine fee strategy based on user selection
        // DeductFromInput variants mean fees are paid from wallet (recipient gets exact amount)
        // ReduceOutput variants mean fees are deducted from output (recipient gets less)
        let fee_deduct_from_output = matches!(
            self.fee_strategy,
            PlatformFeeStrategy::ReduceFirstOutput | PlatformFeeStrategy::ReduceLastOutput
        );

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
        let inputs = Self::normalize_advanced_platform_inputs(&self.platform_inputs)
            .map_err(|error| error.to_string())?;

        if inputs.is_empty() {
            return Err("No valid Platform inputs specified".to_string());
        }

        let outputs = Self::normalize_advanced_platform_outputs(&self.advanced_outputs)
            .map_err(|error| error.to_string())?;

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

        let inputs = Self::normalize_advanced_platform_inputs(&self.platform_inputs)
            .map_err(|error| error.to_string())?;

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

    fn task_result_updates_address_input(result: &BackendTaskSuccessResult) -> bool {
        matches!(
            result,
            BackendTaskSuccessResult::GeneratedReceiveAddress { .. }
                | BackendTaskSuccessResult::GeneratedPlatformReceiveAddress { .. }
                | BackendTaskSuccessResult::PlatformAddressBalances { .. }
                | BackendTaskSuccessResult::PlatformAddressSyncPushed { .. }
                | BackendTaskSuccessResult::RefreshedWallet { .. }
        )
    }
}

impl ScreenLike for WalletSendScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;

        action |= add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Wallets", AppAction::PopScreen),
                (self.flow.heading(), AppAction::None),
            ],
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

            if let Some(status_action) = self.render_send_status(ui) {
                return status_action;
            }

            let form_enabled = self.send_confirmation.is_none();
            ui.add_enabled_ui(form_enabled, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([true; 2])
                    .show(ui, |ui| {
                        // Heading. Advanced options apply to the free-form send only;
                        // shielded presets hide the toggle.
                        ui.horizontal(|ui| {
                            ui.heading(
                                RichText::new(self.flow.heading())
                                    .color(DashColors::text_primary(dark_mode))
                                    .size(24.0),
                            );
                            if !self.flow.is_preset() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.checkbox(
                                            &mut self.show_advanced_options,
                                            "Advanced Options",
                                        );
                                    },
                                );
                            }
                        });

                        if let Some(description) = self.flow.description() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(description)
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        }

                        ui.add_space(15.0);

                        if self.flow.is_preset() {
                            inner_action |= self.render_flow_send(ui);
                        } else if self.show_advanced_options {
                            inner_action |= self.render_advanced_send(ui);
                        } else {
                            inner_action |= self.render_unified_send(ui);
                        }
                    });
            });

            inner_action
        });

        action |= self.render_send_confirmation(ui);

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
        // Banner display is handled globally by AppState; this is only for side-effects.
        match message_type {
            MessageType::Error | MessageType::Warning => {
                self.send_banner.take_and_clear();
                self.send_status = SendStatus::Error;
            }
            MessageType::Success => {
                self.send_banner.take_and_clear();
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
        self.send_banner.take_and_clear();
        if Self::task_result_updates_address_input(&backend_task_success_result) {
            self.address_input_snapshot_signature = None;
        }
        match backend_task_success_result {
            crate::backend_task::BackendTaskSuccessResult::WalletPayment {
                txid: _,
                recipients,
                total_amount,
            } => {
                let msg = if recipients.len() == 1 {
                    let (address, amount) = &recipients[0];
                    format!("Sent {} to {}", format_duffs_as_dash(*amount), address,)
                } else {
                    format!(
                        "Sent {} to {} recipients",
                        format_duffs_as_dash(total_amount),
                        recipients.len(),
                    )
                };
                self.send_status = SendStatus::Complete(msg);
            }
            crate::backend_task::BackendTaskSuccessResult::TransferredCredits(fee_result) => {
                let fee_info = Self::format_fee_info(&fee_result);
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
            crate::backend_task::BackendTaskSuccessResult::ShieldedTransferComplete {
                amount,
                ..
            } => {
                self.send_status = SendStatus::Complete(format!(
                    "Shielded transfer of {} complete!\n\n\
                     Your remaining balance will update after the next block is confirmed. \
                     The recipient's balance will also update after the next block and a wallet sync.",
                    format_credits_as_dash(amount)
                ));
            }
            crate::backend_task::BackendTaskSuccessResult::ShieldedCreditsUnshielded {
                amount,
                ..
            } => {
                self.send_status = SendStatus::Complete(format!(
                    "Unshielded {} to platform address!\n\n\
                     Your remaining balance will update after the next block is confirmed.",
                    format_credits_as_dash(amount)
                ));
            }
            // Core->Identity or Platform->Identity top-up result
            crate::backend_task::BackendTaskSuccessResult::ToppedUpIdentity(
                _identity,
                fee_result,
            ) => {
                let fee_info = Self::format_fee_info(&fee_result);
                self.send_status =
                    SendStatus::Complete(format!("Identity topped up successfully!{}", fee_info));
            }
            // Identity->Core withdrawal result
            crate::backend_task::BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) => {
                let fee_info = Self::format_fee_info(&fee_result);
                self.send_status = SendStatus::Complete(format!(
                    "Identity withdrawal initiated. Funds will appear on the Core chain after confirmation.{}",
                    fee_info
                ));
            }
            // Core->Shielded or Platform->Shielded shield result
            crate::backend_task::BackendTaskSuccessResult::ShieldedCreditsShielded {
                amount,
                ..
            } => {
                self.send_status = SendStatus::Complete(format!(
                    "{} shielded successfully!\n\n\
                     Balance will update after the next block.",
                    format_credits_as_dash(amount)
                ));
            }
            // Core->Shielded via asset lock result
            crate::backend_task::BackendTaskSuccessResult::ShieldedFromAssetLock {
                amount, ..
            } => {
                self.send_status = SendStatus::Complete(format!(
                    "{} shielded from asset lock successfully!\n\n\
                     Balance will update after the next block.",
                    format_credits_as_dash(amount)
                ));
            }
            // Shielded->Core withdrawal result
            crate::backend_task::BackendTaskSuccessResult::ShieldedWithdrawalComplete {
                amount,
                ..
            } => {
                self.send_status = SendStatus::Complete(format!(
                    "Withdrawal of {} from shielded pool initiated.\n\n\
                     Funds will appear after confirmation.",
                    format_credits_as_dash(amount)
                ));
            }
            _ => {
                // Ignore other results
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.address_input_snapshot_signature = None;
    }

    fn refresh(&mut self) {
        self.address_input_snapshot_signature = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dashcore_rpc::dashcore::secp256k1::{Secp256k1, SecretKey};
    use dash_sdk::dashcore_rpc::dashcore::{Network, PrivateKey, PublicKey};
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn testnet_core_address(key_byte: u8) -> Address {
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&[key_byte; 32]).expect("valid secret key");
        let private_key = PrivateKey::new(secret_key, Network::Testnet);
        let public_key = PublicKey::from_private_key(&secp, &private_key);
        Address::p2pkh(&public_key, Network::Testnet)
    }

    fn bip44_receive_path(index: u32) -> DerivationPath {
        DerivationPath::from(
            [
                ChildNumber::Hardened { index: 44 },
                ChildNumber::Hardened { index: 1 },
                ChildNumber::Hardened { index: 0 },
                ChildNumber::Normal { index: 0 },
                ChildNumber::Normal { index },
            ]
            .as_slice(),
        )
    }

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

    fn send_screen() -> (WalletSendScreen, tempfile::TempDir) {
        let (ctx, temp_dir) = offline_ctx();
        let wallet = Wallet::new_from_seed(
            [1u8; 64],
            Network::Testnet,
            Some("Test wallet".to_string()),
            None,
        )
        .expect("wallet from seed");
        (
            WalletSendScreen::new(&ctx, Arc::new(RwLock::new(wallet))),
            temp_dir,
        )
    }

    fn click_in_one_frame(harness: &mut Harness<'_, WalletSendScreen>, label: &str) {
        let pos = harness.get_by_label(label).rect().center();
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
    }

    #[test]
    fn simple_send_click_opens_confirmation_without_dispatching() {
        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        let destination =
            PlatformAddress::try_from(testnet_core_address(3)).expect("platform destination");
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            2 * CREDITS_PER_DUFF * 100_000_000,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Platform {
            address: destination,
            bech32m: destination.to_bech32m_string(Network::Testnet),
        });
        screen.amount = Some(Amount::new_dash(1.0));

        let observed_action = Rc::new(RefCell::new(AppAction::None));
        let captured_action = observed_action.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                move |ui, screen: &mut WalletSendScreen| {
                    let form_enabled = screen.send_confirmation.is_none();
                    let mut action = ui
                        .add_enabled_ui(form_enabled, |ui| {
                            ui.checkbox(&mut screen.show_advanced_options, "Background control");
                            screen.render_send_button(ui)
                        })
                        .inner;
                    action |= screen.render_send_confirmation(ui);
                    if !matches!(action, AppAction::None) {
                        *captured_action.borrow_mut() = action;
                    }
                },
                screen,
            );
        harness.run();

        click_in_one_frame(&mut harness, "Transfer Credits");

        assert!(
            matches!(*observed_action.borrow(), AppAction::None),
            "the original simple Send click must not dispatch a backend task"
        );
        assert!(harness.query_by_label("Confirm and Send").is_some());

        harness.get_by_label("Background control").click_accesskit();
        harness.step();
        assert!(
            !harness.state().show_advanced_options,
            "the confirmation dialog must block controls behind it"
        );

        harness.get_by_label("Confirm and Send").click_accesskit();
        harness.step();

        assert!(
            harness.state().send_confirmation.is_none(),
            "the confirmation dialog must close after confirmation"
        );
        let observed_action = observed_action.borrow();
        assert!(
            matches!(*observed_action, AppAction::BackendTask(_)),
            "the confirm action must dispatch the prepared backend task, got {observed_action:?}"
        );
    }

    #[test]
    fn simple_confirmation_reports_recipient_amount_when_fee_is_deducted() {
        let (mut screen, _temp_dir) = send_screen();
        let destination_address =
            PlatformAddress::try_from(testnet_core_address(8)).expect("platform destination");
        let destination = destination_address.to_bech32m_string(Network::Testnet);
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Platform {
            address: destination_address,
            bech32m: destination.clone(),
        });
        screen.amount = Some(Amount::new_dash(1.0));

        let preview = screen.current_fee_preview().expect("fee preview");
        let amount = format_credits_as_dash(screen.amount.as_ref().expect("amount").value());
        let recipient_amount = format_credits_as_dash(
            preview
                .recipient_receives_credits
                .expect("deducted recipient amount"),
        );
        let estimated_fee = format_credits_as_dash(preview.fee_credits);

        assert_eq!(
            screen.simple_send_confirmation_message(),
            format!(
                "You are about to send an entered amount of {amount} to {destination}. The network fee will be deducted from the output amount, so the recipient will receive {recipient_amount}. The estimated network fee is approximately {estimated_fee}. Confirm this transaction only if the destination, amount, and fee are correct."
            )
        );
    }

    #[test]
    fn simple_confirmation_reports_entered_amount_when_fee_is_added_on_top() {
        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(9);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        let destination_address =
            PlatformAddress::try_from(testnet_core_address(10)).expect("platform destination");
        let destination = destination_address.to_bech32m_string(Network::Testnet);
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            2 * CREDITS_PER_DUFF * 100_000_000,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Platform {
            address: destination_address,
            bech32m: destination.clone(),
        });
        screen.amount = Some(Amount::new_dash(1.0));

        let preview = screen.current_fee_preview().expect("fee preview");
        assert_eq!(preview.recipient_receives_credits, None);
        let amount = format_credits_as_dash(screen.amount.as_ref().expect("amount").value());
        let estimated_fee = format_credits_as_dash(preview.fee_credits);

        assert_eq!(
            screen.simple_send_confirmation_message(),
            format!(
                "You are about to send {amount} to {destination}. The estimated network fee is approximately {estimated_fee}. Confirm this transaction only if the destination, amount, and fee are correct."
            )
        );
    }

    #[test]
    fn advanced_send_click_opens_confirmation_without_dispatching() {
        let (mut screen, _temp_dir) = send_screen();
        let input_address = testnet_core_address(4);
        screen.advanced_source_type = AdvancedSourceType::Core;
        screen.core_inputs = vec![CoreAddressInput {
            address: input_address,
            amount: "2".to_string(),
        }];
        screen.advanced_outputs = vec![AdvancedOutput {
            address: testnet_core_address(5).to_string(),
            amount: "1".to_string(),
        }];

        let observed_action = Rc::new(RefCell::new(AppAction::None));
        let captured_action = observed_action.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                move |ui, screen: &mut WalletSendScreen| {
                    let form_enabled = screen.send_confirmation.is_none();
                    let mut action = ui
                        .add_enabled_ui(form_enabled, |ui| screen.render_advanced_send_button(ui))
                        .inner;
                    action |= screen.render_send_confirmation(ui);
                    if !matches!(action, AppAction::None) {
                        *captured_action.borrow_mut() = action;
                    }
                },
                screen,
            );
        harness.run();

        click_in_one_frame(&mut harness, "Send");

        assert!(
            matches!(*observed_action.borrow(), AppAction::None),
            "the original advanced Send click must not dispatch a backend task"
        );
        assert!(harness.query_by_label("Confirm and Send").is_some());

        harness.get_by_label("Confirm and Send").click_accesskit();
        harness.step();

        let observed_action = observed_action.borrow();
        assert!(
            matches!(*observed_action, AppAction::BackendTask(_)),
            "the confirm action must dispatch the prepared backend task, got {observed_action:?}"
        );
    }

    #[test]
    fn advanced_send_rejects_every_invalid_output_address() {
        let (mut screen, _temp_dir) = send_screen();
        screen.advanced_source_type = AdvancedSourceType::Core;
        screen.core_inputs = vec![CoreAddressInput {
            address: testnet_core_address(6),
            amount: "3".to_string(),
        }];
        screen.advanced_outputs = vec![
            AdvancedOutput {
                address: testnet_core_address(7).to_string(),
                amount: "1".to_string(),
            },
            AdvancedOutput {
                address: "not-a-valid-address".to_string(),
                amount: "1".to_string(),
            },
        ];

        let error = screen
            .validate_and_send_advanced()
            .expect_err("every advanced output must have a valid destination");

        assert_eq!(
            error,
            "Each output must use a valid Core or Platform address. Correct the invalid destination and try again."
        );
    }

    #[test]
    fn advanced_send_rejects_unsupported_output_address_kinds() {
        let (mut screen, _temp_dir) = send_screen();
        screen.advanced_source_type = AdvancedSourceType::Core;
        screen.core_inputs = vec![CoreAddressInput {
            address: testnet_core_address(9),
            amount: "3".to_string(),
        }];
        screen.advanced_outputs = vec![
            AdvancedOutput {
                address: testnet_core_address(10).to_string(),
                amount: "1".to_string(),
            },
            AdvancedOutput {
                address: "00".repeat(43),
                amount: "1".to_string(),
            },
        ];

        let error = screen
            .validate_and_send_advanced()
            .expect_err("advanced sends must reject unsupported address kinds");

        assert_eq!(
            error,
            "Each output must use a valid Core or Platform address. Correct the invalid destination and try again."
        );
    }

    #[test]
    fn advanced_confirmation_discloses_when_the_fee_reduces_the_output() {
        let (mut screen, _temp_dir) = send_screen();
        let destination = PlatformAddress::try_from(testnet_core_address(8))
            .expect("platform destination")
            .to_bech32m_string(Network::Testnet);
        screen.advanced_source_type = AdvancedSourceType::Core;
        screen.fee_strategy = PlatformFeeStrategy::ReduceFirstOutput;
        screen.advanced_outputs = vec![AdvancedOutput {
            address: destination,
            amount: "1".to_string(),
        }];

        let message = screen.advanced_send_confirmation_message();

        assert!(message.contains(
            "The network fee will be deducted from the output amount, so the recipient will receive less than the entered amount."
        ));
        assert!(!message.contains("will receive 1 DASH"));
    }

    #[test]
    fn general_flow_is_not_a_preset() {
        assert!(!SendFlow::General.is_preset());
        assert!(SendFlow::Shield.is_preset());
        assert!(SendFlow::ShieldedSend.is_preset());
        assert!(SendFlow::Unshield.is_preset());
    }

    #[test]
    fn preset_headings_match_former_standalone_screens() {
        assert_eq!(SendFlow::Shield.heading(), "Shield");
        assert_eq!(SendFlow::ShieldedSend.heading(), "Send (Private)");
        assert_eq!(SendFlow::Unshield.heading(), "Unshield Credits");
    }

    #[test]
    fn preset_destination_kinds_scope_each_flow() {
        // Shield targets the own pool — no destination input.
        assert_eq!(SendFlow::Shield.preset_destination_kinds(), None);
        // General derives kinds from the source elsewhere.
        assert_eq!(SendFlow::General.preset_destination_kinds(), None);
        // Private send accepts only shielded recipients.
        assert_eq!(
            SendFlow::ShieldedSend.preset_destination_kinds(),
            Some(vec![AddressKind::Shielded])
        );
        // Unshield exits to platform or core, never back to shielded.
        assert_eq!(
            SendFlow::Unshield.preset_destination_kinds(),
            Some(vec![AddressKind::Platform, AddressKind::Core])
        );
    }

    #[test]
    fn every_preset_has_a_description() {
        assert!(SendFlow::General.description().is_none());
        for flow in [SendFlow::Shield, SendFlow::ShieldedSend, SendFlow::Unshield] {
            let description = flow.description().expect("preset has a description");
            assert!(
                description.ends_with('.'),
                "description is a complete sentence: {description}"
            );
        }
    }

    #[test]
    fn fee_preview_on_top_adds_fee_to_the_total() {
        // Fee paid on top: the recipient gets the full amount and the balance is
        // debited amount + fee.
        let preview = FeePreview::on_top(1_000, 30);
        assert_eq!(preview.fee_credits, 30);
        assert_eq!(preview.total_debit_credits, 1_030);
        assert_eq!(preview.recipient_receives_credits, None);
    }

    #[test]
    fn fee_preview_deducted_takes_fee_from_the_amount() {
        // Fee deducted from the amount: the balance is debited exactly the
        // amount and the recipient receives amount − fee.
        let preview = FeePreview::deducted_from_amount(1_000, 30);
        assert_eq!(preview.fee_credits, 30);
        assert_eq!(preview.total_debit_credits, 1_000);
        assert_eq!(preview.recipient_receives_credits, Some(970));
    }

    #[test]
    fn fee_preview_saturates_instead_of_overflowing() {
        // A fee larger than the amount must not underflow the recipient figure,
        // and an on-top total must not overflow.
        let deducted = FeePreview::deducted_from_amount(10, 25);
        assert_eq!(deducted.recipient_receives_credits, Some(0));

        let on_top = FeePreview::on_top(u64::MAX, 5);
        assert_eq!(on_top.total_debit_credits, u64::MAX);
    }

    #[test]
    fn address_input_snapshot_signature_tracks_paths_and_balances() {
        let wallet = Arc::new(RwLock::new(
            Wallet::new_from_seed(
                [7u8; 64],
                Network::Testnet,
                Some("Wallet".to_string()),
                None,
            )
            .expect("wallet from seed"),
        ));
        let empty_wallets = vec![(wallet.clone(), BTreeMap::new(), BTreeMap::new())];
        let empty_signature = WalletSendScreen::address_input_snapshot_signature(&empty_wallets);
        assert_eq!(
            empty_signature,
            WalletSendScreen::address_input_snapshot_signature(&empty_wallets)
        );

        let address = testnet_core_address(3);
        let paths = BTreeMap::from([(address.clone(), bip44_receive_path(0))]);
        let with_path = vec![(wallet.clone(), BTreeMap::new(), paths.clone())];
        let path_signature = WalletSendScreen::address_input_snapshot_signature(&with_path);
        assert_ne!(empty_signature, path_signature);

        let with_balance = vec![(wallet, BTreeMap::from([(address, 42)]), paths)];
        assert_ne!(
            path_signature,
            WalletSendScreen::address_input_snapshot_signature(&with_balance)
        );
    }

    #[test]
    fn wallet_address_pool_results_request_address_input_refresh() {
        let seed_hash = WalletSeedHash::default();
        for result in [
            BackendTaskSuccessResult::GeneratedReceiveAddress {
                seed_hash,
                address: "core".to_string(),
            },
            BackendTaskSuccessResult::GeneratedPlatformReceiveAddress {
                seed_hash,
                address: "platform".to_string(),
            },
            BackendTaskSuccessResult::PlatformAddressBalances {
                seed_hash,
                balances: BTreeMap::new(),
                network: Network::Testnet,
            },
            BackendTaskSuccessResult::RefreshedWallet { warning: None },
        ] {
            assert!(WalletSendScreen::task_result_updates_address_input(&result));
        }
        assert!(!WalletSendScreen::task_result_updates_address_input(
            &BackendTaskSuccessResult::None
        ));
    }

    #[test]
    fn advanced_platform_outputs_coalesce_duplicates_and_drop_zero_rows() {
        let first = PlatformAddress::try_from(testnet_core_address(4))
            .expect("core address converts to platform address");
        let second = PlatformAddress::try_from(testnet_core_address(5))
            .expect("core address converts to platform address");
        let first_string = first.to_bech32m_string(Network::Testnet);
        let second_string = second.to_bech32m_string(Network::Testnet);
        let outputs = vec![
            AdvancedOutput {
                address: first_string.clone(),
                amount: "1".to_string(),
            },
            AdvancedOutput {
                address: first_string,
                amount: "2".to_string(),
            },
            AdvancedOutput {
                address: second_string,
                amount: "0".to_string(),
            },
        ];

        let normalized =
            WalletSendScreen::normalize_advanced_platform_outputs(&outputs).expect("valid outputs");
        let expected = WalletSendScreen::parse_amount_to_credits("1").expect("valid amount")
            + WalletSendScreen::parse_amount_to_credits("2").expect("valid amount");
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized.get(&first), Some(&expected));
    }

    #[test]
    fn advanced_platform_output_duplicates_reject_aggregate_overflow() {
        let address = PlatformAddress::try_from(testnet_core_address(6))
            .expect("core address converts to platform address")
            .to_bech32m_string(Network::Testnet);
        let outputs = vec![
            AdvancedOutput {
                address: address.clone(),
                amount: "100000000".to_string(),
            },
            AdvancedOutput {
                address,
                amount: "100000000".to_string(),
            },
        ];

        let error = WalletSendScreen::normalize_advanced_platform_outputs(&outputs)
            .expect_err("overflowing duplicate outputs must be rejected");
        assert!(matches!(&error, TaskError::AdvancedPlatformOutputsOverflow));
        assert_eq!(
            error.to_string(),
            "The combined outputs to one Platform address exceed the maximum amount this app can process. Reduce the amounts or remove duplicate output rows, then try again."
        );
    }

    #[test]
    fn advanced_platform_input_duplicates_coalesce_without_overflow() {
        let address = PlatformAddress::try_from(testnet_core_address(7))
            .expect("core address converts to platform address");
        let inputs = vec![
            PlatformAddressInput {
                platform_address: address,
                amount: "1".to_string(),
            },
            PlatformAddressInput {
                platform_address: address,
                amount: "2".to_string(),
            },
        ];

        let normalized =
            WalletSendScreen::normalize_advanced_platform_inputs(&inputs).expect("valid inputs");
        let expected = WalletSendScreen::parse_amount_to_credits("1").expect("valid amount")
            + WalletSendScreen::parse_amount_to_credits("2").expect("valid amount");
        assert_eq!(normalized, BTreeMap::from([(address, expected)]));
    }

    #[test]
    fn advanced_platform_input_duplicates_reject_aggregate_overflow() {
        let address = PlatformAddress::try_from(testnet_core_address(8))
            .expect("core address converts to platform address");
        let inputs = vec![
            PlatformAddressInput {
                platform_address: address,
                amount: "100000000".to_string(),
            },
            PlatformAddressInput {
                platform_address: address,
                amount: "100000000".to_string(),
            },
        ];

        let error = WalletSendScreen::normalize_advanced_platform_inputs(&inputs)
            .expect_err("overflowing duplicate inputs must be rejected");
        assert!(matches!(&error, TaskError::AdvancedPlatformInputsOverflow));
        assert_eq!(
            error.to_string(),
            "The combined inputs from one Platform address exceed the maximum amount this app can process. Reduce the amounts or remove duplicate input rows, then try again."
        );
    }

    #[test]
    fn advanced_platform_outputs_are_empty_when_all_rows_are_zero() {
        let address = PlatformAddress::try_from(testnet_core_address(6))
            .expect("core address converts to platform address")
            .to_bech32m_string(Network::Testnet);
        let outputs = vec![AdvancedOutput {
            address,
            amount: "0".to_string(),
        }];

        assert!(
            WalletSendScreen::normalize_advanced_platform_outputs(&outputs)
                .expect("valid output")
                .is_empty()
        );
    }
}
