use crate::app::AppAction;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::error::{TaskError, ensure_funding_covers_network_fee};
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::address::{AddressKind, ValidatedAddress, parse_shielded_recipient};
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES, validate_nonzero_amount};
use crate::model::asset_lock::{asset_lock_user_amount_range, asset_lock_user_max_amount};
use crate::model::fee_estimation::{
    AddressAllocationResult, PlatformFeeEstimator, allocate_platform_addresses,
    allocate_platform_addresses_with_fee, core_send_fee_at_max_duffs,
    estimate_address_funding_fee_from_transition, estimate_platform_fee,
    estimate_withdrawal_fee_from_transition, format_credits_as_dash, format_duffs_as_dash,
    identity_topup_min_funding_duffs, max_platform_inputs, max_spendable_credits,
    shield_from_balance_fee_headroom,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::spend_limit::SpendLimitKind;
use crate::model::user_role::UserRole;
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::components::address_input::{AddressInput, WalletWithSnapshot};
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::choice_chips::ChoiceChips;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_selector::WalletSelector;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::identity::funding_common::{FUNDING_FEE_NOT_COVERED, asset_lock_amount_refusal};
use crate::ui::identity::top_up_identity_screen::{
    send_top_up_to_background, top_up_in_flight_for, track_top_up_sent_elsewhere,
};
use crate::ui::state::own_receive_addresses::{OwnAddressKind, OwnReceiveAddresses};
use crate::ui::state::send_form::{
    FormLocks, FormStart, PayFrom, ReceiveInto, SendFormState, ToMode,
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

mod form_view;
mod resolve;

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

pub use crate::ui::state::send_form::{AdvancedSourceType, PlatformFeeStrategy};

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

    // Choices on the form, and this frame's resolution of them.
    form: SendFormState,
    /// What the Someone else field holds; kept while To shows My wallets.
    typed_destination: Option<ValidatedAddress>,
    /// Address to put back in the Someone else field when it is rebuilt.
    pending_typed_address: Option<String>,
    route: resolve::RouteView,
    pay_from_chips_widget: Option<ChoiceChips<PayFrom>>,
    receive_chips_widget: Option<ChoiceChips<ReceiveInto>>,
    from_wallet_selector: Option<WalletSelector>,
    to_wallet_selector: Option<WalletSelector>,
    own_addresses: OwnReceiveAddresses,
    /// Identities per wallet, loaded once per refresh.
    identities_by_wallet: Option<BTreeMap<WalletSeedHash, Vec<QualifiedIdentity>>>,
    #[cfg(any(test, feature = "testing"))]
    balances_override: Option<SendBalances>,

    // Explicit inputs and outputs, filled from the form for Advanced sources
    // and several recipients.
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
    /// The wallet the unlock popup unlocks: the paying wallet, or a To wallet
    /// that must derive a receiving address.
    unlock_target: Option<Arc<RwLock<Wallet>>>,
    /// Send was pressed on a locked wallet: confirm once it is unlocked.
    unlock_then_send: bool,
    wallet_open_attempted: bool,
}

/// Balances a test shows the form instead of the wallet backend's.
#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct SendBalances {
    /// Spendable Core funds, in duffs.
    pub core_duffs: u64,
    /// Platform addresses with balances, in credits.
    pub platform: Vec<(Address, PlatformAddress, Credits)>,
    /// The private balance, in credits.
    pub shielded_credits: u64,
    /// Core addresses with balances, in duffs.
    pub core_addresses: Vec<(Address, u64)>,
}

/// Where each entry point opens the form (spec §11).
fn form_start(flow: SendFlow, identity_mode: Option<IdentitySendMode>) -> FormStart {
    let locked_source = FormLocks {
        pay_from: true,
        ..FormLocks::default()
    };
    match (identity_mode, flow) {
        (Some(IdentitySendMode::Send), _) => FormStart {
            pay_from: PayFrom::Identity,
            locks: locked_source,
            ..FormStart::default()
        },
        (Some(IdentitySendMode::TopUp), _) => FormStart {
            locks: FormLocks {
                to: true,
                to_mode: true,
                ..FormLocks::default()
            },
            ..FormStart::default()
        },
        (Some(IdentitySendMode::MoveToWallet), _) => FormStart {
            pay_from: PayFrom::Identity,
            to_mode: ToMode::MyWallets,
            receive_into: ReceiveInto::Core,
            locks: FormLocks {
                pay_from: true,
                receive_into: true,
                ..FormLocks::default()
            },
        },
        (None, SendFlow::General) => FormStart::default(),
        (None, SendFlow::Shield) => FormStart {
            to_mode: ToMode::MyWallets,
            receive_into: ReceiveInto::Shielded,
            locks: FormLocks {
                to: true,
                to_mode: true,
                receive_into: true,
                ..FormLocks::default()
            },
            ..FormStart::default()
        },
        (None, SendFlow::ShieldedSend) => FormStart {
            pay_from: PayFrom::Shielded,
            locks: FormLocks {
                pay_from: true,
                to_mode: true,
                ..FormLocks::default()
            },
            ..FormStart::default()
        },
        (None, SendFlow::Unshield) => FormStart {
            pay_from: PayFrom::Shielded,
            to_mode: ToMode::MyWallets,
            receive_into: ReceiveInto::Auto,
            locks: locked_source,
        },
    }
}

/// Refusal of Core or Platform funds paying someone else's private address.
const SHIELDED_NOT_OWN_POOL: &str = "Core and Platform funds can only move into your own private balance. To pay this private address, choose Shielded under Pay from.";

/// Refusal of several recipients while a row lacks its address or amount.
const RECIPIENT_ROWS_INCOMPLETE: &str =
    "Enter an address and an amount for every recipient, or remove the rows you don't need.";

