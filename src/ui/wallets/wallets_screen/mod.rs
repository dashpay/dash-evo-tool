mod address_table;
mod asset_locks;
mod dialogs;
mod single_key_view;

pub(crate) use single_key_view::SINGLE_KEY_SEND_UNAVAILABLE;

use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::connection_status::spv_phase_summary;
use crate::context::feature_gate::FeatureGate;
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::spv_status::SpvStatus;
use crate::model::user_role::UserRole;
use crate::model::wallet::{TransactionStatus, Wallet, WalletSeedHash, WalletTransaction};
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::global_nav_switcher::GlobalNavEffect;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::{add_top_panel_with_global_nav_capturing, wallet_only_spec};
use crate::ui::components::wallet_unlock_popup::{WalletUnlockPopup, WalletUnlockResult};
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::helpers::{ModalOpeningGuard, clicked_outside_window_after_open};
use crate::ui::state::TrackedAssetLockCache;
use crate::ui::state::account_summary::{
    AccountCategory, AccountSummary, collect_account_summaries,
};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use crate::wallet_backend::TransactionHistoryStatus;
use crate::wallet_backend::poison::RwLockRecover;
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::Address;
use eframe::egui::{self, ComboBox, Context, Ui};
use egui::{Color32, Frame, Margin, RichText};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, RwLock};

use crate::backend_task::migration::single_key_restore::PendingProtectedRestore;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::ui::wallets::import_single_key::ImportSingleKeyDialog;
use crate::ui::wallets::restore_single_key::RestoreSingleKeyDialog;
use crate::ui::wallets::shielded_tab::ShieldedTabView;
use address_table::{SortColumn, SortOrder};
use dialogs::{
    FundPlatformAddressDialogState, MineDialogState, PrivateKeyDialogState, ReceiveDialogState,
};

/// Tab selector for the Accounts & Addresses section.
///
/// Each tab corresponds to either an `AccountCategory` or the special Shielded
/// view.  Visibility is controlled by developer mode: only DashCore, Platform,
/// and Shielded are shown by default; the System tab appears in developer mode
/// and consolidates all system/dev account categories into collapsible sections.
#[derive(Clone, PartialEq, Eq)]
enum AccountTab {
    /// Regular account category (BIP44, PlatformPayment)
    Category(AccountCategory, Option<u32>),
    /// Shielded wallet view.
    Shielded,
    /// Consolidated system tab (developer mode only) — shows all non-primary
    /// account categories as collapsible sections.
    System,
}

enum PendingWalletRemoval {
    Hd {
        seed_hash: WalletSeedHash,
        alias: String,
    },
    SingleKey {
        key_hash: SingleKeyHash,
        address: String,
        alias: String,
    },
}

impl Default for AccountTab {
    fn default() -> Self {
        AccountTab::Category(AccountCategory::Bip44, Some(0))
    }
}

/// Pure tab-visibility planner. Given the per-account summaries and display
/// context, decide which account tabs to show. Extracted from
/// [`WalletScreen::build_account_tabs`] so the visibility rules are unit-testable
/// without a live screen:
///
/// - **Platform is shown immediately** on wallet load (`show_platform_tab`),
///   empty until sync populates it — every HD wallet unconditionally bootstraps
///   a platform-payment receive address, so gating the tab on a completed sync
///   would hide the user's only in-app way to find that receive address until
///   the network responds.
/// - **The header total is always reconciled by a visible tab.** In developer
///   mode the System tab always shows the full breakdown; in default mode a
///   consolidated tab appears whenever a non-visible category actually holds
///   funds (`hidden_balance > 0`), so the visible tabs' balances always sum to
///   the wallet header total instead of silently dropping funds the user cannot
///   see.
/// - **No duplicate Platform tab.** Today upstream's `all_accounts()` excludes
///   the platform-payment pool, so the primary loop never emits a Platform tab
///   and the dedicated push is the sole source. The de-dup guard keeps that true
///   if a future upstream bump ever folds that pool into `all_accounts()`.
fn plan_account_tabs(
    summaries: &[AccountSummary],
    developer_mode: bool,
    shielded_available: bool,
    show_platform_tab: bool,
) -> Vec<AccountTab> {
    let mut tabs: Vec<AccountTab> = Vec::new();

    // Default-visible primary tabs: BIP-44 accounts (the per-category Core
    // breakdown). Platform is normally added by the dedicated block below.
    for summary in summaries {
        if !summary.category.is_visible_in_default_mode() {
            continue;
        }
        tabs.push(AccountTab::Category(
            summary.category.clone(),
            summary.index,
        ));
    }

    // Ensure the Dash Core tab exists even without summaries.
    if !tabs
        .iter()
        .any(|t| matches!(t, AccountTab::Category(AccountCategory::Bip44, Some(0))))
    {
        tabs.insert(0, AccountTab::Category(AccountCategory::Bip44, Some(0)));
    }

    // Platform tab, right after the BIP-44 accounts. De-dup-guarded against a
    // Platform tab the primary loop may already have pushed.
    if show_platform_tab
        && !tabs
            .iter()
            .any(|t| matches!(t, AccountTab::Category(AccountCategory::PlatformPayment, _)))
    {
        tabs.push(AccountTab::Category(AccountCategory::PlatformPayment, None));
    }

    // Shielded tab only when the connected network supports it.
    if shielded_available {
        tabs.push(AccountTab::Shielded);
    }

    // Consolidated System/Other tab: always in developer mode; in default mode
    // only when a non-visible category actually holds funds, so the visible tabs
    // always reconcile the wallet header total.
    let hidden_balance: u64 = summaries
        .iter()
        .filter(|s| s.category.is_system_category())
        .map(|s| s.confirmed_balance)
        .sum();
    if developer_mode || hidden_balance > 0 {
        tabs.push(AccountTab::System);
    }

    tabs
}

/// Refresh mode for dev mode dropdown - controls what gets refreshed.
///
/// There is no "Core only" mode: Core wallet state (balances/UTXOs) is kept
/// current continuously by the upstream runtime and pushed via the EventBridge,
/// so there is nothing to reconcile on demand. Refresh only re-fetches the
/// DAPI-sourced Platform-address balances, optionally alongside the always-live
/// Core view.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum RefreshMode {
    /// Core wallet + Platform address sync
    #[default]
    All,
    /// Only Platform address sync
    PlatformOnly,
}

impl RefreshMode {
    fn label(&self) -> &'static str {
        match self {
            RefreshMode::All => "Core + Platform",
            RefreshMode::PlatformOnly => "Platform Only",
        }
    }

    fn next(self) -> Self {
        match self {
            RefreshMode::All => RefreshMode::PlatformOnly,
            RefreshMode::PlatformOnly => RefreshMode::All,
        }
    }
}

pub struct WalletsBalancesScreen {
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    selected_single_key_wallet: Option<Arc<RwLock<SingleKeyWallet>>>,
    pub(crate) app_context: Arc<AppContext>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    refreshing: bool,
    show_rename_dialog: bool,
    rename_dialog_opening_guard: ModalOpeningGuard,
    rename_input: String,
    wallet_unlock_popup: WalletUnlockPopup,
    show_sk_unlock_dialog: bool,
    sk_password_input: PasswordInput,
    remove_wallet_dialog: Option<ConfirmationDialog>,
    pending_wallet_removal: Option<PendingWalletRemoval>,
    receive_dialog: ReceiveDialogState,
    fund_platform_dialog: FundPlatformAddressDialogState,
    private_key_dialog: PrivateKeyDialogState,
    mine_dialog: MineDialogState,
    selected_account: Option<(AccountCategory, Option<u32>)>,
    show_zero_balance_addresses: bool,
    /// Pending refresh of platform address balances (triggered after transfers)
    pending_platform_balance_refresh: Option<WalletSeedHash>,
    /// Whether we should refresh the wallet after it's unlocked
    pending_refresh_after_unlock: bool,
    /// The refresh mode to use after unlock (if pending_refresh_after_unlock is true)
    pending_refresh_mode: RefreshMode,
    /// Current page for single key wallet UTXO pagination (0-indexed)
    utxo_page: usize,
    /// Selected refresh mode (only shown in dev mode)
    refresh_mode: RefreshMode,
    /// Currently selected account tab in the Accounts & Addresses section
    selected_account_tab: AccountTab,
    /// Shielded tab view component (lazily initialized per wallet)
    shielded_tab_view: Option<ShieldedTabView>,
    /// Whether a wallet switch should trigger a Core refresh on the next frame
    pending_wallet_refresh_on_switch: bool,
    /// Cached filtered transaction indices for the currently selected wallet.
    /// Invalidated (set to None) on wallet switch or transaction updates.
    cached_tx_indices: Option<Vec<usize>>,
    /// Transaction count at the time `cached_tx_indices` was last built.
    /// Used to detect list growth that doesn't make existing indices OOB.
    cached_tx_source_len: Option<usize>,
    /// Last hydration notice applied to the transaction-history banner. This
    /// prevents a dismissed persistent notice from being recreated every frame.
    transaction_history_notice: Option<(WalletSeedHash, TransactionHistoryStatus)>,
    transaction_history_banner: MessageBanner,
    /// Persistent warning banner rendered on the single-key wallet detail
    /// view when the app is running on the SPV backend. Stored on the screen
    /// (rather than constructed fresh each frame) so the underlying tracing
    /// log fires once on mode entry instead of every repaint.
    pub(crate) sk_spv_warning_banner: crate::ui::components::MessageBanner,
    /// "Import private key (advanced)" modal dialog. Routes single-key imports
    /// through [`crate::wallet_backend::SingleKeyView::import_wif`].
    import_single_key_dialog: ImportSingleKeyDialog,
    /// "Restore a protected imported key" modal dialog. Opened from the
    /// post-migration restore banner; routes the password through
    /// [`crate::backend_task::migration::single_key_restore::restore_protected_single_key`].
    restore_single_key_dialog: RestoreSingleKeyDialog,
    /// Protected single-key rows preserved by the migration that still need
    /// the user's old password to restore. Recomputed lazily (see
    /// [`Self::refresh_pending_protected_restores`]); drives the restore
    /// banner and is emptied as keys are restored.
    pending_protected_restores: Vec<PendingProtectedRestore>,
    /// Whether [`Self::pending_protected_restores`] has been computed at
    /// least once this session, so the (DB-touching) scan runs lazily on
    /// first paint rather than in the constructor.
    pending_restores_scanned: bool,
    /// Tracked asset locks for the selected wallet, fetched off the UI thread
    /// via the App Task System and rendered by the Asset Locks tab.
    asset_lock_cache: TrackedAssetLockCache,
}

/// A resolved wallet selection: at most one of an HD wallet or a single-key
/// wallet. The setter-symmetry invariant keeps the two mutually exclusive.
type ResolvedWalletSelection = (
    Option<Arc<RwLock<Wallet>>>,
    Option<Arc<RwLock<SingleKeyWallet>>>,
);

/// Resolve the shared store's per-network selection into wallet handles,
/// single-key preferred then HD; `(None, None)` when the store names no loaded
/// wallet (the caller applies any first-wallet fallback). The setter-symmetry
/// invariant keeps at most one hash set, so the preference never arbitrates a
/// real conflict — construction and `refresh_on_arrival` share this one path.
fn resolve_selection_from_store(app_context: &Arc<AppContext>) -> ResolvedWalletSelection {
    let selected_hd_hash = app_context
        .selected_wallet_hash
        .lock()
        .ok()
        .and_then(|g| *g);
    let selected_sk_hash = app_context
        .selected_single_key_hash
        .lock()
        .ok()
        .and_then(|g| *g);

    if let Some(sk_hash) = selected_sk_hash
        && let Ok(sk_wallets) = app_context.single_key_wallets.read()
        && let Some(wallet) = sk_wallets.get(&sk_hash)
    {
        return (None, Some(wallet.clone()));
    }

    if let Some(hd_hash) = selected_hd_hash
        && let Ok(wallets) = app_context.wallets.read()
        && let Some(wallet) = wallets.get(&hd_hash)
    {
        return (Some(wallet.clone()), None);
    }

    (None, None)
}

