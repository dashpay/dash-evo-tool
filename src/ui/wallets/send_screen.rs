use crate::app::AppAction;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::error::{TaskError, ensure_funding_covers_network_fee};
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::address::{AddressKind, ValidatedAddress};
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES, validate_nonzero_amount};
use crate::model::asset_lock::{asset_lock_user_amount_range, asset_lock_user_max_amount};
use crate::model::fee_estimation::{
    MAX_PLATFORM_INPUTS, PlatformFeeEstimator, allocate_platform_addresses,
    allocate_platform_addresses_with_fee, estimate_address_funding_fee_from_transition,
    estimate_core_l1_send_fee_duffs, estimate_platform_fee,
    estimate_withdrawal_fee_from_transition, format_credits_as_dash, format_duffs_as_dash,
    identity_topup_min_funding_duffs, max_spendable_credits, shield_from_balance_fee_headroom,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::spend_limit::SpendLimitKind;
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
use crate::ui::identity::funding_common::{FUNDING_FEE_NOT_COVERED, asset_lock_amount_refusal};
use crate::ui::identity::top_up_identity_screen::{
    send_top_up_to_background, top_up_in_flight_for, track_top_up_sent_elsewhere,
};
use crate::ui::state::{AssetLockBalanceCache, PendingRequest, SpendLimitCache};
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::wallets::shielded_tab::SHIELDED_FROM_CORE_WITHOUT_AMOUNT;
use crate::ui::{
    MessageType, RootScreenType, ScreenLike, append_concurrent_backend_tasks,
    can_append_concurrent_backend_tasks,
};
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

#[derive(Debug, Clone, PartialEq)]
enum ValidatedSendRoute {
    CoreToCore,
    CoreToPlatform,
    PlatformToPlatform(Vec<(PlatformAddress, Address, u64)>),
    PlatformToCore(Vec<(PlatformAddress, Address, u64)>),
    ShieldedToShielded(WalletSeedHash),
    ShieldedToPlatform(WalletSeedHash),
    CoreToShielded,
    CoreToIdentity,
    PlatformToShielded(Vec<(PlatformAddress, Address, u64)>),
    PlatformToIdentity(Vec<(PlatformAddress, Address, u64)>),
    ShieldedToCore(WalletSeedHash),
    IdentityToCore(Box<QualifiedIdentity>),
    IdentityToPlatform(Box<QualifiedIdentity>),
    IdentityToIdentity(Box<QualifiedIdentity>),
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

/// Identity-bound entry points into the shared send form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySendMode {
    /// Spend the selected identity's balance.
    Send,
    /// Fund the selected identity from a selectable source.
    TopUp,
    /// Withdraw only to a receiving address belonging to a loaded wallet.
    MoveToWallet,
}

pub struct WalletSendScreen {
    pub app_context: Arc<AppContext>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    selected_wallet_seed_hash: Option<WalletSeedHash>,

    // Unified send fields (simple mode)
    selected_source: Option<SourceSelection>,
    address_input: Option<AddressInput>,
    address_input_snapshot_signature: Option<u64>,
    address_input_identities: Vec<QualifiedIdentity>,
    address_input_identity_signature: Option<u64>,
    #[cfg(test)]
    identity_load_count: std::cell::Cell<usize>,
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
    /// The send this screen dispatched and waits on. While it waits, only the
    /// outcome of that send ends the wait: failures of other tasks reach the
    /// visible screen too, and are not about this payment.
    pending_send: PendingRequest,
    send_banner: Option<BannerHandle>,
    send_confirmation: Option<PendingSendConfirmation>,
    asset_lock_balance: AssetLockBalanceCache,
    /// Backend-quoted Max amounts for sends whose builder applies rules the
    /// screen cannot see.
    spend_limits: SpendLimitCache,

    /// Preset flow this screen was opened for. `General` is the full send
    /// screen; the shielded presets lock source/destination for that flow.
    flow: SendFlow,
    identity_flow: Option<(QualifiedIdentity, IdentitySendMode)>,

    // Wallet unlock
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
}

/// Refusal of a top-up while another top-up of the same identity runs.
const TOP_UP_ALREADY_RUNNING: &str = "Funds are already being added to this identity. Wait for that transfer to finish, then try again.";

/// Merge the confirmation dialog's action into the frame action. The dialog
/// consumes the click and clears itself, so a confirmed send is one-shot: it
/// takes the `AppAction` `|=` precedence over a same-frame navigation action
/// and is never silently dropped. It goes out under a context of its own, which
/// a task batch cannot carry, so it does not join one.
fn merge_confirmation_action(mut action: AppAction, confirmation_action: AppAction) -> AppAction {
    action |= confirmation_action;
    action
}

impl WalletSendScreen {
    pub fn new(app_context: &Arc<AppContext>, wallet: Arc<RwLock<Wallet>>) -> Self {
        Self::new_with_wallet(app_context, Some(wallet))
    }

    fn new_with_wallet(app_context: &Arc<AppContext>, wallet: Option<Arc<RwLock<Wallet>>>) -> Self {
        let seed_hash = wallet
            .as_ref()
            .and_then(|w| w.read().ok().map(|w| w.seed_hash()));
        Self {
            app_context: app_context.clone(),
            selected_wallet: wallet,
            selected_wallet_seed_hash: seed_hash,
            selected_source: Some(SourceSelection::CoreWallet),
            address_input: None,
            address_input_snapshot_signature: None,
            address_input_identities: Vec::new(),
            address_input_identity_signature: None,
            #[cfg(test)]
            identity_load_count: std::cell::Cell::new(0),
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
            pending_send: PendingRequest::default(),
            send_banner: None,
            send_confirmation: None,
            asset_lock_balance: AssetLockBalanceCache::default(),
            spend_limits: SpendLimitCache::default(),
            flow: SendFlow::General,
            identity_flow: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
        }
    }

    /// Open Send with a fixed identity sender, or Top up with a fixed recipient.
    pub fn for_identity(
        app_context: &Arc<AppContext>,
        identity: QualifiedIdentity,
        mode: IdentitySendMode,
    ) -> Self {
        let wallet = identity
            .associated_wallets
            .values()
            .next()
            .cloned()
            .or_else(|| {
                app_context
                    .wallet_context()
                    .wallets()
                    .values()
                    .next()
                    .cloned()
            });
        let mut screen = Self::new_with_wallet(app_context, wallet);
        screen.identity_flow = Some((identity, mode));
        screen.restore_identity_constraint();
        screen
    }

    /// Identity and mode retained across retries and Send another.
    pub fn identity_flow(&self) -> Option<&(QualifiedIdentity, IdentitySendMode)> {
        self.identity_flow.as_ref()
    }

    fn refresh_bound_identity(&mut self) {
        if let Some((identity, _)) = &mut self.identity_flow
            && identity.network == self.app_context.network()
            && let Ok(Some(current)) = self.app_context.get_identity_by_id(&identity.identity.id())
        {
            *identity = current;
        }
        self.restore_identity_constraint();
    }

    /// Whether the bound identity still occupies its fixed side of the form.
    /// Holds trivially when the form has no bound identity.
    fn identity_constraint_holds(&self) -> bool {
        let Some((identity, mode)) = &self.identity_flow else {
            return true;
        };
        identity.network == self.app_context.network()
            && match mode {
                IdentitySendMode::Send | IdentitySendMode::MoveToWallet => {
                    matches!(&self.selected_source,
                        Some(SourceSelection::Identity(source)) if source.identity.id() == identity.identity.id())
                }
                IdentitySendMode::TopUp => {
                    self.validated_destination
                        .as_ref()
                        .and_then(ValidatedAddress::as_identity_id)
                        == Some(&identity.identity.id())
                }
            }
    }

    /// Per-frame guard: rebinds only when a frame finds the binding changed,
    /// so an intact binding costs no clone.
    fn keep_identity_constraint(&mut self) {
        if !self.identity_constraint_holds() {
            self.restore_identity_constraint();
        }
    }

    fn restore_identity_constraint(&mut self) {
        let Some((identity, mode)) = &self.identity_flow else {
            return;
        };
        if identity.network != self.app_context.network() {
            return;
        }
        match mode {
            IdentitySendMode::Send | IdentitySendMode::MoveToWallet => {
                self.selected_source = Some(SourceSelection::Identity(Box::new(identity.clone())));
                self.selected_identity = Some(identity.clone());
            }
            IdentitySendMode::TopUp => {
                self.validated_destination = Some(ValidatedAddress::Identity {
                    id: identity.identity.id(),
                    dpns_name: self.app_context.main_username(identity),
                });
            }
        }
    }

    fn heading(&self) -> &'static str {
        match self.identity_flow.as_ref().map(|(_, mode)| mode) {
            Some(IdentitySendMode::TopUp) => "Top up identity",
            Some(IdentitySendMode::MoveToWallet) => "Move to wallet",
            _ => self.flow.heading(),
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
        sorted_addresses.sort_by_key(|a| std::cmp::Reverse(a.2));

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
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
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
        self.pending_send.clear();
        self.send_confirmation = None;
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
        self.refresh_bound_identity();
    }

    /// Enters the waiting state for the send dispatched as `request`.
    fn mark_sending(&mut self, request: BackendTaskContext) {
        self.send_status = SendStatus::WaitingForResult;
        self.pending_send.track(request);
    }

    /// Whether this screen waits on the outcome of a send it dispatched.
    fn awaits_own_send(&self) -> bool {
        self.pending_send.is_pending() && matches!(self.send_status, SendStatus::WaitingForResult)
    }

    /// Whether `context` is the send this screen dispatched and still waits on.
    fn awaits_send(&self, context: &BackendTaskContext) -> bool {
        self.awaits_own_send() && self.pending_send.owns(context)
    }

    /// What confirming a send does: show its progress and dispatch it.
    fn confirm_send(&mut self, ctx: &Context, action: AppAction) -> AppAction {
        self.set_send_progress_banner(ctx);
        self.dispatch_send(action)
    }

    /// Dispatches a confirmed send under a context of its own, so that its
    /// outcome can be told apart from every other task's.
    fn dispatch_send(&mut self, action: AppAction) -> AppAction {
        let AppAction::BackendTask(task) = action else {
            // Every send is prepared as one task; anything else has no context to wait on.
            self.send_status = SendStatus::WaitingForResult;
            self.pending_send.clear();
            return action;
        };
        let context = BackendTaskContext::for_dispatch_on(&task, self.app_context.network());
        track_top_up_sent_elsewhere(&self.app_context, &context);
        self.mark_sending(context.clone());
        AppAction::BackendTaskWithContext { task, context }
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

    /// Fees, in duffs, of shielding from the Core wallet: what the network
    /// takes from the funding, and the Core transaction fee. `None` when they
    /// cannot be read for the protocol version in use.
    fn shield_from_core_fees_duffs(&self) -> Option<(u64, u64)> {
        let (_, platform_version) = self.app_context.shield_from_core_protocol_version();
        self.app_context
            .fee_estimator()
            .estimate_shield_from_core_fees_duffs(platform_version)
            .ok()
    }

    /// Smallest amount, in duffs, the network accepts when the wallet funds a
    /// destination of this kind directly. `None` for other kinds, or when it
    /// cannot be read for the protocol version in use; the backend then decides.
    fn wallet_funding_minimum_duffs(&self, destination: AddressKind) -> Option<u64> {
        let platform_version = self.app_context.sdk_platform_version();
        match destination {
            AddressKind::Identity => identity_topup_min_funding_duffs(platform_version).ok(),
            AddressKind::Platform => Some(
                self.app_context
                    .fee_estimator()
                    .address_funding_min_amount_duffs(true, platform_version),
            ),
            _ => None,
        }
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
            // The wallet funds these directly, and the network takes its fee
            // from the amount.
            (Some(Core), Some(kind @ (Platform | Identity))) => self
                .wallet_funding_minimum_duffs(kind)
                .map(|duffs| duffs.saturating_mul(CREDITS_PER_DUFF))
                .or(Some(platform_min)),
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

    /// Get the Core wallet's display-only balance for labels and ordinary sends.
    ///
    /// Asset-lock Max and validation use [`Self::asset_lock_max_amount`]
    /// instead because those transactions require final inputs and can reject
    /// outputs counted by this snapshot.
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

    /// The builder ceiling an explicit asset-lock amount is checked against:
    /// `None` while another transaction from the wallet is in progress, when
    /// only Max is withheld and the backend decides the amount.
    fn asset_lock_validation_ceiling(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Option<u64>, String> {
        match self.asset_lock_max_amount(seed_hash) {
            Ok(ceiling) => Ok(Some(ceiling)),
            Err(_) if self.asset_lock_balance.is_busy(seed_hash) => Ok(None),
            Err(message) => Err(message),
        }
    }

    fn asset_lock_max_amount(&self, seed_hash: &WalletSeedHash) -> Result<u64, String> {
        let (_, input_state, _) = self.app_context.asset_lock_probe_snapshot(seed_hash);
        self.asset_lock_balance
            .get_current(seed_hash, &input_state)
            .ok_or_else(|| {
                self.asset_lock_balance
                    .validation_unavailable_message(seed_hash)
                    .to_string()
            })
    }

    fn request_asset_lock_max_amount(&mut self) -> Option<BackendTask> {
        if !matches!(self.selected_source, Some(SourceSelection::CoreWallet))
            || !matches!(
                self.destination_kind(),
                Some(AddressKind::Platform | AddressKind::Shielded | AddressKind::Identity)
            )
        {
            return None;
        }
        let seed_hash = self.selected_wallet_seed_hash?;
        let (snapshot_generation, input_state, utxo_revision) =
            self.app_context.asset_lock_probe_snapshot(&seed_hash);
        self.asset_lock_balance.ensure_requested(
            seed_hash,
            snapshot_generation,
            input_state,
            utxo_revision,
        )
    }

    /// The backend-quoted limit the current source and destination need, and
    /// the wallet state it must have been quoted against.
    fn spend_limit_target(&self) -> Option<(WalletSeedHash, SpendLimitKind, u64)> {
        let seed_hash = self.selected_wallet_seed_hash?;
        match (self.selected_source.as_ref()?, self.destination_kind()?) {
            (SourceSelection::PlatformAddresses(_), AddressKind::Shielded) => Some((
                seed_hash,
                SpendLimitKind::ShieldFromPlatform,
                self.live_platform_balances_revision(),
            )),
            (SourceSelection::CoreWallet, AddressKind::Core) => Some((
                seed_hash,
                self.core_send_limit_kind(),
                self.app_context.core_send_quote_revision(&seed_hash),
            )),
            _ => None,
        }
    }

    /// Core send Max is quoted for the script the destination pays to; with
    /// none chosen yet, for P2PKH, the larger output and so the safe bound.
    fn core_send_limit_kind(&self) -> SpendLimitKind {
        let destination = match &self.validated_destination {
            Some(ValidatedAddress::Core(address)) => {
                crate::model::spend_limit::CoreDestinationScript::of(address)
            }
            _ => Default::default(),
        };
        SpendLimitKind::CoreSend { destination }
    }

    /// Names the wallet's live Platform balances, not the ones captured in
    /// `selected_source`: a balance fetch or sync push updates the wallet
    /// without re-selecting the source, and must still re-quote shield Max.
    fn live_platform_balances_revision(&self) -> u64 {
        let addresses: Vec<_> = self
            .get_platform_addresses()
            .into_iter()
            .map(|(core_address, platform_address, balance)| {
                (platform_address, core_address, balance)
            })
            .collect();
        platform_balances_revision(&addresses)
    }

    /// The quoted Max, in credits, for the current source and destination.
    fn spend_limit(&self, kind: SpendLimitKind) -> Option<u64> {
        let (seed_hash, target_kind, revision) = self.spend_limit_target()?;
        (target_kind == kind)
            .then(|| self.spend_limits.get(&seed_hash, kind, revision))
            .flatten()
    }

    fn request_spend_limit(&mut self) -> Option<BackendTask> {
        let (seed_hash, kind, revision) = self.spend_limit_target()?;
        self.spend_limits
            .ensure_requested(seed_hash, kind, revision)
    }

    fn render_spend_limit_status(&mut self, ui: &mut Ui) {
        let Some((seed_hash, kind, revision)) = self.spend_limit_target() else {
            return;
        };
        if self.spend_limits.is_busy(&seed_hash, kind, revision) {
            ui.label(crate::model::spend_limit::SPEND_IN_PROGRESS_MESSAGE);
        } else if self.spend_limits.is_failed(&seed_hash, kind, revision) {
            ui.label("The available amount could not be checked.");
            if ui.button("Retry available amount check").clicked() {
                self.spend_limits.invalidate();
            }
        } else if self.spend_limits.get(&seed_hash, kind, revision).is_none() {
            ui.label("Checking the available amount…");
        }
    }

    fn render_asset_lock_balance_status(&mut self, ui: &mut Ui) {
        let Some(seed_hash) = self.selected_wallet_seed_hash else {
            return;
        };
        if !matches!(self.selected_source, Some(SourceSelection::CoreWallet))
            || !matches!(
                self.destination_kind(),
                Some(AddressKind::Platform | AddressKind::Shielded | AddressKind::Identity)
            )
        {
            return;
        }

        let failed = self.asset_lock_balance.is_failed(&seed_hash);
        let loading = self.asset_lock_max_amount(&seed_hash).is_err();
        if self.asset_lock_balance.is_busy(&seed_hash) {
            ui.label(crate::model::spend_limit::SPEND_IN_PROGRESS_MESSAGE);
        } else if failed {
            ui.label("The available amount could not be checked.");
        } else if loading {
            ui.label("Checking the available amount…");
        } else if self.asset_lock_balance.should_offer_retry(&seed_hash) {
            ui.label("The amount shown is safe but may be lower than your full available amount.");
        }
        if self.asset_lock_balance.should_offer_retry(&seed_hash)
            && ui.button("Retry available amount check").clicked()
        {
            self.asset_lock_balance.invalidate_one(&seed_hash);
        }
    }

    /// Get loaded identities for the current wallet, filtered by wallet seed hash.
    fn get_loaded_identities(&self) -> Vec<QualifiedIdentity> {
        #[cfg(test)]
        self.identity_load_count
            .set(self.identity_load_count.get() + 1);
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
        addresses.sort_by_key(|a| std::cmp::Reverse(a.1));
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

    /// Fee an identity sender keeps back for the chosen destination. Max and
    /// send validation both read it, so they cannot reserve different amounts.
    fn identity_source_fee_reserve(&self) -> u64 {
        let fee_estimator = self.app_context.fee_estimator();
        match self.destination_kind() {
            Some(AddressKind::Core) => fee_estimator.estimate_credit_withdrawal(),
            Some(AddressKind::Platform) => fee_estimator.estimate_credit_transfer_to_addresses(1),
            _ => fee_estimator.estimate_credit_transfer(),
        }
    }

    /// Returns the destination address string, from the validated address if
    /// available or an empty string otherwise.
    fn destination_address_string(&self) -> String {
        self.validated_destination
            .as_ref()
            .map(|v| v.to_address_string())
            .unwrap_or_default()
    }

    fn validated_send_route(&self) -> Result<ValidatedSendRoute, String> {
        if !self.identity_constraint_holds() {
            return Err("This form no longer matches the selected identity or network. Return to the identity and open it again.".into());
        }
        if matches!(
            self.identity_flow,
            Some((_, IdentitySendMode::MoveToWallet))
        ) {
            let is_own_address = match &self.validated_destination {
                Some(ValidatedAddress::Core(address)) => self
                    .address_input_wallets()
                    .iter()
                    .any(|(_, _, paths, _)| paths.contains_key(address)),
                _ => false,
            };
            if !is_own_address {
                return Err("Choose a receiving address from one of your wallets.".into());
            }
        }
        let source = self
            .selected_source
            .as_ref()
            .ok_or("Please select a source")?;
        let destination = self.validated_destination.as_ref().ok_or_else(|| {
            "Invalid destination address. Use a Dash address (X.../y...) or Platform address (dash1.../tdash1...)"
                .to_string()
        })?;
        let amount = self
            .amount
            .as_ref()
            .ok_or_else(|| "Please enter an amount".to_string())?;
        if amount.value() == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        match (source, destination.kind()) {
            (SourceSelection::CoreWallet, AddressKind::Core) => Ok(ValidatedSendRoute::CoreToCore),
            (SourceSelection::CoreWallet, AddressKind::Platform) => {
                Ok(ValidatedSendRoute::CoreToPlatform)
            }
            (SourceSelection::PlatformAddresses(addresses), AddressKind::Platform) => {
                Ok(ValidatedSendRoute::PlatformToPlatform(addresses.clone()))
            }
            (SourceSelection::PlatformAddresses(addresses), AddressKind::Core) => {
                Ok(ValidatedSendRoute::PlatformToCore(addresses.clone()))
            }
            (SourceSelection::Shielded(seed_hash, _), AddressKind::Shielded) => {
                Ok(ValidatedSendRoute::ShieldedToShielded(*seed_hash))
            }
            (SourceSelection::Shielded(seed_hash, _), AddressKind::Platform) => {
                Ok(ValidatedSendRoute::ShieldedToPlatform(*seed_hash))
            }
            (SourceSelection::CoreWallet, AddressKind::Shielded) => {
                Ok(ValidatedSendRoute::CoreToShielded)
            }
            (SourceSelection::CoreWallet, AddressKind::Identity) => {
                Ok(ValidatedSendRoute::CoreToIdentity)
            }
            (SourceSelection::PlatformAddresses(addresses), AddressKind::Shielded) => {
                Ok(ValidatedSendRoute::PlatformToShielded(addresses.clone()))
            }
            (SourceSelection::PlatformAddresses(addresses), AddressKind::Identity) => {
                Ok(ValidatedSendRoute::PlatformToIdentity(addresses.clone()))
            }
            (SourceSelection::Shielded(seed_hash, _), AddressKind::Core) => {
                Ok(ValidatedSendRoute::ShieldedToCore(*seed_hash))
            }
            (SourceSelection::Identity(identity), AddressKind::Core) => {
                Ok(ValidatedSendRoute::IdentityToCore(identity.clone()))
            }
            (SourceSelection::Identity(identity), AddressKind::Platform) => {
                Ok(ValidatedSendRoute::IdentityToPlatform(identity.clone()))
            }
            (SourceSelection::Identity(identity), AddressKind::Identity) => {
                Ok(ValidatedSendRoute::IdentityToIdentity(identity.clone()))
            }
            (SourceSelection::Identity(_), AddressKind::Shielded) => Err(
                "Sending from an identity to the shielded pool is not yet supported. \
                 Transfer to a Platform address first, then shield from there."
                    .to_string(),
            ),
            (SourceSelection::Shielded(..), AddressKind::Identity) => Err(
                "Sending from the shielded pool to an identity is not yet supported. \
                 Transfer to a Platform address first, then top up the identity."
                    .to_string(),
            ),
        }
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
        let route = self.validated_send_route()?;
        if matches!(
            route,
            ValidatedSendRoute::CoreToIdentity | ValidatedSendRoute::PlatformToIdentity(_)
        ) && self
            .validated_destination
            .as_ref()
            .and_then(|destination| destination.as_identity_id())
            .is_some_and(|identity_id| {
                top_up_in_flight_for(self.app_context.egui_ctx(), identity_id)
            })
        {
            return Err(TOP_UP_ALREADY_RUNNING.to_string());
        }
        // Identity signing uses its own key access, including identities imported without a wallet.
        match route {
            ValidatedSendRoute::IdentityToCore(qi) => return self.send_identity_to_core(*qi),
            ValidatedSendRoute::IdentityToPlatform(qi) => {
                return self.send_identity_to_platform(*qi);
            }
            ValidatedSendRoute::IdentityToIdentity(qi) => {
                return self.send_identity_to_identity(*qi);
            }
            _ => {}
        }
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet selected")?;

        let wallet_guard = wallet.read().map_err(|e| e.to_string())?;

        if !wallet_guard.is_open() {
            return Err("Wallet must be unlocked first".to_string());
        }

        let seed_hash = wallet_guard.seed_hash();

        drop(wallet_guard);

        match route {
            ValidatedSendRoute::CoreToCore => self.send_core_to_core(),
            ValidatedSendRoute::CoreToPlatform => self.send_core_to_platform(seed_hash),
            ValidatedSendRoute::PlatformToPlatform(addresses) => {
                self.send_platform_to_platform(seed_hash, addresses)
            }
            ValidatedSendRoute::PlatformToCore(addresses) => {
                self.send_platform_to_core(seed_hash, addresses)
            }
            ValidatedSendRoute::ShieldedToShielded(sh) => self.send_shielded_to_shielded(sh),
            ValidatedSendRoute::ShieldedToPlatform(sh) => self.send_shielded_to_platform(sh),
            ValidatedSendRoute::CoreToShielded => self.send_core_to_shielded(seed_hash),
            ValidatedSendRoute::CoreToIdentity => self.send_core_to_identity(seed_hash),
            ValidatedSendRoute::PlatformToShielded(_) => self.send_platform_to_shielded(seed_hash),
            ValidatedSendRoute::PlatformToIdentity(addresses) => {
                self.send_platform_to_identity(seed_hash, addresses)
            }
            ValidatedSendRoute::ShieldedToCore(sh) => self.send_shielded_to_core(sh),
            ValidatedSendRoute::IdentityToCore(qi) => self.send_identity_to_core(*qi),
            ValidatedSendRoute::IdentityToPlatform(qi) => self.send_identity_to_platform(*qi),
            ValidatedSendRoute::IdentityToIdentity(qi) => self.send_identity_to_identity(*qi),
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

        // No balance check here: the display snapshot can lag the pooled
        // quote behind Max. `WalletBackend::send_payment` dry-runs the pooled
        // build before any prompt and refuses a shortfall with a typed error.
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

        let destination = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied())
            .ok_or_else(|| "Invalid platform address".to_string())?;
        let platform_fee_duffs = estimate_address_funding_fee_from_transition(
            self.app_context.platform_version(),
            &destination,
        )
        .div_ceil(CREDITS_PER_DUFF);
        let asset_lock_max = self.asset_lock_validation_ceiling(&seed_hash)?;
        if let Some(message) = asset_lock_amount_refusal(
            amount_duffs,
            asset_lock_max,
            platform_fee_duffs,
            self.wallet_funding_minimum_duffs(AddressKind::Platform),
        ) {
            return Err(message);
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
                    if ui
                        .button(if self.identity_flow.is_some() {
                            "Back to identity"
                        } else {
                            "Back to Wallet"
                        })
                        .clicked()
                    {
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

        self.keep_identity_constraint();
        let identity_sender = matches!(
            self.identity_flow,
            Some((_, IdentitySendMode::Send | IdentitySendMode::MoveToWallet))
        );
        if matches!(self.identity_flow, Some((_, IdentitySendMode::TopUp))) {
            self.render_funding_wallet(ui);
        } else if !identity_sender {
            self.render_wallet_info(ui);
        }

        ui.add_space(10.0);

        // Source selection
        if let Some((identity, IdentitySendMode::Send | IdentitySendMode::MoveToWallet)) =
            &self.identity_flow
        {
            ui.label("Send from");
            ui.strong(self.app_context.identity_display_label(identity));
            ui.label(format!(
                "Identity balance: {}",
                format_credits_as_dash(identity.identity.balance())
            ));
        } else {
            self.render_source_selection(ui);
        }

        if !matches!(self.selected_source, Some(SourceSelection::Identity(_)))
            && !self.render_unlock_gate(ui)
        {
            return AppAction::None;
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Destination address
        if let Some((identity, IdentitySendMode::TopUp)) = &self.identity_flow {
            ui.label("Send to");
            ui.strong(self.app_context.identity_display_label(identity));
            self.keep_identity_constraint();
        } else {
            self.render_destination_input(ui);
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // Amount
        self.render_asset_lock_balance_status(ui);
        self.render_spend_limit_status(ui);
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

    fn render_funding_wallet(&mut self, ui: &mut Ui) {
        use crate::model::wallet::balance_summary::WalletChoice;
        use crate::ui::components::wallet_selector::WalletSelector;
        let mut selector = WalletSelector::new("identity_top_up_wallet");
        selector.set_entries(self.app_context.wallet_selector_entries(false));
        selector.set_selected(
            self.selected_wallet
                .as_deref()
                .map(WalletChoice::of_hd_wallet),
        );
        let response = selector.show(ui).inner;
        if response.has_changed()
            && let Some(WalletChoice::Hd(seed_hash)) = response.changed_value()
        {
            let wallet = self
                .app_context
                .wallet_context()
                .wallets()
                .get(seed_hash)
                .cloned();
            self.selected_wallet = wallet;
            self.selected_wallet_seed_hash = Some(*seed_hash);
            self.wallet_open_attempted = false;
            self.reset_form();
        }
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

        self.render_asset_lock_balance_status(ui);
        self.render_spend_limit_status(ui);
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
            let alias = self
                .app_context
                .wallet_context()
                .hd_alias(&wallet.seed_hash())
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
        if validate_nonzero_amount(amount_duffs).is_err() {
            return Err("Amount must be greater than 0".to_string());
        }

        let (_, platform_version) = self.app_context.shield_from_core_protocol_version();
        let (platform_fee_duffs, _) = self
            .app_context
            .fee_estimator()
            .estimate_shield_from_core_fees_duffs(platform_version)
            .map_err(|e| {
                TaskError::AssetLockNetworkFeeUnavailable {
                    source_error: Box::new(SdkError::Protocol(*e)),
                }
                .to_string()
            })?;
        let asset_lock_max = self.asset_lock_validation_ceiling(&seed_hash)?;
        if let Some(message) =
            asset_lock_amount_refusal(amount_duffs, asset_lock_max, platform_fee_duffs, None)
        {
            return Err(message);
        }

        Ok(AppAction::BackendTask(BackendTask::ShieldedTask(
            crate::backend_task::shielded::ShieldedTask::ShieldFromAssetLock {
                seed_hash,
                amount_duffs,
            },
        )))
    }

    /// Top up an identity from Core wallet via asset lock (Core -> Identity).
    fn send_core_to_identity(&mut self, seed_hash: WalletSeedHash) -> Result<AppAction, String> {
        let amount_duffs = self
            .amount
            .as_ref()
            .ok_or_else(|| "Amount is required".to_string())?
            .dash_to_duffs()?;
        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        let identity_fee_duffs = self
            .app_context
            .fee_estimator()
            .estimate_identity_topup()
            .div_ceil(CREDITS_PER_DUFF);
        let asset_lock_max = self.asset_lock_validation_ceiling(&seed_hash)?;
        if let Some(message) = asset_lock_amount_refusal(
            amount_duffs,
            asset_lock_max,
            identity_fee_duffs,
            self.wallet_funding_minimum_duffs(AddressKind::Identity),
        ) {
            return Err(message);
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
        let top_up_index = qualified_identity.next_top_up_index();

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

        // No balance check here: the source's captured balances can lag a
        // sync. The shield planner decides, and the backend refuses an amount
        // above its maximum before any prompt.
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

        self.ensure_identity_amount_leaves_fee(amount_credits, identity_balance)?;

        // The backend chooses the key by the same rule; asking here refuses a
        // withdrawal no key can sign before the confirmation.
        qualified_identity
            .resolve_withdrawal_signing_key(None)
            .map_err(|_| TaskError::NoUsableWithdrawalKey.to_string())?;

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

    /// Refuses an amount that, with the fee the destination reserves paid on top
    /// from the same balance, the identity cannot cover.
    fn ensure_identity_amount_leaves_fee(
        &self,
        amount_credits: u64,
        identity_balance: u64,
    ) -> Result<(), String> {
        let spendable = max_spendable_credits(identity_balance, self.identity_source_fee_reserve());
        if spendable == 0 {
            return Err(
                "This identity's balance is too low to cover the network fee. Top up the identity and try again."
                    .to_string(),
            );
        }
        if amount_credits > spendable {
            let max = format_credits_as_dash(spendable);
            return Err(format!(
                "This amount leaves too little in the identity to pay the network fee. Enter {max} or less."
            ));
        }
        Ok(())
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

        self.ensure_identity_amount_leaves_fee(amount_credits, identity_balance)?;

        let platform_addr = self
            .validated_destination
            .as_ref()
            .and_then(|v| v.as_platform().copied())
            .ok_or_else(|| "Invalid Platform address".to_string())?;

        let mut outputs = BTreeMap::new();
        outputs.insert(platform_addr, amount_credits);

        let key_id = Self::transfer_signing_key_id(&qualified_identity)?;
        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TransferToAddresses {
                identity: qualified_identity,
                outputs,
                key_id: Some(key_id),
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

        self.ensure_identity_amount_leaves_fee(amount_credits, identity_balance)?;

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

        let key_id = Self::transfer_signing_key_id(&qualified_identity)?;
        Ok(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::Transfer(
                qualified_identity,
                to_identity_id,
                amount_credits,
                Some(key_id),
            ),
        )))
    }

    /// The key that signs a transfer from `identity`, chosen here so a send no
    /// key in this app can sign is refused before the confirmation.
    fn transfer_signing_key_id(
        identity: &QualifiedIdentity,
    ) -> Result<dash_sdk::dpp::identity::KeyID, String> {
        identity
            .resolve_transfer_signing_key(None)
            .map_err(|_| TaskError::NoUsableTransferKey.to_string())
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
        let mut identities = self.get_loaded_identities();
        if let Some((recipient, IdentitySendMode::TopUp)) = &self.identity_flow {
            identities.retain(|source| source.identity.id() != recipient.identity.id());
        }
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
                                let name = self.app_context.identity_display_label(qi);
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
                                        let name =
                                            self.app_context.identity_display_label(identity);
                                        format!(
                                            "{} ({})",
                                            name,
                                            format_credits_as_dash(identity.identity.balance())
                                        )
                                    };
                                    let is_selected =
                                        self.selected_identity.as_ref().is_some_and(|sel| {
                                            sel.identity.id() == identity.identity.id()
                                        });
                                    if ui.selectable_label(is_selected, &label).clicked() {
                                        self.selected_identity = Some(identity.clone());
                                        self.selected_source = Some(SourceSelection::Identity(
                                            Box::new(identity.clone()),
                                        ));
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
        if !matches!(self.identity_flow, Some((_, IdentitySendMode::TopUp)))
            && FeatureGate::ShieldedOperations.is_available(&self.app_context)
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
        if matches!(
            self.identity_flow,
            Some((_, IdentitySendMode::MoveToWallet))
        ) {
            return vec![AddressKind::Core];
        }
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
        loaded_identities: &[QualifiedIdentity],
    ) -> AddressInput {
        let mut builder = AddressInput::new(self.app_context.network)
            .with_label("Send to")
            .with_address_kinds(allowed_kinds)
            .with_exclude_change(true);

        if !wallets.is_empty() {
            builder = builder.with_wallets(wallets);
        }

        if matches!(
            self.identity_flow,
            Some((_, IdentitySendMode::MoveToWallet))
        ) {
            builder = builder
                .with_selection_only(true)
                .with_dpns_resolution(false)
                .with_label("Receive in your wallet")
                .with_hint_text("Choose one of your wallet addresses");
        }
        // Add identities for autocomplete by profile name or username.
        if !loaded_identities.is_empty() {
            builder = builder.with_identities(&self.app_context, loaded_identities);
        }

        builder
    }

    fn recipient_identities(&self) -> Vec<QualifiedIdentity> {
        // Filter out the source identity (if any) to prevent self-sends.
        let source_identity_id = if let Some(SourceSelection::Identity(qi)) = &self.selected_source
        {
            Some(qi.identity.id())
        } else {
            None
        };
        // An identity-bound send reaches any identity on this device, as the
        // identity transfer screen did; a wallet send keeps to its wallet's.
        let identities = if self.identity_flow.is_some() {
            #[cfg(test)]
            self.identity_load_count
                .set(self.identity_load_count.get() + 1);
            self.app_context
                .load_local_qualified_identities()
                .unwrap_or_default()
        } else {
            self.get_loaded_identities()
        };
        identities
            .into_iter()
            .filter(|qi| Some(qi.identity.id()) != source_identity_id)
            .collect()
    }

    fn address_input_wallets(&self) -> Vec<WalletWithSnapshot> {
        self.app_context
            .wallet_context()
            .wallets()
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
                    self.app_context.wallet_context().hd_alias(&seed_hash),
                )
            })
            .collect()
    }

    fn address_input_snapshot_signature(wallets: &[WalletWithSnapshot]) -> u64 {
        let mut hasher = DefaultHasher::new();
        for (wallet, balances, paths, alias) in wallets {
            let Ok(wallet) = wallet.read() else {
                continue;
            };
            wallet.seed_hash().hash(&mut hasher);
            alias.hash(&mut hasher);
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
        let mut signature = DefaultHasher::new();
        Self::address_input_snapshot_signature(&wallets).hash(&mut signature);
        for (wallet, ..) in &wallets {
            if let Ok(wallet) = wallet.read() {
                self.app_context
                    .snapshot_generation(&wallet.seed_hash())
                    .hash(&mut signature);
            }
        }
        self.selected_wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()))
            .hash(&mut signature);
        if let Some(SourceSelection::Identity(identity)) = &self.selected_source {
            identity.identity.id().hash(&mut signature);
        }
        let snapshot_signature = signature.finish();
        if self.address_input_snapshot_signature != Some(snapshot_signature) {
            self.address_input_identities = self.recipient_identities();
            if let Some(address_input) = &mut self.address_input {
                address_input.set_wallets(&wallets);
            }
            self.address_input_snapshot_signature = Some(snapshot_signature);
        }
        // Profile completions can arrive while the user is typing. Read only the
        // in-memory name cache here; identity records reload on source invalidation.
        let identities = &self.address_input_identities;
        let mut signature = DefaultHasher::new();
        for identity in identities {
            identity.identity.id().hash(&mut signature);
            identity.identity.balance().hash(&mut signature);
            self.app_context
                .identity_display_label(identity)
                .hash(&mut signature);
            identity
                .dpns_names
                .first()
                .map(|name| &name.name)
                .hash(&mut signature);
        }
        let signature = signature.finish();
        if self.address_input_identity_signature != Some(signature) {
            if let Some(address_input) = &mut self.address_input {
                address_input.set_identities(&self.app_context, identities);
            }
            self.address_input_identity_signature = Some(signature);
        }
        if self.address_input.is_none() {
            self.address_input =
                Some(self.build_address_input(allowed_kinds, &wallets, identities));
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
                let dest_kind = self.destination_kind();
                let mut max = self.selected_wallet.as_ref().and_then(|w| {
                    w.read().ok().and_then(|wallet| {
                        let seed_hash = wallet.seed_hash();
                        let max_duffs = if matches!(
                            dest_kind,
                            Some(
                                AddressKind::Platform
                                    | AddressKind::Shielded
                                    | AddressKind::Identity
                            )
                        ) {
                            // Same current-composition accessor validation
                            // uses, so Max can never offer a stale ceiling.
                            self.asset_lock_max_amount(&seed_hash).ok()
                        } else {
                            Some(self.app_context.snapshot_balance(&seed_hash).spendable())
                        };
                        max_duffs.map(|amount| amount.saturating_mul(CREDITS_PER_DUFF))
                    })
                });
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
                            let minimum = format_duffs_as_dash(
                                fee_estimator.address_funding_min_amount_duffs(
                                    true,
                                    self.app_context.sdk_platform_version(),
                                ),
                            );
                            Some(format!(
                                "A Platform fee of approximately {fee} is reserved from your balance and deducted from the amount, so the amount must be at least {minimum}."
                            ))
                        } else {
                            None
                        }
                    }
                    Some(AddressKind::Shielded) => {
                        if let Some((platform_fee_duffs, l1_tx_fee_duffs)) =
                            self.shield_from_core_fees_duffs()
                        {
                            let platform_fee_credits =
                                platform_fee_duffs.saturating_mul(CREDITS_PER_DUFF);
                            let total_fee_credits = platform_fee_duffs
                                .saturating_add(l1_tx_fee_duffs)
                                .saturating_mul(CREDITS_PER_DUFF);
                            max = max.map(|amount| {
                                asset_lock_user_max_amount(amount, platform_fee_credits)
                            });
                            let fee = format_credits_as_dash(total_fee_credits);
                            Some(format!(
                                "Shielding fees of approximately {fee} are reserved from your balance."
                            ))
                        } else {
                            // Without a fee to reserve, Max must offer nothing.
                            max = None;
                            None
                        }
                    }
                    Some(AddressKind::Identity) => {
                        let estimated_fee = fee_estimator.estimate_identity_topup();
                        max = max.map(|amount| amount.saturating_sub(estimated_fee));
                        let fee = format_credits_as_dash(estimated_fee);
                        Some(
                            match self
                                .wallet_funding_minimum_duffs(AddressKind::Identity)
                                .map(format_duffs_as_dash)
                            {
                                Some(minimum) => format!(
                                    "An identity top-up fee of approximately {fee} is reserved from your balance. The network fee is taken from the amount, so it must be at least {minimum}."
                                ),
                                None => format!(
                                    "An identity top-up fee of approximately {fee} is reserved from your balance."
                                ),
                            },
                        )
                    }
                    Some(AddressKind::Core) => {
                        // Core-to-Core "Max": the backend's quote of the largest
                        // payment the pooled build accepts, net of its fee.
                        // Nothing is offered until it arrives.
                        match self.spend_limit(self.core_send_limit_kind()) {
                            Some(0) => {
                                max = None;
                                Some(
                                    "Your balance is too low to cover the network fee.".to_string(),
                                )
                            }
                            Some(max_credits) => {
                                max = Some(max_credits);
                                let utxo_count = self
                                    .selected_wallet_seed_hash
                                    .map(|seed_hash| {
                                        self.app_context.snapshot_utxo_count(&seed_hash)
                                    })
                                    .unwrap_or(0);
                                let fee_duffs =
                                    estimate_core_l1_send_fee_duffs(utxo_count.max(1), 1);
                                let fee = format_credits_as_dash(fee_duffs * CREDITS_PER_DUFF);
                                Some(format!(
                                    "A network fee of approximately {fee} is reserved from your balance."
                                ))
                            }
                            None => {
                                max = None;
                                None
                            }
                        }
                    }
                    _ => None,
                };
                (max, hint)
            }
            Some(SourceSelection::PlatformAddresses(_))
                if self.destination_kind() == Some(AddressKind::Shielded) =>
            {
                // Shield-from-Platform: the upstream planner picks the input
                // addresses in its own order, skips the ones below the minimum
                // input and caps their count, so only its quote is a Max it
                // accepts. Nothing is offered until that quote arrives.
                let headroom =
                    shield_from_balance_fee_headroom(self.app_context.platform_version());
                let fee = format_credits_as_dash(headroom);
                match self.spend_limit(SpendLimitKind::ShieldFromPlatform) {
                    // No address can keep the fee back: a zero Max would only
                    // fill in an amount the form refuses.
                    Some(0) => (
                        None,
                        Some(format!(
                            "None of your Platform addresses can cover the shielding fee of approximately {fee}, so nothing can be shielded right now."
                        )),
                    ),
                    max => (
                        max,
                        Some(format!(
                            "A shielding fee of approximately {fee} is reserved from your balance."
                        )),
                    ),
                }
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
                sorted_addresses.sort_by_key(|a| std::cmp::Reverse(a.2));

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
                let estimated_fee = self.identity_source_fee_reserve();
                let available = max_spendable_credits(qi.identity.balance(), estimated_fee);
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

        // Offer no amount when none the wallet can fund covers the network fee,
        // so Max cannot propose one the network would refuse.
        if input_kind == Some(AddressKind::Core)
            && matches!(
                output_kind,
                Some(AddressKind::Platform | AddressKind::Identity | AddressKind::Shielded)
            )
            && let (Some(max), Some(min)) = (max_amount_credits, min_amount)
            && asset_lock_user_amount_range(max, 0, min).is_none()
        {
            self.amount = None;
            self.amount_input = None;
            ui.colored_label(DashColors::WARNING, FUNDING_FEE_NOT_COVERED);
            return;
        }

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
                let (platform_fee_duffs, l1_tx_fee_duffs) = self.shield_from_core_fees_duffs()?;
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
                let fee_credits =
                    shield_from_balance_fee_headroom(self.app_context.platform_version());
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Identity → Core: credit-withdrawal fee, paid on top.
            (SourceSelection::Identity(_), AddressKind::Core) => {
                let fee_credits = fee_estimator.estimate_address_credit_withdrawal();
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Identity → Platform / Identity: credit-transfer fee, paid on top.
            (SourceSelection::Identity(_), AddressKind::Platform | AddressKind::Identity) => {
                let fee_credits = self.identity_source_fee_reserve();
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
                self.confirm_send(ui.ctx(), *pending.action)
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
        let source_ready =
            matches!(self.selected_source, Some(SourceSelection::Identity(_))) || wallet_open;
        let can_send = source_ready && !is_sending && has_destination && has_amount && has_source;

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

            if ComponentStyles::add_primary_button_enabled(ui, can_send, button_text).clicked() {
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

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Replaces the simple-send fields for headless integration tests.
    pub fn set_simple_send_input_for_test(
        &mut self,
        source: Option<SourceSelection>,
        destination: Option<ValidatedAddress>,
        amount: Option<Amount>,
    ) {
        self.selected_source = source;
        self.validated_destination = destination;
        self.amount = amount;
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Seeds the completed asset-lock maximum lookup for headless integration tests.
    pub fn seed_asset_lock_max_amount_for_test(
        &mut self,
        seed_hash: WalletSeedHash,
        amount_duffs: u64,
    ) {
        let (snapshot_generation, input_state, utxo_revision) =
            self.app_context.asset_lock_probe_snapshot(&seed_hash);
        let Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount { request_id, .. })) =
            self.asset_lock_balance.ensure_requested(
                seed_hash,
                snapshot_generation,
                input_state.clone(),
                utxo_revision,
            )
        else {
            return;
        };
        self.asset_lock_balance.store(
            seed_hash,
            snapshot_generation,
            request_id,
            amount_duffs,
            input_state,
            false,
        );
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Confirms `task` the way the confirmation dialog does and returns its
    /// context, without handing the task to the backend.
    pub fn confirm_send_for_test(&mut self, task: BackendTask) -> Option<BackendTaskContext> {
        let ctx = self.app_context.egui_ctx().clone();
        // The task is dropped on purpose: no funds may move in a test.
        let _action = self.confirm_send(&ctx, AppAction::BackendTask(task));
        self.pending_send.context().cloned()
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Runs the production simple-send validation and dispatch selection.
    pub fn validate_and_send_for_test(&mut self) -> Result<AppAction, String> {
        self.validate_and_send()
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Renders the production simple-send button in a headless harness.
    pub fn render_simple_send_button_for_test(&mut self, ui: &mut Ui) -> AppAction {
        self.render_send_button(ui)
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

            if ComponentStyles::add_primary_button_enabled(ui, can_send, button_text).clicked() {
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
        let minimum_duffs = self
            .app_context
            .fee_estimator()
            .address_funding_min_amount_duffs(
                fee_deduct_from_output,
                self.app_context.sdk_platform_version(),
            );
        ensure_funding_covers_network_fee(amount_duffs, Ok(minimum_duffs))
            .map_err(|error| error.to_string())?;

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
                | BackendTaskSuccessResult::RefreshedIdentity(_)
                | BackendTaskSuccessResult::LoadedIdentity(_)
                | BackendTaskSuccessResult::RegisteredIdentity(..)
                | BackendTaskSuccessResult::ToppedUpIdentity(..)
                | BackendTaskSuccessResult::IdentitiesLoaded { .. }
                | BackendTaskSuccessResult::RemovedIdentities { .. }
                | BackendTaskSuccessResult::RegisteredDpnsName { .. }
                | BackendTaskSuccessResult::RefreshedOwnedDpnsNames
        )
    }

    /// What the form says when `result` completes a send, or `None` for any
    /// other result.
    fn completion_message(
        result: &crate::backend_task::BackendTaskSuccessResult,
    ) -> Option<String> {
        match result {
            crate::backend_task::BackendTaskSuccessResult::WalletPayment {
                txid: _,
                recipients,
                total_amount,
            } => {
                let message = if recipients.len() == 1 {
                    let (address, amount) = &recipients[0];
                    format!("Sent {} to {}", format_duffs_as_dash(*amount), address,)
                } else {
                    format!(
                        "Sent {} to {} recipients",
                        format_duffs_as_dash(*total_amount),
                        recipients.len(),
                    )
                };
                Some(message)
            }
            crate::backend_task::BackendTaskSuccessResult::TransferredCredits(fee_result) => {
                let fee_info = Self::format_fee_info(fee_result);
                Some(format!("Credits transferred successfully!{}", fee_info))
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformAddressFunded { .. } => {
                Some("Platform address funded successfully!".to_string())
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformAddressWithdrawal { .. } => {
                Some("Withdrawal initiated successfully!\n\nNote: It may take a few minutes for funds to appear on the Core chain.".to_string())
            }
            crate::backend_task::BackendTaskSuccessResult::PlatformCreditsTransferred {
                ..
            } => {
                Some("Platform credits transferred successfully!".to_string())
            }
            crate::backend_task::BackendTaskSuccessResult::ShieldedTransferComplete {
                amount,
                ..
            } => {
                Some(format!(
                    "Shielded transfer of {} complete!\n\n\
                     Your remaining balance will update after the next block is confirmed. \
                     The recipient's balance will also update after the next block and a wallet sync.",
                    format_credits_as_dash(*amount)
                ))
            }
            crate::backend_task::BackendTaskSuccessResult::ShieldedCreditsUnshielded {
                amount,
                ..
            } => {
                Some(format!(
                    "Unshielded {} to platform address!\n\n\
                     Your remaining balance will update after the next block is confirmed.",
                    format_credits_as_dash(*amount)
                ))
            }
            // Core->Identity or Platform->Identity top-up result
            crate::backend_task::BackendTaskSuccessResult::ToppedUpIdentity(
                _identity,
                fee_result,
            ) => {
                let fee_info = Self::format_fee_info(fee_result);
                Some(format!("Identity topped up successfully!{}", fee_info))
            }
            // Identity->Core withdrawal result
            crate::backend_task::BackendTaskSuccessResult::WithdrewFromIdentity(fee_result) => {
                let fee_info = Self::format_fee_info(fee_result);
                Some(format!(
                    "Identity withdrawal initiated. Funds will appear on the Core chain after confirmation.{}",
                    fee_info
                ))
            }
            // Core->Shielded or Platform->Shielded shield result
            crate::backend_task::BackendTaskSuccessResult::ShieldedCreditsShielded {
                amount,
                ..
            } => {
                Some(format!(
                    "{} shielded successfully!\n\n\
                     Balance will update after the next block.",
                    format_credits_as_dash(*amount)
                ))
            }
            // Core->Shielded via asset lock result
            crate::backend_task::BackendTaskSuccessResult::ShieldedFromAssetLock {
                amount, ..
            } => {
                Some(match amount {
                    Some(amount) => format!(
                        "{} shielded from asset lock successfully!\n\n\
                         Balance will update after the next block.",
                        format_credits_as_dash(*amount)
                    ),
                    None => SHIELDED_FROM_CORE_WITHOUT_AMOUNT.to_string(),
                })
            }
            // Shielded->Core withdrawal result
            crate::backend_task::BackendTaskSuccessResult::ShieldedWithdrawalComplete {
                amount,
                ..
            } => {
                Some(format!(
                    "Withdrawal of {} from shielded pool initiated.\n\n\
                     Funds will appear after confirmation.",
                    format_credits_as_dash(*amount)
                ))
            }
            _ => None,
        }
    }

    /// Take what `result` says about the form's inputs: a new asset-lock
    /// ceiling, or a change to the addresses the form offers. Returns whether
    /// that is all the result is.
    fn refresh_form_inputs(
        &mut self,
        result: &crate::backend_task::BackendTaskSuccessResult,
    ) -> bool {
        if let crate::backend_task::BackendTaskSuccessResult::AssetLockMaxAmount {
            seed_hash,
            snapshot_generation,
            request_id,
            amount_duffs,
            observed_inputs,
            is_partial,
        } = result
        {
            self.asset_lock_balance.store(
                *seed_hash,
                *snapshot_generation,
                *request_id,
                *amount_duffs,
                observed_inputs.clone(),
                *is_partial,
            );
            return true;
        }
        if let crate::backend_task::BackendTaskSuccessResult::SpendLimit {
            seed_hash,
            kind,
            request_id,
            max_credits,
        } = result
        {
            self.spend_limits
                .store(*seed_hash, *kind, *request_id, *max_credits);
            return true;
        }
        if Self::task_result_updates_address_input(result) {
            self.address_input_snapshot_signature = None;
        }
        false
    }
}

/// What to tell the user about a send whose form was closed before it
/// finished: the text the form would have shown, or `None` for a result that
/// does not complete a send.
pub(crate) fn closed_send_form_message(
    result: &crate::backend_task::BackendTaskSuccessResult,
) -> Option<String> {
    WalletSendScreen::completion_message(result)
}

/// A send keeps running when its form is closed, and nothing is left to end
/// the form's progress banner, so the banner goes with the form. A top-up
/// still in flight is followed by the background banner, as from Add Funds.
impl Drop for WalletSendScreen {
    fn drop(&mut self) {
        self.send_banner.take_and_clear();
        if let Some(dispatch) = self.pending_send.context() {
            send_top_up_to_background(self.app_context.egui_ctx(), dispatch);
        }
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
                (
                    if self.identity_flow.is_some() {
                        "Identities"
                    } else {
                        "Wallets"
                    },
                    AppAction::PopScreen,
                ),
                (self.heading(), AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            if self.identity_flow.is_some() {
                RootScreenType::RootScreenIdentityHub
            } else {
                RootScreenType::RootScreenWalletsBalances
            },
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.style().visuals.dark_mode;

            if self
                .identity_flow
                .as_ref()
                .is_some_and(|(identity, _)| identity.network != self.app_context.network())
            {
                ui.label("The network changed. Return to the identity and open this form again.");
                return if ui.button("Back to identity").clicked() {
                    AppAction::PopScreen
                } else {
                    AppAction::None
                };
            }
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
                                RichText::new(self.heading())
                                    .color(DashColors::text_primary(dark_mode))
                                    .size(24.0),
                            );
                            if !self.flow.is_preset() && self.identity_flow.is_none() {
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

        let confirmation_action = self.render_send_confirmation(ui);
        action = merge_confirmation_action(action, confirmation_action);

        if can_append_concurrent_backend_tasks(&action)
            && let Some(task) = self.request_asset_lock_max_amount()
        {
            action = append_concurrent_backend_tasks(action, vec![task]);
        }
        if can_append_concurrent_backend_tasks(&action)
            && let Some(task) = self.request_spend_limit()
        {
            action = append_concurrent_backend_tasks(action, vec![task]);
        }

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
        // A message carries no task context, so while this screen waits on its
        // own send it cannot be that send's outcome.
        if self.awaits_own_send() {
            return;
        }
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
        if self.refresh_form_inputs(&backend_task_success_result) {
            return;
        }
        self.send_banner.take_and_clear();
        if let Some(message) = Self::completion_message(&backend_task_success_result) {
            self.send_status = SendStatus::Complete(message);
        }
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        backend_task_success_result: crate::backend_task::BackendTaskSuccessResult,
    ) {
        // Other tasks end with results of the kinds a send ends with. While
        // this screen waits on its own send, such a result may refresh what
        // the form offers, but it neither completes that send nor ends its
        // progress banner.
        if self.awaits_own_send() && !self.awaits_send(context) {
            self.refresh_form_inputs(&backend_task_success_result);
            return;
        }
        self.display_task_result(backend_task_success_result);
    }

    fn refresh_on_arrival(&mut self) {
        self.refresh_bound_identity();
        self.address_input_snapshot_signature = None;
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
    }

    fn refresh(&mut self) {
        self.refresh_bound_identity();
        self.address_input_snapshot_signature = None;
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        if self.awaits_send(context) {
            // The global banner says why; the form is handed back.
            self.pending_send.clear();
            self.send_banner.take_and_clear();
            self.send_status = SendStatus::Error;
        }
        if let Some((seed_hash, snapshot_generation, request_id)) =
            context.asset_lock_max_amount_request()
        {
            self.asset_lock_balance.mark_request_error(
                &seed_hash,
                snapshot_generation,
                request_id,
                error,
            );
        }
        if let Some((seed_hash, kind, request_id)) = context.spend_limit_request() {
            self.spend_limits
                .mark_request_error(seed_hash, kind, request_id, error);
        }
    }

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.asset_lock_max_amount_request().is_some() || context.spend_limit_request().is_some()
    }
}

/// Names the Platform balances a shield quote was made against: any change in
/// the offered addresses or their balances asks for a new quote.
fn platform_balances_revision(addresses: &[(PlatformAddress, Address, u64)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (platform_address, _, balance) in addresses {
        Hash::hash(platform_address, &mut hasher);
        Hash::hash(balance, &mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::BackendTasksExecutionMode;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::utils::tasks::TaskManager;
    use crate::wallet_backend::AssetLockInputState;
    use dash_sdk::dashcore_rpc::dashcore::{Network, PrivateKey, PublicKey};
    use dash_sdk::dpp::dashcore::{OutPoint, Txid, hashes::Hash};
    use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
    use dash_sdk::dpp::secp256k1::SecretKey;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn different_asset_lock_inputs(seed_byte: u8) -> AssetLockInputState {
        AssetLockInputState::from_inputs([(
            OutPoint::new(Txid::from_byte_array([seed_byte; 32]), 0),
            1,
        )])
    }

    fn testnet_core_address(key_byte: u8) -> Address {
        let secret_key = SecretKey::from_secret_bytes([key_byte; 32]).expect("valid secret key");
        let private_key =
            PrivateKey::from_byte_array(secret_key.as_secret_bytes(), Network::Testnet)
                .expect("valid secret key");
        let public_key = PublicKey::from_private_key(&private_key);
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

    fn bound_identity() -> QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType};
        use dash_sdk::dpp::identity::{Identity, accessors::IdentitySettersV0};
        let mut identity = Identity::create_basic_identity(
            dash_sdk::platform::Identifier::from([0xA3; 32]),
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("identity");
        identity.set_balance(100_000_000_000);
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    #[test]
    fn identity_send_keeps_sender_after_reset_without_a_wallet() {
        let (ctx, _dir) = offline_ctx();
        let identity = bound_identity();
        let mut screen =
            WalletSendScreen::for_identity(&ctx, identity.clone(), IdentitySendMode::Send);
        assert!(screen.selected_wallet.is_none());
        screen.reset_form();
        screen.validated_destination = Some(identity_destination());
        screen.amount = Some(Amount::new_dash(0.01));
        assert!(
            matches!(screen.validated_send_route(), Ok(ValidatedSendRoute::IdentityToIdentity(q)) if q.identity.id() == identity.identity.id())
        );
        screen.selected_source = Some(SourceSelection::CoreWallet);
        assert!(
            screen.validated_send_route().is_err(),
            "a fixed sender cannot be replaced"
        );
    }

    fn render_identity_form(screen: WalletSendScreen) -> Harness<'static, WalletSendScreen> {
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| {
                screen.render_unified_send(ui);
            },
            screen,
        );
        harness.run();
        harness
    }

    /// Text the binding never writes, so it survives only while frames leave
    /// the bound values alone.
    const UNTOUCHED: &str = "untouched";

    #[test]
    fn identity_binding_is_not_rebuilt_on_frames_where_it_still_holds() {
        let (ctx, _dir) = offline_ctx();

        let mut screen =
            WalletSendScreen::for_identity(&ctx, bound_identity(), IdentitySendMode::Send);
        let Some(SourceSelection::Identity(sender)) = &mut screen.selected_source else {
            panic!("Send binds the identity as the sender");
        };
        sender.alias = Some(UNTOUCHED.into());
        let harness = render_identity_form(screen);
        assert!(
            matches!(&harness.state().selected_source,
                Some(SourceSelection::Identity(sender)) if sender.alias.as_deref() == Some(UNTOUCHED)),
            "an intact sender binding must not be cloned again every frame"
        );

        let mut screen =
            WalletSendScreen::for_identity(&ctx, bound_identity(), IdentitySendMode::TopUp);
        let Some(ValidatedAddress::Identity { dpns_name, .. }) = &mut screen.validated_destination
        else {
            panic!("Top up binds the identity as the recipient");
        };
        *dpns_name = Some(UNTOUCHED.into());
        let harness = render_identity_form(screen);
        assert!(
            matches!(&harness.state().validated_destination,
                Some(ValidatedAddress::Identity { dpns_name, .. }) if dpns_name.as_deref() == Some(UNTOUCHED)),
            "an intact recipient binding must not be rebuilt every frame"
        );
    }

    #[test]
    fn identity_binding_is_restored_when_a_frame_finds_it_broken() {
        let (ctx, _dir) = offline_ctx();
        let identity = bound_identity();

        let mut screen =
            WalletSendScreen::for_identity(&ctx, identity.clone(), IdentitySendMode::Send);
        screen.selected_source = Some(SourceSelection::CoreWallet);
        let harness = render_identity_form(screen);
        assert!(
            matches!(&harness.state().selected_source,
                Some(SourceSelection::Identity(sender)) if sender.identity.id() == identity.identity.id()),
            "a replaced sender must be bound again"
        );

        let mut screen =
            WalletSendScreen::for_identity(&ctx, identity.clone(), IdentitySendMode::TopUp);
        screen.validated_destination = None;
        let harness = render_identity_form(screen);
        assert_eq!(
            harness
                .state()
                .validated_destination
                .as_ref()
                .and_then(ValidatedAddress::as_identity_id),
            Some(&identity.identity.id()),
            "a cleared recipient must be bound again"
        );
    }

    #[test]
    fn identity_top_up_restores_recipient_and_rejects_other_targets() {
        let (ctx, _dir) = offline_ctx();
        let identity = bound_identity();
        let mut screen =
            WalletSendScreen::for_identity(&ctx, identity.clone(), IdentitySendMode::TopUp);
        screen.reset_form();
        screen.amount = Some(Amount::new_dash(0.01));
        assert_eq!(
            screen
                .validated_destination
                .as_ref()
                .and_then(ValidatedAddress::as_identity_id),
            Some(&identity.identity.id())
        );
        assert!(matches!(
            screen.validated_send_route(),
            Ok(ValidatedSendRoute::CoreToIdentity)
        ));
        screen.validated_destination = Some(identity_destination());
        assert!(
            screen.validated_send_route().is_err(),
            "the bound recipient cannot be edited"
        );
    }

    /// Top up from Home for an identity whose stored history has a gap: the new
    /// top-up must be recorded under an index no earlier one holds.
    #[tokio::test(flavor = "multi_thread")]
    async fn identity_top_up_is_recorded_under_an_index_no_earlier_top_up_holds() {
        use crate::backend_task::identity::{IdentityTask, TopUpIdentityFundingMethod};
        let (ctx, _dir) = offline_ctx();
        // Identities and their top-up history live in the wallet backend's store.
        let (sender, _receiver) = tokio::sync::mpsc::channel(32);
        ctx.ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
            sender,
            ctx.egui_ctx().clone(),
        ))
        .await
        .expect("wallet backend");
        let wallet = Wallet::new_from_seed([1u8; 64], Network::Testnet, None, None)
            .expect("wallet from seed");
        let mut identity = bound_identity();
        identity
            .associated_wallets
            .insert(wallet.seed_hash(), Arc::new(RwLock::new(wallet)));
        ctx.insert_local_qualified_identity(&identity, &None)
            .expect("store the identity");
        ctx.save_top_ups(
            &identity.identity.id(),
            &BTreeMap::from([(0, 60_000), (2, 70_000)]),
        )
        .expect("store the top-up history");

        let mut screen = WalletSendScreen::for_identity(&ctx, identity, IdentitySendMode::TopUp);
        let seed_hash = core_source_with_ceiling(&mut screen, 10_000_000);
        screen.amount = Some(Amount::dash_from_duffs(100_000));
        let action = screen
            .send_core_to_identity(seed_hash)
            .expect("the top-up must be dispatched");
        let AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(top_up))) =
            action
        else {
            panic!("expected a top-up, got {action:?}");
        };
        let TopUpIdentityFundingMethod::FundWithWallet(_, _, index) =
            top_up.identity_funding_method
        else {
            panic!("expected a wallet-funded top-up");
        };
        assert_eq!(index, 3, "indexes 0 and 2 already hold earlier top-ups");
    }

    /// A top-up is refused while another top-up of the same identity runs,
    /// whichever funds would pay for it. Other identities are not held back,
    /// and a send that is not a top-up holds nothing back.
    #[test]
    fn a_running_top_up_holds_back_another_top_up_of_the_same_identity() {
        use crate::backend_task::identity::IdentityTask;
        let (ctx, _dir) = offline_ctx();
        let wallet = || {
            let wallet = Wallet::new_from_seed([1u8; 64], Network::Testnet, None, None)
                .expect("wallet from seed");
            Arc::new(RwLock::new(wallet))
        };
        let recipient = bound_identity();
        let recipient_id = recipient.identity.id();
        let to = |id| ValidatedAddress::Identity {
            id,
            dpns_name: None,
        };
        let ValidatedAddress::Identity { id: other_id, .. } = identity_destination() else {
            panic!("an identity destination");
        };

        let mut first = WalletSendScreen::new(&ctx, wallet());
        let _not_a_top_up = first.dispatch_send(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::GenerateReceiveAddress {
                seed_hash: [0x47; 32],
            },
        )));
        assert!(!top_up_in_flight_for(ctx.egui_ctx(), &recipient_id));
        let _top_up = first.dispatch_send(AppAction::BackendTask(BackendTask::IdentityTask(
            IdentityTask::TopUpIdentityFromPlatformAddresses {
                identity: recipient,
                inputs: Default::default(),
                wallet_seed_hash: [0x47; 32],
            },
        )));

        let mut second = WalletSendScreen::new(&ctx, wallet());
        second.amount = Some(Amount::new_dash(0.01));
        let core = testnet_core_address(2);
        let platform = PlatformAddress::try_from(core.clone()).expect("platform source");
        for source in [
            SourceSelection::CoreWallet,
            SourceSelection::PlatformAddresses(vec![(platform, core, 100_000_000_000)]),
        ] {
            second.selected_source = Some(source);
            second.validated_destination = Some(to(recipient_id));
            assert_eq!(
                second.validate_and_send().err().as_deref(),
                Some(TOP_UP_ALREADY_RUNNING),
                "a second top-up of the identity must wait for the first"
            );
            second.validated_destination = Some(to(other_id));
            assert_ne!(
                second.validate_and_send().err().as_deref(),
                Some(TOP_UP_ALREADY_RUNNING),
                "a top-up of another identity is not held back"
            );
        }
    }

    /// What the Max button offers an identity sender for `destination`, next to
    /// its balance less the `reserve` estimate.
    fn identity_max_and_expected(
        mode: IdentitySendMode,
        destination: ValidatedAddress,
        reserve: fn(&PlatformFeeEstimator) -> u64,
    ) -> (u64, u64) {
        let (ctx, _dir) = offline_ctx();
        let identity = bound_identity();
        let expected = identity.identity.balance() - reserve(&ctx.fee_estimator());
        let mut screen = WalletSendScreen::for_identity(&ctx, identity, mode);
        screen.validated_destination = Some(destination);
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.step();
        let max = harness
            .state()
            .amount
            .as_ref()
            .expect("Max sets the amount")
            .value();
        (max, expected)
    }

    #[test]
    fn identity_move_to_wallet_max_reserves_the_withdrawal_fee() {
        let (max, expected) = identity_max_and_expected(
            IdentitySendMode::MoveToWallet,
            ValidatedAddress::Core(testnet_core_address(9)),
            PlatformFeeEstimator::estimate_credit_withdrawal,
        );
        assert_eq!(
            max, expected,
            "Max must leave the withdrawal fee in the identity"
        );
    }

    #[test]
    fn identity_send_max_to_an_identity_reserves_the_transfer_fee() {
        let (max, expected) = identity_max_and_expected(
            IdentitySendMode::Send,
            identity_destination(),
            PlatformFeeEstimator::estimate_credit_transfer,
        );
        assert_eq!(
            max, expected,
            "a transfer between identities keeps its own, smaller reserve"
        );
    }

    #[test]
    fn identity_send_max_to_a_platform_address_reserves_the_address_transfer_fee() {
        let (max, expected) = identity_max_and_expected(
            IdentitySendMode::Send,
            platform_destination(),
            |estimator| estimator.estimate_credit_transfer_to_addresses(1),
        );
        assert_eq!(
            max, expected,
            "Max must leave the fee of a transfer to a Platform address in the identity"
        );
    }

    #[test]
    fn identity_transfer_to_a_platform_address_rejects_an_amount_that_leaves_no_fee() {
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        let (ctx, _dir) = offline_ctx();
        let transfer_key = key(1, dash_sdk::dpp::identity::Purpose::TRANSFER);
        let sender = sender_with_keys(vec![transfer_key.clone()], vec![transfer_key]);
        let highest = sender.identity.balance()
            - ctx.fee_estimator().estimate_credit_transfer_to_addresses(1);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.validated_destination = Some(platform_destination());

        screen.amount = Some(Amount::dash_from_credits(highest));
        assert!(
            screen.validate_and_send().is_ok(),
            "the amount Max offers must be accepted"
        );

        screen.amount = Some(Amount::dash_from_credits(highest + 1));
        let refusal = screen
            .validate_and_send()
            .expect_err("an amount that cannot also pay the fee must not reach the confirmation");
        assert!(
            refusal.contains(&format_credits_as_dash(highest)),
            "the refusal must name the highest amount that works: {refusal}"
        );
    }

    #[test]
    fn identity_fee_preview_shows_the_fee_its_destination_reserves() {
        let (ctx, _dir) = offline_ctx();
        let estimator = ctx.fee_estimator();
        let mut screen =
            WalletSendScreen::for_identity(&ctx, bound_identity(), IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for (destination, fee) in [
            (
                platform_destination(),
                estimator.estimate_credit_transfer_to_addresses(1),
            ),
            (identity_destination(), estimator.estimate_credit_transfer()),
        ] {
            screen.validated_destination = Some(destination);
            assert_eq!(
                screen
                    .current_fee_preview()
                    .map(|preview| preview.fee_credits),
                Some(fee),
                "the fee shown must be the fee Max and validation keep back"
            );
        }
    }

    #[test]
    fn identity_withdrawal_rejects_an_amount_that_leaves_no_fee() {
        let (ctx, _dir) = offline_ctx();
        let identity = sender_holding_a_transfer_key();
        let highest =
            identity.identity.balance() - ctx.fee_estimator().estimate_credit_withdrawal();
        let mut screen = WalletSendScreen::for_identity(&ctx, identity, IdentitySendMode::Send);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(9)));

        screen.amount = Some(Amount::dash_from_credits(highest));
        assert!(
            screen.validate_and_send().is_ok(),
            "the amount Max offers must be accepted"
        );

        screen.amount = Some(Amount::dash_from_credits(highest + 1));
        let refusal = screen
            .validate_and_send()
            .expect_err("an amount that cannot also pay the fee must not reach the confirmation");
        assert!(
            refusal.contains(&format_credits_as_dash(highest)),
            "the refusal must name the highest amount that works: {refusal}"
        );
    }

    #[test]
    fn identity_withdrawal_from_a_balance_that_only_covers_the_fee_asks_for_a_top_up() {
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        let (ctx, _dir) = offline_ctx();
        let mut identity = bound_identity();
        let fee = ctx.fee_estimator().estimate_credit_withdrawal();
        identity.identity.set_balance(fee);
        let mut screen = WalletSendScreen::for_identity(&ctx, identity, IdentitySendMode::Send);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(9)));
        screen.amount = Some(Amount::dash_from_credits(fee / 2));
        let refusal = screen
            .validate_and_send()
            .expect_err("a balance that only covers the fee leaves nothing to withdraw");
        assert!(
            refusal.contains("Top up") && !refusal.contains(&format_credits_as_dash(0)),
            "the refusal must not suggest an amount when none works: {refusal}"
        );
    }

    #[test]
    fn identity_send_without_wallet_can_reach_confirmation() {
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        let (ctx, _dir) = offline_ctx();
        let transfer_key = key(1, dash_sdk::dpp::identity::Purpose::TRANSFER);
        let sender = sender_with_keys(vec![transfer_key.clone()], vec![transfer_key]);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.validated_destination = Some(identity_destination());
        screen.amount = Some(Amount::new_dash(0.01));
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen| {
                screen.render_send_button(ui);
            },
            screen,
        );
        harness.get_by_label("Transfer Credits").click();
        harness.run();
        assert!(
            harness.state().send_confirmation.is_some(),
            "an identity sender needs no unrelated wallet to review its transfer"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_move_to_wallet_withdraws_only_to_an_address_of_a_loaded_wallet() {
        use crate::model::wallet::DerivationPathHelpers;
        const SEED: [u8; 64] = [9; 64];

        let (ctx, _dir) = offline_ctx();
        // Loaded before the backend is wired, so nothing registers the wallet
        // in the background and the awaited calls below are the only writers.
        let wallet = Wallet::new_from_seed(SEED, Network::Testnet, None, None).expect("wallet");
        let (seed_hash, _) = ctx
            .register_wallet(
                wallet,
                &SEED,
                crate::model::wallet::birth_height::WalletOrigin::Fresh,
            )
            .expect("loaded wallet");
        let (sender, _receiver) = tokio::sync::mpsc::channel(32);
        ctx.ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
            sender,
            ctx.egui_ctx().clone(),
        ))
        .await
        .expect("offline wallet backend");
        ctx.wallet_backend()
            .expect("wallet backend")
            .register_wallet_from_seed(&seed_hash, &SEED, None)
            .await
            .expect("wallet addresses");
        let owned = ctx
            .snapshot_address_paths(&seed_hash)
            .into_iter()
            .find_map(|(address, path)| path.is_bip44_external(Network::Testnet).then_some(address))
            .expect("a loaded wallet owns a receiving address");
        let external = testnet_core_address(9);
        assert_ne!(owned, external);

        let identity = sender_holding_a_transfer_key();
        let amount = Amount::new_dash(0.01);
        let mut screen =
            WalletSendScreen::for_identity(&ctx, identity.clone(), IdentitySendMode::MoveToWallet);
        screen.amount = Some(amount.clone());
        screen.validated_destination = Some(ValidatedAddress::Core(external));
        assert!(
            screen.validated_send_route().is_err(),
            "an address outside the loaded wallets must be refused"
        );
        screen.reset_form();
        assert!(matches!(
            screen.selected_source,
            Some(SourceSelection::Identity(_))
        ));

        screen.amount = Some(amount.clone());
        screen.validated_destination = Some(ValidatedAddress::Core(owned.clone()));
        let observed_action = Rc::new(RefCell::new(AppAction::None));
        let captured_action = observed_action.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                move |ui, screen: &mut WalletSendScreen| {
                    let mut action = screen.render_send_button(ui);
                    action |= screen.render_send_confirmation(ui);
                    if !matches!(action, AppAction::None) {
                        *captured_action.borrow_mut() = action;
                    }
                },
                screen,
            );
        harness.run();
        click_in_one_frame(&mut harness, "Withdraw Credits");
        assert!(
            matches!(*observed_action.borrow(), AppAction::None),
            "nothing is dispatched before the user confirms"
        );
        harness.get_by_label("Confirm and Send").click_accesskit();
        harness.step();
        match &*observed_action.borrow() {
            AppAction::BackendTaskWithContext {
                task:
                    BackendTask::IdentityTask(IdentityTask::WithdrawFromIdentity(
                        sender,
                        destination,
                        credits,
                        _,
                    )),
                ..
            } => {
                assert_eq!(sender.identity.id(), identity.identity.id());
                assert_eq!(destination.as_ref(), Some(&owned));
                assert_eq!(*credits, amount.value());
            }
            other => panic!("expected the bound identity's withdrawal, got {other:?}"),
        }

        ctx.remove_wallet(&seed_hash).expect("wallet removal");
        let screen = harness.state_mut();
        screen.amount = Some(amount);
        screen.validated_destination = Some(ValidatedAddress::Core(owned));
        assert!(
            screen.validated_send_route().is_err(),
            "an address stops being a valid destination once its wallet is removed"
        );
    }

    #[test]
    fn identity_send_network_switch_never_rebinds_the_identity() {
        let (ctx, _dir) = offline_ctx();
        let mut identity = bound_identity();
        identity.network = Network::Mainnet;
        let mut screen =
            WalletSendScreen::for_identity(&ctx, identity.clone(), IdentitySendMode::Send);
        screen.reset_for_network_switch();
        screen.selected_source = Some(SourceSelection::Identity(Box::new(identity)));
        screen.validated_destination = Some(identity_destination());
        screen.amount = Some(Amount::new_dash(0.01));
        assert!(screen.validated_send_route().is_err());
    }

    /// A funded sender with `on_chain` keys, of which only `held` can sign here.
    fn sender_with_keys(
        on_chain: Vec<dash_sdk::platform::IdentityPublicKey>,
        held: Vec<dash_sdk::platform::IdentityPublicKey>,
    ) -> QualifiedIdentity {
        use crate::model::qualified_identity::IdentityType;
        use crate::model::qualified_identity::withdrawal_key_tests::build_identity;
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        let mut sender = build_identity(IdentityType::User, on_chain, held);
        sender.identity.set_balance(100_000_000_000);
        sender
    }

    /// A sender holding the one active transfer key it has, so it can sign.
    fn sender_holding_a_transfer_key() -> QualifiedIdentity {
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        let transfer_key = key(1, dash_sdk::dpp::identity::Purpose::TRANSFER);
        sender_with_keys(vec![transfer_key.clone()], vec![transfer_key])
    }

    /// The signing key a validated identity transfer to `destination` carries.
    fn dispatched_transfer_key(
        screen: &mut WalletSendScreen,
        destination: ValidatedAddress,
    ) -> Option<dash_sdk::dpp::identity::KeyID> {
        screen.validated_destination = Some(destination);
        match screen
            .validate_and_send()
            .expect("a usable transfer key lets the send proceed")
        {
            AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::Transfer(
                _,
                _,
                _,
                key_id,
            )))
            | AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::TransferToAddresses { key_id, .. },
            )) => key_id,
            other => panic!("expected an identity transfer, got {other:?}"),
        }
    }

    #[test]
    fn identity_send_signs_with_the_active_transfer_key_when_an_earlier_one_is_disabled() {
        use crate::model::qualified_identity::withdrawal_key_tests::{disabled_key, key};
        use dash_sdk::dpp::identity::Purpose;
        let (ctx, _dir) = offline_ctx();
        let disabled = disabled_key(0, Purpose::TRANSFER);
        let active = key(1, Purpose::TRANSFER);
        let sender = sender_with_keys(
            vec![disabled.clone(), active.clone()],
            vec![disabled, active],
        );
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            assert_eq!(
                dispatched_transfer_key(&mut screen, destination),
                Some(1),
                "the network rejects a disabled key, so the active one must sign"
            );
        }
    }

    #[test]
    fn identity_send_signs_with_the_locally_held_transfer_key_on_a_partial_import() {
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        use dash_sdk::dpp::identity::Purpose;
        let (ctx, _dir) = offline_ctx();
        let not_imported = key(3, Purpose::TRANSFER);
        let imported = key(5, Purpose::TRANSFER);
        let sender = sender_with_keys(vec![not_imported, imported.clone()], vec![imported]);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            assert_eq!(
                dispatched_transfer_key(&mut screen, destination),
                Some(5),
                "only a key held in this app can sign"
            );
        }
    }

    #[test]
    fn identity_send_without_a_usable_transfer_key_is_refused_before_confirmation() {
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        use dash_sdk::dpp::identity::Purpose;
        let (ctx, _dir) = offline_ctx();
        let sender = sender_with_keys(vec![key(3, Purpose::TRANSFER)], vec![]);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            screen.validated_destination = Some(destination);
            assert_eq!(
                screen.validate_and_send().err(),
                Some(TaskError::NoUsableTransferKey.to_string()),
                "a transfer no key here can sign must not reach the confirmation"
            );
        }
    }

    /// A transfer key kept only as a reference into a wallet this device does
    /// not hold cannot sign. The form must pass over it for a key it can use,
    /// and refuse before the confirmation when it is the only one.
    #[test]
    fn identity_send_passes_over_a_transfer_key_whose_wallet_is_not_on_this_device() {
        use crate::context::test_support::hold_as_wallet_reference;
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        use dash_sdk::dpp::identity::Purpose;
        let (ctx, _dir) = offline_ctx();
        let from_missing_wallet = key(1, Purpose::TRANSFER);
        let imported = key(2, Purpose::TRANSFER);

        let mut sender = sender_with_keys(
            vec![from_missing_wallet.clone(), imported.clone()],
            vec![imported],
        );
        hold_as_wallet_reference(&mut sender, 1, [0x5A; 32]);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            assert_eq!(
                dispatched_transfer_key(&mut screen, destination),
                Some(2),
                "the key whose material is on this device must sign"
            );
        }

        let mut sender = sender_with_keys(vec![from_missing_wallet], vec![]);
        hold_as_wallet_reference(&mut sender, 1, [0x5A; 32]);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            screen.validated_destination = Some(destination);
            assert_eq!(
                screen.validate_and_send().err(),
                Some(TaskError::NoUsableTransferKey.to_string()),
                "a transfer only a missing wallet could sign must not reach the confirmation"
            );
        }
    }

    /// A transfer key kept only in the legacy encrypted format cannot sign.
    /// The form must pass over it for a key it can use, and refuse before the
    /// confirmation when it is the only one.
    #[test]
    fn identity_send_passes_over_a_transfer_key_held_only_in_the_legacy_encrypted_format() {
        use crate::context::test_support::hold_as_legacy_encrypted;
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        use dash_sdk::dpp::identity::Purpose;
        let (ctx, _dir) = offline_ctx();
        let legacy = key(1, Purpose::TRANSFER);
        let imported = key(2, Purpose::TRANSFER);

        let mut sender = sender_with_keys(vec![legacy.clone(), imported.clone()], vec![imported]);
        hold_as_legacy_encrypted(&mut sender, 1);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            assert_eq!(
                dispatched_transfer_key(&mut screen, destination),
                Some(2),
                "the key this version can read must sign"
            );
        }

        let mut sender = sender_with_keys(vec![legacy], vec![]);
        hold_as_legacy_encrypted(&mut sender, 1);
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        for destination in [identity_destination(), platform_destination()] {
            screen.validated_destination = Some(destination);
            assert_eq!(
                screen.validate_and_send().err(),
                Some(TaskError::NoUsableTransferKey.to_string()),
                "a transfer only an unreadable key could sign must not reach the confirmation"
            );
        }
    }

    /// Moving funds to a wallet is signed too: with no key that can sign it,
    /// the form refuses before the confirmation instead of after it.
    #[test]
    fn identity_withdrawal_without_a_usable_key_is_refused_before_confirmation() {
        use crate::context::test_support::hold_as_wallet_reference;
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        use dash_sdk::dpp::identity::Purpose;
        let (ctx, _dir) = offline_ctx();
        let from_missing_wallet = key(1, Purpose::TRANSFER);
        let imported = key(2, Purpose::TRANSFER);
        let to_core = || Some(ValidatedAddress::Core(testnet_core_address(9)));

        let mut sender = sender_with_keys(vec![from_missing_wallet.clone()], vec![]);
        hold_as_wallet_reference(&mut sender, 1, [0x5A; 32]);
        let mut screen =
            WalletSendScreen::for_identity(&ctx, sender.clone(), IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        screen.validated_destination = to_core();
        assert_eq!(
            screen.send_identity_to_core(sender).err(),
            Some(TaskError::NoUsableWithdrawalKey.to_string()),
            "a withdrawal no key here can sign must not reach the confirmation"
        );

        let mut sender =
            sender_with_keys(vec![from_missing_wallet, imported.clone()], vec![imported]);
        hold_as_wallet_reference(&mut sender, 1, [0x5A; 32]);
        let mut screen =
            WalletSendScreen::for_identity(&ctx, sender.clone(), IdentitySendMode::Send);
        screen.amount = Some(Amount::new_dash(0.01));
        screen.validated_destination = to_core();
        assert!(
            screen.send_identity_to_core(sender).is_ok(),
            "a key that can sign lets the withdrawal proceed"
        );
    }

    /// The fee of a transfer to another identity is paid on top from the same
    /// balance, so an amount that leaves nothing for it must be refused.
    #[test]
    fn identity_transfer_to_an_identity_rejects_an_amount_that_leaves_no_fee() {
        use crate::model::qualified_identity::withdrawal_key_tests::key;
        let (ctx, _dir) = offline_ctx();
        let transfer_key = key(1, dash_sdk::dpp::identity::Purpose::TRANSFER);
        let sender = sender_with_keys(vec![transfer_key.clone()], vec![transfer_key]);
        let highest = sender.identity.balance() - ctx.fee_estimator().estimate_credit_transfer();
        let mut screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        screen.validated_destination = Some(identity_destination());

        screen.amount = Some(Amount::dash_from_credits(highest));
        assert!(
            screen.validate_and_send().is_ok(),
            "the amount Max offers must be accepted"
        );

        screen.amount = Some(Amount::dash_from_credits(highest + 1));
        let refusal = screen
            .validate_and_send()
            .expect_err("an amount that cannot also pay the fee must not reach the confirmation");
        assert!(
            refusal.contains(&format_credits_as_dash(highest)),
            "the refusal must name the highest amount that works: {refusal}"
        );
    }

    /// A send keeps running when its form is closed, and nothing is left to
    /// end the form's progress banner. The banner goes with the form.
    #[test]
    fn closing_the_form_takes_its_progress_banner_along() {
        let (mut screen, _temp_dir) = send_screen();
        let ctx = screen.app_context.egui_ctx().clone();
        let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
            seed_hash: [0x49; 32],
        });
        let _action = screen.confirm_send(&ctx, AppAction::BackendTask(send));
        assert!(
            MessageBanner::has_global(&ctx),
            "a confirmed send shows its progress"
        );

        drop(screen);
        assert!(
            !MessageBanner::has_global(&ctx),
            "no progress banner may outlive the form that owns it"
        );
    }

    /// Other tasks report results of the kinds a send ends with. While this
    /// screen waits on its own send, only that send's result completes it.
    #[test]
    fn only_the_result_of_its_own_send_completes_the_wait() {
        use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
        let (mut screen, _temp_dir) = send_screen();
        let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
            seed_hash: [0x48; 32],
        });
        let _action = screen.dispatch_send(AppAction::BackendTask(send.clone()));
        let own = screen
            .pending_send
            .context()
            .cloned()
            .expect("the dispatched send");
        let same_task_sent_again = BackendTaskContext::for_dispatch_on(&send, Network::Testnet);
        let credits_moved = || BackendTaskSuccessResult::TransferredCredits(FeeResult::new(1, 1));

        screen.display_backend_task_result(&same_task_sent_again, credits_moved());
        screen.display_backend_task_result(&BackendTaskContext::Unknown, credits_moved());
        assert_eq!(
            screen.send_status,
            SendStatus::WaitingForResult,
            "a result of another task is not the outcome of this send"
        );

        screen.display_backend_task_result(&own, credits_moved());
        assert!(
            matches!(screen.send_status, SendStatus::Complete(_)),
            "its own result completes the send"
        );
    }

    fn asset_lock_request_id(task: Option<BackendTask>) -> u64 {
        match task {
            Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                request_id, ..
            })) => request_id,
            other => panic!("expected asset-lock maximum request, got {other:?}"),
        }
    }

    #[test]
    fn same_frame_probe_and_confirmation_dispatch_are_both_preserved() {
        let seed_hash = [0x43; 32];
        let confirmation =
            BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
        let probe = BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
            seed_hash,
            snapshot_generation: 9,
            request_id: 17,
        });

        let action = crate::ui::append_concurrent_backend_tasks(
            AppAction::BackendTask(confirmation),
            vec![probe],
        );
        let AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Concurrent) = action else {
            panic!("same-frame tasks must be dispatched as one concurrent batch");
        };
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount { request_id: 17, .. })
        )));
        assert!(tasks.iter().any(|task| matches!(
            task,
            BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { .. })
        )));
        assert!(
            !crate::ui::can_append_concurrent_backend_tasks(&AppAction::PopScreen),
            "navigation must keep priority instead of being overwritten by a probe"
        );
        let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
        assert!(
            !crate::ui::can_append_concurrent_backend_tasks(&AppAction::BackendTaskWithContext {
                context: BackendTaskContext::for_dispatch_on(&send, Network::Testnet),
                task: send,
            }),
            "a confirmed send keeps its own context; the probe is asked for on the next frame"
        );
    }

    #[test]
    fn confirmed_send_action_survives_same_frame_navigation() {
        let seed_hash = [0x44; 32];
        let confirmation = || {
            let task = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress { seed_hash });
            AppAction::BackendTaskWithContext {
                context: BackendTaskContext::for_dispatch_on(&task, Network::Testnet),
                task,
            }
        };
        let is_the_send = |action: &AppAction| {
            matches!(
                action,
                AppAction::BackendTaskWithContext {
                    task: BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
                        seed_hash: sh
                    }),
                    ..
                } if *sh == seed_hash
            )
        };

        let merged = merge_confirmation_action(AppAction::PopScreen, confirmation());
        assert!(
            is_the_send(&merged),
            "a confirmed send must outrank a same-frame navigation action, never be dropped"
        );

        let sequential = AppAction::BackendTasks(
            vec![BackendTask::WalletTask(
                WalletTask::GenerateReceiveAddress {
                    seed_hash: [0x45; 32],
                },
            )],
            BackendTasksExecutionMode::Sequential,
        );
        let merged = merge_confirmation_action(sequential, confirmation());
        assert!(
            is_the_send(&merged),
            "a confirmed send must take the pre-existing |= precedence over a sequential batch"
        );

        assert!(
            matches!(
                merge_confirmation_action(AppAction::PopScreen, AppAction::None),
                AppAction::PopScreen
            ),
            "without a confirmation the navigation action must pass through unchanged"
        );

        assert!(
            is_the_send(&merge_confirmation_action(AppAction::None, confirmation())),
            "a confirmed send goes out alone, under its own context"
        );
    }

    #[test]
    fn max_button_offers_nothing_while_the_builder_quote_is_revalidating() {
        const STALE_QUOTE_DUFFS: u64 = 900_000;

        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let (snapshot_generation, current_inputs, utxo_revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            snapshot_generation,
            current_inputs.clone(),
            utxo_revision,
        ));
        // The probe's reply observed a composition that no longer matches the
        // wallet — the exact revalidation window behind issue #929.
        screen.asset_lock_balance.store(
            seed_hash,
            snapshot_generation,
            request_id,
            STALE_QUOTE_DUFFS,
            different_asset_lock_inputs(0x77),
            false,
        );
        assert_eq!(
            screen.asset_lock_balance.get(&seed_hash),
            Some(STALE_QUOTE_DUFFS),
            "sanity: the stale quote must stay displayable"
        );
        assert_eq!(
            screen
                .asset_lock_balance
                .get_current(&seed_hash, &current_inputs),
            None,
            "sanity: validation must reject the stale quote"
        );
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.step();

        assert!(
            harness.state().amount.is_none(),
            "Max must never offer an amount that same-composition validation would refuse"
        );
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
    fn core_asset_lock_max_and_validation_use_builder_quote() {
        const BUILDER_MAX_DUFFS: u64 = 10_000_000;

        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let (platform_fee_duffs, _) = screen
            .shield_from_core_fees_duffs()
            .expect("shield fees for the test protocol version");
        assert!(BUILDER_MAX_DUFFS > platform_fee_duffs);

        let (snapshot_generation, final_funds_duffs, utxo_revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            snapshot_generation,
            final_funds_duffs.clone(),
            utxo_revision,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            snapshot_generation,
            request_id,
            BUILDER_MAX_DUFFS,
            final_funds_duffs,
            false,
        );
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.step();

        let max_amount_duffs = harness
            .state()
            .amount
            .as_ref()
            .expect("Max sets the amount")
            .dash_to_duffs()
            .expect("DASH amount");
        assert_eq!(
            max_amount_duffs,
            BUILDER_MAX_DUFFS - platform_fee_duffs,
            "Max reserves the platform fee from the real builder ceiling"
        );
        assert!(
            harness.state_mut().send_core_to_shielded(seed_hash).is_ok(),
            "the same ceiling must accept the Max amount"
        );

        harness.state_mut().amount = Some(Amount::dash_from_duffs(max_amount_duffs + 1));
        let error = harness
            .state_mut()
            .send_core_to_shielded(seed_hash)
            .expect_err("one duff above Max must be rejected before dispatch");
        assert!(
            error.starts_with("You can transfer up to "),
            "validation should report the builder-derived ceiling: {error}"
        );

        harness.state_mut().validated_destination = Some(ValidatedAddress::Identity {
            id: dash_sdk::dpp::prelude::Identifier::new([0x29; 32]),
            dpns_name: None,
        });
        harness.step();
        harness.get_by_label("Max").click_accesskit();
        harness.step();

        let identity_max_duffs = harness
            .state()
            .amount
            .as_ref()
            .expect("Identity Max sets the amount")
            .dash_to_duffs()
            .expect("DASH amount");
        let identity_fee_credits = harness
            .state()
            .app_context
            .fee_estimator()
            .estimate_identity_topup();
        let identity_fee_duffs = identity_fee_credits.div_ceil(CREDITS_PER_DUFF);
        assert_eq!(
            identity_max_duffs,
            BUILDER_MAX_DUFFS.saturating_sub(identity_fee_duffs),
            "Identity Max must reserve the same top-up fee as the identity top-up screen"
        );
        let downstream_error = harness
            .state_mut()
            .send_core_to_identity(seed_hash)
            .expect_err("the offline test cannot resolve the destination identity");
        assert!(
            downstream_error.starts_with("Could not look up identity:")
                || downstream_error.starts_with("No identity found with this ID."),
            "builder-derived Max must pass amount validation and reach identity lookup: \
             {downstream_error}"
        );

        harness.state_mut().amount = Some(Amount::dash_from_duffs(identity_max_duffs + 1));
        let error = harness
            .state_mut()
            .send_core_to_identity(seed_hash)
            .expect_err("one duff above Identity Max must be rejected before dispatch");
        assert!(
            error.starts_with("You can transfer up to "),
            "validation should report the builder-derived identity ceiling: {error}"
        );

        let platform_destination =
            PlatformAddress::try_from(testnet_core_address(4)).expect("platform destination");
        harness.state_mut().validated_destination = Some(ValidatedAddress::Platform {
            address: platform_destination,
            bech32m: platform_destination.to_bech32m_string(Network::Testnet),
        });
        harness.step();
        harness.get_by_label("Max").click_accesskit();
        harness.step();

        let platform_max_duffs = harness
            .state()
            .amount
            .as_ref()
            .expect("Platform Max sets the amount")
            .dash_to_duffs()
            .expect("DASH amount");
        let platform_fee_credits = estimate_address_funding_fee_from_transition(
            harness.state().app_context.platform_version(),
            &platform_destination,
        );
        assert_eq!(
            platform_max_duffs,
            BUILDER_MAX_DUFFS
                .saturating_mul(CREDITS_PER_DUFF)
                .saturating_sub(platform_fee_credits)
                / CREDITS_PER_DUFF,
            "Platform Max must start from the builder ceiling before reserving its fee"
        );
        assert!(
            harness.state_mut().send_core_to_platform(seed_hash).is_ok(),
            "the builder-derived Platform Max must pass pre-send validation"
        );

        harness.state_mut().amount = Some(Amount::dash_from_duffs(platform_max_duffs + 1));
        let error = harness
            .state_mut()
            .send_core_to_platform(seed_hash)
            .expect_err("one duff above Platform Max must be rejected before dispatch");
        assert!(
            error.starts_with("You can transfer up to "),
            "Platform validation should report the builder-derived ceiling: {error}"
        );
    }

    #[test]
    fn core_asset_lock_validation_rejects_quote_for_stale_utxo_composition() {
        const STALE_MAX_DUFFS: u64 = 10_000_000;

        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let (_, current_final_funds, current_revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let stale_inputs = different_asset_lock_inputs(0x39);

        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            7,
            current_final_funds,
            current_revision,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            7,
            request_id,
            STALE_MAX_DUFFS,
            stale_inputs,
            false,
        );
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        screen.amount = Some(Amount::dash_from_duffs(1));

        let error = screen
            .send_core_to_shielded(seed_hash)
            .expect_err("stale composition quote must not authorize a send");
        assert!(
            error.contains("still being checked"),
            "validation must wait for the current composition quote: {error}"
        );
    }

    #[test]
    fn core_asset_lock_dispatch_distinguishes_failed_probe_from_loading() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let (snapshot_generation, final_funds_duffs, utxo_revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);

        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            snapshot_generation,
            final_funds_duffs,
            utxo_revision,
        ));
        let loading_message = screen
            .asset_lock_max_amount(&seed_hash)
            .expect_err("an in-flight probe must block dispatch");
        screen
            .asset_lock_balance
            .mark_loading_failed(&seed_hash, snapshot_generation, request_id);
        let failed_message = screen
            .asset_lock_max_amount(&seed_hash)
            .expect_err("a failed probe must block dispatch");

        assert!(loading_message.contains("still being checked"));
        assert!(
            failed_message.starts_with("The available amount could not be checked."),
            "failed dispatch must direct the user to the retryable state: {failed_message}"
        );
        assert_ne!(
            failed_message, loading_message,
            "a permanently failed probe must not be described as still loading"
        );
    }

    /// The shield planner decides which Platform addresses it may spend, so
    /// Max for a shield from Platform is the planner's quote — never the
    /// balance minus a fee — and nothing is offered before the quote arrives.
    #[test]
    fn shield_from_platform_max_is_the_planner_quote() {
        const BALANCE_CREDITS: u64 = 2 * CREDITS_PER_DUFF * 100_000_000;
        const QUOTE_CREDITS: u64 = 123_456_789_000;

        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            BALANCE_CREDITS,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");

        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
            kind: SpendLimitKind::ShieldFromPlatform,
            request_id,
            ..
        })) = screen.request_spend_limit()
        else {
            panic!("the shield form must ask for the planner's quote");
        };
        assert_eq!(
            screen.spend_limit(SpendLimitKind::ShieldFromPlatform),
            None,
            "no Max before the quote arrives"
        );
        assert!(
            screen.request_spend_limit().is_none(),
            "the quote is asked for once per balance"
        );

        let quote = BackendTaskSuccessResult::SpendLimit {
            seed_hash,
            kind: SpendLimitKind::ShieldFromPlatform,
            request_id,
            max_credits: QUOTE_CREDITS,
        };
        assert!(screen.refresh_form_inputs(&quote));

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.step();

        assert_eq!(
            harness.state().amount.as_ref().map(Amount::value),
            Some(QUOTE_CREDITS),
            "Max must be exactly what the planner accepts"
        );
    }

    /// The Platform balances captured when the source was picked can lag a
    /// sync that raised them, so they must not veto a shield: the planner
    /// decides, and refuses an amount above its maximum itself.
    #[test]
    fn shield_from_platform_send_is_not_refused_by_the_captured_balances() {
        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            CREDITS_PER_DUFF,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        screen.amount = Some(Amount::new_dash(1.0));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");

        let action = screen.send_platform_to_shielded(seed_hash);

        assert!(
            matches!(
                action,
                Ok(AppAction::BackendTask(BackendTask::ShieldedTask(
                    crate::backend_task::shielded::ShieldedTask::ShieldFromBalance { amount, .. }
                ))) if amount == Amount::new_dash(1.0).value()
            ),
            "the shield must reach the planner: {action:?}"
        );
    }

    /// When the planner can shield nothing (no address can keep the fee
    /// reserve back), there is no Max to offer: the form says why instead of
    /// a Max that fills in zero.
    #[test]
    fn shield_from_platform_offers_no_max_when_nothing_can_be_shielded() {
        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            CREDITS_PER_DUFF,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit { request_id, .. })) =
            screen.request_spend_limit()
        else {
            panic!("the shield form must ask for the planner's quote");
        };
        assert!(
            screen.refresh_form_inputs(&BackendTaskSuccessResult::SpendLimit {
                seed_hash,
                kind: SpendLimitKind::ShieldFromPlatform,
                request_id,
                max_credits: 0,
            })
        );

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.run();

        let field_text = harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .value()
            .unwrap_or_default();
        assert!(
            field_text.is_empty(),
            "a zero quote must not be offered as Max, got {field_text:?}"
        );
        assert!(
            harness
                .query_all(
                    egui_kittest::kittest::By::new()
                        .label_contains("so nothing can be shielded right now")
                        .include_labels()
                )
                .next()
                .is_some(),
            "the form must say why nothing can be shielded"
        );
    }

    /// A Platform balance that changes after the shield quote (a balance
    /// fetch or a sync push, neither of which re-selects the source) makes
    /// that quote stale: Max is withdrawn and a fresh quote is asked for.
    #[test]
    fn shield_from_platform_max_is_requoted_when_live_balances_change() {
        const BALANCE_CREDITS: u64 = 2 * CREDITS_PER_DUFF * 100_000_000;
        const QUOTE_CREDITS: u64 = 123_456_789_000;

        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        let wallet = screen.selected_wallet.clone().expect("selected wallet");
        wallet
            .write()
            .expect("wallet lock")
            .set_platform_address_info(source_core.clone(), BALANCE_CREDITS, 1);
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core.clone(),
            BALANCE_CREDITS,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit { request_id, .. })) =
            screen.request_spend_limit()
        else {
            panic!("the shield form must ask for the planner's quote");
        };
        assert!(
            screen.refresh_form_inputs(&BackendTaskSuccessResult::SpendLimit {
                seed_hash,
                kind: SpendLimitKind::ShieldFromPlatform,
                request_id,
                max_credits: QUOTE_CREDITS,
            })
        );
        assert_eq!(
            screen.spend_limit(SpendLimitKind::ShieldFromPlatform),
            Some(QUOTE_CREDITS)
        );

        // The balance drops behind the form's back; the selected source keeps
        // its old figures.
        wallet
            .write()
            .expect("wallet lock")
            .set_platform_address_info(source_core, BALANCE_CREDITS / 4, 2);

        assert_eq!(
            screen.spend_limit(SpendLimitKind::ShieldFromPlatform),
            None,
            "a quote made against the old balance must not back Max"
        );
        assert!(
            matches!(
                screen.request_spend_limit(),
                Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
                    kind: SpendLimitKind::ShieldFromPlatform,
                    ..
                }))
            ),
            "the changed balance must be quoted afresh"
        );
    }

    /// The display snapshot can lag (or be empty before its first update)
    /// while the pooled quote already offers a positive Max, so it must not
    /// veto a Core send: the backend's pooled build decides and refuses a
    /// real shortfall with a typed error.
    #[test]
    fn core_send_is_not_refused_by_the_display_balance() {
        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(3)));
        screen.amount = Some(Amount::new_dash(1.0));
        assert_eq!(screen.get_core_balance(), 0, "precondition: empty snapshot");

        let action = screen.send_core_to_core();

        assert!(
            matches!(
                action,
                Ok(AppAction::BackendTask(BackendTask::CoreTask(
                    CoreTask::SendWalletPayment { ref request, .. }
                ))) if request.recipients.len() == 1
                    && request.recipients[0].amount_duffs == 100_000_000
            ),
            "the send must reach the backend: {action:?}"
        );
    }

    /// Core sends pool several accounts, so Max for a Core payment is the
    /// backend's pooled quote, never the displayed balance less an estimate.
    #[test]
    fn core_send_max_is_the_backend_quote() {
        const QUOTE_DUFFS: u64 = 1_234_567;

        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(3)));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");

        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
            kind:
                SpendLimitKind::CoreSend {
                    destination: crate::model::spend_limit::CoreDestinationScript::PubkeyHash,
                },
            request_id,
            ..
        })) = screen.request_spend_limit()
        else {
            panic!("the Core send form must ask for the pooled quote");
        };
        assert!(
            screen.refresh_form_inputs(&BackendTaskSuccessResult::SpendLimit {
                seed_hash,
                kind: SpendLimitKind::CoreSend {
                    destination: Default::default()
                },
                request_id,
                max_credits: QUOTE_DUFFS * CREDITS_PER_DUFF,
            })
        );

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.step();

        assert_eq!(
            harness.state().amount.as_ref().map(Amount::value),
            Some(QUOTE_DUFFS * CREDITS_PER_DUFF),
            "Max must be exactly the pooled quote"
        );
    }

    /// Core Max is quoted for the script the chosen destination pays to, and
    /// a change of destination kind asks for that kind's own quote.
    #[test]
    fn core_send_max_is_requested_for_the_destination_script() {
        use crate::model::spend_limit::CoreDestinationScript;
        use dash_sdk::dpp::dashcore::hashes::Hash;

        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        let p2sh = Address::new(
            Network::Testnet,
            dash_sdk::dpp::dashcore::address::Payload::ScriptHash(
                dash_sdk::dpp::dashcore::ScriptHash::from_byte_array([9; 20]),
            ),
        );
        screen.validated_destination = Some(ValidatedAddress::Core(p2sh));
        assert!(
            matches!(
                screen.request_spend_limit(),
                Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
                    kind: SpendLimitKind::CoreSend {
                        destination: CoreDestinationScript::ScriptHash
                    },
                    ..
                }))
            ),
            "a P2SH destination is quoted as P2SH"
        );

        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(3)));
        assert!(
            matches!(
                screen.request_spend_limit(),
                Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
                    kind: SpendLimitKind::CoreSend {
                        destination: CoreDestinationScript::PubkeyHash
                    },
                    ..
                }))
            ),
            "a P2PKH destination needs its own quote"
        );
    }

    /// Busy withholds only Max: an explicit asset-lock amount still reaches
    /// the backend, whose reservation-aware dry run and the build decide it.
    #[test]
    fn an_explicit_asset_lock_amount_is_sent_while_another_transaction_is_in_progress() {
        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        screen.amount = Some(Amount::new_dash(0.5));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let Some(
            task @ BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
                snapshot_generation,
                request_id,
                ..
            }),
        ) = screen.request_asset_lock_max_amount()
        else {
            panic!("the asset-lock form must ask for the builder ceiling");
        };
        let _ = task;
        screen.asset_lock_balance.mark_request_error(
            &seed_hash,
            snapshot_generation,
            request_id,
            &TaskError::SpendInProgress,
        );

        let action = screen.send_core_to_shielded(seed_hash);

        assert!(
            matches!(
                action,
                Ok(AppAction::BackendTask(BackendTask::ShieldedTask(
                    crate::backend_task::shielded::ShieldedTask::ShieldFromAssetLock {
                        amount_duffs: 50_000_000,
                        ..
                    }
                )))
            ),
            "an explicit amount must reach the backend: {action:?}"
        );
    }

    /// While another transaction from the wallet is in progress, the backend
    /// quotes no Max; the form offers none and says why, without a Retry
    /// that could only get the same answer, and asks again only once the
    /// wallet's quote revision moves.
    #[test]
    fn core_send_offers_no_max_while_another_transaction_is_in_progress() {
        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(3)));
        let task = screen
            .request_spend_limit()
            .expect("the Core send form must ask for the pooled quote");
        let context = BackendTaskContext::for_dispatch_on(&task, Network::Testnet);

        screen.display_backend_task_error(&context, &TaskError::SpendInProgress);

        assert!(
            screen.request_spend_limit().is_none(),
            "the same wallet state must not be re-quoted"
        );
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| {
                screen.render_amount_input(ui);
                screen.render_spend_limit_status(ui);
            },
            screen,
        );
        harness.run();
        let caption = harness
            .query_all(
                egui_kittest::kittest::By::new()
                    .label_contains(crate::model::spend_limit::SPEND_IN_PROGRESS_MESSAGE)
                    .include_labels(),
            )
            .count();
        assert_eq!(caption, 1, "the form must say why Max is unavailable");
        assert!(
            harness
                .query_by_label("Retry available amount check")
                .is_none(),
            "retrying cannot help while the transaction is in progress"
        );
        harness.get_by_label("Max").click_accesskit();
        harness.step();
        assert_eq!(
            harness
                .state()
                .amount
                .as_ref()
                .map(Amount::value)
                .unwrap_or(0),
            0,
            "Max must offer nothing"
        );
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
        let AppAction::BackendTaskWithContext { context, .. } = &*observed_action else {
            panic!(
                "the confirm action must dispatch the prepared backend task, got {observed_action:?}"
            );
        };
        assert_eq!(
            harness.state().pending_send.context(),
            Some(context),
            "the screen waits on the send it dispatched, and on no other task"
        );
    }

    /// Undispatched tasks' failures and messages reach whichever screen is in
    /// view (see `app/request_routing.rs`). While this screen waits on a send, only that send's own failure gives the form
    /// back: anything else would let the user send, and pay, a second time.
    #[test]
    fn only_the_failure_of_its_own_send_ends_the_wait() {
        let (mut screen, _temp_dir) = send_screen();
        let send = BackendTask::WalletTask(WalletTask::GenerateReceiveAddress {
            seed_hash: [0x46; 32],
        });
        let own = BackendTaskContext::for_dispatch_on(&send, Network::Testnet);
        let same_task_sent_again = BackendTaskContext::for_dispatch_on(&send, Network::Testnet);
        let error = TaskError::NoIdentitiesFound;
        screen.mark_sending(own.clone());

        screen.display_backend_task_error(&same_task_sent_again, &error);
        screen.display_backend_task_error(&BackendTaskContext::Unknown, &error);
        for message_type in [
            MessageType::Error,
            MessageType::Warning,
            MessageType::Success,
            MessageType::Info,
        ] {
            screen.display_message("Another task reported this.", message_type);
        }
        assert_eq!(
            screen.send_status,
            SendStatus::WaitingForResult,
            "nothing but its own send's outcome ends the wait"
        );

        screen.display_backend_task_error(&own, &error);
        assert_eq!(screen.send_status, SendStatus::Error);

        // With no send in flight a message is a side effect of this screen's
        // own work again.
        screen.display_message("The amount is too high.", MessageType::Error);
        assert_eq!(screen.send_status, SendStatus::Error);
        screen.send_status = SendStatus::NotStarted;
        screen.display_backend_task_error(&own, &error);
        assert_eq!(
            screen.send_status,
            SendStatus::NotStarted,
            "a failure that was already taken is not taken twice"
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
            matches!(*observed_action, AppAction::BackendTaskWithContext { .. }),
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

    /// Home → Send fixes the sender. With no wallet on the device, the
    /// recipient search still offers every other identity stored here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identity_bound_send_offers_the_other_local_identities_without_a_wallet() {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{DPNSNameInfo, IdentityStatus, IdentityType};
        use dash_sdk::dpp::{identity::Identity, version::PlatformVersion};
        let (ctx, _dir) = offline_ctx();
        let (sender, _receiver) = tokio::sync::mpsc::channel(32);
        ctx.ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
            sender,
            ctx.egui_ctx().clone(),
        ))
        .await
        .unwrap();
        let imported = |byte: u8, name: &str| QualifiedIdentity {
            identity: Identity::create_basic_identity(
                dash_sdk::platform::Identifier::from([byte; 32]),
                PlatformVersion::latest(),
            )
            .unwrap(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![DPNSNameInfo {
                name: name.into(),
                acquired_at: 0,
            }],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        let sender = imported(41, "sender.dash");
        for identity in [&sender, &imported(42, "payee.dash")] {
            ctx.insert_local_qualified_identity(identity, &None)
                .unwrap();
        }
        assert!(ctx.wallet_context().wallets().is_empty());

        let screen = WalletSendScreen::for_identity(&ctx, sender, IdentitySendMode::Send);
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_destination_input(ui),
            screen,
        );
        harness.run();
        for (query, offered) in [("payee", Some("payee.dash")), ("sender", None)] {
            let input = harness.state_mut().address_input.take().unwrap();
            harness.state_mut().address_input = Some(input.with_initial_value(query));
            harness.run();
            let rows = harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows();
            let names: Vec<_> = rows.iter().map(|row| row.name.as_deref()).collect();
            assert_eq!(names, Vec::from_iter(offered.map(Some)), "query {query}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn recipient_names_refresh_without_replacing_typed_search() {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{DPNSNameInfo, IdentityStatus, IdentityType};
        use dash_sdk::dpp::{
            identity::{Identity, accessors::IdentitySettersV0},
            version::PlatformVersion,
        };
        let (mut screen, _dir) = send_screen();
        let (sender, _receiver) = tokio::sync::mpsc::channel(32);
        screen
            .app_context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                sender,
                screen.app_context.egui_ctx().clone(),
            ))
            .await
            .unwrap();
        let wallet = Wallet::new_from_seed([1; 64], Network::Testnet, None, None).unwrap();
        let (hash, wallet) = screen
            .app_context
            .register_wallet(
                wallet,
                &[1; 64],
                crate::model::wallet::birth_height::WalletOrigin::Fresh,
            )
            .unwrap();
        screen.selected_wallet = Some(wallet);
        let id = dash_sdk::platform::Identifier::from([42; 32]);
        let identity = QualifiedIdentity {
            identity: Identity::create_basic_identity(id, PlatformVersion::latest()).unwrap(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some("Ignored device name".into()),
            private_keys: KeyStorage::default(),
            dpns_names: vec![DPNSNameInfo {
                name: "recipient.dash".into(),
                acquired_at: 0,
            }],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        screen
            .app_context
            .insert_local_qualified_identity(&identity, &Some((hash, 0)))
            .unwrap();
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_destination_input(ui),
            screen,
        );
        harness.run();
        let initial_loads = harness.state().identity_load_count.get();
        harness.run_steps(3);
        assert_eq!(
            harness.state().identity_load_count.get(),
            initial_loads,
            "stable destination frames must not reload identity storage"
        );
        let input = harness.state_mut().address_input.take().unwrap();
        harness.state_mut().address_input = Some(input.with_initial_value("Later"));
        harness.run();
        assert!(
            harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows()
                .is_empty()
        );
        harness
            .state()
            .app_context
            .save_identity_profile_name(id, Some("Later profile"));
        harness.run();
        let rows = harness
            .state()
            .address_input
            .as_ref()
            .unwrap()
            .rendered_rows();
        assert_eq!(
            rows.len(),
            1,
            "late profile name must match the existing typed query"
        );
        assert_eq!(rows[0].name.as_deref(), Some("Later profile"));
        harness
            .state()
            .app_context
            .save_identity_profile_name(id, None);
        harness.run();
        assert!(
            harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows()
                .is_empty(),
            "clearing the profile must keep the typed query and remove its old match"
        );
        let input = harness.state_mut().address_input.take().unwrap();
        harness.state_mut().address_input = Some(input.with_initial_value("recipient"));
        harness.run();
        assert_eq!(
            harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows()[0]
                .name
                .as_deref(),
            Some("recipient.dash")
        );
        assert_eq!(
            harness.state().identity_load_count.get(),
            initial_loads,
            "profile save and clear must not reload identity records"
        );
        let mut refreshed = identity;
        refreshed.identity.set_balance(123_000);
        refreshed.dpns_names[0].name = "recipient-updated.dash".into();
        harness
            .state()
            .app_context
            .insert_local_qualified_identity(&refreshed, &Some((hash, 0)))
            .unwrap();
        harness
            .state_mut()
            .display_task_result(BackendTaskSuccessResult::RefreshedIdentity(
                refreshed.clone(),
            ));
        harness.run();
        assert_eq!(harness.state().identity_load_count.get(), initial_loads + 1);
        assert_eq!(
            harness.state().address_input_identities[0]
                .identity
                .balance(),
            123_000
        );
        assert_eq!(
            harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows()[0]
                .name
                .as_deref(),
            Some("recipient-updated.dash")
        );
        harness.state_mut().selected_source = Some(SourceSelection::Identity(Box::new(refreshed)));
        harness.run();
        assert!(
            harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows()
                .is_empty(),
            "source identity must be excluded after changing source"
        );
        harness.state_mut().selected_source = Some(SourceSelection::CoreWallet);
        harness.run();
        assert_eq!(
            harness
                .state()
                .address_input
                .as_ref()
                .unwrap()
                .rendered_rows()
                .len(),
            1
        );
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
        let empty_wallets = vec![(wallet.clone(), BTreeMap::new(), BTreeMap::new(), None)];
        let empty_signature = WalletSendScreen::address_input_snapshot_signature(&empty_wallets);
        assert_eq!(
            empty_signature,
            WalletSendScreen::address_input_snapshot_signature(&empty_wallets)
        );

        let address = testnet_core_address(3);
        let paths = BTreeMap::from([(address.clone(), bip44_receive_path(0))]);
        let with_path = vec![(wallet.clone(), BTreeMap::new(), paths.clone(), None)];
        let path_signature = WalletSendScreen::address_input_snapshot_signature(&with_path);
        assert_ne!(empty_signature, path_signature);

        let with_balance = vec![(wallet, BTreeMap::from([(address, 42)]), paths, None)];
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

    /// Select Core as the source and store a builder ceiling for the wallet.
    fn core_source_with_ceiling(
        screen: &mut WalletSendScreen,
        ceiling_duffs: u64,
    ) -> WalletSeedHash {
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let (generation, inputs, revision) =
            screen.app_context.asset_lock_probe_snapshot(&seed_hash);
        let request_id = asset_lock_request_id(screen.asset_lock_balance.ensure_requested(
            seed_hash,
            generation,
            inputs.clone(),
            revision,
        ));
        screen.asset_lock_balance.store(
            seed_hash,
            generation,
            request_id,
            ceiling_duffs,
            inputs,
            false,
        );
        screen.selected_source = Some(SourceSelection::CoreWallet);
        seed_hash
    }

    fn identity_destination() -> ValidatedAddress {
        ValidatedAddress::Identity {
            id: dash_sdk::dpp::prelude::Identifier::new([0x29; 32]),
            dpns_name: None,
        }
    }

    fn platform_destination() -> ValidatedAddress {
        let address =
            PlatformAddress::try_from(testnet_core_address(4)).expect("platform destination");
        ValidatedAddress::Platform {
            address,
            bech32m: address.to_bech32m_string(Network::Testnet),
        }
    }

    /// Render the amount field, press Max when it is offered, and report the
    /// resulting amount and whether the "nothing covers the fee" notice is shown.
    fn press_max(screen: WalletSendScreen) -> (Option<Amount>, bool) {
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        if let Some(max) = harness.query_by_label("Max") {
            max.click_accesskit();
        }
        harness.step();
        let notice = harness
            .query_by_label_contains(FUNDING_FEE_NOT_COVERED)
            .is_some();
        (harness.state().amount.clone(), notice)
    }

    const TOP_UP_FEE_REFUSAL: &str = "This amount is too small to cover the network fee. \
                                      Enter at least 0.000505 DASH and try again.";
    const ADDRESS_FEE_REFUSAL: &str = "This amount is too small to cover the network fee. \
                                       Enter at least 0.00056 DASH and try again.";

    /// The amount sent to an identity is the whole funding, so the field must
    /// refuse anything below the fee the network takes from it.
    #[test]
    fn core_to_identity_field_requires_the_network_fee() {
        let (screen, _temp_dir) = send_screen();
        assert_eq!(
            screen.min_output_amount(Some(AddressKind::Core), Some(AddressKind::Identity)),
            Some(50_500 * CREDITS_PER_DUFF)
        );
    }

    #[test]
    fn core_to_platform_field_requires_the_network_fee() {
        let (screen, _temp_dir) = send_screen();
        assert_eq!(
            screen.min_output_amount(Some(AddressKind::Core), Some(AddressKind::Platform)),
            Some(56_000 * CREDITS_PER_DUFF)
        );
    }

    /// The amount of the reported case, sent from Wallet Send.
    #[test]
    fn core_to_identity_dispatch_refuses_an_amount_below_the_network_fee() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = core_source_with_ceiling(&mut screen, 10_000_000);
        screen.validated_destination = Some(identity_destination());
        screen.amount = Some(Amount::dash_from_duffs(5_237));

        let error = screen
            .send_core_to_identity(seed_hash)
            .expect_err("an amount below the network fee must be refused");
        assert_eq!(error, TOP_UP_FEE_REFUSAL);
    }

    /// A wallet that can build 55 737 duffs used to be told "You can transfer
    /// up to 0.00005237 DASH" — an amount the network refuses.
    #[test]
    fn core_to_identity_dispatch_never_suggests_an_amount_the_network_refuses() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = core_source_with_ceiling(&mut screen, 55_737);
        screen.validated_destination = Some(identity_destination());
        screen.amount = Some(Amount::dash_from_duffs(50_500));

        let error = screen
            .send_core_to_identity(seed_hash)
            .expect_err("no amount this wallet can send covers the network fee");
        assert_eq!(error, FUNDING_FEE_NOT_COVERED);
    }

    #[test]
    fn core_to_identity_max_offers_nothing_when_no_amount_covers_the_network_fee() {
        let (mut screen, _temp_dir) = send_screen();
        core_source_with_ceiling(&mut screen, 55_737);
        screen.validated_destination = Some(identity_destination());

        let (amount, notice) = press_max(screen);
        assert_eq!(amount, None, "Max must not produce an amount");
        assert!(notice, "the field must say why no amount is offered");
    }

    #[test]
    fn core_to_platform_dispatch_refuses_an_amount_below_the_network_fee() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = core_source_with_ceiling(&mut screen, 10_000_000);
        screen.validated_destination = Some(platform_destination());
        screen.amount = Some(Amount::dash_from_duffs(55_999));

        let error = screen
            .send_core_to_platform(seed_hash)
            .expect_err("an amount below the network fee must be refused");
        assert_eq!(error, ADDRESS_FEE_REFUSAL);
    }

    #[test]
    fn core_to_platform_dispatch_never_suggests_an_amount_the_network_refuses() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = core_source_with_ceiling(&mut screen, 100_000);
        screen.validated_destination = Some(platform_destination());
        screen.amount = Some(Amount::dash_from_duffs(56_000));

        let error = screen
            .send_core_to_platform(seed_hash)
            .expect_err("no amount this wallet can send covers the network fee");
        assert_eq!(error, FUNDING_FEE_NOT_COVERED);
    }

    #[test]
    fn core_to_platform_max_offers_nothing_when_no_amount_covers_the_network_fee() {
        let (mut screen, _temp_dir) = send_screen();
        core_source_with_ceiling(&mut screen, 100_000);
        screen.validated_destination = Some(platform_destination());

        let (amount, notice) = press_max(screen);
        assert_eq!(amount, None, "Max must not produce an amount");
        assert!(notice, "the field must say why no amount is offered");
    }

    /// What the network takes from a Core wallet funding that is shielded, in
    /// whole duffs: the shielded fee for the two actions of the bundle plus the
    /// cost of processing the funding transaction.
    fn network_shield_from_core_fee_duffs(screen: &WalletSendScreen) -> u64 {
        let platform_version = screen.app_context.sdk_platform_version();
        let funding_cost_duffs = platform_version
            .dpp
            .state_transitions
            .identities
            .asset_locks
            .required_asset_lock_duff_balance_for_processing_start_for_address_funding;
        let shielded_fee_credits =
            dash_sdk::dpp::shielded::compute_minimum_shielded_fee(2, platform_version)
                .expect("shielded fee");
        (shielded_fee_credits + funding_cost_duffs * CREDITS_PER_DUFF).div_ceil(CREDITS_PER_DUFF)
    }

    /// The fee shown before a shield from the Core wallet must be what the
    /// network takes plus the Core transaction fee, since that is what leaves
    /// the wallet on top of the amount.
    #[test]
    fn core_to_shielded_fee_preview_shows_what_the_network_takes() {
        let (mut screen, _temp_dir) = send_screen();
        core_source_with_ceiling(&mut screen, 10_000_000);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        screen.amount = Some(Amount::dash_from_duffs(1_000_000));
        let network_fee_duffs = network_shield_from_core_fee_duffs(&screen);

        let preview = screen.current_fee_preview().expect("a fee preview");

        assert_eq!(
            preview.fee_credits,
            (network_fee_duffs + 3_000) * CREDITS_PER_DUFF
        );
    }

    /// A wallet that can build less than the network's fee used to be offered
    /// an amount as Max, and shielding it failed.
    #[test]
    fn core_to_shielded_max_offers_nothing_when_the_wallet_cannot_cover_the_fee() {
        let (mut screen, _temp_dir) = send_screen();
        let network_fee_duffs = network_shield_from_core_fee_duffs(&screen);
        core_source_with_ceiling(&mut screen, network_fee_duffs);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));

        let (amount, notice) = press_max(screen);
        assert_eq!(amount, None, "Max must not produce an amount");
        assert!(notice, "the field must say why no amount is offered");
    }

    /// Max leaves exactly the network's fee out of what the wallet can build,
    /// so the funding it leads to is one the wallet can make.
    #[test]
    fn core_to_shielded_max_reserves_what_the_network_takes() {
        const CEILING_DUFFS: u64 = 10_000_000;
        let (mut screen, _temp_dir) = send_screen();
        let network_fee_duffs = network_shield_from_core_fee_duffs(&screen);
        core_source_with_ceiling(&mut screen, CEILING_DUFFS);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));

        let (amount, _) = press_max(screen);
        assert_eq!(
            amount.expect("Max sets the amount").dash_to_duffs(),
            Ok(CEILING_DUFFS - network_fee_duffs)
        );
    }

    /// An advanced Core to Platform send of `amount` DASH with `fee_strategy`.
    fn advanced_core_to_platform(
        amount: &str,
        fee_strategy: PlatformFeeStrategy,
    ) -> (WalletSendScreen, WalletSeedHash, tempfile::TempDir) {
        let (mut screen, temp_dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let destination =
            PlatformAddress::try_from(testnet_core_address(5)).expect("platform destination");
        screen.advanced_source_type = AdvancedSourceType::Core;
        screen.core_inputs = vec![CoreAddressInput {
            address: testnet_core_address(4),
            amount: "2".to_string(),
        }];
        screen.advanced_outputs = vec![AdvancedOutput {
            address: destination.to_bech32m_string(Network::Testnet),
            amount: amount.to_string(),
        }];
        screen.fee_strategy = fee_strategy;
        (screen, seed_hash, temp_dir)
    }

    /// With the fee paid on top, the funding is the amount plus 18 750 duffs,
    /// and the network requires 62 000 for the recipient and the change.
    #[test]
    fn advanced_core_to_platform_refuses_an_amount_the_fee_on_top_cannot_carry() {
        let (mut screen, seed_hash, _temp_dir) =
            advanced_core_to_platform("0.00043249", PlatformFeeStrategy::DeductFromFirstInput);
        let error = screen
            .send_advanced_core_to_platform(seed_hash)
            .expect_err("the funding would be below what the network requires");
        assert_eq!(
            error,
            "This amount is too small to cover the network fee. \
             Enter at least 0.0004325 DASH and try again."
        );

        let (mut screen, seed_hash, _temp_dir) =
            advanced_core_to_platform("0.0004325", PlatformFeeStrategy::DeductFromFirstInput);
        assert!(screen.send_advanced_core_to_platform(seed_hash).is_ok());
    }

    #[test]
    fn advanced_core_to_platform_refuses_an_amount_below_the_fee_taken_from_it() {
        let (mut screen, seed_hash, _temp_dir) =
            advanced_core_to_platform("0.00055999", PlatformFeeStrategy::ReduceFirstOutput);
        let error = screen
            .send_advanced_core_to_platform(seed_hash)
            .expect_err("an amount below the network fee must be refused");
        assert_eq!(error, ADDRESS_FEE_REFUSAL);

        let (mut screen, seed_hash, _temp_dir) =
            advanced_core_to_platform("0.00056", PlatformFeeStrategy::ReduceFirstOutput);
        assert!(screen.send_advanced_core_to_platform(seed_hash).is_ok());
    }

    /// A shield from the Core wallet whose amount is not known still completes
    /// as a success, and names no figure.
    #[test]
    fn shield_from_core_without_an_amount_completes_without_a_figure() {
        use crate::backend_task::BackendTaskSuccessResult;
        let (mut screen, _dir) = send_screen();

        screen.display_task_result(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash: [0; 32],
            amount: None,
        });
        assert!(
            matches!(
                &screen.send_status,
                SendStatus::Complete(message) if message == SHIELDED_FROM_CORE_WITHOUT_AMOUNT
            ),
            "got {:?}",
            screen.send_status
        );

        screen.display_task_result(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash: [0; 32],
            amount: Some(100_000_800),
        });
        assert!(
            matches!(
                &screen.send_status,
                SendStatus::Complete(message)
                    if message.starts_with("0.001000008 DASH shielded from asset lock")
            ),
            "got {:?}",
            screen.send_status
        );
    }

    /// After the app's own SDK has been rebuilt it can be on a newer protocol
    /// version than the wallet backend, which runs the transfer and sizes its
    /// funding. Max and the check before sending must use the backend's fee
    /// (212 852 duffs under protocol 13, not the 164 140 of protocol 14), or
    /// Max proposes an amount whose funding the wallet cannot build.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn core_to_shielded_max_fits_the_ceiling_when_the_sdks_differ() {
        const CEILING_DUFFS: u64 = 10_000_000;
        const BACKEND_FEE_DUFFS: u64 = 212_852;

        let (mut screen, _temp_dir) = send_screen();
        let (sender, _receiver) = tokio::sync::mpsc::channel(32);
        screen
            .app_context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                sender,
                screen.app_context.egui_ctx().clone(),
            ))
            .await
            .expect("wallet backend");
        let pv14 = dash_sdk::dpp::version::PlatformVersion::get(14).expect("PV14");
        screen.app_context.sdk.store(Arc::new(
            dash_sdk::SdkBuilder::new_mock()
                .with_version(pv14)
                .build()
                .expect("mock sdk"),
        ));
        let seed_hash = core_source_with_ceiling(&mut screen, CEILING_DUFFS);
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_amount_input(ui),
            screen,
        );
        harness.run();
        harness.get_by_label("Max").click_accesskit();
        harness.step();
        let max_amount_duffs = harness
            .state()
            .amount
            .as_ref()
            .expect("Max sets the amount")
            .dash_to_duffs()
            .expect("DASH amount");

        let (funding, _) = harness
            .state()
            .app_context
            .plan_shield_from_core(max_amount_duffs)
            .expect("known fee");
        assert!(
            funding.lock_duffs <= CEILING_DUFFS,
            "Max of {max_amount_duffs} duffs leads to a funding of {} duffs, above what the \
             wallet can build",
            funding.lock_duffs
        );
        assert_eq!(max_amount_duffs, CEILING_DUFFS - BACKEND_FEE_DUFFS);

        assert!(
            harness.state_mut().send_core_to_shielded(seed_hash).is_ok(),
            "the check before sending must accept the amount Max proposed"
        );
        harness.state_mut().amount = Some(Amount::dash_from_duffs(max_amount_duffs + 1));
        assert!(
            harness
                .state_mut()
                .send_core_to_shielded(seed_hash)
                .is_err(),
            "one duff more needs a funding above the ceiling and must be refused"
        );
    }
}
