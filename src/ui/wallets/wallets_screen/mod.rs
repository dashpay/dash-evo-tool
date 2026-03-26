mod address_table;
mod asset_locks;
mod dialogs;
mod single_key_view;

use crate::app::{AppAction, BackendTasksExecutionMode, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::shielded::ShieldedTask;
use crate::context::AppContext;
use crate::context::connection_status::spv_phase_summary;
use crate::model::amount::Amount;
use crate::model::wallet::{TransactionStatus, Wallet, WalletSeedHash, WalletTransaction};
use crate::spv::{CoreBackendMode, SpvStatus};
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::selection_dialog::{SelectionDialog, SelectionStatus};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{WalletUnlockPopup, WalletUnlockResult};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::helpers::clicked_outside_window;
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::wallets::account_summary::{
    AccountCategory, AccountSummary, collect_account_summaries,
};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use eframe::egui::{self, ComboBox, Context, Ui};
use egui::{Color32, Frame, Margin, RichText};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, RwLock};

use crate::model::wallet::single_key::SingleKeyWallet;
use crate::ui::wallets::shielded_tab::ShieldedTabView;
use address_table::{SortColumn, SortOrder};
use dialogs::{
    FundPlatformAddressDialogState, MineDialogState, PrivateKeyDialogState, ReceiveDialogState,
    SendDialogState,
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
    /// Shielded wallet view (replaces the old top-level Shielded tab)
    Shielded,
    /// Consolidated system tab (developer mode only) — shows all non-primary
    /// account categories as collapsible sections.
    System,
}

impl Default for AccountTab {
    fn default() -> Self {
        AccountTab::Category(AccountCategory::Bip44, Some(0))
    }
}

/// Refresh mode for dev mode dropdown - controls what gets refreshed
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum RefreshMode {
    /// Core wallet + Platform address sync
    #[default]
    All,
    /// Only refresh Core wallet balances
    CoreOnly,
    /// Only Platform address sync
    PlatformOnly,
}