impl WalletsBalancesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        // Restore the previously selected wallet from the shared store, then fall
        // back to the first available wallet only when the store names nothing.
        let (mut selected_wallet, mut selected_single_key_wallet) =
            resolve_selection_from_store(app_context);

        if selected_wallet.is_none() && selected_single_key_wallet.is_none() {
            selected_wallet = app_context.wallets.read_recover().values().next().cloned();
            if selected_wallet.is_none() {
                selected_single_key_wallet = app_context
                    .single_key_wallets
                    .read_recover()
                    .values()
                    .next()
                    .cloned();
            }
        }

        Self::create_with_selection(app_context, selected_wallet, selected_single_key_wallet)
    }

    fn create_with_selection(
        app_context: &Arc<AppContext>,
        selected_wallet: Option<Arc<RwLock<Wallet>>>,
        selected_single_key_wallet: Option<Arc<RwLock<SingleKeyWallet>>>,
    ) -> Self {
        let shielded_tab_view = selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok().map(|g| g.seed_hash()))
            .map(|hash| ShieldedTabView::new(app_context, hash));

        Self {
            selected_wallet,
            selected_single_key_wallet,
            app_context: app_context.clone(),
            sort_column: SortColumn::Index,
            sort_order: SortOrder::Ascending,
            refreshing: false,
            show_rename_dialog: false,
            rename_dialog_opening_guard: ModalOpeningGuard::default(),
            rename_input: String::new(),
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_sk_unlock_dialog: false,
            sk_password_input: PasswordInput::new().with_hint_text("Enter password"),
            remove_wallet_dialog: None,
            pending_wallet_removal: None,
            receive_dialog: ReceiveDialogState::default(),
            fund_platform_dialog: FundPlatformAddressDialogState::default(),
            private_key_dialog: PrivateKeyDialogState::default(),
            mine_dialog: MineDialogState::default(),
            selected_account: None,
            show_zero_balance_addresses: false,
            pending_platform_balance_refresh: None,
            pending_refresh_after_unlock: false,
            pending_refresh_mode: RefreshMode::default(),
            utxo_page: 0,
            refresh_mode: RefreshMode::default(),
            selected_account_tab: AccountTab::default(),
            shielded_tab_view,
            pending_wallet_refresh_on_switch: false,
            cached_tx_indices: None,
            cached_tx_source_len: None,
            transaction_history_notice: None,
            transaction_history_banner: MessageBanner::new(),
            sk_spv_warning_banner: crate::ui::components::MessageBanner::new(),
            import_single_key_dialog: ImportSingleKeyDialog::new(app_context.network),
            restore_single_key_dialog: RestoreSingleKeyDialog::new(),
            pending_protected_restores: Vec::new(),
            pending_restores_scanned: false,
            asset_lock_cache: TrackedAssetLockCache::default(),
        }
    }

    fn persist_selected_wallet_hash(&self, hash: Option<WalletSeedHash>) {
        self.app_context.set_selected_hd_wallet(hash);
    }

    fn open_rename_dialog(&mut self, alias: Option<String>) {
        self.show_rename_dialog = true;
        self.rename_dialog_opening_guard.arm();
        self.rename_input = alias.unwrap_or_default();
    }

    fn persist_selected_single_key_hash(&self, hash: Option<[u8; 32]>) {
        self.app_context.set_selected_single_key_wallet(hash);
    }

    /// Set the selected HD wallet and update all associated state (persisted
    /// hash).  All code paths that change `selected_wallet` should go through
    /// this helper to keep the panel consistent.
    fn set_selected_hd_wallet(&mut self, wallet: Option<Arc<RwLock<Wallet>>>) {
        let seed_hash = wallet
            .as_ref()
            .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
        self.selected_wallet = wallet;
        self.selected_single_key_wallet = None;
        self.selected_account = None;
        self.selected_account_tab = AccountTab::default();
        self.cached_tx_indices = None;
        self.cached_tx_source_len = None;

        self.shielded_tab_view =
            seed_hash.map(|hash| ShieldedTabView::new(&self.app_context, hash));

        if let Some(hash) = seed_hash {
            self.persist_selected_wallet_hash(Some(hash));
            // Chain sync is SPV-only and continuous; no RPC refresh-on-switch.
        } else {
            self.persist_selected_wallet_hash(None);
        }
    }

    fn select_hd_wallet(&mut self, wallet: Arc<RwLock<Wallet>>) {
        self.set_selected_hd_wallet(Some(wallet));
        self.persist_selected_single_key_hash(None);
    }

    /// Select the HD wallet with `seed_hash`, if this network has it. Used to
    /// mirror a wallet chosen elsewhere — the global-nav pill, or another page —
    /// into this page's own selection. An unknown hash leaves the selection
    /// untouched.
    fn select_hd_wallet_by_hash(&mut self, seed_hash: WalletSeedHash) {
        let wallet = self
            .app_context
            .wallets
            .read()
            .ok()
            .and_then(|wallets| wallets.get(&seed_hash).cloned());
        if let Some(wallet) = wallet {
            self.select_hd_wallet(wallet);
        }
    }

    /// Consume the global-nav effect this page is bound to: a wallet switched on
    /// the pill becomes this page's selected wallet (FR-GLOBAL-NAV-2 rule 2).
    /// The app-global selection itself is already written by the shared applier.
    fn apply_nav_effect(&mut self, effect: GlobalNavEffect) {
        if let GlobalNavEffect::SwitchWallet(seed_hash) = effect {
            self.select_hd_wallet_by_hash(seed_hash);
        }
    }

    fn select_single_key_wallet(&mut self, wallet: Arc<RwLock<SingleKeyWallet>>) {
        self.selected_single_key_wallet = Some(wallet.clone());
        self.selected_wallet = None;
        self.selected_account = None;
        self.utxo_page = 0;

        if let Ok(hash) = wallet.read().map(|g| g.key_hash) {
            self.persist_selected_single_key_hash(Some(hash));
        }
        self.persist_selected_wallet_hash(None);
    }

    /// Adopt a store-resolved selection into the screen cache, routing through
    /// the existing selection setters so dependent view state stays consistent.
    /// Skips when the cache already holds the same wallet, so re-arriving is
    /// idempotent (no spurious account-tab reset).
    fn adopt_store_selection(
        &mut self,
        store_hd: Option<Arc<RwLock<Wallet>>>,
        store_sk: Option<Arc<RwLock<SingleKeyWallet>>>,
    ) {
        if let Some(wallet) = store_hd {
            let already = self
                .selected_wallet
                .as_ref()
                .is_some_and(|cur| Arc::ptr_eq(cur, &wallet));
            if !already {
                self.select_hd_wallet(wallet);
            }
            return;
        }
        if let Some(wallet) = store_sk {
            let already = self
                .selected_single_key_wallet
                .as_ref()
                .is_some_and(|cur| Arc::ptr_eq(cur, &wallet));
            if !already {
                self.select_single_key_wallet(wallet);
            }
        }
    }

    pub(crate) fn update_selected_wallet_for_network(&mut self) {
        // Check if HD wallet selection is still valid
        if let Some(wallet_arc) = &self.selected_wallet {
            let seed_hash = wallet_arc.read().ok().map(|w| w.seed_hash());
            if let Some(hash) = seed_hash
                && let Ok(wallets) = self.app_context.wallets.read()
                && wallets.contains_key(&hash)
            {
                self.selected_account = None;
                return;
            }
            // HD wallet no longer valid
            self.set_selected_hd_wallet(None);
        }

        // Check if single key wallet selection is still valid
        if let Some(wallet_arc) = &self.selected_single_key_wallet {
            let key_hash = wallet_arc.read().ok().map(|w| w.key_hash());
            if let Some(hash) = key_hash
                && let Ok(wallets) = self.app_context.single_key_wallets.read()
                && wallets.contains_key(&hash)
            {
                self.selected_account = None;
                return;
            }
            // Single key wallet no longer valid
            self.selected_single_key_wallet = None;
        }

        // No valid selection, pick a new one (HD wallet first, then single key)
        let next_hd = self
            .app_context
            .wallets
            .read()
            .ok()
            .and_then(|w| w.values().next().cloned());
        if let Some(wallet) = next_hd {
            self.set_selected_hd_wallet(Some(wallet));
            return;
        }

        if let Ok(wallets) = self.app_context.single_key_wallets.read()
            && let Some(wallet) = wallets.values().next().cloned()
        {
            self.selected_single_key_wallet = Some(wallet);
            self.selected_wallet = None;
            self.selected_account = None;
            return;
        }

        self.selected_account = None;
    }

    /// Clear all transient request/pending state that could fire against the
    /// wrong context after a network switch.
    pub(crate) fn reset_transient_state(&mut self) {
        self.pending_platform_balance_refresh = None;
        self.pending_refresh_after_unlock = false;
        self.pending_wallet_refresh_on_switch = false;
        self.refreshing = false;
    }

    /// Reset all cached AddressInput widgets so they pick up the new network.
    pub(crate) fn invalidate_address_inputs(&mut self) {
        self.mine_dialog.address_input = None;
        self.mine_dialog.validated_address = None;
        self.cached_tx_indices = None;
        self.cached_tx_source_len = None;
    }

    /// Request a fresh receive address through the SPV-watched upstream pool.
    ///
    /// Routes through the [`GenerateReceiveAddress`](crate::backend_task::wallet::WalletTask::GenerateReceiveAddress)
    /// backend task (→ `next_receive_address` → upstream `next_unused`) so the
    /// returned address is always inside the gap-limit window SPV monitors.
    /// Deriving DET-side here would hand out addresses past the watched window
    /// and lose deposits sent to them.
    fn add_receiving_address(&mut self) -> AppAction {
        let Some(seed_hash) = self
            .selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok())
            .map(|w| w.seed_hash())
        else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Select a wallet first, then try again.",
                MessageType::Error,
            );
            return AppAction::None;
        };

        AppAction::BackendTask(BackendTask::WalletTask(
            crate::backend_task::wallet::WalletTask::GenerateReceiveAddress { seed_hash },
        ))
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        // Build items for the selector - both HD and single key wallets
        #[derive(Clone)]
        enum WalletItem {
            Hd(Arc<RwLock<Wallet>>),
            SingleKey(Arc<RwLock<SingleKeyWallet>>),
        }

        let mut items: Vec<(String, WalletItem)> = Vec::new();

        // Add HD wallets
        if let Ok(wallets_guard) = self.app_context.wallets.read() {
            for wallet in wallets_guard.values() {
                let guard = wallet.read_recover();
                let seed_hash = guard.seed_hash();
                let core_balance = self.core_balance_duffs(&seed_hash);
                let platform_balance = self.platform_balance_duffs(&seed_hash);
                let shielded_balance = self.shielded_balance_duffs(&seed_hash);
                let balance_dash =
                    (core_balance + platform_balance + shielded_balance) as f64 * 1e-8;
                let label = format!(
                    "HD: {} ({:.4} DASH)",
                    guard.alias.clone().unwrap_or_else(|| "Unnamed".to_string()),
                    balance_dash
                );
                items.push((label, WalletItem::Hd(wallet.clone())));
            }
        }

        // Add single key wallets
        if let Ok(wallets_guard) = self.app_context.single_key_wallets.read() {
            for wallet in wallets_guard.values() {
                let guard = wallet.read_recover();
                let balance_dash = guard.total_balance_duffs() as f64 * 1e-8;
                let label = format!(
                    "SK: {} ({:.4} DASH)",
                    guard.alias.clone().unwrap_or_else(|| "Unnamed".to_string()),
                    balance_dash
                );
                items.push((label, WalletItem::SingleKey(wallet.clone())));
            }
        }

        if items.is_empty() {
            self.render_no_wallets_view(ui);
            return action;
        }

        // Determine the currently selected label
        let selected_label = if let Some(wallet) = &self.selected_wallet {
            wallet
                .read()
                .ok()
                .map(|guard| {
                    format!(
                        "HD: {}",
                        guard.alias.clone().unwrap_or_else(|| "Unnamed".to_string())
                    )
                })
                .unwrap_or_else(|| "Select a wallet".to_string())
        } else if let Some(wallet) = &self.selected_single_key_wallet {
            wallet
                .read()
                .ok()
                .map(|guard| {
                    format!(
                        "SK: {}",
                        guard.alias.clone().unwrap_or_else(|| "Unnamed".to_string())
                    )
                })
                .unwrap_or_else(|| "Select a wallet".to_string())
        } else {
            "Select a wallet".to_string()
        };

        // Get current balance
        let current_balance = if let Some(wallet) = &self.selected_wallet {
            wallet
                .read()
                .ok()
                .map(|g| {
                    let seed_hash = g.seed_hash();
                    let core = self.core_balance_duffs(&seed_hash);
                    let platform = self.platform_balance_duffs(&seed_hash);
                    let shielded = self.shielded_balance_duffs(&seed_hash);
                    core + platform + shielded
                })
                .unwrap_or(0)
        } else if let Some(wallet) = &self.selected_single_key_wallet {
            wallet
                .read()
                .ok()
                .map(|g| g.total_balance_duffs())
                .unwrap_or(0)
        } else {
            0
        };

        ui.with_layout(
            egui::Layout::left_to_right(egui::Align::TOP).with_main_justify(true),
            |ui| {
                ui.horizontal(|ui| {
                    ComboBox::from_id_salt("wallet_selector")
                        .selected_text(&selected_label)
                        .show_ui(ui, |ui| {
                            for (label, wallet_item) in &items {
                                let is_selected = match wallet_item {
                                    WalletItem::Hd(w) => self
                                        .selected_wallet
                                        .as_ref()
                                        .is_some_and(|selected| Arc::ptr_eq(selected, w)),
                                    WalletItem::SingleKey(w) => self
                                        .selected_single_key_wallet
                                        .as_ref()
                                        .is_some_and(|selected| Arc::ptr_eq(selected, w)),
                                };
                                if ui.selectable_label(is_selected, label).clicked() {
                                    match wallet_item {
                                        WalletItem::Hd(w) => {
                                            self.select_hd_wallet(w.clone());
                                        }
                                        WalletItem::SingleKey(w) => {
                                            self.select_single_key_wallet(w.clone());
                                        }
                                    }
                                }
                            }
                        });

                    ui.colored_label(
                        DashColors::text_primary(ui.style().visuals.dark_mode),
                        format!(" Balance: {}", format_duffs_as_dash(current_balance)),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    // Clone wallet arcs before using to avoid borrow conflicts
                    let hd_wallet_opt = self.selected_wallet.clone();
                    let single_key_wallet_opt = self.selected_single_key_wallet.clone();

                    // Buttons for HD wallet
                    if let Some(wallet_arc) = hd_wallet_opt {
                        self.render_remove_wallet_button(ui);
                        ui.add_space(8.0);

                        // Extract wallet state before calling mutable methods
                        let (uses_password, is_open, alias) = {
                            if let Ok(wallet) = wallet_arc.read() {
                                (wallet.uses_password, wallet.is_open(), wallet.alias.clone())
                            } else {
                                (false, false, None)
                            }
                        };

                        let mut should_lock_wallet = false;
                        if uses_password {
                            if is_open {
                                if ui.button("Lock").clicked() {
                                    should_lock_wallet = true;
                                }
                            } else if ui.button("Unlock").clicked() {
                                self.wallet_unlock_popup.open();
                            }
                        }
                        if should_lock_wallet {
                            self.lock_selected_wallet();
                        }
                        ui.add_space(8.0);
                        if ui.button("Rename").clicked() {
                            self.open_rename_dialog(alias);
                        }
                    }

                    // Buttons for single key wallet
                    if let Some(wallet_arc) = single_key_wallet_opt {
                        let alias = wallet_arc.read().ok().and_then(|w| w.alias.clone());

                        self.render_remove_wallet_button(ui);

                        ui.add_space(8.0);

                        // A password-protected single key is never opened into
                        // the shared map (signing decrypts just-in-time), so the
                        // only gesture is "Unlock", which confirms the passphrase
                        // against the vault without retaining any plaintext.
                        let uses_password = wallet_arc
                            .read()
                            .ok()
                            .map(|w| w.uses_password)
                            .unwrap_or(false);

                        if uses_password && ui.button("Unlock").clicked() {
                            self.show_sk_unlock_dialog = true;
                        }

                        ui.add_space(8.0);

                        // Rename button
                        if ui.button("Rename").clicked() {
                            self.open_rename_dialog(alias);
                        }
                    }
                });
            },
        );

        action
    }

    fn render_bottom_options(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|wallet_guard| wallet_guard.read_recover().is_open());

        // Only show "Add Receiving Address" button for Dash Core account (BIP44 account 0)
        let is_main_account = self
            .selected_account
            .as_ref()
            .is_some_and(|(category, index)| {
                *category == AccountCategory::Bip44 && *index == Some(0)
            });

        if wallet_is_open && is_main_account {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("➕ Add Receiving Address").size(14.0))
                    .clicked()
                {
                    action |= self.add_receiving_address();
                }
            });
        }
        action
    }

    fn render_remove_wallet_button(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

        if self.selected_wallet.is_some() || self.selected_single_key_wallet.is_some() {
            let remove_button =
                egui::Button::new(RichText::new("Remove").color(Color32::WHITE).size(14.0))
                    .min_size(egui::vec2(0.0, 28.0))
                    .fill(DashColors::error_color(!dark_mode))
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(4.0);

            if ui.add(remove_button).clicked() {
                self.request_selected_wallet_removal();
            }
        }

        if let Some(dialog) = self.remove_wallet_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(status) = response.inner.dialog_response {
                match status {
                    ConfirmationStatus::Confirmed => {
                        self.remove_wallet_dialog = None;
                        match self.pending_wallet_removal.take() {
                            Some(PendingWalletRemoval::Hd { seed_hash, alias }) => {
                                self.handle_wallet_removal(seed_hash, alias);
                            }
                            Some(PendingWalletRemoval::SingleKey {
                                key_hash,
                                address,
                                alias,
                            }) => {
                                self.handle_single_key_wallet_removal(key_hash, address, alias);
                            }
                            None => {}
                        }
                    }
                    ConfirmationStatus::Canceled => {
                        self.remove_wallet_dialog = None;
                        self.pending_wallet_removal = None;
                    }
                }
            }
        }
    }

    fn request_selected_wallet_removal(&mut self) {
        let (pending, message) = if let Some(wallet) = &self.selected_wallet {
            let wallet = wallet.read_recover();
            let alias = wallet
                .alias
                .clone()
                .unwrap_or_else(|| "Unnamed Wallet".to_string());
            let message = format!(
                "Removing wallet \"{}\" clears the data used by this version, including its addresses, balances, and asset locks. Identities linked to it will remain, but keys derived from this wallet will not work unless the wallet is imported again. If this wallet came from an earlier version, that version's read-only recovery database stays on this device. Continue?",
                alias
            );
            (
                PendingWalletRemoval::Hd {
                    seed_hash: wallet.seed_hash(),
                    alias,
                },
                message,
            )
        } else if let Some(wallet) = &self.selected_single_key_wallet {
            let wallet = wallet.read_recover();
            let alias = wallet
                .alias
                .clone()
                .unwrap_or_else(|| "Unnamed Wallet".to_string());
            let message = format!(
                "Removing wallet \"{}\" will delete its imported private key and local wallet data from this device. Make sure you have a backup of the private key before continuing. Continue?",
                alias
            );
            (
                PendingWalletRemoval::SingleKey {
                    key_hash: wallet.key_hash,
                    address: wallet.address.to_string(),
                    alias,
                },
                message,
            )
        } else {
            return;
        };

        self.pending_wallet_removal = Some(pending);
        self.remove_wallet_dialog = Some(
            ConfirmationDialog::new("Remove Wallet", message)
                .confirm_text(Some("Remove"))
                .cancel_text(Some("Cancel"))
                .danger_mode(true),
        );
    }

    fn handle_single_key_wallet_removal(
        &mut self,
        key_hash: SingleKeyHash,
        address: String,
        alias: String,
    ) {
        let outcome = match self.app_context.wallet_backend() {
            Ok(backend) => backend.single_key().forget(&address).err(),
            Err(error) => Some(error),
        };
        if let Some(error) = outcome {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Failed to remove the imported key. Try again.",
                MessageType::Error,
            )
            .with_details(error);
            return;
        }

        if let Ok(mut wallets) = self.app_context.single_key_wallets.write() {
            wallets.remove(&key_hash);
        }
        self.selected_single_key_wallet = None;
        self.persist_selected_single_key_hash(None);
        MessageBanner::set_global(
            self.app_context.egui_ctx(),
            format!("Removed wallet \"{}\" successfully.", alias),
            MessageType::Success,
        );
    }

    fn handle_wallet_removal(&mut self, seed_hash: WalletSeedHash, alias: String) {
        match self.app_context.remove_wallet(&seed_hash) {
            Ok(()) => {
                let next_wallet = self
                    .app_context
                    .wallets
                    .read()
                    .ok()
                    .and_then(|wallets| wallets.values().next().cloned());

                self.set_selected_hd_wallet(next_wallet);

                self.show_rename_dialog = false;
                self.rename_input.clear();
                self.wallet_unlock_popup.close();
                self.refreshing = false;

                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Removed wallet \"{}\" successfully", alias),
                    MessageType::Success,
                );
            }
            Err(err) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Failed to remove wallet: {}", err),
                    MessageType::Error,
                );
            }
        }
    }

    fn render_no_wallets_view(&self, ui: &mut Ui) {
        // While the cold-start data migration is mid-flight we hold a
        // neutral placeholder — the global migration banner already
        // tells the user what's happening, so the empty-state must not
        // race ahead with Create/Import CTAs that the rehydrated wallet
        // list might invalidate seconds later.
        let migration_state = (*self.app_context.migration_status().state()).clone();
        let migration_running = migration_state.is_in_progress();

        // Optionally put everything in a framed "card"-like container
        Frame::group(ui.style())
            .fill(ui.visuals().extreme_bg_color) // background color
            .corner_radius(5.0) // rounded corners
            .outer_margin(Margin::same(20)) // space around the frame
            .shadow(ui.visuals().window_shadow) // drop shadow
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.add_space(5.0);

                    if migration_running {
                        // Diziet J-4 + D-1: defer the empty-state CTAs
                        // while the migration is still working — the
                        // banner above is the source of truth.
                        ui.label(
                            RichText::new("Preparing your wallet…")
                                .strong()
                                .size(22.0)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            "We are finishing the storage update. Your wallet will appear here in a moment.",
                        );
                        ui.add_space(5.0);
                        return;
                    }

                    // Fresh-install empty state (Diziet J-4). Single
                    // complete sentences per i18n-extraction rules; the
                    // primary Create CTA is the first focusable element
                    // so it lands as Tab-stop #1 from the wallet root
                    // (TC-A11Y-007).
                    ui.label(
                        RichText::new("No wallets yet")
                            .strong()
                            .size(25.0)
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.label(
                        "Create a wallet or import an existing one to get started.",
                    );

                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            "Use the Create Wallet or Import Wallet buttons in the top-right corner.",
                        )
                        .color(DashColors::text_secondary(dark_mode)),
                    );

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Footnote — keeps Dash Core wallet wiring guidance
                    // discoverable for the Power-User persona.
                    ui.label(
                        "Looking for an older wallet? Make sure Dash Core is running and visit the Network tab on the left.",
                    );

                    ui.add_space(5.0);
                });
            });
    }

    /// Format a Unix timestamp (seconds since epoch) as a relative "time ago" string.
    fn format_unix_time_ago(unix_ts: u64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let elapsed_secs = now.saturating_sub(unix_ts);
        Self::format_duration_ago(std::time::Duration::from_secs(elapsed_secs))
    }

    fn format_duration_ago(duration: std::time::Duration) -> String {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{}s ago", secs)
        } else if secs < 3600 {
            format!("{}m ago", secs / 60)
        } else if secs < 86400 {
            format!("{}h ago", secs / 3600)
        } else {
            format!("{}d ago", secs / 86400)
        }
    }

    fn transaction_direction_label(tx: &WalletTransaction) -> &'static str {
        if tx.is_incoming() {
            "Received"
        } else if tx.is_outgoing() {
            "Sent"
        } else {
            "Internal"
        }
    }

    fn transaction_amount_display(tx: &WalletTransaction, dark_mode: bool) -> (String, Color32) {
        let amount = format_duffs_as_dash(tx.amount_abs());
        if tx.is_incoming() {
            (format!("+{}", amount), DashColors::SUCCESS)
        } else if tx.is_outgoing() {
            (format!("-{}", amount), DashColors::ERROR)
        } else {
            (amount, DashColors::text_primary(dark_mode))
        }
    }

    fn format_transaction_status(tx: &WalletTransaction) -> String {
        match tx.status {
            TransactionStatus::Unconfirmed => "Pending".to_string(),
            TransactionStatus::InstantSendLocked => "⚡ InstantSend".to_string(),
            TransactionStatus::Confirmed => tx
                .height
                .map(|h| format!("Confirmed @{}", h))
                .unwrap_or_else(|| "Confirmed".to_string()),
            TransactionStatus::ChainLocked => tx
                .height
                .map(|h| format!("🔒 ChainLocked @{}", h))
                .unwrap_or_else(|| "🔒 ChainLocked".to_string()),
        }
    }

    fn format_transaction_timestamp(ts: u64) -> String {
        // `0` means "no block yet" (mempool or InstantSend-locked-but-
        // unconfirmed) — rendering it through `DateTime` would show the Unix
        // epoch (1970-01-01), which reads as a data bug rather than "still
        // pending".
        if ts == 0 {
            return "Pending…".to_string();
        }
        DateTime::<Utc>::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Platform-address balance in duffs, read from the coordinator-push snapshot.
    ///
    /// The snapshot is written by `on_platform_address_sync_completed` in `EventBridge`
    /// and contains only OWNED addresses (no orphan inflation). Safe to call from the
    /// egui frame loop — synchronous, no blocking I/O (Nagatha ruling).
    fn platform_balance_duffs(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.app_context.platform_balance_duffs(seed_hash)
    }

    fn shielded_balance_duffs(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.app_context.shielded_balance_duffs(seed_hash)
    }

    /// Core (chain) balance in duffs, read from the display-only
    /// `WalletBackend` snapshot (P4a). Pre-first-sync ⇒ `0`, which the
    /// surrounding UI renders as the "syncing" state.
    fn core_balance_duffs(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.app_context
            .wallet_backend()
            .map(|wb| wb.wallet_balance(seed_hash).total)
            .unwrap_or(0)
    }

    /// Seed hash of the currently selected wallet, if any. Frame-safe read.
    fn selected_wallet_seed_hash(&self) -> Option<WalletSeedHash> {
        self.selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok().map(|g| g.seed_hash()))
    }

    /// UTXO-derived per-address balances from the snapshot.
    fn snapshot_address_balances(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::collections::BTreeMap<Address, u64> {
        self.app_context
            .wallet_backend()
            .map(|wb| wb.address_balances(seed_hash))
            .unwrap_or_default()
    }

    /// Authoritative per-address derivation paths from the snapshot. Lets the
    /// account-summary view categorize funded addresses `watched_addresses`
    /// has not indexed yet, so none are dropped from the per-category totals.
    fn snapshot_address_paths(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::collections::BTreeMap<Address, dash_sdk::dpp::key_wallet::bip32::DerivationPath> {
        self.app_context
            .wallet_backend()
            .map(|wb| wb.address_paths(seed_hash))
            .unwrap_or_default()
    }

    /// Full transaction history from the snapshot.
    fn snapshot_transactions(&self, seed_hash: &WalletSeedHash) -> Vec<WalletTransaction> {
        self.app_context
            .wallet_backend()
            .map(|wb| wb.transaction_history(seed_hash))
            .unwrap_or_default()
    }

    fn render_action_buttons(&mut self, ui: &mut Ui, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        ui.add_space(10.0);
        let dark_mode = ui.style().visuals.dark_mode;
        ui.horizontal(|ui| {
            if ui
                .button(
                    RichText::new("Send")
                        .color(DashColors::text_primary(dark_mode))
                        .strong(),
                )
                .clicked()
            {
                if let Some(wallet) = &self.selected_wallet {
                    action = AppAction::AddScreen(
                        crate::ui::ScreenType::WalletSendScreen(
                            wallet.clone(),
                            crate::ui::wallets::send_screen::SendFlow::General,
                        )
                        .create_screen(&self.app_context),
                    );
                } else if self.selected_single_key_wallet.is_some() {
                    // Single-key send cannot work in this version, so state the
                    // limitation here rather than routing the user into a send
                    // screen that could only refuse the payment.
                    MessageBanner::set_global(
                        ui.ctx(),
                        SINGLE_KEY_SEND_UNAVAILABLE,
                        MessageType::Warning,
                    );
                } else {
                    MessageBanner::set_global(
                        ui.ctx(),
                        "Select a wallet first",
                        MessageType::Error,
                    );
                }
            }

            if ui
                .button(RichText::new("Receive").color(DashColors::text_primary(dark_mode)))
                .clicked()
            {
                action |= self.open_receive_dialog(ctx);
            }

            if self.refreshing {
                ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE));
            }

            // Expert-only buttons: right-aligned, filling all remaining space
            if self.app_context.user_role().at_least(UserRole::Power) {
                let remaining = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::vec2(remaining, ui.min_size().y),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if matches!(
                            self.app_context.network,
                            dash_sdk::dpp::dashcore::Network::Testnet
                        ) && ui
                            .button(
                                RichText::new("Get Test Dash")
                                    .color(DashColors::text_primary(dark_mode))
                                    .strong(),
                            )
                            .clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(
                                "https://faucet.testnet.networks.dash.org/",
                            ));
                        }

                        if matches!(
                            self.app_context.network,
                            dash_sdk::dpp::dashcore::Network::Regtest
                                | dash_sdk::dpp::dashcore::Network::Devnet
                        ) && ui
                            .button(
                                RichText::new("Mine")
                                    .color(DashColors::text_primary(dark_mode))
                                    .strong(),
                            )
                            .clicked()
                        {
                            self.open_mine_dialog();
                        }

                        if ui
                            .button(
                                RichText::new(format!(
                                    "Refresh mode: {}",
                                    self.refresh_mode.label()
                                ))
                                .color(DashColors::text_primary(dark_mode))
                                .strong(),
                            )
                            .clicked()
                        {
                            self.refresh_mode = self.refresh_mode.next();
                        }
                    },
                );
            }
        });

        action
    }

    /// Build the list of visible account tabs based on current summaries and dev mode.
    fn build_account_tabs(&self, summaries: &[AccountSummary]) -> Vec<AccountTab> {
        plan_account_tabs(
            summaries,
            self.app_context.user_role().at_least(UserRole::Power),
            FeatureGate::Shielded.is_available(&self.app_context),
            // Every HD wallet unconditionally bootstraps a platform-payment
            // receive address at load (`Wallet::bootstrap_known_addresses`), so
            // an HD wallet always gets a Platform tab. `render_account_tabs`
            // only runs for the selected HD wallet, so a seed hash is present.
            self.selected_wallet_seed_hash().is_some(),
        )
    }

    /// Sum of the per-account balances that are hidden from the default-mode
    /// primary tabs (everything not BIP-44 or Platform). Drives the
    /// header-reconciling System/Other tab and its balance label.
    fn hidden_category_balance(summaries: &[AccountSummary]) -> u64 {
        summaries
            .iter()
            .filter(|s| s.category.is_system_category())
            .map(|s| s.confirmed_balance)
            .sum()
    }

    /// Collect the system account categories to display inside the System tab.
    /// Returns `(category, index, address_count, balance_duffs)` tuples in a
    /// fixed display order (identity categories first, then provider, then legacy).
    fn system_tab_sections(
        &self,
        summaries: &[AccountSummary],
    ) -> Vec<(AccountCategory, Option<u32>, usize, u64)> {
        let all_system_categories = [
            AccountCategory::IdentityRegistration,
            AccountCategory::IdentitySystem,
            AccountCategory::IdentityTopup,
            AccountCategory::IdentityInvitation,
            AccountCategory::CoinJoin,
            AccountCategory::ProviderOwner,
            AccountCategory::ProviderVoting,
            AccountCategory::ProviderOperator,
            AccountCategory::ProviderPlatform,
            AccountCategory::Bip32,
        ];

        // Precompute per-category address counts in a single pass over
        // watched_addresses to avoid O(num_categories * num_addresses)
        // per frame.
        let address_counts = self.precompute_address_counts();

        let mut sections = Vec::new();
        for cat in &all_system_categories {
            let matching: Vec<_> = summaries.iter().filter(|s| &s.category == cat).collect();
            let address_count = address_counts.get(cat).copied().unwrap_or(0);
            let balance: u64 = matching.iter().map(|s| s.confirmed_balance).sum();
            let idx = matching.first().and_then(|s| s.index);
            sections.push((cat.clone(), idx, address_count, balance));
        }

        // Also include any Other(...) categories from summaries
        for summary in summaries {
            if matches!(summary.category, AccountCategory::Other(_))
                && !sections.iter().any(|(c, _, _, _)| *c == summary.category)
            {
                let address_count = address_counts.get(&summary.category).copied().unwrap_or(0);
                sections.push((
                    summary.category.clone(),
                    summary.index,
                    address_count,
                    summary.confirmed_balance,
                ));
            }
        }

        sections
    }

    /// Build a per-category address count map in a single pass over
    /// `watched_addresses`. Used by `system_tab_sections` to avoid
    /// O(num_categories * num_addresses) per frame.
    fn precompute_address_counts(&self) -> std::collections::HashMap<AccountCategory, usize> {
        let mut counts = std::collections::HashMap::new();
        let Some(wallet_arc) = self.selected_wallet.as_ref() else {
            return counts;
        };
        let Ok(wallet) = wallet_arc.read() else {
            return counts;
        };
        let network = self.app_context.network;
        for (path, info) in &wallet.watched_addresses {
            let (cat, _) = crate::ui::state::account_summary::categorize_account_path(
                path,
                network,
                info.path_reference,
            );
            *counts.entry(cat).or_insert(0) += 1;
        }
        counts
    }

    /// Format a duffs balance for tab labels: max 4 decimal places, trimmed.
    fn format_tab_balance(duffs: u64) -> String {
        let dash = duffs as f64 / 100_000_000.0;
        // Format with 4 decimal places, then trim trailing zeros
        let formatted = format!("{:.4}", dash);
        let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
        format!("{} DASH", trimmed)
    }

    /// Render the Accounts & Addresses tab bar and content.
    fn render_account_tabs(&mut self, ui: &mut Ui, summaries: &[AccountSummary]) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

        ui.add_space(14.0);

        let tabs = self.build_account_tabs(summaries);

        // Ensure the selected tab is still valid
        if !tabs.contains(&self.selected_account_tab)
            && let Some(first) = tabs.first()
        {
            self.selected_account_tab = first.clone();
        }

        // Tab bar
        ui.horizontal_wrapped(|ui| {
            for tab in &tabs {
                let (base_label, balance_duffs) = match tab {
                    AccountTab::Category(cat, idx) => {
                        let balance = if matches!(cat, AccountCategory::PlatformPayment) {
                            // Single authoritative Platform total — the same
                            // figure the wallet header shows, so the two cannot
                            // diverge.
                            self.selected_wallet_seed_hash()
                                .map(|seed_hash| self.platform_balance_duffs(&seed_hash))
                                .unwrap_or(0)
                        } else {
                            summaries
                                .iter()
                                .filter(|s| s.category == *cat && s.index == *idx)
                                .map(|s| s.confirmed_balance)
                                .sum::<u64>()
                        };
                        (cat.tab_label(*idx).to_string(), balance)
                    }
                    AccountTab::Shielded => {
                        let balance = self
                            .selected_wallet
                            .as_ref()
                            .and_then(|w| w.read().ok())
                            .map(|g| self.shielded_balance_duffs(&g.seed_hash()))
                            .unwrap_or(0);
                        ("Shielded".to_string(), balance)
                    }
                    AccountTab::System => {
                        let balance = Self::hidden_category_balance(summaries);
                        // "System" in developer mode (full breakdown); "Other"
                        // in default mode, where the tab only appears to
                        // reconcile funds outside the primary Core/Platform
                        // tabs.
                        let label = if self.app_context.user_role().at_least(UserRole::Power) {
                            "System"
                        } else {
                            "Other"
                        };
                        (label.to_string(), balance)
                    }
                };
                let label = if balance_duffs == 0 {
                    format!("{} (empty)", base_label)
                } else {
                    format!(
                        "{} ({})",
                        base_label,
                        Self::format_tab_balance(balance_duffs)
                    )
                };
                let is_selected = &self.selected_account_tab == tab;
                let text = if is_selected {
                    RichText::new(&label)
                        .strong()
                        .color(DashColors::text_primary(dark_mode))
                } else {
                    RichText::new(&label).color(DashColors::text_secondary(dark_mode))
                };
                ui.add_space(4.0);
                if ui.selectable_label(is_selected, text).clicked() {
                    self.selected_account_tab = tab.clone();
                    // Sync the selected_account for address_table filtering
                    if let AccountTab::Category(cat, idx) = tab {
                        self.selected_account = Some((cat.clone(), *idx));
                    }
                }
            }
        });
        ui.separator();
        ui.add_space(4.0);

        // Tab content — extract category data to avoid cloning the whole enum
        let tab_category = match &self.selected_account_tab {
            AccountTab::Category(cat, idx) => Some((cat.clone(), *idx)),
            _ => None,
        };
        match (&self.selected_account_tab, tab_category) {
            (AccountTab::Shielded, _) => {
                let seed_hash = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
                if let Some(seed_hash) = seed_hash {
                    let shielded_view = self
                        .shielded_tab_view
                        .get_or_insert_with(|| ShieldedTabView::new(&self.app_context, seed_hash));
                    shielded_view.update_seed_hash(seed_hash);
                    shielded_view.update_app_context(&self.app_context);
                    action |= shielded_view.ui(ui);
                }
            }
            (AccountTab::System, _) => {
                action |= self.render_system_tab_content(ui, summaries);
            }
            (AccountTab::Category(..), Some((cat, idx))) => {
                // Show empty state if no summaries match this category. Dash
                // Core and Platform always render their address tables — Platform
                // is driven by the coordinator snapshot, not the Core summaries.
                if !summaries
                    .iter()
                    .any(|s| s.category == cat && s.index == idx)
                    && !matches!(
                        cat,
                        AccountCategory::Bip44 | AccountCategory::PlatformPayment
                    )
                {
                    ui.label(
                        RichText::new("No account activity yet.")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    return action;
                }

                // Show description for the selected account category
                if let Some(description) = cat.description() {
                    ui.label(
                        RichText::new(description)
                            .color(DashColors::text_secondary(dark_mode))
                            .italics()
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                }

                self.selected_account = Some((cat.clone(), idx));

                // Addresses (collapsible)
                let addresses_heading = format!("Addresses ({})", cat.label(idx));
                let addr_header = egui::CollapsingHeader::new(
                    RichText::new(addresses_heading)
                        .size(16.0)
                        .color(DashColors::text_primary(dark_mode)),
                )
                .id_salt(format!("addresses_{}_{:?}", cat.tab_label(idx), idx))
                .default_open(true);
                addr_header.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(
                                &mut self.show_zero_balance_addresses,
                                "Show zero-balance addresses",
                            );
                        });
                    });
                    ui.add_space(4.0);
                    action |= self.render_address_table(ui, (cat.clone(), idx));
                    action |= self.render_bottom_options(ui);
                });

                // Dash Core tab: transaction history + asset locks
                if cat == AccountCategory::Bip44 && idx == Some(0) {
                    // Transaction History (collapsible)
                    ui.add_space(10.0);
                    let tx_header = egui::CollapsingHeader::new(
                        RichText::new("Transaction History")
                            .size(16.0)
                            .color(DashColors::text_primary(dark_mode)),
                    )
                    .id_salt("transaction_history")
                    .default_open(false);
                    tx_header.show(ui, |ui| {
                        self.render_transactions_section(ui);
                    });

                    // Asset Locks (collapsible)
                    ui.add_space(10.0);
                    let locks_header = egui::CollapsingHeader::new(
                        RichText::new("Asset Locks")
                            .size(16.0)
                            .color(DashColors::text_primary(dark_mode)),
                    )
                    .id_salt("asset_locks")
                    .default_open(true);
                    locks_header.show(ui, |ui| {
                        action |= self.render_wallet_asset_locks(ui);
                    });
                }
            }
            _ => {}
        }

        action
    }

    /// Render the System tab content: each system account category as a
    /// collapsible section, collapsed by default.
    ///
    /// In developer mode every system category is listed (diagnostic view). In
    /// default mode this tab only appears to reconcile funds held outside the
    /// primary Core/Platform tabs, so it shows a short explanation and lists
    /// only the categories that actually hold a balance.
    fn render_system_tab_content(
        &mut self,
        ui: &mut Ui,
        summaries: &[AccountSummary],
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        let developer_mode = self.app_context.user_role().at_least(UserRole::Power);
        let sections = self.system_tab_sections(summaries);

        if !developer_mode {
            ui.label(
                RichText::new(
                    "These funds are held on addresses outside your main and Platform accounts. \
                     They are included in your total balance.",
                )
                .color(DashColors::text_secondary(dark_mode))
                .italics()
                .size(12.0),
            );
            ui.add_space(6.0);
        }

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(
                    &mut self.show_zero_balance_addresses,
                    "Show zero-balance addresses",
                );
            });
        });
        ui.add_space(4.0);

        for (cat, idx, addr_count, balance) in &sections {
            // Default mode only reconciles funded categories — empty system
            // sections are diagnostic noise for a non-developer user.
            if !developer_mode && *balance == 0 {
                continue;
            }
            let balance_text = if *balance == 0 {
                "empty".to_string()
            } else {
                Self::format_tab_balance(*balance)
            };
            let heading = format!(
                "{} ({} addresses, {})",
                cat.label(*idx),
                addr_count,
                balance_text
            );
            let header = egui::CollapsingHeader::new(
                RichText::new(heading)
                    .size(14.0)
                    .color(DashColors::text_primary(dark_mode)),
            )
            .id_salt(format!("system_section_{:?}_{:?}", cat, idx))
            .default_open(false);
            header.show(ui, |ui| {
                if let Some(description) = cat.description() {
                    ui.label(
                        RichText::new(description)
                            .color(DashColors::text_secondary(dark_mode))
                            .italics()
                            .size(12.0),
                    );
                    ui.add_space(4.0);
                }

                self.selected_account = Some((cat.clone(), *idx));
                action |= self.render_address_table(ui, (cat.clone(), *idx));
            });
            ui.add_space(2.0);
        }

        action
    }

    fn render_transactions_section(&mut self, ui: &mut Ui) {
        ui.add_space(10.0);
        // TODO: Synchronize transactions display with selected account type
        // (main account -> Core transactions, platform account -> platform state transitions, etc.)
        ui.heading("Dash Core Transactions");
        let Some(wallet_arc) = self.selected_wallet.as_ref() else {
            ui.label("Select a wallet to view its transaction history.");
            return;
        };

        let selected_seed_hash = {
            let wallet_guard = wallet_arc.read_recover();
            wallet_guard.seed_hash()
        };

        // Transaction history comes from the display-only `WalletBackend`
        // snapshot. Pre-first-sync there is no snapshot yet → render the
        // "syncing" state rather than a misleading "no transactions" message.
        let backend_ready = self
            .app_context
            .wallet_backend()
            .map(|wb| wb.has_snapshot(&selected_seed_hash))
            .unwrap_or(false);
        let transactions = self.snapshot_transactions(&selected_seed_hash);

        let history_status = self
            .app_context
            .wallet_backend()
            .map(|backend| backend.transaction_history_status(&selected_seed_hash))
            .unwrap_or_default();
        let notice = (selected_seed_hash, history_status.clone());
        if self.transaction_history_notice.as_ref() != Some(&notice) {
            self.transaction_history_notice = Some(notice);
            match history_status.error() {
                Some(error) => {
                    self.transaction_history_banner
                        .set_message(error, MessageType::Warning)
                        .set_details(error)
                        .disable_auto_dismiss();
                }
                None => self.transaction_history_banner.clear(),
            }
        }
        self.transaction_history_banner.show(ui);

        if !backend_ready {
            ui.label("Syncing transactions from the network…");
            return;
        }

        if transactions.is_empty() {
            ui.label("No transactions found for this wallet yet.");
            return;
        }

        // Filter to transactions involving this wallet's addresses (`is_ours`
        // is always true on the per-wallet snapshot, but the filter is kept
        // for parity and future cross-wallet views). Invalidate the index
        // cache when the source length changes or cached indices go stale.
        let tx_len = transactions.len();
        if self.cached_tx_source_len != Some(tx_len)
            || self
                .cached_tx_indices
                .as_ref()
                .is_some_and(|cached| cached.iter().any(|&i| i >= tx_len))
        {
            self.cached_tx_indices = None;
            self.cached_tx_source_len = Some(tx_len);
        }
        let relevant_indices = self
            .cached_tx_indices
            .get_or_insert_with(|| (0..tx_len).filter(|&i| transactions[i].is_ours).collect());

        if relevant_indices.is_empty() {
            ui.label("No transactions found for this wallet yet.");
            return;
        }

        let dark_mode = ui.style().visuals.dark_mode;
        let show_fee = self.app_context.user_role().at_least(UserRole::Power);
        let mut order: Vec<usize> = relevant_indices.clone();
        order.sort_by(|&a, &b| {
            transactions[b]
                .timestamp
                .cmp(&transactions[a].timestamp)
                .then_with(|| transactions[b].txid.cmp(&transactions[a].txid))
        });

        let row_height = 26.0;
        let mut builder = TableBuilder::new(ui)
            .id_salt("transactions_table")
            .striped(true)
            .column(Column::initial(150.0)) // Date
            .column(Column::initial(80.0)) // Type
            .column(Column::initial(120.0)); // Amount

        if show_fee {
            builder = builder.column(Column::initial(100.0)); // Fee
        }

        builder
            .column(Column::initial(150.0)) // Status
            .column(Column::remainder()) // TxID
            .header(row_height, |mut header| {
                header.col(|ui| {
                    ui.label(
                        RichText::new("Date")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                header.col(|ui| {
                    ui.label(
                        RichText::new("Type")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                header.col(|ui| {
                    ui.label(
                        RichText::new("Amount")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                if show_fee {
                    header.col(|ui| {
                        ui.label(
                            RichText::new("Fee")
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    });
                }
                header.col(|ui| {
                    ui.label(
                        RichText::new("Status")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
                header.col(|ui| {
                    ui.label(
                        RichText::new("TxID")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                });
            })
            .body(|mut body| {
                for idx in order {
                    let tx = &transactions[idx];
                    body.row(row_height, |mut row| {
                        row.col(|ui| {
                            ui.label(Self::format_transaction_timestamp(tx.timestamp));
                        });
                        row.col(|ui| {
                            ui.label(Self::transaction_direction_label(tx));
                        });
                        row.col(|ui| {
                            let (amount_text, amount_color) =
                                Self::transaction_amount_display(tx, dark_mode);
                            ui.label(RichText::new(amount_text).color(amount_color).strong());
                        });
                        if show_fee {
                            row.col(|ui| {
                                let fee_text = tx
                                    .fee
                                    .map(format_duffs_as_dash)
                                    .unwrap_or_else(|| "-".to_string());
                                ui.label(fee_text);
                            });
                        }
                        row.col(|ui| {
                            ui.label(Self::format_transaction_status(tx));
                        });
                        row.col(|ui| {
                            let full_txid = tx.txid.to_string();
                            ui.horizontal(|ui| {
                                let response = ui.label(RichText::new(&full_txid).monospace());
                                response.info_tooltip(&full_txid);
                                if ui
                                    .small_button("Copy")
                                    .clickable_tooltip("Copy transaction ID")
                                    .clicked()
                                {
                                    let _ = copy_text_to_clipboard(&full_txid);
                                }
                                // Show "View" button for networks with a public explorer
                                let explorer_base = match self.app_context.network {
                                    dash_sdk::dpp::dashcore::Network::Mainnet => {
                                        Some("https://insight.dash.org/insight/tx/")
                                    }
                                    dash_sdk::dpp::dashcore::Network::Testnet => Some(
                                        "https://insight.testnet.networks.dash.org/insight/tx/",
                                    ),
                                    _ => None,
                                };
                                if let Some(base_url) = explorer_base
                                    && ui
                                        .small_button("View")
                                        .clickable_tooltip("View on block explorer")
                                        .clicked()
                                {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                                        "{}{}",
                                        base_url, full_txid
                                    )));
                                }
                            });
                        });
                    });
                }
            });
    }

    /// Render a compact sync status panel showing Core, Platform, and Shielded sync progress.
    fn render_sync_status(&self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        let secondary = DashColors::text_secondary(dark_mode);
        let syncing_color = DashColors::DASH_BLUE;
        let sz = 12.0;

        ui.collapsing(
            RichText::new("Sync Status").size(sz).color(secondary),
            |ui| {
                // -- Core sync status --
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").size(sz).color(secondary));
                    ui.label(
                        RichText::new("Core:")
                            .size(sz)
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    {
                        // Chain sync is owned by upstream platform-wallet; the
                        // EventBridge pushes live status + per-phase progress
                        // into ConnectionStatus, the single source of truth.
                        let snapshot = self.app_context.connection_status().spv_status_snapshot();
                        match snapshot.status {
                            SpvStatus::Idle | SpvStatus::Stopped => {
                                ui.label(RichText::new("Disconnected").size(sz).color(secondary));
                            }
                            SpvStatus::Starting => {
                                ui.add(egui::Spinner::new().size(sz).color(syncing_color));
                                ui.label(
                                    RichText::new("Connecting...").size(sz).color(syncing_color),
                                );
                            }
                            SpvStatus::Syncing => {
                                ui.add(egui::Spinner::new().size(sz).color(syncing_color));
                                let phase_text = snapshot
                                    .sync_progress
                                    .as_ref()
                                    .map(spv_phase_summary)
                                    .unwrap_or_else(|| "starting...".to_string());
                                ui.label(
                                    RichText::new(format!("Syncing — {phase_text}"))
                                        .size(sz)
                                        .color(syncing_color),
                                );
                            }
                            SpvStatus::Running => {
                                ui.colored_label(
                                    Color32::DARK_GREEN,
                                    RichText::new(format!(
                                        "Synced — {} peers",
                                        snapshot.connected_peers
                                    ))
                                    .size(sz),
                                );
                            }
                            SpvStatus::Stopping => {
                                ui.add(egui::Spinner::new().size(sz).color(syncing_color));
                                ui.label(
                                    RichText::new("Disconnecting...")
                                        .size(sz)
                                        .color(syncing_color),
                                );
                            }
                            SpvStatus::Error => {
                                ui.colored_label(
                                    DashColors::ERROR,
                                    RichText::new("Error").size(sz),
                                );
                            }
                        }
                    }
                });

                // -- Platform: Addresses --
                // Count and sync cursor both come from the coordinator-push
                // snapshot, read live each frame so the label stays truthful
                // without a manual refresh — and even while the wallet is locked.
                let (addr_count, platform_sync_info) = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok())
                    .map(|w| {
                        (
                            w.platform_address_info.len(),
                            self.app_context.platform_sync_info(&w.seed_hash()),
                        )
                    })
                    .unwrap_or((0, None));
                let addr_color = if self.refreshing {
                    syncing_color
                } else {
                    secondary
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").size(sz).color(secondary));
                    if self.refreshing {
                        ui.add(egui::Spinner::new().size(sz).color(syncing_color));
                    }
                    let addr_text = if let Some((last_sync_ts, sync_height)) = platform_sync_info {
                        let ago = Self::format_unix_time_ago(last_sync_ts);
                        format!("Addresses: {addr_count} synced (blk {sync_height}, {ago})")
                    } else {
                        "Addresses: never synced".to_string()
                    };
                    ui.label(RichText::new(addr_text).size(sz).color(addr_color));
                });

                // -- Shielded balance --
                // The upstream coordinator's 60-second sync loop keeps the push
                // snapshot current.
                // TODO: restore the detailed per-note / nullifier sync display
                // once the coordinator exposes a read path for it.
                let shielded_seed_hash = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").size(sz).color(secondary));
                    let shielded_text = match shielded_seed_hash {
                        Some(hash) => format!(
                            "Shielded: {}",
                            format_duffs_as_dash(self.app_context.shielded_balance_duffs(&hash))
                        ),
                        None => "Shielded: unavailable".to_string(),
                    };
                    ui.label(RichText::new(shielded_text).size(sz).color(secondary));
                });
            },
        );
    }

    /// Render the total balance label only (used in the left column of the header).
    fn render_balance_total(&self, ui: &mut Ui, wallet: &Wallet) {
        let dark_mode = ui.style().visuals.dark_mode;
        let seed_hash = wallet.seed_hash();
        let core_balance = self.core_balance_duffs(&seed_hash);
        let platform_balance = self.platform_balance_duffs(&seed_hash);
        let shielded_balance = self.shielded_balance_duffs(&seed_hash);
        let total = core_balance + platform_balance + shielded_balance;

        ui.label(
            RichText::new(format!("Balance: {}", format_duffs_as_dash(total)))
                .color(DashColors::text_primary(dark_mode))
                .size(20.0)
                .strong(),
        );
    }

    /// Render the collapsible breakdown detail (used in the right column of the header).
    fn render_balance_breakdown_detail(&mut self, ui: &mut Ui, wallet: &Wallet) {
        let dark_mode = ui.style().visuals.dark_mode;
        let seed_hash = wallet.seed_hash();
        let core_balance = self.core_balance_duffs(&seed_hash);
        let platform_balance = self.platform_balance_duffs(&seed_hash);
        let shielded_balance = self.shielded_balance_duffs(&seed_hash);

        let header = egui::CollapsingHeader::new(
            RichText::new("Balance breakdown")
                .size(13.0)
                .color(DashColors::text_secondary(dark_mode)),
        )
        .id_salt("balance_breakdown")
        .default_open(self.app_context.user_role().at_least(UserRole::Power));

        header.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Core: {}", format_duffs_as_dash(core_balance)));
                ui.label(" | ");
                ui.label(format!(
                    "Platform: {}",
                    format_duffs_as_dash(platform_balance)
                ));
                ui.label(" | ");
                ui.label(format!(
                    "Shielded: {}",
                    format_duffs_as_dash(shielded_balance)
                ));
            });
        });
    }

    fn render_wallet_detail_panel(&mut self, ui: &mut Ui, ctx: &Context) -> AppAction {
        let Some(wallet_arc) = self.selected_wallet.clone() else {
            self.render_no_wallets_view(ui);
            return AppAction::None;
        };

        let (alias, _seed_hash, _wallet_is_main) = {
            let wallet = wallet_arc.read_recover();
            (
                wallet
                    .alias
                    .clone()
                    .unwrap_or_else(|| "Unnamed Wallet".to_string()),
                wallet.seed_hash(),
                wallet.is_main,
            )
        };
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

        let detail_width = ui.available_width();
        ui.horizontal(|row| {
            row.vertical(|col| {
                col.set_width(detail_width);
                Frame::group(col.style())
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(18, 16))
                    .show(col, |ui| {
                        // --- Two-column header ---
                        let available = ui.available_width();
                        let left_width = available * 0.55;
                        let right_width = available - left_width;

                        ui.horizontal(|ui| {
                            // LEFT COLUMN: name, total balance
                            ui.vertical(|ui| {
                                ui.set_width(left_width);

                                // Wallet name + [DEV] badge
                                ui.horizontal(|ui| {
                                    ui.heading(
                                        RichText::new(alias.clone())
                                            .color(DashColors::text_primary(dark_mode))
                                            .size(25.0),
                                    );
                                    if self.app_context.user_role().at_least(UserRole::Power) {
                                        ui.label(
                                            RichText::new("[DEV]")
                                                .color(DashColors::text_secondary(dark_mode))
                                                .size(12.0),
                                        );
                                    }
                                });

                                // Total balance line
                                {
                                    let wallet = wallet_arc.read_recover();
                                    self.render_balance_total(ui, &wallet);
                                }
                            });

                            // RIGHT COLUMN: balance breakdown + sync status, right-aligned
                            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                                ui.set_width(right_width);

                                // Collapsible balance breakdown
                                {
                                    let wallet = wallet_arc.read_recover();
                                    self.render_balance_breakdown_detail(ui, &wallet);
                                }

                                // Collapsible sync status
                                self.render_sync_status(ui);
                            });
                        });

                        // Action buttons span full width below the header
                        action |= self.render_action_buttons(ui, ctx);

                        // --- Accounts & Addresses (tabs, full-width below header) ---
                        ui.add_space(10.0);
                        ui.separator();

                        let summaries = {
                            let seed_hash = wallet_arc.read_recover().seed_hash();
                            let address_balances = self.snapshot_address_balances(&seed_hash);
                            let address_paths = self.snapshot_address_paths(&seed_hash);
                            collect_account_summaries(
                                self.app_context.network,
                                &address_balances,
                                &address_paths,
                            )
                        };
                        self.ensure_account_selection(&summaries);
                        action |= self.render_account_tabs(ui, &summaries);
                    });
            });
        });

        action
    }

    fn ensure_account_selection(&mut self, summaries: &[AccountSummary]) {
        if summaries.is_empty() {
            self.selected_account = None;
            return;
        }

        if let Some((cat, idx)) = &self.selected_account
            && summaries
                .iter()
                .any(|summary| &summary.category == cat && summary.index == *idx)
        {
            return;
        }

        if let Some(first) = summaries.first() {
            self.selected_account = Some((first.category.clone(), first.index));
        }
    }

    fn lock_selected_wallet(&mut self) {
        let Some(wallet_arc) = self.selected_wallet.clone() else {
            return;
        };

        let locked = {
            let mut wallet = match wallet_arc.write() {
                Ok(guard) => guard,
                Err(err) => {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        format!("Failed to lock wallet: {}", err),
                        MessageType::Error,
                    );
                    return;
                }
            };

            if !wallet.is_open() {
                return;
            }

            wallet.wallet_seed.close();
            true
        };

        if locked {
            self.app_context.handle_wallet_locked(&wallet_arc);
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Wallet locked",
                MessageType::Info,
            );
        }
    }

    /// Creates the appropriate refresh action based on the current refresh mode
    fn create_refresh_action(&self, wallet_arc: &Arc<RwLock<Wallet>>) -> AppAction {
        self.create_refresh_action_for_mode(wallet_arc, self.refresh_mode)
    }

    /// Creates the appropriate refresh action using the pending refresh mode
    fn create_pending_refresh_action(&self, wallet_arc: &Arc<RwLock<Wallet>>) -> AppAction {
        self.create_refresh_action_for_mode(wallet_arc, self.pending_refresh_mode)
    }

    fn create_refresh_action_for_mode(
        &self,
        wallet_arc: &Arc<RwLock<Wallet>>,
        mode: RefreshMode,
    ) -> AppAction {
        let seed_hash = wallet_arc
            .read()
            .ok()
            .map(|w| w.seed_hash())
            .unwrap_or_default();

        let core_task = match mode {
            RefreshMode::All => {
                // Core + Platform
                BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet_arc.clone(), true))
            }
            RefreshMode::PlatformOnly => {
                // Platform only
                BackendTask::WalletTask(
                    crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances {
                        seed_hash,
                    },
                )
            }
        };

        // Shielded balances are kept current by the upstream coordinator's
        // 60-second sync loop and the post-op snapshot refresh — DET no longer
        // dispatches a manual shielded sync from the refresh chain.
        AppAction::BackendTask(core_task)
    }

    /// Run the single import path
    /// ([`AppContext::import_single_key_wif`]) for `wif`, then select the
    /// resulting wallet so it is immediately visible in the picker.
    /// Returns the inserted in-memory wallet.
    fn register_imported_single_key(
        &mut self,
        wif: &str,
        passphrase: crate::wallet_backend::single_key::ImportPassphrase,
        alias: Option<String>,
    ) -> Result<Arc<RwLock<SingleKeyWallet>>, TaskError> {
        let (_imported, wallet_arc) = self
            .app_context
            .import_single_key_wif(wif, alias, passphrase)?;
        self.select_single_key_wallet(wallet_arc.clone());
        Ok(wallet_arc)
    }

    /// Test-only seam: run the single-key import end to end (vault write +
    /// in-memory sync) the way the confirmed dialog does, then return the
    /// in-memory wallet that became selectable, so an integration test can
    /// assert the imported key becomes visible in the same session. Not
    /// exposed for production callers.
    #[doc(hidden)]
    pub fn import_single_key_for_test(
        &mut self,
        wif: &str,
        alias: Option<String>,
    ) -> Result<Arc<RwLock<SingleKeyWallet>>, String> {
        self.register_imported_single_key(
            wif,
            crate::wallet_backend::single_key::ImportPassphrase::default(),
            alias,
        )
        .map_err(|e| e.to_string())
    }

    /// Render the J-6 "Import private key (advanced)" modal and route a
    /// confirmed WIF through [`crate::wallet_backend::SingleKeyView::import_wif`].
    /// Errors surface as a global banner with the typed `TaskError` details
    /// attached; success emits a confirmation toast naming the derived
    /// address so the user can match it against their records.
    fn render_import_single_key_dialog(&mut self, ctx: &Context) {
        let response = self.import_single_key_dialog.show(ctx);
        if response.cancelled {
            self.import_single_key_dialog.open = false;
            self.import_single_key_dialog.reset();
        }
        if let Some(request) = response.confirmed {
            let passphrase = crate::wallet_backend::single_key::ImportPassphrase {
                passphrase: request.passphrase.clone(),
                hint: request.passphrase_hint.clone(),
            };
            match self.register_imported_single_key(&request.wif, passphrase, request.alias.clone())
            {
                Ok(_) => {
                    MessageBanner::set_global(
                        ctx,
                        format!("Imported key added for {}.", request.address_preview),
                        MessageType::Success,
                    );
                    self.import_single_key_dialog.open = false;
                    self.import_single_key_dialog.reset();
                }
                Err(e) => {
                    MessageBanner::set_global(ctx, e.to_string(), MessageType::Error)
                        .with_details(&e);
                }
            }
        }
    }

    /// Recompute the set of protected single-key rows still awaiting
    /// restore. Cheap DB scan; runs lazily on first paint and after each
    /// successful restore. A scan failure leaves the list empty (the
    /// banner simply does not appear) and is logged — it must never block
    /// the wallets screen.
    fn refresh_pending_protected_restores(&mut self) {
        self.pending_restores_scanned = true;
        match crate::backend_task::migration::single_key_restore::list_pending_protected_restores(
            &self.app_context,
        ) {
            Ok(list) => self.pending_protected_restores = list,
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "Failed to scan for protected single-key restores; banner suppressed",
                );
                self.pending_protected_restores.clear();
            }
        }
    }

    /// Render the post-migration "protected imported keys need your
    /// password" banner. Offers to open the per-key restore dialog for the
    /// first outstanding key. No-op when nothing is pending.
    fn render_protected_restore_banner(&mut self, ui: &mut Ui) {
        if !self.pending_restores_scanned {
            self.refresh_pending_protected_restores();
        }
        let Some(next) = self.pending_protected_restores.first().cloned() else {
            return;
        };
        let count = self.pending_protected_restores.len();
        let dark_mode = ui.style().visuals.dark_mode;

        egui::Frame::group(ui.style())
            .fill(DashColors::input_background(dark_mode))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let message = if count == 1 {
                        "One imported key needs your old password to restore it.".to_string()
                    } else {
                        format!("{count} imported keys need your old password to restore them.")
                    };
                    ui.label(RichText::new(message).color(DashColors::text_primary(dark_mode)));
                    if ui.button("Restore now").clicked() {
                        self.restore_single_key_dialog.set_target(next);
                    }
                });
            });
        ui.add_space(8.0);
    }

    /// Render the per-key restore dialog and route a confirmed request
    /// through the restore bridge. On success the key becomes visible and
    /// the banner shrinks; on failure a generic, actionable banner appears
    /// (no oracle distinguishing a wrong password from a corrupt blob).
    fn render_restore_single_key_dialog(&mut self, ctx: &Context) {
        let response = self.restore_single_key_dialog.show(ctx);
        if response.cancelled {
            self.restore_single_key_dialog.open = false;
            self.restore_single_key_dialog.reset();
        }
        if let Some(request) = response.confirmed {
            let new_passphrase = crate::wallet_backend::single_key::ImportPassphrase {
                passphrase: request.new_passphrase.clone(),
                hint: request.new_hint.clone(),
            };
            match crate::backend_task::migration::single_key_restore::restore_protected_single_key(
                &self.app_context,
                &request.address,
                &request.legacy_password,
                new_passphrase,
            ) {
                Ok(address) => {
                    MessageBanner::set_global(
                        ctx,
                        format!(
                            "Restored your imported key for {address}. \
                             Balance and sending for single-key wallets are coming in a future update."
                        ),
                        MessageType::Success,
                    );
                    self.restore_single_key_dialog.open = false;
                    self.restore_single_key_dialog.reset();
                    // Re-scan so the restored key drops off the banner.
                    self.refresh_pending_protected_restores();
                }
                Err(e) => {
                    MessageBanner::set_global(ctx, e.to_string(), MessageType::Error)
                        .with_details(&e);
                }
            }
        }
    }
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // Check for pending platform balance refresh (triggered after transfers)
        let pending_refresh_action = if let Some(seed_hash) =
            self.pending_platform_balance_refresh.take()
        {
            AppAction::BackendTask(BackendTask::WalletTask(
                crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances { seed_hash },
            ))
        } else {
            AppAction::None
        };

        // Trigger a wallet refresh after a wallet switch
        let pending_switch_action = if self.pending_wallet_refresh_on_switch {
            self.pending_wallet_refresh_on_switch = false;
            if let Some(wallet_arc) = &self.selected_wallet {
                let is_locked = wallet_arc.read().map(|w| !w.is_open()).unwrap_or(true);
                if !is_locked {
                    self.refreshing = true;
                    self.create_refresh_action(wallet_arc)
                } else {
                    AppAction::None
                }
            } else {
                AppAction::None
            }
        } else {
            AppAction::None
        };

        // Tick the shielded tab view to drain any pending user-initiated
        // tasks (e.g. Resync) even when the Shielded tab is not active.
        // Skip when the Shielded tab IS active — its ui() method already
        // calls tick(), and double-ticking would acquire the lock twice
        // per frame for no benefit.
        let shielded_tick_action = if self.selected_account_tab != AccountTab::Shielded {
            self.shielded_tab_view
                .as_mut()
                .map(|v| v.tick())
                .unwrap_or(AppAction::None)
        } else {
            AppAction::None
        };

        let mut right_buttons = vec![
            (
                "Import Wallet",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportMnemonic)),
            ),
            (
                "Import key (advanced)",
                DesiredAppAction::Custom("OpenImportSingleKey".to_string()),
            ),
            (
                "Create Wallet",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
            ),
        ];

        // Add Refresh button for HD wallet
        if !self.refreshing && self.selected_wallet.is_some() {
            right_buttons.push((
                "Refresh",
                DesiredAppAction::Custom("RefreshHDWallet".to_string()),
            ));
        }

        // Add Refresh button for single key wallet
        if !self.refreshing && self.selected_single_key_wallet.is_some() {
            right_buttons.push((
                "Refresh",
                DesiredAppAction::Custom("RefreshSKWallet".to_string()),
            ));
        }
        // Capturing variant: the effect is already applied to the app-global
        // selection, but this page owns the wallet-selection surface, so it must
        // also mirror the switch into its own cache — otherwise the pill and the
        // page body would disagree until the next arrival.
        let (mut action, effect) = add_top_panel_with_global_nav_capturing(
            ui,
            &self.app_context,
            wallet_only_spec("Wallets", RootScreenType::RootScreenWalletsBalances),
            right_buttons,
        );
        self.apply_nav_effect(effect);

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.style().visuals.dark_mode;

            // Message display is handled by the global MessageBanner

            egui::ScrollArea::vertical()
                .auto_shrink([true; 2])
                .show(ui, |ui| {
                    // Post-migration restore banner — shown even on an
                    // otherwise-empty wallets screen, since protected
                    // single keys may be all the user has left to restore.
                    self.render_protected_restore_banner(ui);

                    let has_hd_wallets = !self.app_context.wallets.read_recover().is_empty();
                    let has_single_key_wallets = !self
                        .app_context
                        .single_key_wallets
                        .read_recover()
                        .is_empty();

                    if !has_hd_wallets && !has_single_key_wallets {
                        self.render_no_wallets_view(ui);
                        return;
                    }

                    // Unified wallet selector (includes both HD and single key wallets)
                    Frame::group(ui.style())
                        .fill(DashColors::surface(dark_mode))
                        .inner_margin(Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            inner_action |= self.render_wallet_selection(ui);
                        });

                    ui.add_space(10.0);

                    // Render the appropriate detail view based on selection
                    if self.selected_wallet.is_some() {
                        inner_action |= self.render_wallet_detail_panel(ui, ctx);
                    } else if self.selected_single_key_wallet.is_some() {
                        inner_action |= self.render_single_key_wallet_view(ui, dark_mode);
                    }
                });

            inner_action
        });

        action |= self.render_receive_dialog(ctx);
        action |= self.render_fund_platform_dialog(ctx);
        action |= self.render_mine_dialog(ctx);
        self.render_private_key_dialog(ctx);
        self.render_import_single_key_dialog(ctx);
        self.render_restore_single_key_dialog(ctx);

        // Drain a queued "view private key" request into a backend task that
        // fetches the seed just-in-time and derives the key off the UI thread.
        if let Some((seed_hash, derivation_path, address)) =
            self.private_key_dialog.pending_view_key_request.take()
        {
            self.private_key_dialog.address = address;
            action |= AppAction::BackendTask(BackendTask::WalletTask(
                crate::backend_task::wallet::WalletTask::DeriveKeyForDisplay {
                    seed_hash,
                    derivation_path,
                },
            ));
        }

        // Drain a queued "generate Platform receive address" request into a
        // backend task that fetches the seed just-in-time and derives + registers
        // the new address off the UI thread.
        if let Some(seed_hash) = self.receive_dialog.pending_platform_address_request.take() {
            action |= AppAction::BackendTask(BackendTask::WalletTask(
                crate::backend_task::wallet::WalletTask::GeneratePlatformReceiveAddress {
                    seed_hash,
                },
            ));
        }

        // Drain a queued "generate Core receive address" request into a backend
        // task that derives the next address from the SPV-watched upstream pool.
        // The new address returns via `GeneratedReceiveAddress`.
        if let Some(seed_hash) = self.receive_dialog.pending_core_address_request.take() {
            action |= AppAction::BackendTask(BackendTask::WalletTask(
                crate::backend_task::wallet::WalletTask::GenerateReceiveAddress { seed_hash },
            ));
        }

        // Rename dialog
        if self.show_rename_dialog {
            let window_response = egui::Window::new("Rename Wallet")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.vertical(|ui| {
                        ui.label("Enter new wallet name:");
                        ui.add_space(5.0);

                        let text_edit = egui::TextEdit::singleline(&mut self.rename_input)
                            .hint_text("Enter wallet name")
                            .desired_width(250.0);
                        ui.add(text_edit);

                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode)
                                .clicked()
                            {
                                self.show_rename_dialog = false;
                                self.rename_input.clear();
                            }

                            ui.add_space(8.0);

                            if ComponentStyles::add_primary_button(ui, "Save").clicked() {
                                // Limit the alias length to 64 characters
                                if self.rename_input.len() > 64 {
                                    self.rename_input.truncate(64);
                                }

                                // Handle HD wallet rename
                                if let Some(selected_wallet) = &self.selected_wallet {
                                    let mut wallet = selected_wallet.write_recover();
                                    wallet.alias = Some(self.rename_input.clone());

                                    // T-W-01: alias persistence goes
                                    // through the wallet-meta sidecar.
                                    // The cold-boot picker reads from
                                    // the same key shape, so the new
                                    // name surfaces on the next launch
                                    // without touching the legacy
                                    // `wallet` table.
                                    let seed_hash = wallet.seed_hash();
                                    if let Ok(backend) = self.app_context.wallet_backend() {
                                        let meta_view = backend.wallet_meta();
                                        let mut meta = meta_view
                                            .get(self.app_context.network, &seed_hash)
                                            .unwrap_or_default();
                                        meta.alias = self.rename_input.clone();
                                        // Backfill the xpub on first
                                        // rename after migration so old
                                        // entries written before T-W-00.5
                                        // get a non-empty picker hint.
                                        if meta.xpub_encoded.is_empty() {
                                            meta.xpub_encoded = wallet
                                                .master_bip44_ecdsa_extended_public_key
                                                .encode()
                                                .to_vec();
                                        }
                                        if let Err(e) = meta_view.set(
                                            self.app_context.network,
                                            &seed_hash,
                                            &meta,
                                        ) {
                                            tracing::warn!(
                                                wallet = %hex::encode(seed_hash),
                                                error = ?e,
                                                "Failed to persist wallet alias to sidecar",
                                            );
                                        }
                                    }
                                    self.show_rename_dialog = false;
                                    self.rename_input.clear();
                                }
                                // Handle single key wallet rename
                                else if let Some(selected_sk_wallet) =
                                    &self.selected_single_key_wallet
                                {
                                    // Persist FIRST so the in-memory display
                                    // alias and the "renamed" outcome only
                                    // reflect a durable change. Alias
                                    // persistence goes through the modern
                                    // single-key sidecar (matching the
                                    // HD-wallet rename path above), so the
                                    // new name survives a restart without
                                    // touching the legacy `single_key_wallet`
                                    // table.
                                    let address =
                                        selected_sk_wallet.read_recover().address.to_string();
                                    let new_alias = self.rename_input.clone();
                                    let persisted = match self.app_context.wallet_backend() {
                                        Ok(backend) => backend
                                            .single_key()
                                            .set_alias(&address, Some(new_alias.clone())),
                                        Err(e) => Err(e),
                                    };
                                    match persisted {
                                        Ok(()) => {
                                            selected_sk_wallet.write_recover().alias =
                                                Some(new_alias);
                                            self.show_rename_dialog = false;
                                            self.rename_input.clear();
                                        }
                                        Err(e) => {
                                            MessageBanner::set_global(
                                                ctx,
                                                "Could not rename the imported key. Check available disk space and try again."
                                                    .to_string(),
                                                MessageType::Error,
                                            )
                                            .with_details(&e);
                                        }
                                    }
                                } else {
                                    self.show_rename_dialog = false;
                                    self.rename_input.clear();
                                }
                            }
                        });
                    });
                });

            if let Some(ref resp) = window_response
                && clicked_outside_window_after_open(
                    ctx,
                    resp.response.rect,
                    &mut self.rename_dialog_opening_guard,
                )
            {
                self.show_rename_dialog = false;
                self.rename_input.clear();
            }
        }

        // HD Wallet unlock popup
        if let Some(wallet_arc) = &self.selected_wallet.clone() {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet_arc, &self.app_context);
            match result {
                WalletUnlockResult::Unlocked => {
                    // Check if we were trying to view a private key
                    if let Some(path) = self.private_key_dialog.pending_derivation_path.take()
                        && let Some(address) = self.private_key_dialog.pending_address.take()
                    {
                        self.queue_view_key_request(&path, address);
                    }

                    // Check if we were trying to fund a Platform address
                    if self.fund_platform_dialog.pending_fund_after_unlock {
                        self.fund_platform_dialog.pending_fund_after_unlock = false;
                        action |= self.prepare_fund_platform_action();
                    }

                    // Check if we were trying to refresh the wallet
                    // Note: handle_wallet_unlocked also queues a refresh in the background,
                    // but we dispatch our own so the UI gets the result and can stop the spinner
                    if self.pending_refresh_after_unlock {
                        self.pending_refresh_after_unlock = false;
                        if let Some(wallet_arc) = &self.selected_wallet {
                            self.refreshing = true;
                            action |= self.create_pending_refresh_action(wallet_arc);
                        }
                    }
                }
                WalletUnlockResult::Cancelled => {
                    // Clear any pending private key view request on cancel
                    self.private_key_dialog.pending_derivation_path = None;
                    self.private_key_dialog.pending_address = None;

                    // Clear pending fund request on cancel
                    self.fund_platform_dialog.pending_fund_after_unlock = false;

                    // Clear pending refresh request on cancel
                    self.pending_refresh_after_unlock = false;
                }
                WalletUnlockResult::Pending => {}
            }
        }

        // SK wallet unlock dialog
        if self.show_sk_unlock_dialog {
            let mut close_dialog = false;
            egui::Window::new("Unlock Wallet")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        if let Some(wallet_arc) = &self.selected_single_key_wallet
                            && let Ok(wallet) = wallet_arc.read() {
                                if let Some(alias) = &wallet.alias {
                                    ui.label(format!(
                                        "Wallet \"{}\" is locked. Please enter the password to unlock it:",
                                        alias
                                    ));
                                } else {
                                    ui.label("This wallet is locked. Please enter the password to unlock it:");
                                }
                            }

                        ui.add_space(10.0);

                        let mut attempt_unlock = false;

                        let pw_response = self.sk_password_input.show(ui);

                        if pw_response.response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        {
                            attempt_unlock = true;
                        }

                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            let dark_mode = ui.style().visuals.dark_mode;
                            if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode)
                                .clicked()
                            {
                                close_dialog = true;
                            }

                            ui.add_space(8.0);

                            if ComponentStyles::add_primary_button(ui, "Unlock").clicked() {
                                attempt_unlock = true;
                            }
                        });

                        if attempt_unlock {
                            if let Some(wallet_arc) = &self.selected_single_key_wallet {
                                // Verify the passphrase against the encrypted
                                // vault without opening the map entry: signing
                                // decrypts just-in-time, so the key stays closed
                                // (no plaintext re-parked in the session map).
                                let address = wallet_arc
                                    .read()
                                    .ok()
                                    .map(|w| w.address.to_string());
                                let verify_result = match address {
                                    Some(addr) => self
                                        .app_context
                                        .verify_single_key_passphrase(
                                            &addr,
                                            self.sk_password_input.text(),
                                        ),
                                    None => Err(TaskError::ImportedKeyNotFound),
                                };

                                match verify_result {
                                    Ok(()) => {
                                        MessageBanner::set_global(
                                            ui.ctx(),
                                            "Password confirmed. This key is ready to use.",
                                            MessageType::Success,
                                        );
                                        close_dialog = true;
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
                            self.sk_password_input.clear();
                        }

                        // Error display is handled by the global MessageBanner.
                    });
                });

            if close_dialog {
                self.show_sk_unlock_dialog = false;
                self.sk_password_input.clear();
                // Check if we were trying to refresh the SK wallet
                if self.pending_refresh_after_unlock {
                    self.pending_refresh_after_unlock = false;
                    if let Some(wallet_arc) = &self.selected_single_key_wallet {
                        self.refreshing = true;
                        action |= AppAction::BackendTask(BackendTask::CoreTask(
                            CoreTask::RefreshSingleKeyWalletInfo(wallet_arc.clone()),
                        ));
                    }
                }
            }
        }

        if let AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(_, _))) =
            action
        {
            self.refreshing = true;
        }

        // Handle custom refresh actions - check wallet lock status
        if let AppAction::Custom(ref cmd) = action {
            if cmd == "OpenImportSingleKey" {
                // Sync the dialog's network with the active context every
                // open so a quick network switch can't show a stale preview.
                self.import_single_key_dialog
                    .set_network(self.app_context.network);
                self.import_single_key_dialog.reset();
                self.import_single_key_dialog.open = true;
                action = AppAction::None;
            } else if cmd == "RefreshHDWallet" {
                if let Some(wallet_arc) = &self.selected_wallet {
                    let is_locked = wallet_arc.read().map(|w| !w.is_open()).unwrap_or(true);
                    if is_locked {
                        // Wallet is locked - open unlock popup and store the refresh mode
                        self.pending_refresh_after_unlock = true;
                        self.pending_refresh_mode = self.refresh_mode;
                        self.wallet_unlock_popup.open();
                        action = AppAction::None;
                    } else {
                        // Wallet is unlocked - proceed with refresh using selected mode
                        self.refreshing = true;
                        action = self.create_refresh_action(wallet_arc);
                    }
                }
            } else if cmd == "RefreshSKWallet"
                && let Some(wallet_arc) = &self.selected_single_key_wallet
            {
                let is_locked = wallet_arc.read().map(|w| !w.is_open()).unwrap_or(true);
                if is_locked {
                    // SK wallet is locked - open unlock dialog
                    self.pending_refresh_after_unlock = true;
                    self.show_sk_unlock_dialog = true;
                    action = AppAction::None;
                } else {
                    // SK wallet is unlocked - proceed with refresh
                    self.refreshing = true;
                    action = AppAction::BackendTask(BackendTask::CoreTask(
                        CoreTask::RefreshSingleKeyWalletInfo(wallet_arc.clone()),
                    ));
                }
            }
        }

        // Combine with pending actions
        action |= pending_refresh_action;
        action |= pending_switch_action;
        action |= shielded_tick_action;
        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        // Always clear refreshing — the originating task is done regardless of result type.
        self.refreshing = false;

        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            // If the fund platform dialog is processing, show error in the dialog instead
            if self.fund_platform_dialog.is_processing {
                self.fund_platform_dialog.is_processing = false;
                self.fund_platform_dialog.status = Some(message.to_string());
                self.fund_platform_dialog.status_is_error = true;
            }

            // Forward errors to the shielded tab view so it can reset spinner states
            if let Some(shielded_view) = &mut self.shielded_tab_view {
                shielded_view.handle_error(message);
            }
        }
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::ui::BackendTaskSuccessResult::RefreshedWallet { warning } => {
                self.refreshing = false;
                self.cached_tx_indices = None;
                self.cached_tx_source_len = None;
                if let Some(err) = warning {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Wallet refreshed, but platform balances could not be updated. Retry in a moment.",
                        MessageType::Info,
                    )
                    .with_details(err.as_ref());
                } else {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Successfully refreshed wallet",
                        MessageType::Success,
                    );
                }
            }
            crate::ui::BackendTaskSuccessResult::WalletPayment {
                txid,
                recipients,
                total_amount,
            } => {
                let msg = if recipients.len() == 1 {
                    let (address, amount) = &recipients[0];
                    format!(
                        "Sent {} to {}\nTxID: {}",
                        format_duffs_as_dash(*amount),
                        address,
                        txid
                    )
                } else {
                    format!(
                        "Sent {} total to {} recipients\nTxID: {}",
                        format_duffs_as_dash(total_amount),
                        recipients.len(),
                        txid
                    )
                };
                MessageBanner::set_global(self.app_context.egui_ctx(), &msg, MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::TrackedAssetLocks { seed_hash, locks } => {
                self.asset_lock_cache.store(seed_hash, locks);
            }
            crate::ui::BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address } => {
                let is_selected = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok())
                    .is_some_and(|g| g.seed_hash() == seed_hash);
                if is_selected {
                    // Look up the address balance in the display snapshot
                    // (P4a). A freshly-derived receive address is virtually
                    // always 0 until it is funded; absent ⇒ 0.
                    let balances = self.snapshot_address_balances(&seed_hash);
                    let balance = address
                        .parse::<Address<_>>()
                        .ok()
                        .and_then(|addr| balances.get(&addr.assume_checked()).copied())
                        .unwrap_or(0);
                    self.receive_dialog
                        .core_addresses
                        .push((address.clone(), balance));
                    self.receive_dialog.selected_core_index =
                        self.receive_dialog.core_addresses.len() - 1;
                    self.receive_dialog.qr_texture = None;
                    self.receive_dialog.qr_address = None;
                    self.receive_dialog.status = None;
                }
            }
            crate::ui::BackendTaskSuccessResult::WalletKeyForDisplay { wif, .. } => {
                // The backend derived the key just-in-time; show it in the
                // private-key dialog (hidden until the user reveals it).
                self.private_key_dialog.private_key_wif = wif;
                self.private_key_dialog.show_key = false;
                self.private_key_dialog.is_open = true;
            }
            crate::ui::BackendTaskSuccessResult::GeneratedPlatformReceiveAddress {
                address,
                ..
            } => {
                // The backend derived + registered the new Platform address
                // just-in-time; surface it in the receive dialog.
                self.receive_dialog.platform_addresses.push((address, 0));
                self.receive_dialog.selected_platform_index =
                    self.receive_dialog.platform_addresses.len() - 1;
                self.receive_dialog.qr_texture = None;
                self.receive_dialog.qr_address = None;
                self.receive_dialog.status = Some("New address generated!".to_string());
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressWithdrawal { .. } => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Platform withdrawal successful. Note: It may take a few minutes for funds to appear on the Core chain.",
                    MessageType::Success,
                );
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressFunded { .. } => {
                self.fund_platform_dialog.is_processing = false;
                self.fund_platform_dialog.status = Some("Funding successful!".to_string());
                self.fund_platform_dialog.status_is_error = false;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Platform address funded successfully",
                    MessageType::Success,
                );
            }
            crate::ui::BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash } => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Platform credits transferred successfully",
                    MessageType::Success,
                );
                // Schedule a refresh of platform address balances to update the UI
                self.pending_platform_balance_refresh = Some(seed_hash);
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressBalances {
                seed_hash,
                balances,
                network,
            } => {
                self.refreshing = false;
                // Skip stale results from a different network
                if network != self.app_context.network {
                    tracing::warn!(
                        result_network = ?network,
                        current_network = ?self.app_context.network,
                        "Discarding PlatformAddressBalances from a previous network"
                    );
                    return;
                }
                // Update wallet's platform_address_info if this is for the selected wallet
                if let Some(selected) = &self.selected_wallet
                    && let Ok(mut wallet) = selected.write()
                    && wallet.seed_hash() == seed_hash
                {
                    // Convert PlatformAddress back to Core Address for wallet storage
                    for (platform_addr, (balance, nonce)) in balances {
                        let core_addr = platform_addr.to_address_with_network(network);
                        wallet.set_platform_address_info(core_addr, balance, nonce);
                    }
                }
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Successfully synced Platform balances",
                    MessageType::Success,
                );
            }
            crate::ui::BackendTaskSuccessResult::Message(msg) => {
                self.refreshing = false;
                MessageBanner::set_global(self.app_context.egui_ctx(), &msg, MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::MineBlocksSuccess(count) => {
                self.refreshing = false;
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Mined {} block(s)", count),
                    MessageType::Success,
                );
            }
            // Shielded pool results
            result @ (crate::ui::BackendTaskSuccessResult::ShieldedCreditsShielded { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedTransferComplete { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedCreditsUnshielded { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedFromAssetLock { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedWithdrawalComplete {
                ..
            }) => {
                if let Some(shielded_view) = &mut self.shielded_tab_view {
                    shielded_view.handle_result(&result);
                }
            }
            _ => {}
        }
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        // A failed asset-lock fetch would otherwise strand the tab on a spinner;
        // flip the in-flight fetch to a retryable state. The error carries no
        // seed_hash, so this routes any in-flight lock fetch to Failed.
        self.asset_lock_cache.mark_loading_failed();
        false
    }

    fn refresh_on_arrival(&mut self) {
        // Clear the spinner in case a refresh completed while this screen was not
        // visible (task results are dispatched to the visible screen, so ours would
        // have been silently discarded).
        self.refreshing = false;
        // Asset-lock mutations happen on pushed screens. Returning to this root
        // screen must re-fetch instead of reusing a terminal Loaded(empty) entry.
        if let Some(seed_hash) = self
            .selected_wallet
            .as_ref()
            .map(|wallet| wallet.read_recover().seed_hash())
        {
            self.asset_lock_cache.invalidate_one(&seed_hash);
        }

        // Check if there's a pending wallet selection (e.g., from wallet creation/import)
        let pending_seed_hash = self
            .app_context
            .pending_wallet_selection
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());

        if let Some(seed_hash) = pending_seed_hash {
            let selected_wallet = self
                .app_context
                .wallets
                .read()
                .ok()
                .and_then(|wallets| wallets.get(&seed_hash).cloned());

            if let Some(wallet) = selected_wallet {
                self.select_hd_wallet(wallet);
                self.persist_selected_wallet_hash(Some(seed_hash));
                return;
            }
        }

        // Re-sync the cached selection from the shared store so arriving here
        // always shows the wallet last chosen on any surface (e.g. the top-nav
        // pill), replacing a stale cache. Skips when the cache already agrees.
        let (store_hd, store_sk) = resolve_selection_from_store(&self.app_context);
        if store_hd.is_some() || store_sk.is_some() {
            self.adopt_store_selection(store_hd, store_sk);
            return;
        }

        // If no wallet of either type is selected but wallets exist, select the first HD wallet
        if self.selected_wallet.is_none() && self.selected_single_key_wallet.is_none() {
            let next_hd = self
                .app_context
                .wallets
                .read()
                .ok()
                .and_then(|w| w.values().next().cloned());
            if let Some(wallet) = next_hd {
                self.set_selected_hd_wallet(Some(wallet));
                return;
            }
            // If no HD wallets, try single key wallets
            if let Ok(wallets) = self.app_context.single_key_wallets.read() {
                self.selected_single_key_wallet = wallets.values().next().cloned();
            }
        }
    }

    fn refresh(&mut self) {
        self.refreshing = false;
        // Re-scan for protected single-key rows still awaiting restore so a
        // post-migration refresh surfaces (or clears) the restore banner.
        self.pending_restores_scanned = false;
        self.refresh_on_arrival();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::wallet::DerivationPathReference;

    /// A tx with no block yet (mempool or InstantSend-locked-but-
    /// unconfirmed) carries `timestamp == 0`. Must render a "still pending"
    /// placeholder, never the Unix epoch.
    #[test]
    fn format_transaction_timestamp_zero_renders_pending_placeholder() {
        let rendered = WalletsBalancesScreen::format_transaction_timestamp(0);
        assert_eq!(rendered, "Pending…");
        assert!(!rendered.contains("1970"));
    }

    #[test]
    fn format_transaction_timestamp_nonzero_renders_the_block_date() {
        assert_eq!(
            WalletsBalancesScreen::format_transaction_timestamp(1_700_000_000),
            "2023-11-14 22:13:20"
        );
    }

    fn summary(
        category: AccountCategory,
        index: Option<u32>,
        confirmed_balance: u64,
    ) -> AccountSummary {
        AccountSummary {
            category,
            index,
            confirmed_balance,
        }
    }

    fn platform_tab_count(tabs: &[AccountTab]) -> usize {
        tabs.iter()
            .filter(|t| matches!(t, AccountTab::Category(AccountCategory::PlatformPayment, _)))
            .count()
    }

    /// QA-004: an HD wallet shows the Platform tab immediately on load — before
    /// any platform-address sync completes — so the user's only in-app route to
    /// their Platform receive address is never hidden waiting on the network.
    /// The tab is present even with empty summaries and no platform balance.
    #[test]
    fn platform_tab_present_immediately_pre_sync() {
        let tabs = plan_account_tabs(&[], false, false, true);
        assert_eq!(
            platform_tab_count(&tabs),
            1,
            "Platform tab must appear on wallet load, before first sync"
        );
        // The Dash Core tab is always present too.
        assert!(
            tabs.iter()
                .any(|t| matches!(t, AccountTab::Category(AccountCategory::Bip44, Some(0)))),
            "Dash Core tab is always present"
        );
    }

    /// A wallet with no platform-payment bootstrap (e.g. single-key) gets no
    /// Platform tab.
    #[test]
    fn no_platform_tab_when_not_applicable() {
        let tabs = plan_account_tabs(&[], false, false, false);
        assert_eq!(platform_tab_count(&tabs), 0);
    }

    /// The snapshot carries the DIP-17 platform-payment pool, so the primary
    /// loop emits a `PlatformPayment` tab from the summaries. The dedicated
    /// push must not add a second one.
    #[test]
    fn platform_tab_is_never_duplicated() {
        let summaries = vec![summary(AccountCategory::PlatformPayment, None, 1_000)];
        let tabs = plan_account_tabs(&summaries, false, false, true);
        assert_eq!(
            platform_tab_count(&tabs),
            1,
            "exactly one Platform tab, never a duplicate"
        );
    }

    /// QA-002: in default mode, funds parked in a non-visible category surface a
    /// consolidated tab so the visible tabs reconcile the header total. With no
    /// such funds, no extra tab appears.
    #[test]
    fn default_mode_surfaces_other_tab_only_when_hidden_funds_exist() {
        // No hidden funds → no System/Other tab in default mode.
        let visible_only = vec![summary(AccountCategory::Bip44, Some(0), 500)];
        let tabs = plan_account_tabs(&visible_only, false, false, true);
        assert!(
            !tabs.contains(&AccountTab::System),
            "no Other tab without hidden funds"
        );

        // Funded CoinJoin (a hidden/system category) → the reconciling tab
        // appears so visible tabs still sum to the header total.
        let with_hidden = vec![
            summary(AccountCategory::Bip44, Some(0), 500),
            summary(AccountCategory::CoinJoin, None, 700),
        ];
        let tabs = plan_account_tabs(&with_hidden, false, false, true);
        assert!(
            tabs.contains(&AccountTab::System),
            "hidden funds must surface a reconciling tab in default mode"
        );
    }

    /// A funded `Other(Unknown)` address surfaces the reconciling tab in
    /// default mode.
    #[test]
    fn default_mode_surfaces_other_tab_for_unknown_funded_address() {
        let summaries = vec![
            summary(AccountCategory::Bip44, Some(0), 500),
            summary(
                AccountCategory::Other(DerivationPathReference::Unknown),
                None,
                9,
            ),
        ];
        let tabs = plan_account_tabs(&summaries, false, false, true);
        assert!(tabs.contains(&AccountTab::System));
    }

    /// Developer mode always shows the System tab, even with no hidden funds —
    /// it is the full diagnostic breakdown there.
    #[test]
    fn developer_mode_always_shows_system_tab() {
        let tabs = plan_account_tabs(&[], true, false, true);
        assert!(tabs.contains(&AccountTab::System));
    }

    /// The Shielded tab appears whenever the shielded pool is available, so a
    /// wallet that tracks a shielded balance always has a tab to view it.
    /// Regression guard for the bug where the balance breakdown showed shielded
    /// funds but the tab bar offered no Shielded tab.
    #[test]
    fn shielded_tab_present_when_available() {
        let tabs = plan_account_tabs(&[], false, true, true);
        assert!(
            tabs.contains(&AccountTab::Shielded),
            "Shielded tab must appear when shielded is available"
        );

        let tabs = plan_account_tabs(&[], false, false, true);
        assert!(
            !tabs.contains(&AccountTab::Shielded),
            "no Shielded tab when shielded is unavailable"
        );
    }

    /// Build an offline `AppContext` (no network I/O, throwaway data dir).
    fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::tasks::TaskManager;
        use dash_sdk::dpp::dashcore::Network;

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

    /// Register an HD wallet derived from a distinct seed, returning its hash.
    fn seed_hd_wallet(ctx: &Arc<AppContext>, seed_byte: u8) -> WalletSeedHash {
        let wallet =
            Wallet::new_from_seed([seed_byte; 64], ctx.network(), None, None).expect("wallet");
        let seed_hash = wallet.seed_hash();
        ctx.wallets
            .write()
            .expect("wallets")
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));
        seed_hash
    }

    /// FR-GLOBAL-NAV-2 rule 2 — the Wallets page is two-way bound with the nav
    /// wallet pill. Switching on the pill selects that wallet on the page; the
    /// page's own selection is what the pill reads back (the app-global hash).
    /// An unknown wallet leaves the selection untouched.
    #[test]
    fn wallet_pill_switch_selects_that_wallet_on_the_page() {
        let (ctx, _tmp) = offline_ctx();
        let first = seed_hd_wallet(&ctx, 0xAA);
        let second = seed_hd_wallet(&ctx, 0xBB);
        let mut screen = WalletsBalancesScreen::new(&ctx);

        screen.select_hd_wallet_by_hash(first);
        assert_eq!(screen.selected_wallet_seed_hash(), Some(first));

        // Pill → page.
        screen.apply_nav_effect(GlobalNavEffect::SwitchWallet(second));
        assert_eq!(screen.selected_wallet_seed_hash(), Some(second));
        // Page → pill: the pill renders from the app-global hash every frame.
        assert_eq!(ctx.selected_wallet_hash(), Some(second));

        // A wallet this network does not have changes nothing.
        screen.apply_nav_effect(GlobalNavEffect::SwitchWallet([0xEE; 32]));
        assert_eq!(screen.selected_wallet_seed_hash(), Some(second));
    }

    /// Why this page uses the *capturing* top-panel variant: the shared applier
    /// the panel runs on a pill click moves only the **app-global** selection.
    /// This page caches its own wallet handle, so without mirroring the effect
    /// back into that cache the pill and the page body would disagree until the
    /// user navigated away and returned — the arrival re-sync cannot help, since
    /// a pill click performs no navigation.
    ///
    /// Pins the seam between the two halves of the wallet-selector linking: drop
    /// `apply_nav_effect` from the `ui()` path and the second assertion here is
    /// what the user would experience — a click that moves the pill but not the
    /// page.
    #[test]
    fn a_pill_click_must_be_mirrored_into_the_page_cache() {
        let (ctx, _tmp) = offline_ctx();
        let first = seed_hd_wallet(&ctx, 0xAA);
        let second = seed_hd_wallet(&ctx, 0xBB);
        let mut screen = WalletsBalancesScreen::new(&ctx);
        screen.select_hd_wallet_by_hash(first);

        // Exactly what the top panel does on a pill click, before the page gets
        // a say: the shared applier writes the app-global selection.
        let effect = GlobalNavEffect::SwitchWallet(second);
        crate::ui::components::top_panel::apply_global_nav_effect(&ctx, effect.clone());

        assert_eq!(
            ctx.selected_wallet_hash(),
            Some(second),
            "the applier moves the app-global selection"
        );
        assert_eq!(
            screen.selected_wallet_seed_hash(),
            Some(first),
            "...but it does NOT touch this page's cached wallet — the page is still on the old one"
        );

        // The mirroring step this page owns is what closes that gap, in-frame.
        screen.apply_nav_effect(effect);
        assert_eq!(
            screen.selected_wallet_seed_hash(),
            Some(second),
            "the page now shows the wallet the pill shows"
        );
    }

    /// A wallet switched from another page's nav pill while this screen was away
    /// is adopted on arrival — otherwise the pill and the page would disagree.
    #[test]
    fn arriving_adopts_a_wallet_switched_elsewhere() {
        let (ctx, _tmp) = offline_ctx();
        let first = seed_hd_wallet(&ctx, 0xAA);
        let second = seed_hd_wallet(&ctx, 0xBB);
        let mut screen = WalletsBalancesScreen::new(&ctx);
        screen.select_hd_wallet_by_hash(first);

        // Another page's pill switches the app-global wallet.
        ctx.set_selected_hd_wallet(Some(second));
        screen.refresh_on_arrival();

        assert_eq!(screen.selected_wallet_seed_hash(), Some(second));
    }

    /// Arriving with nothing selected must honour the app-global wallet, not the
    /// first-wallet default — the default would otherwise silently overrule a
    /// wallet the user picked from the nav pill on another page.
    #[test]
    fn arriving_with_no_selection_honours_the_app_global_wallet() {
        let (ctx, _tmp) = offline_ctx();
        // Built before any wallet exists → the screen starts with no selection.
        let mut screen = WalletsBalancesScreen::new(&ctx);
        seed_hd_wallet(&ctx, 0xAA);
        seed_hd_wallet(&ctx, 0xBB);

        // Target the wallet the first-wallet default would NOT pick.
        let hashes: Vec<WalletSeedHash> = ctx
            .wallets
            .read()
            .expect("wallets")
            .keys()
            .copied()
            .collect();
        let default_pick = hashes.first().copied().expect("a wallet");
        let target = hashes.last().copied().expect("a wallet");
        assert_ne!(
            default_pick, target,
            "the two wallets must be distinguishable"
        );

        ctx.set_selected_hd_wallet(Some(target));
        screen.refresh_on_arrival();

        assert_eq!(screen.selected_wallet_seed_hash(), Some(target));
    }

    mod wallet_selection_linking {
        use super::super::{WalletsBalancesScreen, resolve_selection_from_store};
        use crate::context::AppContext;
        use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
        use crate::model::wallet::{Wallet, WalletSeedHash};
        use crate::ui::ScreenLike;
        use dash_sdk::dpp::dashcore::Network;
        use std::sync::{Arc, RwLock};

        fn test_app_context(dir: &std::path::Path) -> Arc<AppContext> {
            crate::app_dir::ensure_env_file(dir);
            let db_file = dir.join("data.db");
            let db = Arc::new(crate::database::Database::new(&db_file).expect("db"));
            db.create_tables(true).expect("create tables");
            db.set_default_version().expect("set version");
            let app_kv = AppContext::open_app_kv(dir).expect("open app k/v");
            let secret_store = AppContext::open_secret_store(dir).expect("open secret store");
            AppContext::new(
                dir.to_path_buf(),
                Network::Testnet,
                db,
                Default::default(),
                Default::default(),
                egui::Context::default(),
                app_kv,
                secret_store,
                crate::model::user_role::UserRoleCell::default(),
            )
            .expect("AppContext")
        }

        fn seed_hd(ctx: &Arc<AppContext>, seed_byte: u8) -> (WalletSeedHash, Arc<RwLock<Wallet>>) {
            let wallet = Wallet::new_from_seed(
                [seed_byte; 64],
                Network::Testnet,
                Some(format!("hd-{seed_byte}")),
                None,
            )
            .expect("hd wallet");
            let hash = wallet.seed_hash();
            let arc = Arc::new(RwLock::new(wallet));
            ctx.wallets.write().unwrap().insert(hash, arc.clone());
            (hash, arc)
        }

        fn seed_sk(
            ctx: &Arc<AppContext>,
            key_byte: u8,
        ) -> (SingleKeyHash, Arc<RwLock<SingleKeyWallet>>) {
            let mut pk = [1u8; 32];
            pk[31] = key_byte.max(1);
            let wallet =
                SingleKeyWallet::new(pk, Network::Testnet, None, Some(format!("sk-{key_byte}")))
                    .expect("sk wallet");
            let hash = wallet.key_hash;
            let arc = Arc::new(RwLock::new(wallet));
            ctx.single_key_wallets
                .write()
                .unwrap()
                .insert(hash, arc.clone());
            (hash, arc)
        }

        #[test]
        fn removing_a_single_key_wallet_requires_confirmation() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (key_hash, wallet) = seed_sk(&ctx, 7);
            let mut screen = WalletsBalancesScreen::create_with_selection(&ctx, None, Some(wallet));

            screen.request_selected_wallet_removal();

            assert!(
                screen.remove_wallet_dialog.is_some(),
                "a single-key removal request opens the confirmation dialog"
            );
            assert!(
                ctx.single_key_wallets
                    .read()
                    .unwrap()
                    .contains_key(&key_hash),
                "requesting removal must not delete the key before confirmation"
            );
        }

        #[test]
        fn returning_to_wallets_rearms_asset_lock_fetch() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (seed_hash, wallet) = seed_hd(&ctx, 7);
            ctx.set_selected_hd_wallet(Some(seed_hash));
            let mut screen = WalletsBalancesScreen::create_with_selection(&ctx, Some(wallet), None);
            screen.asset_lock_cache.store(seed_hash, Vec::new());
            assert!(
                screen
                    .asset_lock_cache
                    .ensure_requested(seed_hash)
                    .is_none()
            );

            screen.refresh_on_arrival();

            assert!(
                screen
                    .asset_lock_cache
                    .ensure_requested(seed_hash)
                    .is_some(),
                "returning to the wallets root must re-fetch asset locks"
            );
        }

        /// TC-WALLETLINK-07 (the dual-hash trap, highest-risk case). (a) A
        /// single-key selection survives navigation without auto-picking an HD
        /// wallet; (b) a later HD pick from the pill supersedes the stale
        /// single-key selection. Goes RED against a setter that preserves the
        /// stale single-key hash on an HD set (store then resolves single-key
        /// first and re-shows the wrong wallet).
        #[test]
        fn hd_pick_supersedes_a_prior_single_key_selection() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (hd_hash, hd_arc) = seed_hd(&ctx, 5);
            let (sk_hash, sk_arc) = seed_sk(&ctx, 7);

            // Leg (a): select the single-key wallet → store (HD=None, SK=Some).
            ctx.set_selected_single_key_wallet(Some(sk_hash));
            let (hd, sk) = resolve_selection_from_store(&ctx);
            assert!(
                hd.is_none(),
                "a single-key selection must not auto-pick an HD wallet on arrival"
            );
            assert!(
                sk.as_ref().is_some_and(|w| Arc::ptr_eq(w, &sk_arc)),
                "the single-key wallet stays selected across arrival"
            );

            // Leg (b): pick an HD wallet from the pill → it supersedes the SK one.
            ctx.set_selected_hd_wallet(Some(hd_hash));
            let (hd, sk) = resolve_selection_from_store(&ctx);
            assert!(
                sk.is_none(),
                "an explicit HD pick clears the stale single-key selection"
            );
            assert!(
                hd.as_ref().is_some_and(|w| Arc::ptr_eq(w, &hd_arc)),
                "the HD wallet the user picked is shown, not the stale single-key one"
            );
        }

        /// Mirror of TC-WALLETLINK-07 for the single-key setter: an explicit
        /// single-key pick at the context layer supersedes a prior HD selection,
        /// so the invariant is self-enforcing regardless of caller (not just via
        /// the screen path that clears HD itself).
        #[test]
        fn single_key_pick_supersedes_a_prior_hd_selection() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (hd_hash, _hd_arc) = seed_hd(&ctx, 5);
            let (sk_hash, sk_arc) = seed_sk(&ctx, 7);

            ctx.set_selected_hd_wallet(Some(hd_hash));
            ctx.set_selected_single_key_wallet(Some(sk_hash));
            let (hd, sk) = resolve_selection_from_store(&ctx);
            assert!(
                hd.is_none(),
                "an explicit single-key pick clears the stale HD selection"
            );
            assert!(sk.as_ref().is_some_and(|w| Arc::ptr_eq(w, &sk_arc)));
        }

        /// The store never holds both an HD and a single-key hash: neither setter
        /// leaves the ambiguous `(Some, Some)` state, so resolution is always
        /// unambiguous. Guards fixes to `set_selected_hd_wallet`,
        /// `set_selected_single_key_wallet`, and `set_selected_identity`.
        #[test]
        fn setters_never_leave_both_hashes_set() {
            use dash_sdk::platform::Identifier;
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (hd_hash, _hd) = seed_hd(&ctx, 5);
            let (sk_hash, _sk) = seed_sk(&ctx, 7);

            let both_set = |ctx: &Arc<AppContext>| {
                ctx.selected_wallet_hash().is_some()
                    && ctx.selected_single_key_hash.lock().unwrap().is_some()
            };

            ctx.set_selected_hd_wallet(Some(hd_hash));
            ctx.set_selected_single_key_wallet(Some(sk_hash));
            assert!(!both_set(&ctx), "single-key set clears HD");

            ctx.set_selected_hd_wallet(Some(hd_hash));
            assert!(!both_set(&ctx), "HD set clears single-key");

            // Selecting a single-key wallet, then an identity, must not resurrect
            // the HD hash alongside the live single-key one.
            ctx.set_selected_single_key_wallet(Some(sk_hash));
            ctx.set_selected_identity(Some(Identifier::from([9u8; 32])));
            assert!(
                !both_set(&ctx),
                "identity select never leaves both hashes set"
            );
        }

        /// TC-WALLETLINK-03: a pill-driven HD switch made on another page is
        /// reflected by the store resolution — no stale wallet.
        #[test]
        fn store_resolution_reflects_a_pill_driven_hd_switch() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (_a, _a_arc) = seed_hd(&ctx, 3);
            let (b_hash, b_arc) = seed_hd(&ctx, 5);

            ctx.set_selected_hd_wallet(Some(b_hash));
            let (hd, sk) = resolve_selection_from_store(&ctx);
            assert!(sk.is_none());
            assert!(hd.as_ref().is_some_and(|w| Arc::ptr_eq(w, &b_arc)));
        }

        /// TC-WALLETLINK-03 at the screen level: arriving re-syncs the cache to a
        /// switch made elsewhere, replacing the stale handle.
        #[test]
        fn refresh_on_arrival_adopts_a_switch_made_elsewhere() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (a_hash, a_arc) = seed_hd(&ctx, 3);
            let (b_hash, b_arc) = seed_hd(&ctx, 5);

            ctx.set_selected_hd_wallet(Some(a_hash));
            let mut screen = WalletsBalancesScreen::new(&ctx);
            assert!(
                screen
                    .selected_wallet
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, &a_arc)),
                "screen starts on the stored wallet A"
            );

            // A switch to B happens on another surface.
            ctx.set_selected_hd_wallet(Some(b_hash));
            screen.refresh_on_arrival();
            assert!(
                screen
                    .selected_wallet
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, &b_arc)),
                "arriving adopts the newly-selected wallet B"
            );
            assert!(screen.selected_single_key_wallet.is_none());
        }

        /// TC-WALLETLINK-04: arriving when the cache already agrees with the
        /// store is idempotent — the same wallet Arc stays selected, never reset
        /// to first-wallet.
        #[test]
        fn refresh_on_arrival_is_idempotent_when_cache_matches_store() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (_a, _a_arc) = seed_hd(&ctx, 3);
            let (b_hash, b_arc) = seed_hd(&ctx, 5);

            ctx.set_selected_hd_wallet(Some(b_hash));
            let mut screen = WalletsBalancesScreen::new(&ctx);
            screen.refresh_on_arrival();
            assert!(
                screen
                    .selected_wallet
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, &b_arc)),
                "the valid selection B is preserved, not clobbered by first-wallet"
            );
        }

        /// TC-WALLETLINK-06: a stored hash naming no currently-loaded wallet (the
        /// cross-network case) resolves to nothing rather than the wrong wallet,
        /// leaving the caller's fallback to choose.
        #[test]
        fn resolution_ignores_a_hash_naming_no_loaded_wallet() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (_a, _a_arc) = seed_hd(&ctx, 3);

            ctx.set_selected_hd_wallet(Some([0xAB; 32]));
            let (hd, sk) = resolve_selection_from_store(&ctx);
            assert!(
                hd.is_none() && sk.is_none(),
                "an unknown/cross-network hash resolves to nothing"
            );
        }

        /// TC-WALLETLINK-11: a `pending_wallet_selection` (from create/import) is
        /// honored on arrival, ahead of the store re-sync.
        #[test]
        fn refresh_on_arrival_honors_pending_selection_first() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (a_hash, _a_arc) = seed_hd(&ctx, 3);
            let (b_hash, b_arc) = seed_hd(&ctx, 5);

            ctx.set_selected_hd_wallet(Some(a_hash));
            let mut screen = WalletsBalancesScreen::new(&ctx);
            *ctx.pending_wallet_selection.lock().unwrap() = Some(b_hash);
            screen.refresh_on_arrival();
            assert!(
                screen
                    .selected_wallet
                    .as_ref()
                    .is_some_and(|w| Arc::ptr_eq(w, &b_arc)),
                "the pending wallet B wins on arrival"
            );
        }

        /// TC-WALLETLINK-13: `refresh_on_arrival` always clears the spinner flag.
        #[test]
        fn refresh_on_arrival_clears_the_refreshing_flag() {
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (a_hash, _a_arc) = seed_hd(&ctx, 3);
            ctx.set_selected_hd_wallet(Some(a_hash));
            let mut screen = WalletsBalancesScreen::new(&ctx);
            screen.refreshing = true;
            screen.refresh_on_arrival();
            assert!(!screen.refreshing);
        }

        /// TC-WALLETLINK-12: the pill-driven HD switch keeps its cross-axis
        /// identity reconciliation — switching to a wallet with no owned User
        /// identity clears a stale app-global identity to `None` (never leaves a
        /// masternode/evonode or an unrelated identity selected).
        #[test]
        fn pill_hd_switch_reconciles_stale_identity_to_none() {
            use dash_sdk::platform::Identifier;
            let tmp = tempfile::tempdir().unwrap();
            let ctx = test_app_context(tmp.path());
            let (a_hash, _a_arc) = seed_hd(&ctx, 3);

            ctx.set_selected_identity(Some(Identifier::from([9u8; 32])));
            ctx.set_selected_hd_wallet(Some(a_hash));
            assert!(
                ctx.selected_identity_id().is_none(),
                "switching to a wallet owning no User identity clears the stale identity"
            );
        }
    }
}