/// Refusal of several recipients on a route that carries one output.
const SEVERAL_RECIPIENTS_ROUTE: &str = "Several recipients can be paid only from Core to Core addresses, or from Platform to Platform addresses. Change Pay from or send separately.";

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
            form: SendFormState::new(form_start(SendFlow::General, None)),
            typed_destination: None,
            pending_typed_address: None,
            route: resolve::RouteView::default(),
            pay_from_chips_widget: None,
            receive_chips_widget: None,
            from_wallet_selector: None,
            to_wallet_selector: None,
            own_addresses: OwnReceiveAddresses::default(),
            identities_by_wallet: None,
            #[cfg(any(test, feature = "testing"))]
            balances_override: None,
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
            unlock_target: None,
            unlock_then_send: false,
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
        screen.form = SendFormState::new(form_start(SendFlow::General, Some(mode)));
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
        self.form = SendFormState::new(form_start(flow, None));
        // A preset's source is resolved on its first frame.
        if flow.is_preset() {
            self.selected_source = None;
        }
        self
    }

    fn estimate_max_fee_for_platform_send(
        &self,
        fee_estimator: &PlatformFeeEstimator,
        addresses: &[(PlatformAddress, Address, u64)],
        recipients: &[PlatformAddress],
    ) -> u64 {
        // A recipient never pays; each one is an output of the transfer.
        let outputs = recipients.len().max(1);
        let mut sorted_addresses: Vec<_> = addresses
            .iter()
            .filter(|(addr, _, _)| !recipients.contains(addr))
            .cloned()
            .collect();
        sorted_addresses.sort_by_key(|a| std::cmp::Reverse(a.2));

        let usable_count = sorted_addresses
            .len()
            .min(fee_estimator.max_platform_inputs());
        if usable_count == 0 {
            return estimate_platform_fee(fee_estimator, 1, outputs);
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

        estimate_platform_fee(fee_estimator, usable_count, outputs)
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
        self.typed_destination = None;
        self.form.reset();
        self.own_addresses.invalidate();
        self.identities_by_wallet = None;
        self.send_confirmation = None;
        self.unlock_then_send = false;
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
        if self.identity_flow.is_none() {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "The network changed, so the send form was cleared.",
                MessageType::Info,
            );
        }
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
        self.typed_destination = None;
        self.pending_typed_address = None;
        self.form.reset();
        self.own_addresses.invalidate();
        self.identities_by_wallet = None;
        self.unlock_then_send = false;
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
                    .address_funding_min_amount_duffs(!self.funding_fee_on_top()),
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

        #[cfg(any(test, feature = "testing"))]
        if let Some(balances) = &self.balances_override {
            return balances.platform.clone();
        }

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
        #[cfg(any(test, feature = "testing"))]
        if let Some(balances) = &self.balances_override {
            return (balances.shielded_credits > 0)
                .then_some((seed_hash, balances.shielded_credits));
        }
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
        #[cfg(any(test, feature = "testing"))]
        if let Some(balances) = &self.balances_override {
            return balances.core_duffs;
        }
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

    /// Whether paying `destination` from `source` funds an asset lock, whose
    /// Max is the builder's asset-lock quote.
    fn funds_asset_lock(source: &SourceSelection, destination: Option<AddressKind>) -> bool {
        matches!(source, SourceSelection::CoreWallet)
            && matches!(
                destination,
                Some(AddressKind::Platform | AddressKind::Shielded | AddressKind::Identity)
            )
    }

    #[cfg(test)]
    fn request_asset_lock_max_amount(&mut self) -> Option<BackendTask> {
        let source = self.selected_source.clone()?;
        self.request_asset_lock_max_amount_for(&source, self.destination_kind())
    }

    fn request_asset_lock_max_amount_for(
        &mut self,
        source: &SourceSelection,
        destination: Option<AddressKind>,
    ) -> Option<BackendTask> {
        if !Self::funds_asset_lock(source, destination) {
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
    /// the wallet state it must have been quoted against. A Core wallet with
    /// no destination yet is quoted as a Core send to P2PKH (see
    /// [`Self::core_send_limit_kind`]), so Max never falls back to the
    /// displayed balance.
    fn spend_limit_target(&self) -> Option<(WalletSeedHash, SpendLimitKind, u64)> {
        self.spend_limit_target_for(self.selected_source.as_ref()?, self.destination_kind())
    }

    /// The backend-quoted limit paying `destination` from `source` needs.
    fn spend_limit_target_for(
        &self,
        source: &SourceSelection,
        destination: Option<AddressKind>,
    ) -> Option<(WalletSeedHash, SpendLimitKind, u64)> {
        let seed_hash = self.selected_wallet_seed_hash?;
        match (source, destination) {
            (SourceSelection::PlatformAddresses(_), Some(AddressKind::Shielded)) => Some((
                seed_hash,
                SpendLimitKind::ShieldFromPlatform,
                // A shield in flight has spent credits the live balances
                // still show, so its opening and settling re-key the quote.
                self.live_platform_balances_revision()
                    ^ self
                        .app_context
                        .platform_shield_quote_revision(&seed_hash)
                        .rotate_left(32),
            )),
            (SourceSelection::CoreWallet, Some(AddressKind::Core) | None) => Some((
                seed_hash,
                self.core_send_limit_kind(),
                self.app_context.core_send_quote_revision(&seed_hash),
            )),
            _ => None,
        }
    }

    /// Core send Max is quoted for the script the destination pays to; with
    /// none chosen yet, for P2PKH, the larger output and so the safe bound.
    pub(super) fn core_send_limit_kind(&self) -> SpendLimitKind {
        SpendLimitKind::CoreSend {
            destination: self.core_send_destination(),
        }
    }

    /// The script kind a Core send pays to: the destination's, or P2PKH, the
    /// conservative bound, before one is chosen.
    pub(super) fn core_send_destination(&self) -> crate::model::spend_limit::CoreDestinationScript {
        match &self.validated_destination {
            Some(ValidatedAddress::Core(address)) => {
                crate::model::spend_limit::CoreDestinationScript::of(address)
            }
            _ => Default::default(),
        }
    }

    /// Names the wallet's live Platform balances, not the ones captured in
    /// a resolved source: a balance fetch or sync push updates the wallet
    /// without re-resolving the source, and must still re-quote shield Max.
    fn live_platform_balances_revision(&self) -> u64 {
        platform_balances_revision(&self.platform_source_addresses())
    }

    /// What the loaded Core send Max was proven against, for paying
    /// `destination` from `source`.
    pub(super) fn core_send_quote_fee_for(
        &self,
        source: &SourceSelection,
        destination: Option<AddressKind>,
    ) -> Option<crate::model::spend_limit::CoreSendQuoteFee> {
        let (seed_hash, target_kind, revision) =
            self.spend_limit_target_for(source, destination)?;
        let kind = self.core_send_limit_kind();
        (target_kind == kind)
            .then(|| self.spend_limits.core_fee(&seed_hash, kind, revision))
            .flatten()
    }

    /// The quoted Max, in credits, for paying `destination` from `source`.
    fn spend_limit_for(
        &self,
        source: &SourceSelection,
        destination: Option<AddressKind>,
        kind: SpendLimitKind,
    ) -> Option<u64> {
        let (seed_hash, target_kind, revision) =
            self.spend_limit_target_for(source, destination)?;
        (target_kind == kind)
            .then(|| self.spend_limits.get(&seed_hash, kind, revision))
            .flatten()
    }

    /// The quoted Max, in credits, for the current source and destination.
    #[cfg(test)]
    fn spend_limit(&self, kind: SpendLimitKind) -> Option<u64> {
        self.spend_limit_for(
            self.selected_source.as_ref()?,
            self.destination_kind(),
            kind,
        )
    }

    /// Render the whole Send form for this frame.
    fn render_unified_send(&mut self, ui: &mut Ui) -> AppAction {
        self.render_form(ui)
    }

    #[cfg(test)]
    fn request_spend_limit(&mut self) -> Option<BackendTask> {
        let source = self.selected_source.clone()?;
        self.request_spend_limit_for(&source, self.destination_kind())
    }

    fn request_spend_limit_for(
        &mut self,
        source: &SourceSelection,
        destination: Option<AddressKind>,
    ) -> Option<BackendTask> {
        let (seed_hash, kind, revision) = self.spend_limit_target_for(source, destination)?;
        self.spend_limits
            .ensure_requested(seed_hash, kind, revision)
    }

    fn render_spend_limit_status(&mut self, ui: &mut Ui) {
        let Some((seed_hash, kind, revision)) = self.spend_limit_target() else {
            return;
        };
        if self.spend_limits.is_busy(&seed_hash, kind, revision) {
            ui.label(crate::model::spend_limit::SPEND_IN_PROGRESS_MESSAGE);
        } else if let Some(reason) = self.spend_limits.unavailable(&seed_hash, kind, revision) {
            ui.label(reason.message());
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
        #[cfg(any(test, feature = "testing"))]
        if let Some(balances) = &self.balances_override {
            return balances.core_addresses.clone();
        }
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

    /// Returns the address kind of the current validated destination, if any.
    fn destination_kind(&self) -> Option<AddressKind> {
        self.validated_destination.as_ref().map(|v| v.kind())
    }

    /// Fee an identity sender keeps back for the chosen destination. Max and
    /// send validation both read it, so they cannot reserve different amounts.
    fn identity_source_fee_reserve(&self) -> u64 {
        self.identity_source_fee_reserve_for(self.destination_kind())
    }

    /// Fee an identity sender keeps back for a destination of this kind.
    fn identity_source_fee_reserve_for(&self, destination: Option<AddressKind>) -> u64 {
        let fee_estimator = self.app_context.fee_estimator();
        match destination {
            Some(AddressKind::Core) => fee_estimator.estimate_credit_withdrawal(),
            Some(AddressKind::Platform) => fee_estimator.estimate_credit_transfer_to_addresses(1),
            _ => fee_estimator.estimate_credit_transfer(),
        }
    }

    /// The recipient on the confirmation. Addresses are shown whole: a
    /// lookalike sharing only the first and last characters must not pass
    /// the last check before an irreversible payment (address poisoning).
    fn confirmation_recipient(&self) -> String {
        match &self.validated_destination {
            Some(ValidatedAddress::Shielded(address)) if address.is_empty() => {
                self.destination_label()
            }
            Some(ValidatedAddress::Identity {
                dpns_name: Some(name),
                ..
            }) => format!("{name} ({id})", id = self.destination_address_string()),
            _ => self.destination_address_string(),
        }
    }

    /// Take a receiving address the backend generated, returning whether it
    /// answered this form's request. An address for another network (a reply
    /// to a request sent before a network switch) is consumed but not kept:
    /// the request stays open for the right one.
    fn take_own_address(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: OwnAddressKind,
        address: &str,
    ) -> bool {
        if !self.own_addresses.awaits(&seed_hash, kind) {
            return false;
        }
        let expected = match kind {
            OwnAddressKind::Core => AddressKind::Core,
            OwnAddressKind::Platform => AddressKind::Platform,
        };
        if self
            .row_address(address)
            .is_some_and(|valid| valid.kind() == expected)
        {
            self.own_addresses
                .store(seed_hash, kind, address.to_string());
        }
        true
    }

    fn destination_address_string(&self) -> String {
        self.validated_destination
            .as_ref()
            .map(|v| v.to_address_string())
            .unwrap_or_default()
    }

    /// Whether the shielded destination is the paying wallet's own private
    /// balance — the only one Core and Platform funds can shield into.
    ///
    /// The empty address is the Shield flow's own-pool marker. A typed address
    /// counts only when it decodes to the same bytes as the wallet's private
    /// address; while that address is still unknown, nothing typed matches.
    /// DET shows only the default address of Orchard account 0, so another
    /// diversified address of the same wallet is treated as someone else's.
    pub(super) fn shielded_destination_is_own_pool(&self) -> bool {
        let Some(ValidatedAddress::Shielded(address)) = &self.validated_destination else {
            return false;
        };
        if address.is_empty() {
            return true;
        }
        let Some(typed) = parse_shielded_recipient(address) else {
            return false;
        };
        self.selected_wallet_seed_hash
            .and_then(|seed_hash| self.app_context.shielded_receive_address(&seed_hash))
            .and_then(|own| parse_shielded_recipient(&own))
            .is_some_and(|own| own == typed)
    }

    /// Why the selected source cannot pay the typed private address, if it
    /// cannot. Core and Platform can only shield into the paying wallet's own
    /// private balance: the shield tasks carry no recipient and always deposit
    /// into the wallet's own pool. Letting someone else's private address
    /// through would keep the money while reporting success.
    fn shielded_destination_refusal(&self) -> Option<&'static str> {
        let shields_into_own_pool = matches!(
            self.selected_source,
            Some(SourceSelection::CoreWallet | SourceSelection::PlatformAddresses(_))
        );
        let private_destination = self
            .validated_destination
            .as_ref()
            .is_some_and(|destination| destination.kind() == AddressKind::Shielded);
        (shields_into_own_pool && private_destination && !self.shielded_destination_is_own_pool())
            .then_some(SHIELDED_NOT_OWN_POOL)
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

        if let Some(refusal) = self.shielded_destination_refusal() {
            return Err(refusal.to_string());
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
            // Calculate the max we can send with the protocol's input limit (minus fees)
            let max_inputs = fee_estimator.max_platform_inputs();
            let addresses_available = allocation.sorted_addresses.len().min(max_inputs);
            let max_balance: u64 = allocation
                .sorted_addresses
                .iter()
                .take(max_inputs)
                .map(|(_, _, b)| *b)
                .sum();
            let max_fee = self.estimate_max_fee_for_platform_send(
                &fee_estimator,
                &allocation.sorted_addresses,
                std::slice::from_ref(&destination),
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
                max_inputs,
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
        let max_inputs = max_platform_inputs(platform_version);

        // Allocate addresses using state-transition-based fee estimation (no destination filter)
        let allocation = allocate_platform_addresses_with_fee(
            &addresses,
            amount_credits,
            None,
            max_inputs,
            |inputs| {
                estimate_withdrawal_fee_from_transition(platform_version, inputs, &output_script)
            },
        );

        if allocation.shortfall > 0 {
            // Calculate the max we can send with the protocol's input limit (minus fees)
            let addresses_available = allocation.sorted_addresses.len().min(max_inputs);
            let max_balance: u64 = allocation
                .sorted_addresses
                .iter()
                .take(max_inputs)
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
                max_inputs,
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

    /// Destination address kinds the Someone else field accepts. The field is
    /// built once with every kind the form can pay, whatever Pay from shows,
    /// so changing Pay from never clears what was typed; a kind the chosen
    /// source cannot pay is explained inline instead. Shielded destinations
    /// are offered only where the network can settle them and the feature is
    /// unlocked.
    fn general_destination_kinds(&self) -> Vec<AddressKind> {
        match self.identity_flow.as_ref().map(|(_, mode)| *mode) {
            Some(IdentitySendMode::MoveToWallet) => return vec![AddressKind::Core],
            // An identity pays Core, Platform and identities only.
            Some(IdentitySendMode::Send) => {
                return vec![
                    AddressKind::Core,
                    AddressKind::Platform,
                    AddressKind::Identity,
                ];
            }
            _ => {}
        }
        let mut kinds = vec![AddressKind::Core, AddressKind::Platform];
        if FeatureGate::ShieldedOperations.is_available(&self.app_context) {
            kinds.push(AddressKind::Shielded);
        }
        kinds.push(AddressKind::Identity);
        kinds
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
        let hint = if allowed_kinds.contains(&AddressKind::Identity) {
            "Address, username or identity ID"
        } else {
            "Address"
        };
        let mut builder = AddressInput::new(self.app_context.network)
            .with_label("Send to")
            .with_hint_text(hint)
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
            let mut input = self.build_address_input(allowed_kinds, &wallets, identities);
            match self.pending_typed_address.take() {
                Some(address) => input = input.with_initial_value(address),
                // A fresh, typeable field waits with the cursor in it (W1).
                None if !matches!(
                    self.identity_flow,
                    Some((_, IdentitySendMode::MoveToWallet))
                ) =>
                {
                    input.request_focus()
                }
                None => {}
            }
            self.address_input = Some(input);
        }
        let expert = self.app_context.user_role().at_least(UserRole::Power);
        let address_input = self
            .address_input
            .as_mut()
            .expect("invariant: address_input set to Some immediately above");
        let search_tags = expert.then(|| address_input.dynamic_hint());
        let resp = address_input.show(ui);
        // Not while typing, so the hint never covers the suggestions.
        if let Some(tags) = search_tags.filter(|tags| !tags.is_empty())
            && !resp.inner.response.has_focus()
        {
            resp.inner
                .response
                .clone()
                .on_hover_text(format!("Narrow the suggestions with: {tags}"));
        }
        if resp.inner.update(&mut self.typed_destination) {
            self.form.touched();
        }
        // The field is the destination while To pays someone else.
        if self.form.to_mode() == ToMode::SomeoneElse
            && self.validated_destination != self.typed_destination
        {
            self.validated_destination = self.typed_destination.clone();
        }
    }

    #[cfg(test)]
    fn render_destination_input(&mut self, ui: &mut Ui) {
        let kinds = self.general_destination_kinds();
        self.render_destination_input_with_kinds(ui, &kinds);
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

        let source = self.selected_source.as_ref()?;
        match (source, dest_kind) {
            // Core → Core: L1 network fee, paid on top (reserved from balance).
            (SourceSelection::CoreWallet, AddressKind::Core) => {
                // Never below what the pooled build charges for this amount
                // and destination; at the Max, exactly the Max's fee.
                let seed_hash = self.selected_wallet_seed_hash?;
                let quote = self
                    .spend_limit_for(source, Some(dest_kind), self.core_send_limit_kind())
                    .zip(self.core_send_quote_fee_for(source, Some(dest_kind)))
                    .map(|(max_credits, quote)| (max_credits / CREDITS_PER_DUFF, quote));
                let fee_duffs = self.app_context.snapshot_core_payment_fee_duffs(
                    &seed_hash,
                    self.core_send_destination(),
                    amount_credits / CREDITS_PER_DUFF,
                    quote,
                );
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
                    max_platform_inputs(platform_version),
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
            // Platform → Shielded: the two-action shield fee, paid on top. The
            // planner keeps a larger reserve back; what the fee does not use
            // stays on the Platform address.
            (SourceSelection::PlatformAddresses(_), AddressKind::Shielded) => {
                let fee_credits = crate::model::fee_estimation::shield_from_balance_fee_estimate(
                    self.app_context.platform_version(),
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
                let fee_credits = self.identity_source_fee_reserve();
                Some(FeePreview::on_top(amount_credits, fee_credits))
            }
            // Identity → Shielded, Shielded → anything, Core → Identity: the fee
            // depends on inputs the backend selects at send time.
            _ => None,
        }
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
                // Paid from the pooled accounts, whichever addresses are
                // listed: the fee is bounded over every coin they could spend.
                let seed_hash = self.selected_wallet_seed_hash?;
                let amount_duffs = self
                    .advanced_outputs
                    .iter()
                    .filter_map(|output| Self::parse_amount_to_duffs(&output.amount).ok())
                    .fold(0, u64::saturating_add);
                let quote = self
                    .core_send_quote_fee_for(&SourceSelection::CoreWallet, Some(AddressKind::Core));
                let fee_duffs = self.app_context.snapshot_core_several_outputs_fee_duffs(
                    &seed_hash,
                    row_output_count,
                    amount_duffs,
                    quote,
                );
                Some(fee_duffs.saturating_mul(CREDITS_PER_DUFF))
            }
            AdvancedSourceType::Platform if has_platform_out && !has_core_out => {
                let outputs = Self::normalize_advanced_platform_outputs(&self.advanced_outputs)
                    .ok()
                    .filter(|outputs| !outputs.is_empty())?;
                if let Some(addresses) = self.platform_addresses_paying_many() {
                    return Some(
                        self.allocate_platform_to_many(addresses, &outputs)
                            .estimated_fee,
                    );
                }
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

    fn simple_send_confirmation_message(&self) -> String {
        let amount_credits = self.amount.as_ref().map(Amount::value).unwrap_or(0);
        let fee_preview = self.current_fee_preview();
        let mut lines = vec![
            format!("From: {source}", source = self.source_label()),
            format!(
                "To: {destination}",
                destination = self.confirmation_recipient()
            ),
            format!(
                "Amount: {amount}",
                amount = format_credits_as_dash(amount_credits)
            ),
        ];
        match fee_preview {
            Some(preview) => {
                lines.push(format!(
                    "Fee: ≈ {fee}",
                    fee = format_credits_as_dash(preview.fee_credits)
                ));
                lines.push(format!(
                    "Total: ≈ {total}",
                    total = format_credits_as_dash(preview.total_debit_credits)
                ));
                if let Some(receives) = preview.recipient_receives_credits {
                    let recipient_amount = format_credits_as_dash(receives);
                    lines.push(format!(
                        "The network fee will be deducted from the output amount, so the recipient will receive {recipient_amount}."
                    ));
                }
            }
            None => lines.push("Fee: calculated when you send".to_string()),
        }
        if let Some(note) = self.route_note() {
            lines.push(note.to_string());
        }
        lines.push("Check the recipient. Payments can't be undone.".to_string());
        lines.join("\n")
    }

    /// Whether the payment goes out with explicit inputs or several outputs,
    /// through the advanced route functions.
    pub(super) fn uses_explicit_payment(&self) -> bool {
        self.form.has_several_recipients() || self.advanced_amounts_entered()
    }

    /// Whether a Core-to-Platform payment adds the Platform fee on top of the
    /// amount. Only an explicit payment reads the fee strategy; every other
    /// one takes the fee from the amount.
    pub(super) fn funding_fee_on_top(&self) -> bool {
        self.uses_explicit_payment() && !self.form.fee_strategy().reduces_output()
    }

    /// Whether an Advanced source names an amount for any ticked address.
    fn advanced_amounts_entered(&self) -> bool {
        self.form.pay_from() == PayFrom::Advanced
            && self
                .form
                .ticked_advanced()
                .iter()
                .any(|(_, amount)| !amount.trim().is_empty())
    }

    /// Fill the explicit inputs and outputs the advanced route functions read
    /// from the form's Advanced ticks and recipients.
    pub(super) fn sync_explicit_payment(&mut self) {
        self.fee_strategy = self.form.fee_strategy();
        if !self.uses_explicit_payment() {
            return;
        }
        let advanced = self.form.pay_from() == PayFrom::Advanced;
        self.advanced_source_type = if advanced {
            self.form.advanced_type()
        } else {
            match &self.selected_source {
                Some(SourceSelection::PlatformAddresses(_)) => AdvancedSourceType::Platform,
                _ => AdvancedSourceType::Core,
            }
        };
        let ticked: BTreeMap<String, String> = self.form.ticked_advanced().into_iter().collect();
        let network = self.app_context.network;
        match self.advanced_source_type {
            AdvancedSourceType::Core => {
                // Core is paid from the pooled accounts: each address offers
                // its typed amount, or its whole balance when none is typed.
                self.core_inputs = self
                    .get_core_addresses()
                    .into_iter()
                    .filter(|(address, _)| !advanced || ticked.contains_key(&address.to_string()))
                    .map(|(address, duffs)| {
                        let typed = ticked
                            .get(&address.to_string())
                            .map(|amount| amount.trim().to_string())
                            .filter(|amount| !amount.is_empty());
                        CoreAddressInput {
                            amount: typed.unwrap_or_else(|| {
                                Amount::dash_from_duffs(duffs).to_string_without_unit()
                            }),
                            address,
                        }
                    })
                    .collect();
            }
            AdvancedSourceType::Platform => {
                self.platform_inputs = self
                    .platform_source_addresses()
                    .into_iter()
                    .filter_map(|(platform_address, _, _)| {
                        let amount = ticked.get(&platform_address.to_bech32m_string(network))?;
                        Some(PlatformAddressInput {
                            platform_address,
                            amount: amount.trim().to_string(),
                        })
                    })
                    .collect();
            }
        }
        self.advanced_outputs = if self.form.has_several_recipients() {
            self.form
                .recipients()
                .iter()
                .filter(|row| !row.address.trim().is_empty())
                .map(|row| AdvancedOutput {
                    address: row.address.trim().to_string(),
                    amount: row.amount.trim().to_string(),
                })
                .collect()
        } else {
            vec![AdvancedOutput {
                address: self.destination_address_string(),
                amount: self
                    .amount
                    .as_ref()
                    .map(Amount::to_string_without_unit)
                    .unwrap_or_default(),
            }]
        };
    }

    /// Build the payment Send would make, and what the confirmation says.
    /// The action is fixed here, so the confirmation shows exactly what will
    /// be sent even if Auto would choose differently later.
    pub(super) fn prepare_send(&mut self) -> Result<(AppAction, String), String> {
        if let Some(refusal) = self.route.refusal {
            return Err(match refusal {
                crate::model::send_route::RouteRefusal::ShieldedNotOwnPool => {
                    SHIELDED_NOT_OWN_POOL.to_string()
                }
                crate::model::send_route::RouteRefusal::TickedPlatformToShielded => {
                    form_view::refusal_message(refusal, false).to_string()
                }
                _ => self
                    .validated_send_route()
                    .err()
                    .unwrap_or_else(|| SHIELDED_NOT_OWN_POOL.to_string()),
            });
        }
        self.sync_explicit_payment();
        if self.form.has_several_recipients() {
            if !self.recipient_rows_complete() {
                return Err(RECIPIENT_ROWS_INCOMPLETE.to_string());
            }
            let kinds: Vec<_> = self
                .advanced_outputs
                .iter()
                .map(|output| AddressKind::detect(&output.address))
                .collect();
            let platform_outputs = kinds
                .iter()
                .all(|kind| *kind == Some(AddressKind::Platform));
            if platform_outputs && let Some(addresses) = self.platform_addresses_paying_many() {
                let addresses = addresses.to_vec();
                let outputs = Self::normalize_advanced_platform_outputs(&self.advanced_outputs)
                    .map_err(|error| error.to_string())?;
                let seed_hash = self.open_wallet_seed_hash()?;
                let action = self.send_platform_to_many(seed_hash, addresses, outputs)?;
                return Ok((action, self.advanced_send_confirmation_message()));
            }
            if !matches!(
                (&self.selected_source, platform_outputs),
                (Some(SourceSelection::CoreWallet), false)
                    | (Some(SourceSelection::PlatformAddresses(_)), true)
            ) {
                return Err(SEVERAL_RECIPIENTS_ROUTE.to_string());
            }
            let action = self.validate_and_send_advanced()?;
            return Ok((action, self.advanced_send_confirmation_message()));
        }
        if self.advanced_amounts_entered() {
            if !matches!(
                self.destination_kind(),
                Some(AddressKind::Core | AddressKind::Platform)
            ) {
                return Err("Amounts per address work only for payments to Core or Platform addresses. Clear the amounts to pay this recipient.".to_string());
            }
            if self
                .form
                .ticked_advanced()
                .iter()
                .any(|(_, amount)| amount.trim().is_empty())
            {
                return Err(
                    "Enter an amount for every ticked address, or leave them all empty."
                        .to_string(),
                );
            }
            let action = self.validate_and_send_advanced()?;
            return Ok((action, self.advanced_send_confirmation_message()));
        }
        if self.form.pay_from() == PayFrom::Advanced {
            self.ensure_advanced_selection_covers()?;
        }
        let action = self.validate_and_send()?;
        Ok((action, self.simple_send_confirmation_message()))
    }

    /// The seed hash of the paying wallet, which must be open.
    fn open_wallet_seed_hash(&self) -> Result<WalletSeedHash, String> {
        let wallet = self.selected_wallet.as_ref().ok_or("No wallet selected")?;
        let guard = wallet.read().map_err(|e| e.to_string())?;
        if !guard.is_open() {
            return Err("Wallet must be unlocked first".to_string());
        }
        Ok(guard.seed_hash())
    }

    /// An Advanced source with no amounts typed spends from the ticked
    /// addresses; they must hold the amount.
    fn ensure_advanced_selection_covers(&self) -> Result<(), String> {
        let ticked = self.form.ticked_advanced();
        if ticked.is_empty() {
            return Err("Tick the addresses to spend from.".to_string());
        }
        if self.form.advanced_type() == AdvancedSourceType::Core {
            let ticked: Vec<String> = ticked.into_iter().map(|(address, _)| address).collect();
            let selected_duffs: u64 = self
                .get_core_addresses()
                .into_iter()
                .filter(|(address, _)| ticked.contains(&address.to_string()))
                .map(|(_, duffs)| duffs)
                .sum();
            let amount_duffs = self
                .amount
                .as_ref()
                .map(|amount| amount.dash_to_duffs())
                .transpose()?
                .unwrap_or(0);
            if amount_duffs > selected_duffs {
                let selected = format_duffs_as_dash(selected_duffs);
                return Err(format!(
                    "The ticked addresses hold {selected}. Tick more addresses or lower the amount."
                ));
            }
        }
        Ok(())
    }

    /// The Platform addresses to choose from when several recipients are paid
    /// without per-address amounts.
    fn platform_addresses_paying_many(&self) -> Option<&[(PlatformAddress, Address, u64)]> {
        match &self.selected_source {
            Some(SourceSelection::PlatformAddresses(addresses))
                if self.form.has_several_recipients() && !self.advanced_amounts_entered() =>
            {
                Some(addresses)
            }
            _ => None,
        }
    }

    /// The inputs that pay `outputs`, chosen from `addresses`; none of the
    /// recipients pays. The payment and the fee shown for it both read this.
    fn allocate_platform_to_many(
        &self,
        addresses: &[(PlatformAddress, Address, u64)],
        outputs: &BTreeMap<PlatformAddress, Credits>,
    ) -> AddressAllocationResult {
        let total = outputs
            .values()
            .fold(0u64, |total, amount| total.saturating_add(*amount));
        let candidates: Vec<_> = addresses
            .iter()
            .filter(|(address, _, _)| !outputs.contains_key(address))
            .cloned()
            .collect();
        let fee_estimator = self.app_context.fee_estimator();
        allocate_platform_addresses_with_fee(
            &candidates,
            total,
            None,
            fee_estimator.max_platform_inputs(),
            |inputs| estimate_platform_fee(&fee_estimator, inputs.len().max(1), outputs.len()),
        )
    }

    /// Pay several Platform recipients from Platform addresses chosen for
    /// them (none of the recipients pays).
    fn send_platform_to_many(
        &mut self,
        seed_hash: WalletSeedHash,
        addresses: Vec<(PlatformAddress, Address, u64)>,
        outputs: BTreeMap<PlatformAddress, Credits>,
    ) -> Result<AppAction, String> {
        if outputs.is_empty() {
            return Err("No valid Platform outputs specified".to_string());
        }
        let total = outputs
            .values()
            .try_fold(0u64, |total, amount| total.checked_add(*amount))
            .ok_or_else(|| TaskError::AdvancedPlatformOutputsOverflow.to_string())?;
        let allocation = self.allocate_platform_to_many(&addresses, &outputs);
        if allocation.inputs.is_empty() {
            return Err("Every Platform address that could pay is also a recipient. Choose other recipients.".to_string());
        }
        if allocation.shortfall > 0 {
            let needed = format_credits_as_dash(total);
            return Err(format!(
                "One payment from Platform can't cover {needed} plus the fee. Lower the amounts and try again."
            ));
        }
        Ok(AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::TransferPlatformCredits {
                seed_hash,
                inputs: allocation.inputs,
                outputs,
                fee_payer_index: allocation.fee_payer_index,
            },
        )))
    }

    /// Whether the explicit payment goes from Core to Platform, the only route
    /// that reads the fee strategy.
    fn explicit_core_to_platform(&self) -> bool {
        self.advanced_source_type == AdvancedSourceType::Core
            && self.advanced_outputs.iter().any(|output| {
                self.detect_address_kind(&output.address) == Some(AddressKind::Platform)
            })
    }

    /// Whether the explicit payment takes the fee out of the recipient's
    /// amount. A strategy chosen on another route has no effect here.
    pub(super) fn explicit_fee_reduces_output(&self) -> bool {
        self.explicit_core_to_platform() && self.fee_strategy.reduces_output()
    }

    /// The confirmation for explicit inputs or several recipients, in the
    /// same From / To / Amount / Fee / Total lines as a single payment.
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
                (amount_credits > 0).then(|| (output.address.trim().to_string(), amount_credits))
            })
            .collect();
        let several = recipients.len() > 1;
        let total_credits = recipients
            .iter()
            .map(|(_, credits)| *credits)
            .fold(0u64, u64::saturating_add);
        let mut lines = vec![format!("From: {source}", source = self.source_label())];
        for (address, credits) in &recipients {
            let destination = address;
            lines.push(if several {
                format!(
                    "To: {destination} · {amount}",
                    amount = format_credits_as_dash(*credits)
                )
            } else {
                format!("To: {destination}")
            });
        }
        lines.push(format!(
            "Amount: {amount}",
            amount = format_credits_as_dash(total_credits)
        ));
        let core_to_platform = self.explicit_core_to_platform();
        let reduces_output = self.explicit_fee_reduces_output();
        match self.advanced_fee_estimate_credits() {
            Some(fee) => {
                lines.push(format!("Fee: ≈ {fee}", fee = format_credits_as_dash(fee)));
                let total = if reduces_output {
                    total_credits
                } else {
                    total_credits.saturating_add(fee)
                };
                lines.push(format!(
                    "Total: ≈ {total}",
                    total = format_credits_as_dash(total)
                ));
            }
            None => lines.push("Fee: calculated when you send".to_string()),
        }
        if reduces_output {
            lines.push("The network fee will be deducted from the output amount, so the recipient will receive less than the entered amount.".to_string());
        } else if core_to_platform {
            lines.push("The network fee will be paid from the selected inputs, so the recipient will receive the entered amount.".to_string());
        }
        if let Some(note) = self.route_note() {
            lines.push(note.to_string());
        }
        lines.push(if several {
            "Check the recipients. Payments can't be undone.".to_string()
        } else {
            "Check the recipient. Payments can't be undone.".to_string()
        });
        lines.join("\n")
    }

    fn open_send_confirmation(&mut self, action: AppAction, message: String) {
        let title = match self.requested_credits() {
            Some(amount) => format!("Send {amount}?", amount = format_credits_as_dash(amount)),
            None => "Send this payment?".to_string(),
        };
        self.send_confirmation = Some(PendingSendConfirmation {
            dialog: ConfirmationDialog::new(title, message)
                .confirm_text(Some("Send"))
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
        // The fields show what was set, as if it had been typed.
        self.address_input = None;
        self.pending_typed_address = destination
            .as_ref()
            .map(ValidatedAddress::to_address_string);
        self.typed_destination = destination.clone();
        self.validated_destination = destination;
        self.amount_input = amount.clone().map(|amount| {
            let mut input = Self::new_amount_input();
            input.set_value(amount);
            input
        });
        self.amount = amount;
    }

    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    /// Shows these balances instead of the wallet backend's.
    pub fn set_balances_for_test(&mut self, balances: SendBalances) {
        self.balances_override = Some(balances);
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// The source and destination the form resolved to on its last frame.
    pub fn resolved_route_for_test(&self) -> (Option<SourceSelection>, Option<ValidatedAddress>) {
        (
            self.selected_source.clone(),
            self.validated_destination.clone(),
        )
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Puts an address the wallet backend returned for `seed_hash`, as if the
    /// form had asked for it.
    pub fn seed_own_core_address_for_test(&mut self, seed_hash: WalletSeedHash, address: String) {
        let _ = self
            .own_addresses
            .ensure_requested(seed_hash, OwnAddressKind::Core);
        let _ = self
            .own_addresses
            .store(seed_hash, OwnAddressKind::Core, address);
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Seeds the backend's quote of the largest Core payment, in credits,
    /// replacing any quote request still in flight.
    pub fn seed_core_send_limit_for_test(&mut self, max_credits: Credits) {
        let Some((seed_hash, kind, revision)) =
            self.spend_limit_target_for(&SourceSelection::CoreWallet, Some(AddressKind::Core))
        else {
            return;
        };
        self.spend_limits.invalidate();
        if let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit { request_id, .. })) = self
            .spend_limits
            .ensure_requested(seed_hash, kind, revision)
        {
            self.spend_limits
                .store(seed_hash, kind, request_id, max_credits, None);
        }
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Answers the Core send quote with "another transaction is in
    /// progress", as the backend does while a build holds reservations.
    pub fn mark_core_send_busy_for_test(&mut self) {
        self.mark_core_send_error_for_test(&TaskError::SpendInProgress);
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Answers the Core send quote with `error`, replacing any quote
    /// request still in flight.
    pub fn mark_core_send_error_for_test(&mut self, error: &TaskError) {
        let Some((seed_hash, kind, revision)) =
            self.spend_limit_target_for(&SourceSelection::CoreWallet, Some(AddressKind::Core))
        else {
            return;
        };
        self.spend_limits.invalidate();
        if let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit { request_id, .. })) = self
            .spend_limits
            .ensure_requested(seed_hash, kind, revision)
        {
            self.spend_limits
                .mark_request_error(seed_hash, kind, request_id, error);
        }
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Whether a confirmation dialog is waiting.
    pub fn has_send_confirmation_for_test(&self) -> bool {
        self.send_confirmation.is_some()
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Whether the wallet unlock popup is open.
    pub fn is_unlock_open_for_test(&self) -> bool {
        self.wallet_unlock_popup.is_open()
    }

    #[cfg(any(test, feature = "testing"))]
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

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Renders the production send confirmation in a headless harness.
    pub fn render_send_confirmation_for_test(&mut self, ui: &mut Ui) -> AppAction {
        self.render_send_confirmation(ui)
    }

    #[cfg(feature = "testing")]
    #[doc(hidden)]
    /// Renders the whole production Send form in a headless harness.
    pub fn render_form_for_test(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = self.render_form(ui);
        action |= self.render_send_confirmation(ui);
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
            .address_funding_min_amount_duffs(fee_deduct_from_output);
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
            core_fee,
        } = result
        {
            self.spend_limits
                .store(*seed_hash, *kind, *request_id, *max_credits, *core_fee);
            return true;
        }
        // A receiving address this form asked for is consumed here: it is
        // not the outcome of a send.
        let own_address = match result {
            crate::backend_task::BackendTaskSuccessResult::GeneratedReceiveAddress {
                seed_hash,
                address,
            } => self.take_own_address(*seed_hash, OwnAddressKind::Core, address),
            crate::backend_task::BackendTaskSuccessResult::GeneratedPlatformReceiveAddress {
                seed_hash,
                address,
            } => self.take_own_address(*seed_hash, OwnAddressKind::Platform, address),
            _ => false,
        };
        if Self::task_result_updates_address_input(result) {
            self.address_input_snapshot_signature = None;
            self.identities_by_wallet = None;
        }
        own_address
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
                        ui.heading(
                            RichText::new(self.heading())
                                .color(DashColors::text_primary(dark_mode))
                                .size(24.0),
                        );

                        if let Some(description) = self.flow.description() {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(description)
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        }

                        ui.add_space(15.0);
                        inner_action |= self.render_unified_send(ui);
                    });
            });

            inner_action
        });

        let confirmation_action = self.render_send_confirmation(ui);
        action = merge_confirmation_action(action, confirmation_action);

        // Max quotes and own receiving addresses this frame's resolution needs.
        if can_append_concurrent_backend_tasks(&action) {
            let tasks = self.route_requests();
            if !tasks.is_empty() {
                action = append_concurrent_backend_tasks(action, tasks);
            }
        }

        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = self
                .unlock_target
                .clone()
                .or_else(|| self.selected_wallet.clone())
        {
            match self
                .wallet_unlock_popup
                .show(ctx, &wallet, &self.app_context)
            {
                WalletUnlockResult::Unlocked => {
                    // A receiving address may now be derived.
                    self.unlock_target = None;
                    if std::mem::take(&mut self.unlock_then_send) {
                        self.sync_route();
                        match self.prepare_send() {
                            Ok((send_action, message)) => {
                                self.open_send_confirmation(send_action, message)
                            }
                            Err(e) => {
                                MessageBanner::set_global(ctx, &e, MessageType::Error);
                                self.send_status = SendStatus::Error;
                            }
                        }
                    }
                }
                WalletUnlockResult::Cancelled => {
                    // The form stays as it was.
                    self.unlock_target = None;
                    self.unlock_then_send = false;
                }
                WalletUnlockResult::Pending => {}
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
        self.identities_by_wallet = None;
        // Own receiving addresses stay: refetching one could change where an
        // open form pays. A new form or network fetches them again.
        self.asset_lock_balance.invalidate();
        self.spend_limits.invalidate();
    }

    fn refresh(&mut self) {
        self.refresh_bound_identity();
        self.address_input_snapshot_signature = None;
        self.identities_by_wallet = None;
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

    /// Pay from › Identity lists only identities the From wallet owns.
    /// Hydration links every identity to every loaded wallet, so that link is
    /// not ownership.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pay_from_offers_only_the_identities_the_wallet_owns() {
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
        let owner = screen.selected_wallet.clone().expect("owner wallet");
        let owner_hash = screen.selected_wallet_seed_hash.expect("owner hash");
        let other = Arc::new(RwLock::new(
            // Generated per run: no wallet secret is ever committed.
            Wallet::new_from_seed(rand::random(), Network::Testnet, None, None).expect("wallet"),
        ));
        let other_hash = other.read().expect("wallet").seed_hash();
        let ctx = screen.app_context.clone();
        ctx.wallet_context().insert_test_wallet(owner_hash, owner);
        ctx.wallet_context()
            .insert_test_wallet(other_hash, other.clone());
        ctx.insert_local_qualified_identity(&bound_identity(), &Some((owner_hash, 0)))
            .expect("store identity");

        assert_eq!(screen.paying_identities().len(), 1, "the owner pays");

        screen.selected_wallet = Some(other);
        screen.selected_wallet_seed_hash = Some(other_hash);
        screen.identities_by_wallet = None;
        assert!(
            screen.paying_identities().is_empty(),
            "another wallet has no identity to pay from"
        );
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
        harness.get_by_label("Send").click();
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
                    // The form is disabled behind the dialog, as on the screen.
                    let form_enabled = screen.send_confirmation.is_none();
                    let mut action = ui
                        .add_enabled_ui(form_enabled, |ui| screen.render_send_button(ui))
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
            "nothing is dispatched before the user confirms"
        );
        click_confirm(&mut harness);
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

    /// The confirmation's Send: the one enabled while the dialog blocks the
    /// form's own Send behind it.
    fn click_confirm(harness: &mut Harness<'_, WalletSendScreen>) {
        // Let the dialog settle (its first frame only measures it).
        harness.run();
        harness
            .get_all_by_label("Send")
            .find(|node| {
                let node = egui_kittest::kittest::NodeT::accesskit_node(node);
                node.role() == egui::accesskit::Role::Button && !node.is_disabled()
            })
            .expect("an enabled confirmation Send")
            .click_accesskit();
    }

    fn confirmation_is_open(harness: &Harness<'_, WalletSendScreen>) -> bool {
        harness
            .query_by_label_contains("Check the recipient")
            .is_some()
            || harness
                .query_by_label_contains("Confirm this transaction")
                .is_some()
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
            error.starts_with("You can use up to "),
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
            error.starts_with("You can use up to "),
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
            error.starts_with("You can use up to "),
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
            core_fee: None,
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
                core_fee: None,
            })
        );
        // Auto reads the same limits: a settled zero, not a pending quote.
        let source = screen.selected_source.clone().expect("source");
        let destination = screen.validated_destination.clone();
        let limits = screen.amount_limits_for(&source, destination.as_ref());
        assert_eq!(limits.max_credits, None);
        assert_eq!(limits.quote, resolve::QuoteState::Ready);

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
                core_fee: None,
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
        // The form's Amount field and Auto read the same quote.
        let source = screen.selected_source.clone().expect("source");
        let destination = screen.validated_destination.clone();
        assert_eq!(
            screen
                .amount_limits_for(&source, destination.as_ref())
                .max_credits,
            None,
            "the Amount field must not offer the stale quote as Max"
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
                core_fee: None,
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

    /// Before a destination is chosen, Core Max is still the backend's pooled
    /// quote, taken for P2PKH (the larger output, so the safe bound), never
    /// the displayed spendable balance, which can include inputs the pooled
    /// build cannot spend and keeps no fee back.
    #[test]
    fn core_send_max_without_destination_is_the_p2pkh_quote() {
        use crate::model::spend_limit::CoreDestinationScript;
        const QUOTE_DUFFS: u64 = 7_654_321;

        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = None;
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");

        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
            kind:
                SpendLimitKind::CoreSend {
                    destination: CoreDestinationScript::PubkeyHash,
                },
            request_id,
            ..
        })) = screen.request_spend_limit()
        else {
            panic!("with no destination, Core Max must ask for the P2PKH pooled quote");
        };
        assert!(
            screen.refresh_form_inputs(&BackendTaskSuccessResult::SpendLimit {
                seed_hash,
                kind: SpendLimitKind::CoreSend {
                    destination: CoreDestinationScript::PubkeyHash,
                },
                request_id,
                max_credits: QUOTE_DUFFS * CREDITS_PER_DUFF,
                core_fee: None,
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
            "Max with no destination must be exactly the P2PKH pooled quote"
        );
    }

    /// Through the form: Pay from Core with no destination yet still asks for
    /// the P2PKH pooled quote, so Max has a backend figure to offer.
    #[test]
    fn the_form_asks_for_the_p2pkh_core_quote_before_a_destination() {
        use crate::model::spend_limit::CoreDestinationScript;

        let (mut screen, _temp_dir) = funded_send_screen();
        screen
            .form
            .choose_pay_from(PayFrom::Core, crate::model::send_route::PayFromKind::Core);
        screen.sync_route();
        assert!(screen.validated_destination.is_none());

        assert!(
            screen.route_requests().iter().any(|task| matches!(
                task,
                BackendTask::WalletTask(WalletTask::GetSpendLimit {
                    kind: SpendLimitKind::CoreSend {
                        destination: CoreDestinationScript::PubkeyHash,
                    },
                    ..
                })
            )),
            "with no destination, the form must ask for the P2PKH pooled quote"
        );
    }

    /// Auto measures Core while another balance is the selected source: the
    /// candidate's Max and fee caption are still Core's own quote, not a
    /// figure read through the selected source.
    #[test]
    fn an_auto_core_candidate_reads_its_own_quote_and_fee() {
        use crate::model::spend_limit::CoreSendQuoteFee;
        const QUOTE_CREDITS: u64 = 7_654_321 * CREDITS_PER_DUFF;

        let (mut screen, _temp_dir) = funded_send_screen();
        let destination = ValidatedAddress::Core(testnet_core_address(3));
        screen.validated_destination = Some(destination.clone());
        screen.selected_source = Some(SourceSelection::PlatformAddresses(
            screen.platform_source_addresses(),
        ));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let core = SourceSelection::CoreWallet;
        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
            kind, request_id, ..
        })) = screen.request_spend_limit_for(&core, Some(AddressKind::Core))
        else {
            panic!("the Core candidate must ask for its own quote");
        };
        let quote = CoreSendQuoteFee {
            fee_at_max_duffs: 4_321,
            spendable_input_count: 2,
            spendable_total_duffs: Some(7_658_642),
        };
        assert!(
            screen.refresh_form_inputs(&BackendTaskSuccessResult::SpendLimit {
                seed_hash,
                kind,
                request_id,
                max_credits: QUOTE_CREDITS,
                core_fee: Some(quote),
            })
        );

        let limits = screen.amount_limits_for(&core, Some(&destination));

        assert_eq!(limits.max_credits, Some(QUOTE_CREDITS));
        let fee = format_credits_as_dash(core_send_fee_at_max_duffs(&quote) * CREDITS_PER_DUFF);
        let hint = limits.hint.expect("a fee caption");
        assert!(
            hint.contains(&fee),
            "the caption must show the fee the quote proved ({fee}): {hint}"
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

    /// When the wallet says a Core send has nothing to offer, the form names
    /// the reason (here: only CoinJoin funds) instead of a fee shortfall or a
    /// retry, and Max fills nothing.
    #[test]
    fn core_send_states_why_no_max_is_offered() {
        use crate::model::spend_limit::CoreSendShortfall;
        let (mut screen, _temp_dir) = send_screen();
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(3)));
        let task = screen
            .request_spend_limit()
            .expect("the Core send form must ask for the pooled quote");
        let context = BackendTaskContext::for_dispatch_on(&task, Network::Testnet);

        screen.display_backend_task_error(
            &context,
            &TaskError::CoreSendMaxUnavailable {
                reason: CoreSendShortfall::NothingSpendable,
            },
        );

        assert!(screen.request_spend_limit().is_none());
        // Auto reads the same limits: a settled zero, not a pending quote.
        let source = screen.selected_source.clone().expect("source");
        let destination = screen.validated_destination.clone();
        let limits = screen.amount_limits_for(&source, destination.as_ref());
        assert_eq!(limits.max_credits, None);
        assert_eq!(
            limits.quote,
            resolve::QuoteState::Shortfall(CoreSendShortfall::NothingSpendable)
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
                    .label_contains(CoreSendShortfall::NothingSpendable.message())
                    .include_labels(),
            )
            .count();
        assert_eq!(caption, 1, "the form must say why Max is unavailable");
        assert!(
            harness
                .query_by_label("Retry available amount check")
                .is_none(),
            "retrying cannot change the answer"
        );
        assert!(
            harness
                .query_by_label_contains("Checking the available amount")
                .is_none(),
            "the answer is in, not pending"
        );
    }

    /// The shield fee preview states the fee the shield is charged, not the
    /// larger reserve the planner keeps back on the Platform address.
    #[test]
    fn shield_from_platform_preview_states_the_shield_fee_not_the_reserve() {
        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            100 * CREDITS_PER_DUFF,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        screen.amount = Some(Amount::new_dash(1.0));
        let platform_version = screen.app_context.platform_version();

        let preview = screen.current_fee_preview().expect("shield fee preview");

        let fee = crate::model::fee_estimation::shield_from_balance_fee_estimate(platform_version);
        assert_eq!(preview.fee_credits, fee);
        assert_eq!(
            preview.total_debit_credits,
            Amount::new_dash(1.0).value() + fee
        );
    }

    /// A shield from Platform spends credits the live balances keep showing
    /// until a sync pass reads them back, so the shield Max key also follows
    /// the wallet's shields in flight: a quote made before one opens is not
    /// offered while it runs, nor after it settles.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_shield_in_flight_moves_the_shield_from_platform_max_key() {
        let (mut screen, _temp_dir) = send_screen();
        let source_core = testnet_core_address(2);
        let source_platform =
            PlatformAddress::try_from(source_core.clone()).expect("platform source");
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            source_platform,
            source_core,
            100 * CREDITS_PER_DUFF,
        )]));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        let key = |screen: &WalletSendScreen| {
            let (_, kind, revision) = screen.spend_limit_target().expect("shield target");
            assert_eq!(kind, SpendLimitKind::ShieldFromPlatform);
            revision
        };
        let before = key(&screen);
        let balances = screen.live_platform_balances_revision();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        let (sender, _receiver) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(16);
        screen
            .app_context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                sender,
                egui::Context::default(),
            ))
            .await
            .expect("wallet backend");
        let backend = screen.app_context.wallet_backend().expect("wallet backend");

        let shielding = backend.begin_platform_shield(&seed_hash);
        let during = key(&screen);
        assert_ne!(during, before, "a quote from before the shield is stale");
        drop(shielding);
        let after = key(&screen);
        assert_eq!(
            screen.live_platform_balances_revision(),
            balances,
            "precondition: the balances never moved, only the shield did"
        );
        assert_ne!(after, during, "a quote from during the shield is stale");
        assert_ne!(
            after, before,
            "at the same balances, a quote from before the shield stays stale"
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
        let background = Rc::new(std::cell::Cell::new(false));
        let background_flag = background.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                move |ui, screen: &mut WalletSendScreen| {
                    let form_enabled = screen.send_confirmation.is_none();
                    let mut action = ui
                        .add_enabled_ui(form_enabled, |ui| {
                            let mut flag = background_flag.get();
                            if ui.checkbox(&mut flag, "Background control").changed() {
                                background_flag.set(flag);
                            }
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

        click_in_one_frame(&mut harness, "Send");

        assert!(
            matches!(*observed_action.borrow(), AppAction::None),
            "the original simple Send click must not dispatch a backend task"
        );
        assert!(confirmation_is_open(&harness));

        harness.get_by_label("Background control").click_accesskit();
        harness.step();
        assert!(
            !background.get(),
            "the confirmation dialog must block controls behind it"
        );

        click_confirm(&mut harness);
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

        let total = format_credits_as_dash(preview.total_debit_credits);
        assert_eq!(
            screen.simple_send_confirmation_message(),
            format!(
                "From: Unnamed wallet · Core\nTo: {short}\nAmount: {amount}\nFee: ≈ {estimated_fee}\nTotal: ≈ {total}\nThe network fee will be deducted from the output amount, so the recipient will receive {recipient_amount}.\nYour Dash is moved to Platform first, so this takes a little longer than a regular payment.\nCheck the recipient. Payments can't be undone.",
                short = destination
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

        let total = format_credits_as_dash(preview.total_debit_credits);
        assert_eq!(
            screen.simple_send_confirmation_message(),
            format!(
                "From: Unnamed wallet · Platform\nTo: {short}\nAmount: {amount}\nFee: ≈ {estimated_fee}\nTotal: ≈ {total}\nCheck the recipient. Payments can't be undone.",
                short = destination
            )
        );
    }

    #[test]
    fn advanced_send_click_opens_confirmation_without_dispatching() {
        let (mut screen, _temp_dir) = send_screen();
        let input_address = testnet_core_address(4);
        screen.set_balances_for_test(SendBalances {
            core_duffs: 200_000_000,
            core_addresses: vec![(input_address.clone(), 200_000_000)],
            ..SendBalances::default()
        });
        // Advanced › Core: one ticked address with its own amount.
        screen.form.choose_pay_from(
            PayFrom::Advanced,
            crate::model::send_route::PayFromKind::Core,
        );
        screen
            .form
            .set_advanced_ticked(&input_address.to_string(), true);
        screen
            .form
            .set_advanced_amount(&input_address.to_string(), "2".to_string());
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination = Some(ValidatedAddress::Core(testnet_core_address(5)));
        screen.amount = Some(Amount::new_dash(1.0));

        let observed_action = Rc::new(RefCell::new(AppAction::None));
        let captured_action = observed_action.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                move |ui, screen: &mut WalletSendScreen| {
                    let form_enabled = screen.send_confirmation.is_none();
                    let mut action = ui
                        .add_enabled_ui(form_enabled, |ui| screen.render_send_button(ui))
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
        assert!(confirmation_is_open(&harness));

        click_confirm(&mut harness);
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

    /// Several recipients confirm with the same From / To / Amount / Fee /
    /// Total lines as one recipient, addresses shortened so they never wrap.
    #[test]
    fn several_recipients_confirm_in_the_single_recipient_layout() {
        let (mut screen, _temp_dir) = send_screen();
        let first = PlatformAddress::try_from(testnet_core_address(8))
            .expect("platform destination")
            .to_bech32m_string(Network::Testnet);
        let second = PlatformAddress::try_from(testnet_core_address(9))
            .expect("platform destination")
            .to_bech32m_string(Network::Testnet);
        let source_core = testnet_core_address(10);
        screen.selected_source = Some(SourceSelection::PlatformAddresses(vec![(
            PlatformAddress::try_from(source_core.clone()).expect("platform source"),
            source_core,
            2 * CREDITS_PER_DUFF * 100_000_000,
        )]));
        screen.advanced_source_type = AdvancedSourceType::Platform;
        screen.advanced_outputs = vec![
            AdvancedOutput {
                address: first.clone(),
                amount: "0.02".to_string(),
            },
            AdvancedOutput {
                address: second.clone(),
                amount: "0.02".to_string(),
            },
        ];

        let message = screen.advanced_send_confirmation_message();
        let lines: Vec<&str> = message.lines().collect();

        assert_eq!(lines[0], "From: Unnamed wallet · Platform");
        assert_eq!(lines[1], format!("To: {} · 0.02 DASH", first));
        assert_eq!(lines[2], format!("To: {} · 0.02 DASH", second));
        assert_eq!(lines[3], "Amount: 0.04 DASH");
        assert!(lines[4].starts_with("Fee: "), "{message}");
        assert!(lines[5].starts_with("Total: "), "{message}");
        assert_eq!(
            lines.last().copied(),
            Some("Check the recipients. Payments can't be undone.")
        );
        assert!(
            message.contains(&format!("To: {first} · ")),
            "addresses are shown whole: {message}"
        );
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

    /// With the fee paid on top, the funding is the amount plus the 62 000
    /// duffs the network requires for the recipient and the change, so the
    /// amount only has to meet the smallest a named recipient may receive.
    #[test]
    fn advanced_core_to_platform_refuses_an_amount_the_fee_on_top_cannot_carry() {
        let (mut screen, seed_hash, _temp_dir) =
            advanced_core_to_platform("0.00000499", PlatformFeeStrategy::DeductFromFirstInput);
        let error = screen
            .send_advanced_core_to_platform(seed_hash)
            .expect_err("the amount is below the smallest a recipient may receive");
        assert_eq!(
            error,
            "This amount is too small to cover the network fee. \
             Enter at least 0.000005 DASH and try again."
        );

        let (mut screen, seed_hash, _temp_dir) =
            advanced_core_to_platform("0.000005", PlatformFeeStrategy::DeductFromFirstInput);
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

    /// Someone else's private address. The form cannot tell it from its own
    /// pool, so it is never the paying wallet's.
    fn foreign_shielded_destination() -> ValidatedAddress {
        ValidatedAddress::Shielded("tdash1zforeignprivateaddressforthetest".to_string())
    }

    fn funded_send_screen() -> (WalletSendScreen, tempfile::TempDir) {
        let (mut screen, temp_dir) = send_screen();
        let address = testnet_core_address(9);
        screen.set_balances_for_test(SendBalances {
            core_duffs: 200_000_000,
            core_addresses: vec![(address.clone(), 200_000_000)],
            platform: vec![(
                address.clone(),
                PlatformAddress::try_from(address).expect("platform source"),
                3 * 100_000_000 * CREDITS_PER_DUFF,
            )],
            shielded_credits: 0,
        });
        (screen, temp_dir)
    }

    /// Core and Platform funds shield only into the paying wallet's own
    /// pool, whatever address is entered. Paying someone else's private
    /// address from them must be refused before any task is built.
    #[test]
    fn core_and_platform_never_route_to_someone_elses_private_address() {
        let (mut screen, _temp_dir) = funded_send_screen();
        let platform_source = screen.get_platform_addresses();
        for source in [
            SourceSelection::CoreWallet,
            SourceSelection::PlatformAddresses(
                platform_source
                    .iter()
                    .map(|(core, platform, credits)| (*platform, core.clone(), *credits))
                    .collect(),
            ),
        ] {
            screen.selected_source = Some(source.clone());
            screen.validated_destination = Some(foreign_shielded_destination());
            screen.amount = Some(Amount::new_dash(0.5));
            assert_eq!(
                screen.validated_send_route().err().as_deref(),
                Some(SHIELDED_NOT_OWN_POOL),
                "{source:?} must not pay someone else's private address"
            );
            assert_eq!(
                screen.validate_and_send().err().as_deref(),
                Some(SHIELDED_NOT_OWN_POOL),
                "{source:?} must not build a task for someone else's private address"
            );
        }
    }

    /// Through the form: Pay from Core with a foreign private address
    /// keeps Send disabled, and pressing it anyway opens no confirmation.
    #[test]
    fn the_form_refuses_core_to_someone_elses_private_address() {
        let (mut screen, _temp_dir) = funded_send_screen();
        screen
            .form
            .choose_pay_from(PayFrom::Core, crate::model::send_route::PayFromKind::Core);
        screen.typed_destination = Some(foreign_shielded_destination());
        screen.amount = Some(Amount::new_dash(0.5));
        screen.sync_route();
        assert_eq!(
            screen.route.refusal,
            Some(crate::model::send_route::RouteRefusal::ShieldedNotOwnPool)
        );
        assert_eq!(screen.missing_step(), Some("Fix the highlighted field."));

        let ctx = egui::Context::default();
        screen.press_send(&ctx);
        assert!(
            screen.send_confirmation.is_none(),
            "a refused route must not reach the confirmation"
        );
        assert!(
            screen.prepare_send().is_err(),
            "a refused route must not produce a send action"
        );
    }

    /// UX-001: what was typed for Someone else survives a trip to My wallets.
    #[test]
    fn the_typed_recipient_survives_switching_to_my_wallets_and_back() {
        let (mut screen, _temp_dir) = funded_send_screen();
        let typed = platform_destination();
        screen.typed_destination = Some(typed.clone());
        let wallet = screen.selected_wallet_seed_hash;
        let seed_hash = wallet.expect("a wallet");
        let own = testnet_core_address(11);
        let _ = screen
            .own_addresses
            .ensure_requested(seed_hash, OwnAddressKind::Core);
        screen
            .own_addresses
            .store(seed_hash, OwnAddressKind::Core, own.to_string());
        screen.form.choose_to_mode(
            ToMode::MyWallets,
            wallet,
            crate::model::send_route::PayFromKind::Core,
        );
        screen.sync_route();
        assert_eq!(
            screen.validated_destination,
            Some(ValidatedAddress::Core(own)),
            "My wallets pays the wallet's own receiving address"
        );
        screen.form.choose_to_mode(
            ToMode::SomeoneElse,
            wallet,
            crate::model::send_route::PayFromKind::Core,
        );
        screen.sync_route();
        assert_eq!(screen.validated_destination.as_ref(), Some(&typed));
        assert_eq!(screen.typed_destination.as_ref(), Some(&typed));
    }

    /// A receiving address the form asked for is consumed, so it never ends a
    /// send's progress; one it did not ask for is left alone.
    #[test]
    fn an_own_receiving_address_reply_is_consumed_only_when_asked_for() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen.selected_wallet_seed_hash.expect("a wallet");
        let reply = BackendTaskSuccessResult::GeneratedReceiveAddress {
            seed_hash,
            address: testnet_core_address(12).to_string(),
        };
        assert!(!screen.refresh_form_inputs(&reply), "nobody asked for it");
        let _ = screen
            .own_addresses
            .ensure_requested(seed_hash, OwnAddressKind::Core);
        assert!(screen.refresh_form_inputs(&reply));
        assert!(
            screen
                .own_addresses
                .get(&seed_hash, OwnAddressKind::Core)
                .is_some()
        );
    }

    /// A Platform address of the paying wallet is its own even with nothing
    /// on it, so Auto never pays it from the same Platform balance.
    #[test]
    fn an_empty_platform_address_of_the_paying_wallet_is_its_own() {
        let (screen, _temp_dir) = funded_send_screen();
        let own = testnet_core_address(13);
        screen
            .selected_wallet
            .as_ref()
            .expect("a wallet")
            .write()
            .expect("wallet lock")
            .platform_address_info
            .insert(
                own.clone(),
                crate::model::wallet::PlatformAddressInfo {
                    balance: 0,
                    nonce: 0,
                },
            );
        let address = PlatformAddress::try_from(own).expect("platform address");
        let destination = ValidatedAddress::Platform {
            address,
            bech32m: address.to_bech32m_string(Network::Testnet),
        };
        assert_eq!(
            screen.own_account_of(&destination),
            Some(crate::model::send_route::PayFromKind::Platform)
        );
    }

    /// A refresh while the form is open keeps the own receiving address it
    /// already fetched, so the payment destination never changes unasked.
    #[test]
    fn a_refresh_keeps_the_own_receiving_address() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen.selected_wallet_seed_hash.expect("a wallet");
        let own = testnet_core_address(14).to_string();
        screen.seed_own_core_address_for_test(seed_hash, own.clone());
        screen.refresh();
        screen.refresh_on_arrival();
        assert_eq!(
            screen.own_addresses.get(&seed_hash, OwnAddressKind::Core),
            Some(own.as_str())
        );
        screen.reset_form();
        assert_eq!(
            screen.own_addresses.get(&seed_hash, OwnAddressKind::Core),
            None,
            "a new form asks again"
        );
    }

    /// The Platform maximum for several recipients leaves out every
    /// recipient's address and reserves the fee for all of their outputs, as
    /// the dispatched payment does.
    #[test]
    fn platform_max_for_several_recipients_matches_the_dispatched_payment() {
        let (mut screen, _temp_dir) = send_screen();
        let paying = testnet_core_address(15);
        let recipient_own = testnet_core_address(16);
        let platform = |address: &Address| PlatformAddress::try_from(address.clone()).unwrap();
        let addresses = vec![
            (
                paying.clone(),
                platform(&paying),
                100_000_000 * CREDITS_PER_DUFF,
            ),
            (
                recipient_own.clone(),
                platform(&recipient_own),
                200_000_000 * CREDITS_PER_DUFF,
            ),
        ];
        screen.set_balances_for_test(SendBalances {
            core_duffs: 0,
            core_addresses: vec![],
            platform: addresses.clone(),
            shielded_credits: 0,
        });
        let external = platform(&testnet_core_address(17));
        screen.form.add_recipient(
            platform(&recipient_own).to_bech32m_string(Network::Testnet),
            "0.1".to_string(),
        );
        screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
            address: external.to_bech32m_string(Network::Testnet),
            amount: "0.1".to_string(),
        };
        let source = SourceSelection::PlatformAddresses(
            addresses
                .iter()
                .map(|(core, platform, credits)| (*platform, core.clone(), *credits))
                .collect(),
        );
        let fee = estimate_platform_fee(&screen.app_context.fee_estimator(), 1, 2);
        assert_eq!(
            screen.amount_limits_for(&source, None).max_credits,
            Some(100_000_000 * CREDITS_PER_DUFF - fee)
        );
    }

    /// Fee options appear only where the payment uses them: an Advanced Core
    /// payment to Platform without per-address amounts always takes the fee
    /// from the output.
    #[test]
    fn fee_options_show_only_where_the_fee_strategy_is_used() {
        let (mut screen, _temp_dir) = funded_send_screen();
        screen
            .app_context
            .set_user_role(crate::model::user_role::UserRole::Power);
        let core = testnet_core_address(9).to_string();
        screen.form.choose_pay_from(
            PayFrom::Advanced,
            crate::model::send_route::PayFromKind::Core,
        );
        screen.form.choose_advanced_type(AdvancedSourceType::Core);
        screen.form.set_advanced_ticked(&core, true);
        screen.typed_destination = Some(platform_destination());
        screen.sync_route();
        assert!(!screen.shows_fee_options());
        screen.form.set_advanced_amount(&core, "0.5".to_string());
        assert!(screen.shows_fee_options());

        // Platform sources pick the paying input themselves.
        screen.form.choose_pay_from(
            PayFrom::Platform,
            crate::model::send_route::PayFromKind::Platform,
        );
        screen
            .form
            .add_recipient(testnet_platform_bech32m(21), "0.1".to_string());
        screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
            address: testnet_platform_bech32m(22),
            amount: "0.1".to_string(),
        };
        screen.sync_route();
        assert!(screen.uses_explicit_payment());
        assert!(!screen.shows_fee_options());
    }

    /// A receiving address from another network (an answer to a request
    /// sent before a network switch) never becomes the destination: the
    /// request stays open so the right address can still arrive.
    #[test]
    fn an_own_receiving_address_from_another_network_is_not_kept() {
        let (mut screen, _temp_dir) = send_screen();
        let seed_hash = screen.selected_wallet_seed_hash.expect("seed hash");
        let mainnet_core = mainnet_core_address();
        let mainnet_platform = PlatformAddress::try_from(mainnet_core.clone())
            .expect("platform address")
            .to_bech32m_string(Network::Mainnet);
        for (kind, reply) in [
            (
                OwnAddressKind::Core,
                BackendTaskSuccessResult::GeneratedReceiveAddress {
                    seed_hash,
                    address: mainnet_core.to_string(),
                },
            ),
            (
                OwnAddressKind::Platform,
                BackendTaskSuccessResult::GeneratedPlatformReceiveAddress {
                    seed_hash,
                    address: mainnet_platform.clone(),
                },
            ),
        ] {
            assert!(
                screen
                    .own_addresses
                    .ensure_requested(seed_hash, kind)
                    .is_some()
            );
            // The reply answers this form's request, so it is consumed and
            // never read as the end of a send, but it is not kept.
            assert!(
                screen.refresh_form_inputs(&reply),
                "{kind:?} reply consumed"
            );
            assert_eq!(screen.own_addresses.get(&seed_hash, kind), None);
            assert!(screen.own_addresses.is_fetching(&seed_hash, kind));
        }
    }

    fn mainnet_core_address() -> Address {
        Address::p2pkh(
            &dash_sdk::dpp::dashcore::PublicKey::new(
                dash_sdk::dpp::secp256k1::SecretKey::from_secret_bytes([3; 32])
                    .expect("key")
                    .public_key(),
            ),
            Network::Mainnet,
        )
    }

    /// Recipient rows accept only Core and Platform addresses of the active
    /// network.
    #[test]
    fn recipient_rows_need_an_address_of_the_active_network() {
        let (screen, _temp_dir) = send_screen();
        let mainnet = mainnet_core_address();
        let mainnet_platform = PlatformAddress::try_from(mainnet.clone())
            .expect("platform address")
            .to_bech32m_string(Network::Mainnet);
        assert_eq!(screen.row_address(&mainnet.to_string()), None);
        assert_eq!(screen.row_address(&mainnet_platform), None);
        assert!(matches!(
            screen.row_address(&testnet_core_address(4).to_string()),
            Some(ValidatedAddress::Core(_))
        ));
        assert!(matches!(
            screen.row_address(&testnet_platform_bech32m(4)),
            Some(ValidatedAddress::Platform { .. })
        ));
    }

    /// A row with a zero amount, or with no address, is incomplete, and Send
    /// refuses to build the payment.
    #[test]
    fn several_recipients_refuse_an_incomplete_row() {
        let (mut screen, _temp_dir) = funded_send_screen();
        screen
            .form
            .add_recipient(testnet_core_address(21).to_string(), "0.1".to_string());
        screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
            address: testnet_core_address(22).to_string(),
            amount: "0".to_string(),
        };
        screen.sync_route();
        assert!(!screen.recipient_rows_complete(), "a zero amount");
        assert_eq!(
            screen.prepare_send().err().as_deref(),
            Some(RECIPIENT_ROWS_INCOMPLETE)
        );

        screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
            address: String::new(),
            amount: "0.2".to_string(),
        };
        screen.sync_route();
        assert!(!screen.recipient_rows_complete(), "no address");
        assert_eq!(
            screen.prepare_send().err().as_deref(),
            Some(RECIPIENT_ROWS_INCOMPLETE)
        );

        screen.form.recipients_mut()[1].address = testnet_core_address(22).to_string();
        assert!(screen.recipient_rows_complete());
    }

    fn testnet_platform_bech32m(seed: u8) -> String {
        PlatformAddress::try_from(testnet_core_address(seed))
            .expect("platform address")
            .to_bech32m_string(Network::Testnet)
    }

    /// Shielding from Platform lets the planner pick the inputs, so the
    /// Summary does not list addresses it may not use.
    #[test]
    fn summary_details_are_not_shown_for_a_platform_shield() {
        let (mut screen, _temp_dir) = funded_send_screen();
        let source = screen
            .get_platform_addresses()
            .into_iter()
            .map(|(core, platform, credits)| (platform, core, credits))
            .collect();
        screen.selected_source = Some(SourceSelection::PlatformAddresses(source));
        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        screen.amount = Some(Amount::new_dash(0.5));
        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut WalletSendScreen| screen.render_summary_details(ui),
            screen,
        );
        harness.run();
        assert!(harness.query_by_label("Details").is_none());
    }

    /// Unshield leaves the private balance whichever wallet receives: its
    /// Auto never picks another wallet's private address.
    #[test]
    fn unshield_auto_receives_into_a_public_address_of_another_wallet() {
        let (screen, _temp_dir) = send_screen();
        let mut screen = screen.with_flow(SendFlow::Unshield);
        screen.form.choose_to_wallet([0x42; 32]);
        screen.sync_route();
        assert_eq!(
            screen.route.receive_kind,
            Some(crate::model::send_route::PayFromKind::Platform)
        );
    }

    /// A valid testnet private (Orchard) address derived from `seed_byte`.
    fn testnet_shielded_address(seed_byte: u8) -> String {
        let keys = platform_wallet::wallet::shielded::OrchardKeySet::from_seed(
            &[seed_byte; 64],
            Network::Testnet,
            0,
        )
        .expect("ZIP-32 derivation from a valid test seed");
        crate::model::address::encode_shielded_address(
            &keys.address_at(0).to_raw_address_bytes(),
            Network::Testnet,
        )
        .expect("valid Orchard payload encodes")
    }

    /// A send screen whose wallet has `own_address` as its private address.
    fn send_screen_with_own_shielded_address(
        own_address: &str,
    ) -> (WalletSendScreen, tempfile::TempDir) {
        let (mut screen, temp_dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        screen
            .app_context
            .shielded_addresses
            .lock()
            .expect("shielded address cache")
            .insert(seed_hash, own_address.to_string());
        screen.amount = Some(Amount::new_dash(0.01));
        (screen, temp_dir)
    }

    fn non_shielded_sources() -> [SourceSelection; 2] {
        [
            SourceSelection::CoreWallet,
            SourceSelection::PlatformAddresses(Vec::new()),
        ]
    }

    /// Core and Platform funds can only be shielded into the paying wallet's
    /// own pool: the backend ignores the typed address. Someone else's private
    /// address must be refused before any task is built, or the sender would
    /// keep the money while the app reports success.
    #[test]
    fn core_and_platform_refuse_someone_elses_private_address() {
        let (mut screen, _dir) =
            send_screen_with_own_shielded_address(&testnet_shielded_address(7));
        screen.validated_destination =
            Some(ValidatedAddress::Shielded(testnet_shielded_address(9)));

        for source in non_shielded_sources() {
            screen.selected_source = Some(source.clone());
            assert_eq!(
                screen.validated_send_route().err().as_deref(),
                Some(SHIELDED_NOT_OWN_POOL),
                "{source:?} must not pay someone else's private address"
            );
            assert_eq!(
                screen.validate_and_send().err().as_deref(),
                Some(SHIELDED_NOT_OWN_POOL),
                "{source:?} must not build a task for someone else's private address"
            );
        }
    }

    /// Before the wallet's private address is known, a typed private address
    /// cannot be proven to be its own, so it is refused as well. The Shield
    /// flow's own-pool marker needs no address and keeps working.
    #[test]
    fn core_refuses_a_private_address_while_its_own_is_unknown() {
        let (mut screen, _dir) = send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        assert_eq!(
            screen.app_context.shielded_receive_address(&seed_hash),
            None,
            "sanity: the wallet's private address is not known yet"
        );
        screen.amount = Some(Amount::new_dash(0.01));
        screen.selected_source = Some(SourceSelection::CoreWallet);
        screen.validated_destination =
            Some(ValidatedAddress::Shielded(testnet_shielded_address(9)));

        assert_eq!(
            screen.validated_send_route().err().as_deref(),
            Some(SHIELDED_NOT_OWN_POOL)
        );

        screen.validated_destination = Some(ValidatedAddress::Shielded(String::new()));
        assert!(
            matches!(
                screen.validated_send_route(),
                Ok(ValidatedSendRoute::CoreToShielded)
            ),
            "the Shield flow must not depend on the wallet's private address"
        );
    }

    /// Shielding into the wallet's own pool keeps working, whether the Shield
    /// flow's own-pool marker or the wallet's own address (in any accepted
    /// spelling) is the destination.
    #[test]
    fn core_and_platform_still_shield_into_the_own_pool() {
        let own = testnet_shielded_address(7);
        let own_raw = parse_shielded_recipient(&own).expect("own parses");
        let (mut screen, _dir) = send_screen_with_own_shielded_address(&own);

        for destination in [
            String::new(),
            own.clone(),
            own.to_uppercase(),
            hex::encode(own_raw),
        ] {
            screen.validated_destination = Some(ValidatedAddress::Shielded(destination.clone()));
            screen.selected_source = Some(SourceSelection::CoreWallet);
            assert!(
                matches!(
                    screen.validated_send_route(),
                    Ok(ValidatedSendRoute::CoreToShielded)
                ),
                "Core must shield into the own pool via {destination:?}"
            );
            screen.selected_source = Some(SourceSelection::PlatformAddresses(Vec::new()));
            assert!(
                matches!(
                    screen.validated_send_route(),
                    Ok(ValidatedSendRoute::PlatformToShielded(_))
                ),
                "Platform must shield into the own pool via {destination:?}"
            );
        }
    }

    /// The private balance can still pay someone else's private address.
    #[test]
    fn private_balance_still_pays_someone_elses_private_address() {
        let (mut screen, _dir) =
            send_screen_with_own_shielded_address(&testnet_shielded_address(7));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        screen.selected_source = Some(SourceSelection::Shielded(seed_hash, 1_000_000_000));
        screen.validated_destination =
            Some(ValidatedAddress::Shielded(testnet_shielded_address(9)));

        assert!(matches!(
            screen.validated_send_route(),
            Ok(ValidatedSendRoute::ShieldedToShielded(sh)) if sh == seed_hash
        ));
    }

    /// Someone else's private address with Pay from Core is refused before
    /// the click: Send is turned off and the reason is shown under the address,
    /// so the form never offers a send it will refuse.
    #[test]
    fn someone_elses_private_address_turns_core_shielding_off_before_the_click() {
        use egui_kittest::kittest::NodeT;
        const REFUSAL: &str = "can only move into your own private balance";

        let (mut screen, _dir) =
            send_screen_with_own_shielded_address(&testnet_shielded_address(7));
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        screen.seed_asset_lock_max_amount_for_test(seed_hash, 200_000_000);
        screen
            .form
            .choose_pay_from(PayFrom::Core, crate::model::send_route::PayFromKind::Core);
        screen.typed_destination = Some(ValidatedAddress::Shielded(testnet_shielded_address(9)));
        let mut amount_input = WalletSendScreen::new_amount_input();
        amount_input.set_value(Amount::new_dash(0.01));
        screen.amount_input = Some(amount_input);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut WalletSendScreen| {
                    let _ = screen.render_form(ui);
                    let _ = screen.render_send_confirmation(ui);
                },
                screen,
            );
        harness.run();

        assert!(
            harness.query_by_label_contains(REFUSAL).is_some(),
            "the refusal must be visible before any click"
        );
        // A turned-off primary button keeps its look but stops sensing clicks.
        let clickable = |harness: &Harness<'static, WalletSendScreen>| {
            harness
                .get_by_label("Send")
                .accesskit_node()
                .data()
                .supports_action(egui::accesskit::Action::Click)
        };
        assert!(
            !clickable(&harness),
            "Send must be turned off for someone else's private address"
        );
        harness.get_by_label("Send").click();
        harness.run();
        assert!(harness.state().send_confirmation.is_none());
        assert!(
            !matches!(harness.state().send_status, SendStatus::Error),
            "nothing is refused on click: the button never offered the send"
        );

        harness.state_mut().typed_destination =
            Some(ValidatedAddress::Shielded(testnet_shielded_address(7)));
        harness.run();
        assert!(
            harness.query_by_label_contains(REFUSAL).is_none(),
            "the wallet's own private address needs no refusal"
        );
        assert!(
            clickable(&harness),
            "shielding into the own pool stays available: {:?}",
            harness.state().missing_step()
        );
    }

    /// An explicit Core-to-Platform payment with the fee on top locks the
    /// amount plus what the backend adds, so Max leaves exactly that room below
    /// the asset-lock ceiling. A strategy that reduces the output keeps the
    /// simple route's figures.
    #[test]
    fn explicit_core_to_platform_max_reserves_what_dispatch_adds() {
        const CEILING_DUFFS: u64 = 100_000_000;
        let (mut screen, _temp_dir) = funded_send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        screen.seed_asset_lock_max_amount_for_test(seed_hash, CEILING_DUFFS);
        let core = testnet_core_address(9).to_string();
        screen.form.choose_pay_from(
            PayFrom::Advanced,
            crate::model::send_route::PayFromKind::Core,
        );
        screen.form.choose_advanced_type(AdvancedSourceType::Core);
        screen.form.set_advanced_ticked(&core, true);
        screen.form.set_advanced_amount(&core, "0.5".to_string());
        let destination = platform_destination();
        let estimator = screen.app_context.fee_estimator();

        let limits = screen.amount_limits_for(&SourceSelection::CoreWallet, Some(&destination));
        let added_duffs = estimator.estimate_address_funding_from_asset_lock_duffs(2);
        assert_eq!(
            limits.max_credits,
            Some((CEILING_DUFFS - added_duffs) * CREDITS_PER_DUFF),
            "the amount plus what dispatch adds must fit the ceiling"
        );
        assert_eq!(
            limits.min_credits,
            Some(estimator.address_funding_min_amount_duffs(false) * CREDITS_PER_DUFF)
        );
        let hint = limits.hint.expect("a fee caption");
        assert!(hint.contains("on top of the amount"), "{hint}");

        screen
            .form
            .choose_fee_strategy(PlatformFeeStrategy::ReduceFirstOutput);
        let limits = screen.amount_limits_for(&SourceSelection::CoreWallet, Some(&destination));
        let ValidatedAddress::Platform { address, .. } = destination else {
            unreachable!("a Platform destination");
        };
        let reserved = estimate_address_funding_fee_from_transition(
            screen.app_context.platform_version(),
            &address,
        );
        assert_eq!(
            limits.max_credits,
            Some(CEILING_DUFFS * CREDITS_PER_DUFF - reserved)
        );
        assert_eq!(
            limits.min_credits,
            Some(estimator.address_funding_min_amount_duffs(true) * CREDITS_PER_DUFF)
        );
        let hint = limits.hint.expect("a fee caption");
        assert!(hint.contains("deducted from the amount"), "{hint}");
    }

    /// Several Core recipients are paid from the pooled accounts, so the fee
    /// counts every coin the build could spend, not the address rows, and the
    /// confirmation shows that same figure.
    #[test]
    fn several_core_recipients_fee_counts_every_spendable_coin() {
        use crate::model::fee_estimation::estimate_core_l1_send_fee_duffs;
        use crate::model::spend_limit::CoreSendQuoteFee;

        let (mut screen, _temp_dir) = funded_send_screen();
        let seed_hash = screen
            .selected_wallet_seed_hash
            .expect("selected wallet seed hash");
        screen
            .form
            .choose_pay_from(PayFrom::Core, crate::model::send_route::PayFromKind::Core);
        screen
            .form
            .add_recipient(testnet_core_address(21).to_string(), "0.1".to_string());
        screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
            address: testnet_core_address(22).to_string(),
            amount: "0.1".to_string(),
        };
        screen.sync_route();
        let Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
            kind, request_id, ..
        })) = screen.request_spend_limit_for(&SourceSelection::CoreWallet, Some(AddressKind::Core))
        else {
            panic!("the form must ask for the Core quote");
        };
        assert!(
            screen.refresh_form_inputs(&BackendTaskSuccessResult::SpendLimit {
                seed_hash,
                kind,
                request_id,
                max_credits: 190_000_000 * CREDITS_PER_DUFF,
                core_fee: Some(CoreSendQuoteFee {
                    fee_at_max_duffs: 15_000,
                    spendable_input_count: 100,
                    spendable_total_duffs: Some(200_000_000),
                }),
            })
        );
        screen.sync_route();
        screen.sync_explicit_payment();

        let fee = screen.advanced_fee_estimate_credits().expect("a fee");
        assert!(
            fee >= estimate_core_l1_send_fee_duffs(100, 2) * CREDITS_PER_DUFF,
            "one address row must not hide 100 coins: {fee}"
        );
        let message = screen.advanced_send_confirmation_message();
        assert!(
            message.contains(&format!("Fee: ≈ {}", format_credits_as_dash(fee))),
            "{message}"
        );
    }

    /// Several Platform recipients without per-address amounts: the summary
    /// and the confirmation show the fee of the inputs the payment allocates,
    /// not a one-input guess.
    #[test]
    fn several_platform_recipients_show_the_fee_of_the_inputs_they_use() {
        const ONE_DASH_CREDITS: u64 = 100_000_000 * CREDITS_PER_DUFF;
        let (mut screen, _temp_dir) = send_screen();
        screen.set_balances_for_test(SendBalances {
            core_duffs: 0,
            core_addresses: Vec::new(),
            platform: (31..=33u8)
                .map(|seed| {
                    let core = testnet_core_address(seed);
                    let platform = PlatformAddress::try_from(core.clone()).expect("platform");
                    (core, platform, ONE_DASH_CREDITS)
                })
                .collect(),
            shielded_credits: 0,
        });
        screen.form.choose_pay_from(
            PayFrom::Platform,
            crate::model::send_route::PayFromKind::Platform,
        );
        screen
            .form
            .add_recipient(testnet_platform_bech32m(21), "1.25".to_string());
        screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
            address: testnet_platform_bech32m(22),
            amount: "1.25".to_string(),
        };
        screen.sync_route();

        let (action, message) = screen.prepare_send().expect("the payment builds");
        let AppAction::BackendTask(BackendTask::WalletTask(WalletTask::TransferPlatformCredits {
            inputs,
            ..
        })) = action
        else {
            panic!("a Platform transfer");
        };
        assert_eq!(inputs.len(), 3, "2.5 DASH needs all three addresses");
        let fee = estimate_platform_fee(&screen.app_context.fee_estimator(), 3, 2);
        assert_eq!(screen.advanced_fee_estimate_credits(), Some(fee));
        assert!(
            message.contains(&format!("Fee: ≈ {}", format_credits_as_dash(fee))),
            "{message}"
        );
    }

    /// A shield's planner picks its own input addresses, so ticked Platform
    /// addresses cannot pay into the private balance: the form says so, keeps
    /// Send off, and drops the promise that the ticks decide who pays.
    #[test]
    fn ticked_platform_addresses_cannot_shield() {
        use egui_kittest::kittest::NodeT;
        const REFUSAL: &str = "A payment into your private balance can't use only the addresses you tick. Choose Auto or Platform under Pay from.";
        const PROMISE: &str = "spread the payment over the ticked addresses";

        let (mut screen, _dir) =
            send_screen_with_own_shielded_address(&testnet_shielded_address(7));
        screen
            .app_context
            .set_user_role(crate::model::user_role::UserRole::Power);
        let core = testnet_core_address(9);
        let platform = PlatformAddress::try_from(core.clone()).expect("platform source");
        screen.set_balances_for_test(SendBalances {
            core_duffs: 0,
            core_addresses: Vec::new(),
            platform: vec![(core, platform, 300_000_000 * CREDITS_PER_DUFF)],
            shielded_credits: 0,
        });
        screen.form.choose_pay_from(
            PayFrom::Advanced,
            crate::model::send_route::PayFromKind::Platform,
        );
        screen
            .form
            .choose_advanced_type(AdvancedSourceType::Platform);
        screen
            .form
            .set_advanced_ticked(&platform.to_bech32m_string(Network::Testnet), true);
        screen.typed_destination = Some(ValidatedAddress::Shielded(testnet_shielded_address(7)));
        let mut amount_input = WalletSendScreen::new_amount_input();
        amount_input.set_value(Amount::new_dash(0.01));
        screen.amount_input = Some(amount_input);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 1200.0))
            .build_ui_state(
                |ui, screen: &mut WalletSendScreen| {
                    let _ = screen.render_form(ui);
                },
                screen,
            );
        harness.run();

        assert!(
            harness.query_by_label_contains(REFUSAL).is_some(),
            "the reason must be visible before any click"
        );
        assert!(harness.query_by_label_contains(PROMISE).is_none());
        assert!(
            !harness
                .get_by_label("Send")
                .accesskit_node()
                .data()
                .supports_action(egui::accesskit::Action::Click),
            "Send must be off"
        );
        assert_eq!(
            harness.state_mut().prepare_send().err().as_deref(),
            Some(REFUSAL)
        );

        // Other destinations still spread over the ticks, so the panel says so.
        harness.state_mut().typed_destination = Some(platform_destination());
        harness.run();
        assert!(harness.query_by_label_contains(REFUSAL).is_none());
        assert!(harness.query_by_label_contains(PROMISE).is_some());
    }

    /// Renders the form with "Reduce first output" chosen on an Advanced
    /// Core-to-Platform payment, then switches to `route`, a route that ignores
    /// the strategy, and checks the rendered Summary against the confirmation.
    fn assert_summary_ignores_a_stale_strategy(
        route: impl FnOnce(&mut WalletSendScreen),
        total_label: &str,
    ) {
        let (mut screen, _temp_dir) = funded_send_screen();
        screen
            .app_context
            .set_user_role(crate::model::user_role::UserRole::Power);
        let core = testnet_core_address(9).to_string();
        screen.form.choose_pay_from(
            PayFrom::Advanced,
            crate::model::send_route::PayFromKind::Core,
        );
        screen.form.choose_advanced_type(AdvancedSourceType::Core);
        screen.form.set_advanced_ticked(&core, true);
        screen.form.set_advanced_amount(&core, "0.5".to_string());
        screen
            .form
            .choose_fee_strategy(PlatformFeeStrategy::ReduceFirstOutput);
        screen.typed_destination = Some(platform_destination());
        let mut amount_input = WalletSendScreen::new_amount_input();
        amount_input.set_value(Amount::new_dash(0.2));
        screen.amount_input = Some(amount_input);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 1400.0))
            .build_ui_state(
                |ui, screen: &mut WalletSendScreen| {
                    let _ = screen.render_form(ui);
                },
                screen,
            );
        harness.run();
        assert!(
            harness.state().shows_fee_options(),
            "precondition: this route reads the strategy"
        );

        route(harness.state_mut());
        harness.run();

        let message = harness.state().advanced_send_confirmation_message();
        let total = message
            .lines()
            .find_map(|line| line.strip_prefix("Total: "))
            .expect("the confirmation states a total");
        assert!(
            harness.query_by_label("Recipient gets").is_none(),
            "this route charges the fee on top: {message}"
        );
        assert!(harness.query_by_label(total_label).is_some());
        assert!(
            harness.query_by_label(total).is_some(),
            "the Summary total must be the confirmation's {total}"
        );
    }

    #[test]
    fn a_stale_strategy_leaves_several_core_recipients_on_top() {
        assert_summary_ignores_a_stale_strategy(
            |screen| {
                screen
                    .form
                    .choose_pay_from(PayFrom::Core, crate::model::send_route::PayFromKind::Core);
                screen
                    .form
                    .add_recipient(testnet_core_address(21).to_string(), "0.1".to_string());
                screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
                    address: testnet_core_address(22).to_string(),
                    amount: "0.1".to_string(),
                };
            },
            "Total from Core",
        );
    }

    #[test]
    fn a_stale_strategy_leaves_several_platform_recipients_on_top() {
        assert_summary_ignores_a_stale_strategy(
            |screen| {
                screen.form.choose_pay_from(
                    PayFrom::Platform,
                    crate::model::send_route::PayFromKind::Platform,
                );
                screen
                    .form
                    .add_recipient(testnet_platform_bech32m(21), "0.1".to_string());
                screen.form.recipients_mut()[1] = crate::ui::state::send_form::RecipientRow {
                    address: testnet_platform_bech32m(22),
                    amount: "0.1".to_string(),
                };
            },
            "Total from Platform",
        );
    }
}