impl RefreshMode {
    fn label(&self) -> &'static str {
        match self {
            RefreshMode::All => "Core + Platform",
            RefreshMode::CoreOnly => "Core Only",
            RefreshMode::PlatformOnly => "Platform Only",
        }
    }

    fn next(self) -> Self {
        match self {
            RefreshMode::All => RefreshMode::CoreOnly,
            RefreshMode::CoreOnly => RefreshMode::PlatformOnly,
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
    rename_input: String,
    wallet_unlock_popup: WalletUnlockPopup,
    show_sk_unlock_dialog: bool,
    sk_password_input: PasswordInput,
    remove_wallet_dialog: Option<ConfirmationDialog>,
    pending_wallet_removal: Option<WalletSeedHash>,
    pending_wallet_removal_alias: Option<String>,
    send_dialog: SendDialogState,
    receive_dialog: ReceiveDialogState,
    fund_platform_dialog: FundPlatformAddressDialogState,
    private_key_dialog: PrivateKeyDialogState,
    mine_dialog: MineDialogState,
    show_zero_balance_addresses: bool,
    /// Pending refresh of platform address balances (triggered after transfers)
    pending_platform_balance_refresh: Option<WalletSeedHash>,
    /// Whether we should refresh the wallet after it's unlocked
    pending_refresh_after_unlock: bool,
    /// The refresh mode to use after unlock (if pending_refresh_after_unlock is true)
    pending_refresh_mode: RefreshMode,
    /// Whether we should search for asset locks after wallet is unlocked
    pending_asset_lock_search_after_unlock: bool,
    /// Banner handle for asset lock search progress
    asset_lock_search_banner: Option<BannerHandle>,
    /// Current page for single key wallet UTXO pagination (0-indexed)
    utxo_page: usize,
    /// Selected refresh mode (only shown in dev mode)
    refresh_mode: RefreshMode,
    /// Currently selected account tab in the Accounts & Addresses section
    selected_account_tab: AccountTab,
    /// Shielded tab view component (lazily initialized per wallet)
    shielded_tab_view: Option<ShieldedTabView>,
    /// Cached platform sync info: (last_sync_timestamp, last_sync_height)
    platform_sync_info: Option<(u64, u64)>,
    /// Core wallet selection dialog (shown when auto-detection fails)
    core_wallet_dialog: Option<SelectionDialog>,
    /// Seed/key hash of the wallet pending Core wallet selection
    pending_core_wallet_seed_hash: Option<[u8; 32]>,
    /// Core wallet options for the pending selection
    pending_core_wallet_options: Option<Vec<String>>,
    /// Whether the pending Core wallet selection is for a single-key wallet
    pending_core_wallet_is_single_key: bool,
    /// Whether a wallet switch should trigger a Core refresh on the next frame
    pending_wallet_refresh_on_switch: bool,
    /// Whether we need to fire a ListCoreWallets backend task (set on CoreWalletNotConfigured error)
    pending_list_core_wallets: bool,
    /// Wallet hash pending the ListCoreWallets response
    pending_list_wallet_hash: Option<[u8; 32]>,
    /// Whether the wallet pending list is a single-key wallet
    pending_list_is_single_key: bool,
    /// Cached filtered transaction indices for the currently selected wallet.
    /// Invalidated (set to None) on wallet switch or transaction updates.
    cached_tx_indices: Option<Vec<usize>>,
    /// Whether a Core receive address generation is in progress (disables button)
    generating_core_address: bool,
}

impl WalletsBalancesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        // Try to restore previously selected wallet from AppContext
        let (selected_wallet, selected_single_key_wallet) = {
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

            // If we have a persisted single key selection, try to find it
            if let Some(sk_hash) = selected_sk_hash
                && let Ok(sk_wallets) = app_context.single_key_wallets.read()
                && let Some(wallet) = sk_wallets.get(&sk_hash)
            {
                return Self::create_with_selection(app_context, None, Some(wallet.clone()));
            }

            // If we have a persisted HD wallet selection, try to find it
            if let Some(hd_hash) = selected_hd_hash
                && let Ok(wallets) = app_context.wallets.read()
                && let Some(wallet) = wallets.get(&hd_hash)
            {
                return Self::create_with_selection(app_context, Some(wallet.clone()), None);
            }

            // Default: try HD wallet first, then single key wallet
            let hd_wallet = app_context.wallets.read().unwrap().values().next().cloned();
            let sk_wallet = if hd_wallet.is_none() {
                app_context
                    .single_key_wallets
                    .read()
                    .unwrap()
                    .values()
                    .next()
                    .cloned()
            } else {
                None
            };
            (hd_wallet, sk_wallet)
        };

        Self::create_with_selection(app_context, selected_wallet, selected_single_key_wallet)
    }

    fn create_with_selection(
        app_context: &Arc<AppContext>,
        selected_wallet: Option<Arc<RwLock<Wallet>>>,
        selected_single_key_wallet: Option<Arc<RwLock<SingleKeyWallet>>>,
    ) -> Self {
        let platform_sync_info = selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok().map(|g| g.seed_hash()))
            .and_then(|hash| app_context.db.get_platform_sync_info(&hash).ok())
            .filter(|(ts, _)| *ts > 0);

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
            rename_input: String::new(),
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_sk_unlock_dialog: false,
            sk_password_input: PasswordInput::new().with_hint_text("Enter password"),
            remove_wallet_dialog: None,
            pending_wallet_removal: None,
            pending_wallet_removal_alias: None,
            send_dialog: SendDialogState::default(),
            receive_dialog: ReceiveDialogState::default(),
            fund_platform_dialog: FundPlatformAddressDialogState::default(),
            private_key_dialog: PrivateKeyDialogState::default(),
            mine_dialog: MineDialogState::default(),
            show_zero_balance_addresses: false,
            pending_platform_balance_refresh: None,
            pending_refresh_after_unlock: false,
            pending_refresh_mode: RefreshMode::default(),
            pending_asset_lock_search_after_unlock: false,
            asset_lock_search_banner: None,
            utxo_page: 0,
            refresh_mode: RefreshMode::default(),
            selected_account_tab: AccountTab::default(),
            shielded_tab_view,
            platform_sync_info,
            core_wallet_dialog: None,
            pending_core_wallet_seed_hash: None,
            pending_core_wallet_options: None,
            pending_core_wallet_is_single_key: false,
            pending_wallet_refresh_on_switch: false,
            pending_list_core_wallets: false,
            pending_list_wallet_hash: None,
            pending_list_is_single_key: false,
            cached_tx_indices: None,
            generating_core_address: false,
        }
    }

    fn persist_selected_wallet_hash(&self, hash: Option<WalletSeedHash>) {
        if let Ok(mut guard) = self.app_context.selected_wallet_hash.lock() {
            *guard = hash;
        }
        let _ = self
            .app_context
            .db
            .update_selected_wallet_hash(hash.as_ref());
    }

    fn persist_selected_single_key_hash(&self, hash: Option<[u8; 32]>) {
        if let Ok(mut guard) = self.app_context.selected_single_key_hash.lock() {
            *guard = hash;
        }
        let _ = self
            .app_context
            .db
            .update_selected_single_key_hash(hash.as_ref());
    }

    /// Persist the selected Core wallet name to the DB and in-memory wallet.
    ///
    /// Returns `Ok(())` on success or `Err` with a user-facing message on failure.
    fn apply_core_wallet_selection(
        &mut self,
        wallet_hash: &[u8; 32],
        wallet_name: &str,
        is_single_key: bool,
    ) -> Result<(), String> {
        if !is_single_key {
            match self
                .app_context
                .db
                .set_wallet_core_wallet_name(wallet_hash, Some(wallet_name))
            {
                Ok(false) => {
                    return Err("Wallet not found in database".to_string());
                }
                Err(e) => {
                    return Err(format!("Failed to save Dash Core wallet: {e}"));
                }
                Ok(true) => {}
            }
            if let Ok(wallets) = self.app_context.wallets.read()
                && let Some(w) = wallets.get(wallet_hash)
                && let Ok(mut guard) = w.write()
            {
                guard.core_wallet_name = Some(wallet_name.to_string());
            }
        } else {
            match self
                .app_context
                .db
                .set_single_key_wallet_core_wallet_name(wallet_hash, Some(wallet_name))
            {
                Ok(false) => {
                    return Err("Wallet not found in database".to_string());
                }
                Err(e) => {
                    return Err(format!("Failed to save Dash Core wallet: {e}"));
                }
                Ok(true) => {}
            }
            if let Ok(skw) = self.app_context.single_key_wallets.read()
                && let Some(w) = skw.get(wallet_hash)
                && let Ok(mut guard) = w.write()
            {
                guard.core_wallet_name = Some(wallet_name.to_string());
            }
        }

        Ok(())
    }

    /// Refresh the cached platform sync info from the database.
    fn refresh_platform_sync_info_cache(&mut self, seed_hash: &WalletSeedHash) {
        self.platform_sync_info = self
            .app_context
            .db
            .get_platform_sync_info(seed_hash)
            .ok()
            .filter(|(ts, _)| *ts > 0);
    }

    /// Set the selected HD wallet and update all associated state (persisted
    /// hash, platform sync info cache).  All code paths that change
    /// `selected_wallet` should go through this helper to keep the sync
    /// status panel consistent.
    fn set_selected_hd_wallet(&mut self, wallet: Option<Arc<RwLock<Wallet>>>) {
        let seed_hash = wallet
            .as_ref()
            .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
        self.selected_wallet = wallet;
        self.selected_single_key_wallet = None;

        self.selected_account_tab = AccountTab::default();
        self.cached_tx_indices = None;

        self.shielded_tab_view =
            seed_hash.map(|hash| ShieldedTabView::new(&self.app_context, hash));

        if let Some(hash) = seed_hash {
            self.persist_selected_wallet_hash(Some(hash));
            self.refresh_platform_sync_info_cache(&hash);
            // Trigger a refresh on the next frame for the newly selected wallet
            if self.app_context.core_backend_mode() == CoreBackendMode::Rpc {
                self.pending_wallet_refresh_on_switch = true;
            }
        } else {
            self.persist_selected_wallet_hash(None);
            self.platform_sync_info = None;
        }
    }

    fn select_hd_wallet(&mut self, wallet: Arc<RwLock<Wallet>>) {
        self.set_selected_hd_wallet(Some(wallet));
        self.persist_selected_single_key_hash(None);
    }

    fn select_single_key_wallet(&mut self, wallet: Arc<RwLock<SingleKeyWallet>>) {
        self.selected_single_key_wallet = Some(wallet.clone());
        self.selected_wallet = None;

        self.platform_sync_info = None;
        self.utxo_page = 0;

        if let Ok(hash) = wallet.read().map(|g| g.key_hash) {
            self.persist_selected_single_key_hash(Some(hash));
        }
        self.persist_selected_wallet_hash(None);
    }

    pub(crate) fn update_selected_wallet_for_network(&mut self) {
        // Check if HD wallet selection is still valid
        if let Some(wallet_arc) = &self.selected_wallet {
            let seed_hash = wallet_arc.read().ok().map(|w| w.seed_hash());
            if let Some(hash) = seed_hash
                && let Ok(wallets) = self.app_context.wallets.read()
                && wallets.contains_key(&hash)
            {
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

            self.platform_sync_info = None;
            return;
        }

        self.platform_sync_info = None;
    }

    pub(crate) fn reset_pending_list_state(&mut self) {
        self.pending_list_core_wallets = false;
        self.pending_list_wallet_hash = None;
        self.pending_list_is_single_key = false;
    }

    /// Reset all cached AddressInput widgets so they pick up the new network.
    pub(crate) fn invalidate_address_inputs(&mut self) {
        self.mine_dialog.address_input = None;
        self.mine_dialog.validated_address = None;
        self.cached_tx_indices = None;
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
                let guard = wallet.read().unwrap();
                let core_balance = guard.total_balance_duffs();
                let platform_balance = Self::platform_balance_duffs(&guard);
                let shielded_balance = self.shielded_balance_duffs(&guard.seed_hash());
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
                let guard = wallet.read().unwrap();
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
                    let core = g.total_balance_duffs();
                    let platform = Self::platform_balance_duffs(&g);
                    let shielded = self.shielded_balance_duffs(&g.seed_hash());
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
                        DashColors::text_primary(ui.ctx().style().visuals.dark_mode),
                        format!(" Balance: {}", Self::format_dash(current_balance)),
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
                            self.show_rename_dialog = true;
                            self.rename_input = alias.unwrap_or_default();
                        }
                    }

                    // Buttons for single key wallet
                    if let Some(wallet_arc) = single_key_wallet_opt {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let (key_hash, alias) = wallet_arc
                            .read()
                            .ok()
                            .map(|w| (w.key_hash, w.alias.clone()))
                            .unwrap_or(([0u8; 32], None));

                        // Remove button (styled red like HD wallet)
                        let remove_button = egui::Button::new(
                            RichText::new("Remove").color(Color32::WHITE).size(14.0),
                        )
                        .min_size(egui::vec2(0.0, 28.0))
                        .fill(DashColors::error_color(!dark_mode))
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(4.0);

                        if ui.add(remove_button).clicked() {
                            if let Err(e) = self
                                .app_context
                                .db
                                .remove_single_key_wallet(&key_hash, self.app_context.network)
                            {
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    format!("Failed to remove: {}", e),
                                    MessageType::Error,
                                );
                            } else {
                                if let Ok(mut wallets) = self.app_context.single_key_wallets.write()
                                {
                                    wallets.remove(&key_hash);
                                }
                                self.selected_single_key_wallet = None;
                                // Clear persisted selection in AppContext and database
                                self.persist_selected_single_key_hash(None);
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    "Wallet removed",
                                    MessageType::Success,
                                );
                            }
                        }

                        ui.add_space(8.0);

                        // Lock/Unlock buttons for SK wallet
                        let (uses_password, is_open) = wallet_arc
                            .read()
                            .ok()
                            .map(|w| (w.uses_password, w.is_open()))
                            .unwrap_or((false, false));

                        let mut should_lock_sk_wallet = false;
                        if uses_password {
                            if is_open {
                                if ui.button("Lock").clicked() {
                                    should_lock_sk_wallet = true;
                                }
                            } else if ui.button("Unlock").clicked() {
                                self.show_sk_unlock_dialog = true;
                            }
                        }
                        if should_lock_sk_wallet && let Ok(mut wallet) = wallet_arc.write() {
                            wallet.private_key_data.close();
                        }

                        ui.add_space(8.0);

                        // Rename button
                        if ui.button("Rename").clicked() {
                            self.show_rename_dialog = true;
                            self.rename_input = alias.unwrap_or_default();
                        }
                    }
                });
            },
        );

        action
    }

    fn render_bottom_options(
        &mut self,
        ui: &mut Ui,
        account_filter: &(AccountCategory, Option<u32>),
    ) -> AppAction {
        let mut action = AppAction::None;

        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|wallet_guard| wallet_guard.read().unwrap().is_open());

        if !wallet_is_open {
            return action;
        }

        let is_bip44 = account_filter.0 == AccountCategory::Bip44;
        let is_platform = account_filter.0 == AccountCategory::PlatformPayment;

        if is_bip44 {
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let button = egui::Button::new(RichText::new("+ New Receive Address").size(13.0))
                    .min_size(egui::vec2(0.0, 24.0));
                if ui
                    .add_enabled(!self.generating_core_address, button)
                    .clicked()
                    && let Some(wallet) = &self.selected_wallet
                {
                    let seed_hash = wallet.read().unwrap().seed_hash();
                    self.generating_core_address = true;
                    action = AppAction::BackendTask(BackendTask::WalletTask(
                        crate::backend_task::wallet::WalletTask::GenerateReceiveAddress {
                            seed_hash,
                        },
                    ));
                }
            });
        } else if is_platform {
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let button = egui::Button::new(RichText::new("+ New Platform Address").size(13.0))
                    .min_size(egui::vec2(0.0, 24.0));
                if ui.add(button).clicked() {
                    self.add_new_platform_address();
                }
            });
        }

        action
    }

    fn add_new_platform_address(&mut self) {
        if let Some(wallet) = &self.selected_wallet {
            let result = {
                let mut wallet = wallet.write().unwrap();
                wallet.platform_receive_address(
                    self.app_context.network,
                    true,
                    Some(&self.app_context),
                )
            };
            match result {
                Ok(address) => {
                    use dash_sdk::dpp::address_funds::PlatformAddress;
                    let display = PlatformAddress::try_from(address)
                        .map(|pa| pa.to_bech32m_string(self.app_context.network))
                        .unwrap_or_else(|_| "new address".to_string());
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        format!("New Platform address generated: {display}"),
                        MessageType::Success,
                    );
                }
                Err(e) => {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Could not generate a new Platform address. Please try again.",
                        MessageType::Error,
                    )
                    .with_details(e);
                }
            }
        }
    }

    fn render_remove_wallet_button(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        if let Some(selected_wallet) = &self.selected_wallet {
            let remove_button =
                egui::Button::new(RichText::new("Remove").color(Color32::WHITE).size(14.0))
                    .min_size(egui::vec2(0.0, 28.0))
                    .fill(DashColors::error_color(!dark_mode))
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(4.0);

            if ui.add(remove_button).clicked() {
                let wallet = selected_wallet.read().unwrap();
                let alias = wallet
                    .alias
                    .clone()
                    .unwrap_or_else(|| "Unnamed Wallet".to_string());
                let seed_hash = wallet.seed_hash();
                drop(wallet);

                self.pending_wallet_removal = Some(seed_hash);
                self.pending_wallet_removal_alias = Some(alias.clone());

                let message = format!(
                    "Removing wallet \"{}\" will delete its local data, including addresses, balances, and asset locks stored on this device. Identities linked to it will remain but the keys derived from this wallet will no longer work unless the wallet is re-imported. Continue?",
                    alias
                );

                self.remove_wallet_dialog = Some(
                    ConfirmationDialog::new("Remove Wallet", message)
                        .confirm_text(Some("Remove"))
                        .cancel_text(Some("Cancel"))
                        .danger_mode(true),
                );
            }
        }

        if let Some(dialog) = self.remove_wallet_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(status) = response.inner.dialog_response {
                match status {
                    ConfirmationStatus::Confirmed => {
                        self.remove_wallet_dialog = None;
                        if let Some(seed_hash) = self.pending_wallet_removal.take() {
                            let alias = self
                                .pending_wallet_removal_alias
                                .take()
                                .unwrap_or_else(|| "Unnamed Wallet".to_string());
                            self.handle_wallet_removal(seed_hash, alias);
                        } else {
                            self.pending_wallet_removal_alias = None;
                        }
                    }
                    ConfirmationStatus::Canceled => {
                        self.remove_wallet_dialog = None;
                        self.pending_wallet_removal = None;
                        self.pending_wallet_removal_alias = None;
                    }
                }
            }
        }
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
        // Optionally put everything in a framed "card"-like container
        Frame::group(ui.style())
            .fill(ui.visuals().extreme_bg_color) // background color
            .corner_radius(5.0) // rounded corners
            .outer_margin(Margin::same(20)) // space around the frame
            .shadow(ui.visuals().window_shadow) // drop shadow
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    // Heading
                    ui.add_space(5.0);
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.label(
                        RichText::new("No Wallets Loaded")
                            .strong()
                            .size(25.0)
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    // A separator line for visual clarity
                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Description
                    ui.label("It looks like you are not tracking any wallets yet.");

                    ui.add_space(10.0);

                    // Subheading or emphasis
                    ui.heading(
                        RichText::new("Here’s what you can do:")
                            .strong()
                            .size(18.0)
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(5.0);

                    // Bullet points
                    ui.label(
                        "• IMPORT a Dash wallet by clicking \
                         on \"Import Wallet\" at the top right, or",
                    );
                    ui.add_space(1.0);
                    ui.label(
                        "• CREATE a new Dash wallet by clicking \
                         on \"Create Wallet\".",
                    );

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Footnote or extra info
                    ui.label(
                        "(Make sure Dash Core is running. You can check in the \
                         network tab on the left.)",
                    );

                    ui.add_space(5.0);
                });
            });
    }

    fn format_dash(amount_duffs: u64) -> String {
        Amount::dash_from_duffs(amount_duffs).to_string()
    }

    /// Format a `std::time::Instant` as a relative "time ago" string.
    fn format_instant_ago(instant: std::time::Instant) -> String {
        Self::format_duration_ago(instant.elapsed())
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
        let amount = Self::format_dash(tx.amount_abs());
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
        DateTime::<Utc>::from_timestamp(ts as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    fn platform_balance_duffs(wallet: &Wallet) -> u64 {
        // Only sum Platform address balances
        // Identity balances are shown separately on the Identities screen
        wallet
            .platform_address_info
            .values()
            .map(|info| info.balance / CREDITS_PER_DUFF)
            .sum()
    }

    fn shielded_balance_duffs(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.app_context
            .shielded_states
            .lock()
            .ok()
            .and_then(|states| states.get(seed_hash).map(|s| s.shielded_balance))
            .unwrap_or(0)
            / CREDITS_PER_DUFF
    }

    fn render_action_buttons(&mut self, ui: &mut Ui, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        ui.add_space(10.0);
        let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                        crate::ui::ScreenType::WalletSendScreen(wallet.clone())
                            .create_screen(&self.app_context),
                    );
                } else if let Some(sk_wallet) = &self.selected_single_key_wallet {
                    action = AppAction::AddScreen(
                        crate::ui::ScreenType::SingleKeyWalletSendScreen(sk_wallet.clone())
                            .create_screen(&self.app_context),
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

            // Dev-mode buttons: right-aligned, filling all remaining space
            if self.app_context.is_developer_mode() {
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
                        ) && self.app_context.core_backend_mode() == CoreBackendMode::Rpc
                            && ui
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
        let developer_mode = self.app_context.is_developer_mode();
        let mut tabs: Vec<AccountTab> = Vec::new();

        // Always-visible primary tabs: all BIP44 accounts and Platform
        for summary in summaries {
            if !summary.category.is_visible_in_default_mode() {
                continue;
            }
            tabs.push(AccountTab::Category(
                summary.category.clone(),
                summary.index,
            ));
        }

        // Ensure Dash Core tab exists even without summaries
        if !tabs
            .iter()
            .any(|t| matches!(t, AccountTab::Category(AccountCategory::Bip44, Some(0))))
        {
            tabs.insert(0, AccountTab::Category(AccountCategory::Bip44, Some(0)));
        }

        // Always add the Shielded tab
        tabs.push(AccountTab::Shielded);

        // In developer mode, add the consolidated System tab last
        if developer_mode {
            tabs.push(AccountTab::System);
        }

        tabs
    }

    /// Collect the system account categories to display inside the System tab.
    /// Returns `(category, index, address_count, balance_duffs)` tuples in a
    /// fixed display order (identity categories first, then provider, then legacy).
    /// Each `(category, index)` pair gets its own section with accurate counts.
    fn system_tab_sections(
        &self,
        summaries: &[AccountSummary],
    ) -> Vec<(AccountCategory, Option<u32>, usize, u64)> {
        let category_order: &[AccountCategory] = &[
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

        // Precompute per-(category, index) address counts in a single pass.
        let address_counts = self.precompute_address_counts();

        let mut sections = Vec::new();

        // For each category, emit one section per distinct index found in
        // summaries. Categories with no summary entries get a single section
        // with index from the first matching summary (or None).
        for cat in category_order {
            let matching: Vec<_> = summaries.iter().filter(|s| &s.category == cat).collect();
            if matching.is_empty() {
                let address_count = address_counts
                    .get(&(cat.clone(), None))
                    .copied()
                    .unwrap_or(0);
                sections.push((cat.clone(), None, address_count, 0u64));
            } else {
                for summary in &matching {
                    let key = (cat.clone(), summary.index);
                    let address_count = address_counts.get(&key).copied().unwrap_or(0);
                    sections.push((
                        cat.clone(),
                        summary.index,
                        address_count,
                        summary.confirmed_balance,
                    ));
                }
            }
        }

        // Also include any Other(...) categories from summaries
        for summary in summaries {
            if matches!(summary.category, AccountCategory::Other(_))
                && !sections
                    .iter()
                    .any(|(c, idx, _, _)| *c == summary.category && *idx == summary.index)
            {
                let key = (summary.category.clone(), summary.index);
                let address_count = address_counts.get(&key).copied().unwrap_or(0);
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

    /// Build a per-(category, index) address count map in a single pass over
    /// `watched_addresses`. Used by `system_tab_sections` to avoid
    /// O(num_categories * num_addresses) per frame.
    fn precompute_address_counts(
        &self,
    ) -> std::collections::HashMap<(AccountCategory, Option<u32>), usize> {
        let mut counts = std::collections::HashMap::new();
        let Some(wallet_arc) = self.selected_wallet.as_ref() else {
            return counts;
        };
        let Ok(wallet) = wallet_arc.read() else {
            return counts;
        };
        let network = self.app_context.network;
        for (path, info) in &wallet.watched_addresses {
            let (cat, idx) = crate::ui::wallets::account_summary::categorize_account_path(
                path,
                network,
                info.path_reference,
            );
            *counts.entry((cat, idx)).or_insert(0) += 1;
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
        let dark_mode = ui.ctx().style().visuals.dark_mode;

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
                            summaries
                                .iter()
                                .filter(|s| s.category == *cat && s.index == *idx)
                                .map(|s| s.platform_credits / CREDITS_PER_DUFF)
                                .sum::<u64>()
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
                        let balance: u64 = summaries
                            .iter()
                            .filter(|s| s.category.is_system_category())
                            .map(|s| s.confirmed_balance)
                            .sum();
                        ("System".to_string(), balance)
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
                // Show empty state if no summaries match this category
                if !summaries
                    .iter()
                    .any(|s| s.category == cat && s.index == idx)
                    && !matches!(cat, AccountCategory::Bip44)
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

                let account_filter = (cat.clone(), idx);

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
                    action |= self.render_address_table(ui, account_filter.clone());
                    action |= self.render_bottom_options(ui, &account_filter);
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
    fn render_system_tab_content(
        &mut self,
        ui: &mut Ui,
        summaries: &[AccountSummary],
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let sections = self.system_tab_sections(summaries);

        for (cat, idx, addr_count, balance) in &sections {
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

        // Defensive check: verify the selected wallet Arc matches the one in
        // app_context.wallets. If they diverge (stale reference), skip rendering
        // to avoid showing another wallet's data.
        let wallet_guard = wallet_arc.read().unwrap();
        let selected_seed_hash = wallet_guard.seed_hash();
        let arc_matches = self
            .app_context
            .wallets
            .read()
            .ok()
            .and_then(|wallets| wallets.get(&selected_seed_hash).cloned())
            .is_some_and(|canonical| Arc::ptr_eq(wallet_arc, &canonical));
        if !arc_matches {
            tracing::warn!(
                "selected_wallet Arc does not match app_context.wallets — skipping transaction render"
            );
            ui.label("Wallet data is being updated. Please re-select the wallet.");
            return;
        }

        if wallet_guard.transactions.is_empty() {
            ui.label(
                "No transactions found. Try refreshing your wallet to load transaction history.",
            );
            return;
        }

        // Filter to transactions involving this wallet's addresses.
        // The `is_ours` flag is set by both RPC and SPV paths for all
        // transactions that belong to this wallet (sends and receives).
        let relevant_indices = self.cached_tx_indices.get_or_insert_with(|| {
            (0..wallet_guard.transactions.len())
                .filter(|&i| wallet_guard.transactions[i].is_ours)
                .collect()
        });

        if relevant_indices.is_empty() {
            ui.label(
                "No transactions found. Try refreshing your wallet to load transaction history.",
            );
            return;
        }

        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let show_fee = self.app_context.is_developer_mode();
        let mut order: Vec<usize> = relevant_indices.clone();
        order.sort_by(|&a, &b| {
            wallet_guard.transactions[b]
                .timestamp
                .cmp(&wallet_guard.transactions[a].timestamp)
                .then_with(|| {
                    wallet_guard.transactions[b]
                        .txid
                        .cmp(&wallet_guard.transactions[a].txid)
                })
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
                    let tx = &wallet_guard.transactions[idx];
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
                                    .map(Self::format_dash)
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
        let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                    match self.app_context.core_backend_mode() {
                        CoreBackendMode::Rpc => {
                            if self.app_context.connection_status().rpc_online() {
                                ui.colored_label(
                                    Color32::DARK_GREEN,
                                    RichText::new("Connected").size(sz),
                                );
                            } else {
                                ui.colored_label(
                                    DashColors::ERROR,
                                    RichText::new("Disconnected").size(sz),
                                );
                            }
                        }
                        CoreBackendMode::Spv => {
                            let snapshot = self.app_context.spv_manager().status();
                            match snapshot.status {
                                SpvStatus::Idle | SpvStatus::Stopped => {
                                    ui.label(
                                        RichText::new("Disconnected").size(sz).color(secondary),
                                    );
                                }
                                SpvStatus::Starting => {
                                    ui.add(egui::Spinner::new().size(sz).color(syncing_color));
                                    ui.label(
                                        RichText::new("Connecting...")
                                            .size(sz)
                                            .color(syncing_color),
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
                    }
                });

                // -- Platform: Addresses --
                let addr_count = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok())
                    .map(|w| w.platform_address_info.len())
                    .unwrap_or(0);
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
                    let addr_text =
                        if let Some((last_sync_ts, sync_height)) = self.platform_sync_info {
                            let ago = Self::format_unix_time_ago(last_sync_ts);
                            format!(
                                "Addresses: {} synced (blk {}, {})",
                                addr_count, sync_height, ago
                            )
                        } else {
                            "Addresses: never synced".to_string()
                        };
                    ui.label(RichText::new(addr_text).size(sz).color(addr_color));
                });

                // -- Shielded: Notes + Nullifiers --
                let seed_hash = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
                let shielded_info = seed_hash.and_then(|hash| {
                    let states = self.app_context.shielded_states.lock().ok()?;
                    let state = states.get(&hash)?;
                    Some((
                        state.last_synced_index,
                        state.notes.iter().filter(|n| !n.is_spent).count(),
                        state.last_nullifier_sync_height,
                        state.last_notes_synced_at,
                        state.last_nullifiers_synced_at,
                    ))
                });
                let shielded_syncing = self
                    .shielded_tab_view
                    .as_ref()
                    .is_some_and(|v| v.is_syncing());
                let shielded_color = if shielded_syncing {
                    syncing_color
                } else {
                    secondary
                };

                match shielded_info {
                    Some((synced_index, note_count, nf_height, notes_synced_at, nf_synced_at)) => {
                        // Notes bullet
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("•").size(sz).color(secondary));
                            if shielded_syncing {
                                ui.add(egui::Spinner::new().size(sz).color(syncing_color));
                            }
                            let notes_text = if let Some(t) = notes_synced_at {
                                let ago = Self::format_instant_ago(t);
                                format!(
                                    "Notes: {} synced ({} notes, {})",
                                    synced_index, note_count, ago
                                )
                            } else if synced_index > 0 {
                                format!("Notes: {} synced ({} notes)", synced_index, note_count)
                            } else {
                                "Notes: never synced".to_string()
                            };
                            ui.label(RichText::new(notes_text).size(sz).color(shielded_color));
                        });
                        // Nullifiers bullet
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("•").size(sz).color(secondary));
                            let nf_text = if let Some(t) = nf_synced_at {
                                let ago = Self::format_instant_ago(t);
                                format!("Nullifiers: height {} ({})", nf_height, ago)
                            } else if nf_height > 0 {
                                format!("Nullifiers: height {}", nf_height)
                            } else {
                                "Nullifiers: never synced".to_string()
                            };
                            ui.label(RichText::new(nf_text).size(sz).color(shielded_color));
                        });
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("•").size(sz).color(secondary));
                            ui.label(
                                RichText::new("Notes: never synced")
                                    .size(sz)
                                    .color(secondary),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("•").size(sz).color(secondary));
                            ui.label(
                                RichText::new("Nullifiers: never synced")
                                    .size(sz)
                                    .color(secondary),
                            );
                        });
                    }
                }
            },
        );
    }

    /// Render the total balance label only (used in the left column of the header).
    fn render_balance_total(&self, ui: &mut Ui, wallet: &Wallet) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let core_balance = wallet.total_balance_duffs();
        let platform_balance = Self::platform_balance_duffs(wallet);
        let shielded_balance = self.shielded_balance_duffs(&wallet.seed_hash());
        let total = core_balance + platform_balance + shielded_balance;

        ui.label(
            RichText::new(format!("Balance: {}", Self::format_dash(total)))
                .color(DashColors::text_primary(dark_mode))
                .size(20.0)
                .strong(),
        );
    }

    /// Render the collapsible breakdown detail (used in the right column of the header).
    fn render_balance_breakdown_detail(&mut self, ui: &mut Ui, wallet: &Wallet) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let core_balance = wallet.total_balance_duffs();
        let platform_balance = Self::platform_balance_duffs(wallet);
        let shielded_balance = self.shielded_balance_duffs(&wallet.seed_hash());

        let header = egui::CollapsingHeader::new(
            RichText::new("Balance breakdown")
                .size(13.0)
                .color(DashColors::text_secondary(dark_mode)),
        )
        .id_salt("balance_breakdown")
        .default_open(self.app_context.is_developer_mode());

        header.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Core: {}", Self::format_dash(core_balance)));
                ui.label(" | ");
                ui.label(format!("Platform: {}", Self::format_dash(platform_balance)));
                ui.label(" | ");
                ui.label(format!("Shielded: {}", Self::format_dash(shielded_balance)));
            });
        });
    }

    fn render_wallet_detail_panel(&mut self, ui: &mut Ui, ctx: &Context) -> AppAction {
        let Some(wallet_arc) = self.selected_wallet.clone() else {
            self.render_no_wallets_view(ui);
            return AppAction::None;
        };

        let (alias, _seed_hash, _wallet_is_main) = {
            let wallet = wallet_arc.read().unwrap();
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
        let dark_mode = ui.ctx().style().visuals.dark_mode;

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
                                    if self.app_context.is_developer_mode() {
                                        ui.label(
                                            RichText::new("[DEV]")
                                                .color(DashColors::text_secondary(dark_mode))
                                                .size(12.0),
                                        );
                                    }
                                });

                                // Total balance line
                                {
                                    let wallet = wallet_arc.read().unwrap();
                                    self.render_balance_total(ui, &wallet);
                                }
                            });

                            // RIGHT COLUMN: balance breakdown + sync status, right-aligned
                            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                                ui.set_width(right_width);

                                // Collapsible balance breakdown
                                {
                                    let wallet = wallet_arc.read().unwrap();
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
                            let wallet = wallet_arc.read().unwrap();
                            collect_account_summaries(&wallet, self.app_context.network)
                        };
                        self.ensure_account_selection(&summaries);
                        action |= self.render_account_tabs(ui, &summaries);
                    });
            });
        });

        action
    }

    fn ensure_account_selection(&mut self, _summaries: &[AccountSummary]) {
        // The tab bar in `render_account_tabs` already validates
        // `selected_account_tab` against the built tab list and resets it
        // to the first tab if invalid. Nothing extra needed here.
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

    /// Returns a SyncNotes backend task if the shielded wallet has been initialized
    /// for the given seed hash.
    fn shielded_sync_task(&self, seed_hash: &WalletSeedHash) -> Option<BackendTask> {
        let states = self.app_context.shielded_states.lock().ok()?;
        if states.contains_key(seed_hash) {
            Some(BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
                seed_hash: *seed_hash,
            }))
        } else {
            None
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
            RefreshMode::CoreOnly => {
                // Core only, no Platform sync
                BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet_arc.clone(), false))
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

        // Also trigger shielded note sync if initialized
        if let Some(shielded_task) = self.shielded_sync_task(&seed_hash) {
            AppAction::BackendTasks(
                vec![core_task, shielded_task],
                BackendTasksExecutionMode::Concurrent,
            )
        } else {
            AppAction::BackendTask(core_task)
        }
    }
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
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
                "Create Wallet",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
            ),
        ];

        // Add Refresh button for HD wallet
        if !self.refreshing
            && self.app_context.core_backend_mode() == CoreBackendMode::Rpc
            && self.selected_wallet.is_some()
        {
            right_buttons.push((
                "Refresh",
                DesiredAppAction::Custom("RefreshHDWallet".to_string()),
            ));
        }

        // Add Refresh button for single key wallet
        if !self.refreshing
            && self.app_context.core_backend_mode() == CoreBackendMode::Rpc
            && self.selected_single_key_wallet.is_some()
        {
            right_buttons.push((
                "Refresh",
                DesiredAppAction::Custom("RefreshSKWallet".to_string()),
            ));
        }
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Wallets", AppAction::None)],
            right_buttons,
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // Message display is handled by the global MessageBanner

            egui::ScrollArea::vertical()
                .auto_shrink([true; 2])
                .show(ui, |ui| {
                    let has_hd_wallets = !self.app_context.wallets.read().unwrap().is_empty();
                    let has_single_key_wallets = !self
                        .app_context
                        .single_key_wallets
                        .read()
                        .unwrap()
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

        action |= self.render_send_dialog(ctx);
        action |= self.render_receive_dialog(ctx);
        action |= self.render_fund_platform_dialog(ctx);
        action |= self.render_mine_dialog(ctx);
        self.render_private_key_dialog(ctx);

        // Rename dialog
        if self.show_rename_dialog {
            let window_response = egui::Window::new("Rename Wallet")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                                    let mut wallet = selected_wallet.write().unwrap();
                                    wallet.alias = Some(self.rename_input.clone());

                                    // Update the alias in the database
                                    let seed_hash = wallet.seed_hash();
                                    self.app_context
                                        .db
                                        .set_wallet_alias(
                                            &seed_hash,
                                            Some(self.rename_input.clone()),
                                        )
                                        .ok();
                                }
                                // Handle single key wallet rename
                                else if let Some(selected_sk_wallet) =
                                    &self.selected_single_key_wallet
                                {
                                    let mut wallet = selected_sk_wallet.write().unwrap();
                                    wallet.alias = Some(self.rename_input.clone());

                                    // Update the alias in the database
                                    let key_hash = wallet.key_hash;
                                    self.app_context
                                        .db
                                        .update_single_key_wallet_alias(
                                            &key_hash,
                                            Some(&self.rename_input),
                                        )
                                        .ok();
                                }

                                self.show_rename_dialog = false;
                                self.rename_input.clear();
                            }
                        });
                    });
                });

            if let Some(ref resp) = window_response
                && clicked_outside_window(ctx, resp.response.rect)
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
                        match self.derive_private_key_wif(&path) {
                            Ok(key) => {
                                self.private_key_dialog.is_open = true;
                                self.private_key_dialog.address = address;
                                self.private_key_dialog.private_key_wif = key;
                                self.private_key_dialog.show_key = false;
                            }
                            Err(err) => {
                                MessageBanner::set_global(ctx, &err, MessageType::Error);
                            }
                        }
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

                    // Check if we were trying to search for asset locks
                    if self.pending_asset_lock_search_after_unlock {
                        self.pending_asset_lock_search_after_unlock = false;
                        if let Some(wallet_arc) = self.selected_wallet.clone() {
                            self.asset_lock_search_banner.take_and_clear();
                            let handle = MessageBanner::set_global(
                                ctx,
                                "Searching for unused asset locks...",
                                MessageType::Info,
                            );
                            handle.with_elapsed();
                            self.asset_lock_search_banner = Some(handle);
                            action |= AppAction::BackendTask(BackendTask::CoreTask(
                                CoreTask::RecoverAssetLocks(wallet_arc),
                            ));
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

                    // Clear pending asset lock search on cancel
                    self.pending_asset_lock_search_after_unlock = false;
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
                            let dark_mode = ui.ctx().style().visuals.dark_mode;
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
                                let mut wallet = wallet_arc.write().unwrap();
                                let unlock_result = wallet.open(self.sk_password_input.text());

                                match unlock_result {
                                    Ok(_) => {
                                        close_dialog = true;
                                    }
                                    Err(_) => {
                                        MessageBanner::set_global(ui.ctx(), "Incorrect Password", MessageType::Error);
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
            if cmd == "RefreshHDWallet" {
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
            } else if cmd == "SearchAssetLocks"
                && let Some(wallet_arc) = self.selected_wallet.clone()
            {
                let is_locked = wallet_arc.read().map(|w| !w.is_open()).unwrap_or(true);
                if is_locked {
                    // Wallet is locked - open unlock popup
                    self.pending_asset_lock_search_after_unlock = true;
                    self.wallet_unlock_popup.open();
                    action = AppAction::None;
                } else {
                    // Wallet is unlocked - proceed with search
                    self.asset_lock_search_banner.take_and_clear();
                    let handle = MessageBanner::set_global(
                        ctx,
                        "Searching for unused asset locks...",
                        MessageType::Info,
                    );
                    handle.with_elapsed();
                    self.asset_lock_search_banner = Some(handle);
                    action = AppAction::BackendTask(BackendTask::CoreTask(
                        CoreTask::RecoverAssetLocks(wallet_arc),
                    ));
                }
            }
        }

        // Dispatch the async ListCoreWallets task if pending
        if self.pending_list_core_wallets {
            self.pending_list_core_wallets = false;
            action |= AppAction::BackendTask(BackendTask::CoreTask(CoreTask::ListCoreWallets));
        }

        // Show Core wallet selection dialog if active
        if let Some(dialog) = self.core_wallet_dialog.as_mut()
            && let Some(status) = dialog.show_modal(ctx)
        {
            self.core_wallet_dialog = None;
            match status {
                SelectionStatus::Selected(idx) => {
                    if let Some(wallet_hash) = self.pending_core_wallet_seed_hash.take()
                        && let Some(wallets) = self.pending_core_wallet_options.take()
                        && let Some(wallet_name) = wallets.get(idx).cloned()
                    {
                        let is_single_key = self.pending_core_wallet_is_single_key;
                        match self.apply_core_wallet_selection(
                            &wallet_hash,
                            &wallet_name,
                            is_single_key,
                        ) {
                            Ok(()) => {
                                MessageBanner::set_global(
                                    ctx,
                                    format!(
                                        "Dash Core wallet '{}' assigned — refreshing wallet. If you were performing another operation, please retry it.",
                                        wallet_name
                                    ),
                                    MessageType::Success,
                                );
                                self.refresh();
                            }
                            Err(e) => {
                                MessageBanner::set_global(
                                    ctx,
                                    "Failed to save Dash Core wallet",
                                    MessageType::Error,
                                )
                                .with_details(e);
                            }
                        }
                    }
                }
                SelectionStatus::Canceled => {
                    self.pending_core_wallet_seed_hash = None;
                    self.pending_core_wallet_options = None;
                    self.pending_core_wallet_is_single_key = false;
                    MessageBanner::set_global(
                        ctx,
                        "Dash Core wallet not selected. Some operations may fail until a wallet is assigned.",
                        MessageType::Info,
                    );
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
        self.generating_core_address = false;

        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.asset_lock_search_banner.take_and_clear();

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

    /// Intercept Core-wallet-not-configured errors and schedule an async
    /// `ListCoreWallets` backend task (instead of blocking the UI thread).
    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if matches!(error, TaskError::CoreWalletNotConfigured) {
            self.refreshing = false;
            self.asset_lock_search_banner.take_and_clear();

            // Determine the wallet hash and whether it is a single-key wallet
            let (wallet_hash, is_single_key) = if let Some(hash) = self
                .selected_wallet
                .as_ref()
                .and_then(|w| w.read().ok().map(|g| g.seed_hash()))
            {
                (Some(hash), false)
            } else if let Some(hash) = self
                .selected_single_key_wallet
                .as_ref()
                .and_then(|w| w.read().ok().map(|g| g.key_hash))
            {
                (Some(hash), true)
            } else {
                (None, false)
            };

            self.pending_list_core_wallets = true;
            self.pending_list_wallet_hash = wallet_hash;
            self.pending_list_is_single_key = is_single_key;
            true // Suppress generic error banner
        } else {
            false
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
                // Refresh the cached platform sync info so the panel shows
                // updated timestamps and block heights after a wallet sync.
                let seed_hash = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
                if let Some(hash) = seed_hash {
                    self.refresh_platform_sync_info_cache(&hash);
                }
                if let Some(warn_msg) = warning {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        format!("Wallet refreshed with warning: {}", warn_msg),
                        MessageType::Info,
                    );
                } else {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Successfully refreshed wallet",
                        MessageType::Success,
                    );
                }
            }
            crate::ui::BackendTaskSuccessResult::RecoveredAssetLocks {
                recovered_count,
                total_amount,
            } => {
                self.asset_lock_search_banner.take_and_clear();
                let msg = if recovered_count == 0 {
                    "No additional unused asset locks found".to_string()
                } else {
                    format!(
                        "Found {} unused asset lock(s) worth {} Dash",
                        recovered_count,
                        Self::format_dash(total_amount)
                    )
                };
                MessageBanner::set_global(self.app_context.egui_ctx(), &msg, MessageType::Success);
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
                        Self::format_dash(*amount),
                        address,
                        txid
                    )
                } else {
                    format!(
                        "Sent {} total to {} recipients\nTxID: {}",
                        Self::format_dash(total_amount),
                        recipients.len(),
                        txid
                    )
                };
                MessageBanner::set_global(self.app_context.egui_ctx(), &msg, MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address } => {
                self.generating_core_address = false;
                if let Some(selected) = &self.selected_wallet
                    && let Ok(wallet) = selected.read()
                    && wallet.seed_hash() == seed_hash
                {
                    // Parse address and get balance
                    let balance = address
                        .parse::<Address<_>>()
                        .ok()
                        .and_then(|addr| {
                            wallet.address_balances.get(&addr.assume_checked()).copied()
                        })
                        .unwrap_or(0);
                    self.receive_dialog
                        .core_addresses
                        .push((address.clone(), balance));
                    self.receive_dialog.selected_core_index =
                        self.receive_dialog.core_addresses.len() - 1;
                    self.receive_dialog.qr_texture = None;
                    self.receive_dialog.qr_address = None;
                    self.receive_dialog.status = None;

                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        format!("New receive address generated: {address}"),
                        MessageType::Success,
                    );
                }
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
            } => {
                self.refreshing = false;
                // Update wallet's platform_address_info if this is for the selected wallet
                if let Some(selected) = &self.selected_wallet
                    && let Ok(mut wallet) = selected.write()
                    && wallet.seed_hash() == seed_hash
                {
                    // Update balances in the wallet
                    for (addr, (balance, nonce)) in balances {
                        wallet.set_platform_address_info(addr, balance, nonce);
                    }
                }
                self.refresh_platform_sync_info_cache(&seed_hash);
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
            result @ (crate::ui::BackendTaskSuccessResult::ShieldedInitialized { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedNotesSynced { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedCreditsShielded { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedTransferComplete { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedCreditsUnshielded { .. }
            | crate::ui::BackendTaskSuccessResult::ShieldedNullifiersChecked { .. }) => {
                if let Some(shielded_view) = &mut self.shielded_tab_view {
                    shielded_view.handle_result(&result);
                }
            }
            crate::ui::BackendTaskSuccessResult::CoreWalletsList(wallets) => {
                let wallet_hash = self.pending_list_wallet_hash.take();
                let is_single_key = self.pending_list_is_single_key;
                self.pending_list_is_single_key = false;

                if wallets.len() == 1 {
                    if let Some(hash) = wallet_hash {
                        match self.apply_core_wallet_selection(&hash, &wallets[0], is_single_key) {
                            Ok(()) => {
                                MessageBanner::set_global(
                                    self.app_context.egui_ctx(),
                                    format!(
                                        "Auto-selected Core wallet '{}' — refreshing wallet. If you were performing another operation, please retry it.",
                                        wallets[0]
                                    ),
                                    MessageType::Success,
                                );
                                self.refresh();
                            }
                            Err(e) => {
                                MessageBanner::set_global(
                                    self.app_context.egui_ctx(),
                                    "Failed to save Core wallet selection",
                                    MessageType::Error,
                                )
                                .with_details(e);
                            }
                        }
                    }
                } else if wallets.len() > 1 {
                    let dialog = SelectionDialog::new(
                        "Select Dash Core Wallet",
                        "Multiple wallets loaded in Dash Core. Select the one to use:",
                        wallets.clone(),
                    );
                    self.core_wallet_dialog = Some(dialog);
                    self.pending_core_wallet_seed_hash = wallet_hash;
                    self.pending_core_wallet_options = Some(wallets);
                    self.pending_core_wallet_is_single_key = is_single_key;
                } else {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "No wallets loaded in Dash Core",
                        MessageType::Error,
                    );
                }
            }
            _ => {}
        }
    }

    fn refresh_on_arrival(&mut self) {
        // Clear the spinner in case a refresh completed while this screen was not
        // visible (task results are dispatched to the visible screen, so ours would
        // have been silently discarded).
        self.refreshing = false;

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
        self.refresh_on_arrival();
    }
}
