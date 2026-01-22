use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::wallet::{
    DerivationPathHelpers, DerivationPathReference, Wallet, WalletSeedHash, WalletTransaction,
};
use crate::spv::CoreBackendMode;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{WalletUnlockPopup, WalletUnlockResult};
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::DashColors;
use crate::ui::wallets::account_summary::{
    AccountCategory, AccountSummary, collect_account_summaries,
};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use eframe::egui::{self, ComboBox, Context, Ui};
use eframe::epaint::TextureHandle;
use egui::load::SizedTexture;
use egui::{Color32, Frame, Margin, RichText, TextureOptions};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, RwLock};

use crate::model::wallet::single_key::SingleKeyWallet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Address,
    Balance,
    UTXOs,
    TotalReceived,
    Type,
    Index,
    DerivationPath,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

/// Refresh mode for dev mode dropdown - controls what gets refreshed
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum RefreshMode {
    /// Current behavior: Core wallet + Platform (auto decides full vs terminal)
    #[default]
    All,
    /// Only refresh Core wallet balances
    CoreOnly,
    /// Only Platform sync - force full sync
    PlatformFull,
    /// Only Platform sync - terminal only
    PlatformTerminal,
    /// Core wallet + Platform full sync
    CoreAndPlatformFull,
    /// Core wallet + Platform terminal sync
    CoreAndPlatformTerminal,
}

impl RefreshMode {
    fn label(&self) -> &'static str {
        match self {
            RefreshMode::All => "All (Auto)",
            RefreshMode::CoreOnly => "Core Only",
            RefreshMode::PlatformFull => "Platform (Full)",
            RefreshMode::PlatformTerminal => "Platform (Terminal)",
            RefreshMode::CoreAndPlatformFull => "Core + Platform (Full)",
            RefreshMode::CoreAndPlatformTerminal => "Core + Platform (Terminal)",
        }
    }

    fn all_modes() -> &'static [RefreshMode] {
        &[
            RefreshMode::All,
            RefreshMode::CoreOnly,
            RefreshMode::PlatformFull,
            RefreshMode::PlatformTerminal,
            RefreshMode::CoreAndPlatformFull,
            RefreshMode::CoreAndPlatformTerminal,
        ]
    }
}

pub struct WalletsBalancesScreen {
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    selected_single_key_wallet: Option<Arc<RwLock<SingleKeyWallet>>>,
    pub(crate) app_context: Arc<AppContext>,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    refreshing: bool,
    show_rename_dialog: bool,
    rename_input: String,
    wallet_unlock_popup: WalletUnlockPopup,
    show_sk_unlock_dialog: bool,
    sk_wallet_password: String,
    sk_show_password: bool,
    sk_error_message: Option<String>,
    remove_wallet_dialog: Option<ConfirmationDialog>,
    pending_wallet_removal: Option<WalletSeedHash>,
    pending_wallet_removal_alias: Option<String>,
    send_dialog: SendDialogState,
    receive_dialog: ReceiveDialogState,
    fund_platform_dialog: FundPlatformAddressDialogState,
    private_key_dialog: PrivateKeyDialogState,
    selected_account: Option<(AccountCategory, Option<u32>)>,
    /// Pending refresh of platform address balances (triggered after transfers)
    pending_platform_balance_refresh: Option<WalletSeedHash>,
    /// Whether we should refresh the wallet after it's unlocked
    pending_refresh_after_unlock: bool,
    /// The refresh mode to use after unlock (if pending_refresh_after_unlock is true)
    pending_refresh_mode: RefreshMode,
    /// Whether we should search for asset locks after wallet is unlocked
    pending_asset_lock_search_after_unlock: bool,
    /// Current page for single key wallet UTXO pagination (0-indexed)
    utxo_page: usize,
    /// Selected refresh mode (only shown in dev mode)
    refresh_mode: RefreshMode,
}

// Define a struct to hold the address data
struct AddressData {
    address: Address,
    balance: u64,
    /// Platform credits balance for Platform Payment addresses
    platform_credits: u64,
    utxo_count: usize,
    total_received: u64,
    address_type: String,
    index: u32,
    derivation_path: DerivationPath,
    account_category: AccountCategory,
    account_index: Option<u32>,
}

#[derive(Default)]
struct SendDialogState {
    is_open: bool,
    address: String,
    amount: Option<Amount>,
    amount_input: Option<AmountInput>,
    subtract_fee: bool,
    memo: String,
    error: Option<String>,
}

/// Type of address to receive to
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ReceiveAddressType {
    /// Core (L1) address for receiving Dash
    #[default]
    Core,
    /// Platform address for receiving credits
    Platform,
}

/// Unified state for the receive dialog (Core and Platform)
#[derive(Default)]
struct ReceiveDialogState {
    is_open: bool,
    /// Selected address type (Core or Platform)
    address_type: ReceiveAddressType,
    /// Core addresses with balances: (address, balance_duffs)
    core_addresses: Vec<(String, u64)>,
    /// Currently selected Core address index
    selected_core_index: usize,
    /// Platform addresses with balances: (display_address, balance_credits)
    platform_addresses: Vec<(String, u64)>,
    /// Currently selected Platform address index
    selected_platform_index: usize,
    qr_texture: Option<TextureHandle>,
    qr_address: Option<String>,
    status: Option<String>,
}

/// State for the Fund Platform Address from Asset Lock dialog
#[derive(Default)]
struct FundPlatformAddressDialogState {
    is_open: bool,
    /// Selected asset lock index
    selected_asset_lock_index: Option<usize>,
    /// Selected Platform address to fund
    selected_platform_address: Option<String>,
    /// List of Platform addresses available
    platform_addresses: Vec<(String, u64)>,
    status: Option<String>,
    /// Whether the current status is an error message
    status_is_error: bool,
    is_processing: bool,
    /// Whether we should continue funding after the wallet is unlocked
    pending_fund_after_unlock: bool,
}

/// State for the Private Key dialog
#[derive(Default)]
struct PrivateKeyDialogState {
    is_open: bool,
    /// The address being displayed
    address: String,
    /// The private key in WIF format
    private_key_wif: String,
    /// Whether to show the private key (hidden by default)
    show_key: bool,
    /// Pending derivation path (when wallet needs unlock first)
    pending_derivation_path: Option<DerivationPath>,
    /// Pending address string (when wallet needs unlock first)
    pending_address: Option<String>,
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
        Self {
            selected_wallet,
            selected_single_key_wallet,
            app_context: app_context.clone(),
            message: None,
            sort_column: SortColumn::Index,
            sort_order: SortOrder::Ascending,
            refreshing: false,
            show_rename_dialog: false,
            rename_input: String::new(),
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_sk_unlock_dialog: false,
            sk_wallet_password: String::new(),
            sk_show_password: false,
            sk_error_message: None,
            remove_wallet_dialog: None,
            pending_wallet_removal: None,
            pending_wallet_removal_alias: None,
            send_dialog: SendDialogState::default(),
            receive_dialog: ReceiveDialogState::default(),
            fund_platform_dialog: FundPlatformAddressDialogState::default(),
            private_key_dialog: PrivateKeyDialogState::default(),
            selected_account: None,
            pending_platform_balance_refresh: None,
            pending_refresh_after_unlock: false,
            pending_refresh_mode: RefreshMode::default(),
            pending_asset_lock_search_after_unlock: false,
            utxo_page: 0,
            refresh_mode: RefreshMode::default(),
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
            self.selected_wallet = None;
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
        if let Ok(wallets) = self.app_context.wallets.read()
            && let Some(wallet) = wallets.values().next().cloned()
        {
            self.selected_wallet = Some(wallet);
            self.selected_single_key_wallet = None;
            self.selected_account = None;
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

    fn add_receiving_address(&mut self) {
        if let Some(wallet) = &self.selected_wallet {
            let result = {
                let mut wallet = wallet.write().unwrap();
                wallet.receive_address(self.app_context.network, true, Some(&self.app_context))
            };

            match result {
                Ok(address) => {
                    let message = format!("Added new receiving address: {}", address);
                    self.display_message(&message, MessageType::Success);
                }
                Err(e) => {
                    self.display_message(&e, MessageType::Error);
                }
            }
        } else {
            self.display_message("No wallet selected", MessageType::Error);
        }
    }

    fn toggle_sort(&mut self, column: SortColumn) {
        if self.sort_column == column {
            self.sort_order = match self.sort_order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
        } else {
            self.sort_column = column;
            self.sort_order = SortOrder::Ascending;
        }
    }

    #[allow(clippy::ptr_arg)]
    fn sort_address_data(&self, data: &mut Vec<AddressData>) {
        data.sort_by(|a, b| {
            let order = match self.sort_column {
                SortColumn::Address => a.address.cmp(&b.address),
                SortColumn::Balance => a.balance.cmp(&b.balance),
                SortColumn::UTXOs => a.utxo_count.cmp(&b.utxo_count),
                SortColumn::TotalReceived => a.total_received.cmp(&b.total_received),
                SortColumn::Type => a.address_type.cmp(&b.address_type),
                SortColumn::Index => a.index.cmp(&b.index),
                SortColumn::DerivationPath => a.derivation_path.cmp(&b.derivation_path),
            };

            if self.sort_order == SortOrder::Ascending {
                order
            } else {
                order.reverse()
            }
        });
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
                let balance_dash = guard.total_balance_duffs() as f64 * 1e-8;
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
                .map(|g| g.total_balance_duffs())
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
                                            self.selected_wallet = Some(w.clone());
                                            self.selected_single_key_wallet = None;
                                            // Persist selection to AppContext and database
                                            if let Ok(hash) = w.read().map(|g| g.seed_hash())
                                                && let Ok(mut guard) =
                                                    self.app_context.selected_wallet_hash.lock()
                                            {
                                                *guard = Some(hash);
                                                // Save to database for persistence across restarts
                                                let _ = self
                                                    .app_context
                                                    .db
                                                    .update_selected_wallet_hash(Some(&hash));
                                            }
                                            if let Ok(mut guard) =
                                                self.app_context.selected_single_key_hash.lock()
                                            {
                                                *guard = None;
                                                let _ = self
                                                    .app_context
                                                    .db
                                                    .update_selected_single_key_hash(None);
                                            }
                                        }
                                        WalletItem::SingleKey(w) => {
                                            self.selected_single_key_wallet = Some(w.clone());
                                            self.selected_wallet = None;
                                            self.utxo_page = 0; // Reset pagination
                                            // Persist selection to AppContext and database
                                            if let Ok(hash) = w.read().map(|g| g.key_hash)
                                                && let Ok(mut guard) =
                                                    self.app_context.selected_single_key_hash.lock()
                                            {
                                                *guard = Some(hash);
                                                // Save to database for persistence across restarts
                                                let _ = self
                                                    .app_context
                                                    .db
                                                    .update_selected_single_key_hash(Some(&hash));
                                            }
                                            if let Ok(mut guard) =
                                                self.app_context.selected_wallet_hash.lock()
                                            {
                                                *guard = None;
                                                let _ = self
                                                    .app_context
                                                    .db
                                                    .update_selected_wallet_hash(None);
                                            }
                                        }
                                    }
                                    self.selected_account = None;
                                }
                            }
                        });

                    ui.colored_label(
                        DashColors::text_primary(ui.ctx().style().visuals.dark_mode),
                        format!(" Balance: {}", Self::format_dash(current_balance)),
                    );

                    ui.separator();

                    // Dev mode: Refresh mode selector
                    if self.app_context.is_developer_mode() {
                        ui.label(
                            egui::RichText::new("Refresh Mode:").color(DashColors::text_primary(
                                ui.ctx().style().visuals.dark_mode,
                            )),
                        );

                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            ComboBox::from_id_salt("refresh_mode_selector")
                                .selected_text(self.refresh_mode.label())
                                .show_ui(ui, |ui| {
                                    for mode in RefreshMode::all_modes() {
                                        ui.selectable_value(
                                            &mut self.refresh_mode,
                                            *mode,
                                            mode.label(),
                                        );
                                    }
                                });
                        });
                    }
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
                                self.display_message(
                                    &format!("Failed to remove: {}", e),
                                    MessageType::Error,
                                );
                            } else {
                                if let Ok(mut wallets) = self.app_context.single_key_wallets.write()
                                {
                                    wallets.remove(&key_hash);
                                }
                                self.selected_single_key_wallet = None;
                                // Clear persisted selection in AppContext and database
                                if let Ok(mut guard) =
                                    self.app_context.selected_single_key_hash.lock()
                                {
                                    *guard = None;
                                }
                                let _ = self.app_context.db.update_selected_single_key_hash(None);
                                self.display_message("Wallet removed", MessageType::Success);
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

    fn render_address_table(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        // Move the data preparation into its own scope
        let mut address_data = {
            let wallet = self.selected_wallet.as_ref().unwrap().read().unwrap();

            // Prepare data for the table
            wallet
                .known_addresses
                .iter()
                .map(|(address, derivation_path)| {
                    let utxo_info = wallet.utxos.get(address);

                    let utxo_count = utxo_info.map(|outpoints| outpoints.len()).unwrap_or(0);

                    // Get total received from the wallet (fetched from Core RPC)
                    let total_received = wallet
                        .address_total_received
                        .get(address)
                        .cloned()
                        .unwrap_or(0u64);

                    let index = derivation_path
                        .into_iter()
                        .last()
                        .cloned()
                        .unwrap_or(ChildNumber::Normal { index: 0 });
                    let index = match index {
                        ChildNumber::Normal { index } => index,
                        ChildNumber::Hardened { index } => index,
                        _ => 0,
                    };
                    let address_type =
                        if derivation_path.is_bip44_external(self.app_context.network) {
                            "Funds".to_string()
                        } else if derivation_path.is_bip44_change(self.app_context.network) {
                            "Change".to_string()
                        } else if derivation_path.is_asset_lock_funding(self.app_context.network) {
                            "Identity Creation".to_string()
                        } else if derivation_path.is_platform_payment(self.app_context.network) {
                            "Platform".to_string()
                        } else {
                            "System".to_string()
                        };

                    let path_reference = wallet
                        .watched_addresses
                        .get(derivation_path)
                        .map(|info| info.path_reference)
                        .unwrap_or(DerivationPathReference::Unknown);
                    let (account_category, account_index) =
                        Self::categorize_path(derivation_path, path_reference);

                    // Get Platform credits balance for Platform Payment addresses
                    // Use canonical lookup to handle potential Address key mismatches
                    let platform_credits = wallet
                        .get_platform_address_info(address)
                        .map(|info| info.balance)
                        .unwrap_or_default();

                    AddressData {
                        address: address.clone(),
                        balance: wallet
                            .address_balances
                            .get(address)
                            .cloned()
                            .unwrap_or_default(),
                        platform_credits,
                        utxo_count,
                        total_received,
                        address_type,
                        index,
                        derivation_path: derivation_path.clone(),
                        account_category,
                        account_index,
                    }
                })
                .collect::<Vec<AddressData>>()
        }; // The borrow of `wallet` ends here

        // Now you can use `self` mutably without conflict
        // Sort the data
        self.sort_address_data(&mut address_data);

        if let Some((category, index)) = self.selected_account.clone() {
            address_data
                .retain(|data| data.account_category == category && data.account_index == index);
        }

        // Space allocation for UI elements is handled by the layout system

        // Render the table
        TableBuilder::new(ui)
            .id_salt("addresses_table")
            .striped(false)
            .resizable(true)
            .vscroll(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto()) // Address
            .column(Column::initial(140.0)) // Balance
            .column(Column::initial(70.0)) // UTXOs
            .column(Column::initial(150.0)) // Total Received
            .column(Column::initial(100.0)) // Type
            .column(Column::initial(70.0)) // Index
            .column(Column::initial(120.0)) // Derivation Path
            .column(Column::initial(120.0)) // Actions
            .header(30.0, |mut header| {
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::Address {
                        match self.sort_order {
                            SortOrder::Ascending => "Address ^",
                            SortOrder::Descending => "Address v",
                        }
                    } else {
                        "Address"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::Address);
                    }
                });
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::Balance {
                        match self.sort_order {
                            SortOrder::Ascending => "Balance (DASH) ^",
                            SortOrder::Descending => "Balance (DASH) v",
                        }
                    } else {
                        "Balance (DASH)"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::Balance);
                    }
                });
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::UTXOs {
                        match self.sort_order {
                            SortOrder::Ascending => "UTXOs ^",
                            SortOrder::Descending => "UTXOs v",
                        }
                    } else {
                        "UTXOs"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::UTXOs);
                    }
                });
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::TotalReceived {
                        match self.sort_order {
                            SortOrder::Ascending => "Total Received (DASH) ^",
                            SortOrder::Descending => "Total Received (DASH) v",
                        }
                    } else {
                        "Total Received (DASH)"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::TotalReceived);
                    }
                });
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::Type {
                        match self.sort_order {
                            SortOrder::Ascending => "Type ^",
                            SortOrder::Descending => "Type v",
                        }
                    } else {
                        "Type"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::Type);
                    }
                });
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::Index {
                        match self.sort_order {
                            SortOrder::Ascending => "Index ^",
                            SortOrder::Descending => "Index v",
                        }
                    } else {
                        "Index"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::Index);
                    }
                });
                header.col(|ui| {
                    let label = if self.sort_column == SortColumn::DerivationPath {
                        match self.sort_order {
                            SortOrder::Ascending => "Full Path ^",
                            SortOrder::Descending => "Full Path v",
                        }
                    } else {
                        "Full Path"
                    };
                    if ui.button(label).clicked() {
                        self.toggle_sort(SortColumn::DerivationPath);
                    }
                });
                header.col(|ui| {
                    ui.label("Private Key");
                });
            })
            .body(|mut body| {
                let network = self.app_context.network;
                for data in &address_data {
                    body.row(25.0, |mut row| {
                        row.col(|ui| {
                            // For Platform Payment addresses, display in DIP-18 Bech32m format
                            if data.account_category == AccountCategory::PlatformPayment {
                                use dash_sdk::dpp::address_funds::PlatformAddress;
                                if let Ok(platform_addr) =
                                    PlatformAddress::try_from(data.address.clone())
                                {
                                    ui.label(platform_addr.to_bech32m_string(network));
                                } else {
                                    ui.label(data.address.to_string());
                                }
                            } else {
                                ui.label(data.address.to_string());
                            }
                        });
                        row.col(|ui| {
                            // These address types are used for key derivation/proofs, not holding funds
                            let is_key_only_address = matches!(
                                data.account_category,
                                AccountCategory::IdentityRegistration
                                    | AccountCategory::IdentityTopup
                                    | AccountCategory::IdentityInvitation
                                    | AccountCategory::IdentitySystem
                                    | AccountCategory::ProviderVoting
                                    | AccountCategory::ProviderOwner
                                    | AccountCategory::ProviderOperator
                                    | AccountCategory::ProviderPlatform
                            );

                            if is_key_only_address {
                                ui.label("N/A");
                            } else if data.account_category == AccountCategory::PlatformPayment {
                                // Platform credits: convert from credits to DASH
                                // Credits are in duffs * 1000, so divide by 1000 then by 1e8
                                let dash_balance =
                                    data.platform_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.label(format!("{:.8}", dash_balance));
                            } else {
                                let dash_balance = data.balance as f64 * 1e-8;
                                ui.label(format!("{:.8}", dash_balance));
                            }
                        });
                        row.col(|ui| {
                            // Key-only addresses don't hold UTXOs
                            let is_key_only_address = matches!(
                                data.account_category,
                                AccountCategory::IdentityRegistration
                                    | AccountCategory::IdentityTopup
                                    | AccountCategory::IdentityInvitation
                                    | AccountCategory::IdentitySystem
                                    | AccountCategory::ProviderVoting
                                    | AccountCategory::ProviderOwner
                                    | AccountCategory::ProviderOperator
                                    | AccountCategory::ProviderPlatform
                            );

                            if is_key_only_address {
                                ui.label("N/A");
                            } else {
                                ui.label(format!("{}", data.utxo_count));
                            }
                        });
                        row.col(|ui| {
                            // These address types are used for key derivation/proofs, not receiving funds
                            let is_key_only_address = matches!(
                                data.account_category,
                                AccountCategory::IdentityRegistration
                                    | AccountCategory::IdentityTopup
                                    | AccountCategory::IdentityInvitation
                                    | AccountCategory::IdentitySystem
                                    | AccountCategory::ProviderVoting
                                    | AccountCategory::ProviderOwner
                                    | AccountCategory::ProviderOperator
                                    | AccountCategory::ProviderPlatform
                            );

                            if is_key_only_address {
                                ui.label("N/A");
                            } else if data.account_category == AccountCategory::PlatformPayment {
                                // For Platform addresses, show platform credits balance
                                // (since we don't track historical Platform received)
                                let dash_received =
                                    data.platform_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.label(format!("{:.8}", dash_received));
                            } else {
                                let dash_received = data.total_received as f64 * 1e-8;
                                ui.label(format!("{:.8}", dash_received));
                            }
                        });
                        row.col(|ui| {
                            ui.label(&data.address_type);
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", data.index));
                        });
                        row.col(|ui| {
                            ui.label(format!("{}", data.derivation_path));
                        });
                        row.col(|ui| {
                            if ui.button("View Key").clicked() {
                                // Check if wallet is locked first
                                let wallet_locked = self
                                    .selected_wallet
                                    .as_ref()
                                    .map(|w| {
                                        w.read()
                                            .map(|g| g.uses_password && !g.is_open())
                                            .unwrap_or(false)
                                    })
                                    .unwrap_or(false);

                                if wallet_locked {
                                    // Store pending info and show unlock popup
                                    self.private_key_dialog.pending_derivation_path =
                                        Some(data.derivation_path.clone());
                                    self.private_key_dialog.pending_address =
                                        Some(data.address.to_string());
                                    self.wallet_unlock_popup.open();
                                } else {
                                    match self.derive_private_key_wif(&data.derivation_path) {
                                        Ok(key) => {
                                            self.private_key_dialog.is_open = true;
                                            self.private_key_dialog.address =
                                                data.address.to_string();
                                            self.private_key_dialog.private_key_wif = key;
                                            self.private_key_dialog.show_key = false;
                                        }
                                        Err(err) => self.display_message(&err, MessageType::Error),
                                    }
                                }
                            }
                        });
                    });
                }
            });
        action
    }

    fn render_bottom_options(&mut self, ui: &mut Ui) {
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|wallet_guard| wallet_guard.read().unwrap().is_open());

        // Only show "Add Receiving Address" button for Main Account (BIP44 account 0)
        let is_main_account = self
            .selected_account
            .as_ref()
            .is_some_and(|(category, index)| {
                *category == AccountCategory::Bip44 && index.unwrap_or(0) == 0
            });

        if wallet_is_open && is_main_account {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("➕ Add Receiving Address").size(14.0))
                    .clicked()
                {
                    self.add_receiving_address();
                }
            });
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

                self.selected_wallet = next_wallet.clone();

                // Update persisted selection in AppContext and database
                let new_hash = next_wallet
                    .as_ref()
                    .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
                if let Ok(mut guard) = self.app_context.selected_wallet_hash.lock() {
                    *guard = new_hash;
                }
                // Persist to database
                let _ = self
                    .app_context
                    .db
                    .update_selected_wallet_hash(new_hash.as_ref());

                self.show_rename_dialog = false;
                self.rename_input.clear();
                self.wallet_unlock_popup.close();
                self.refreshing = false;

                self.display_message(
                    &format!("Removed wallet \"{}\" successfully", alias),
                    MessageType::Success,
                );
            }
            Err(err) => {
                self.display_message(
                    &format!("Failed to remove wallet: {}", err),
                    MessageType::Error,
                );
            }
        }
    }

    fn render_wallet_asset_locks(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut open_fund_dialog_for_idx: Option<(usize, Vec<(String, u64)>)> = None;
        let mut recover_asset_locks_clicked = false;

        if let Some(arc_wallet) = &self.selected_wallet {
            let wallet = arc_wallet.read().unwrap();

            let dark_mode = ui.ctx().style().visuals.dark_mode;
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .corner_radius(5.0)
                .inner_margin(Margin::same(15))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .show(ui, |ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.horizontal(|ui| {
                        ui.heading(RichText::new("Asset Locks").color(DashColors::text_primary(dark_mode)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Create Asset Lock").clicked() {
                                app_action = AppAction::AddScreen(
                                    ScreenType::CreateAssetLock(arc_wallet.clone()).create_screen(&self.app_context)
                                );
                            }
                            if ui.button("Search for Unused").on_hover_text("Scan Core wallet for untracked asset locks").clicked() {
                                recover_asset_locks_clicked = true;
                            }
                        });
                    });
                    ui.add_space(10.0);

                    if wallet.unused_asset_locks.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("No asset locks found").color(Color32::GRAY).size(14.0));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Asset locks are special transactions that can be used to create identities or fund Platform addresses").color(Color32::GRAY).size(12.0));
                            ui.add_space(20.0);
                        });
                    } else {
                        // Collect Platform addresses for the fund dialog (using DIP-18 Bech32m format)
                        // Get from known_addresses where path is platform payment
                        let network = self.app_context.network;
                        let platform_addresses: Vec<(String, u64)> = wallet
                            .known_addresses
                            .iter()
                            .filter(|(_, path)| path.is_platform_payment(network))
                            .filter_map(|(addr, _)| {
                                use dash_sdk::dpp::address_funds::PlatformAddress;
                                let balance = wallet
                                    .get_platform_address_info(addr)
                                    .map(|info| info.balance)
                                    .unwrap_or(0);
                                PlatformAddress::try_from(addr.clone())
                                    .ok()
                                    .map(|pa| (pa.to_bech32m_string(network), balance))
                            })
                            .collect();

                        egui::ScrollArea::both()
                            .id_salt("asset_locks_table")
                            .min_scrolled_height(200.0)
                            .show(ui, |ui| {
                                TableBuilder::new(ui)
                        .striped(false)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::initial(200.0)) // Transaction ID
                        .column(Column::initial(100.0)) // Address
                        .column(Column::initial(100.0)) // Amount (Duffs)
                        .column(Column::initial(100.0)) // InstantLock status
                        .column(Column::initial(100.0)) // Usable status
                        .column(Column::initial(200.0)) // Actions
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.label("Transaction ID");
                            });
                            header.col(|ui| {
                                ui.label("Address");
                            });
                            header.col(|ui| {
                                ui.label("Amount (Duffs)");
                            });
                            header.col(|ui| {
                                ui.label("InstantLock");
                            });
                            header.col(|ui| {
                                ui.label("Usable");
                            });
                            header.col(|ui| {
                                ui.label("Actions");
                            });
                        })
                        .body(|mut body| {
                            for (index, (tx, address, amount, islock, proof)) in wallet.unused_asset_locks.iter().enumerate() {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(tx.txid().to_string());
                                    });
                                    row.col(|ui| {
                                        ui.label(address.to_string());
                                    });
                                    row.col(|ui| {
                                        ui.label(format!("{}", amount));
                                    });
                                    row.col(|ui| {
                                        let status = if islock.is_some() { "Yes" } else { "No" };
                                        ui.label(status);
                                    });
                                    row.col(|ui| {
                                        let status = if proof.is_some() { "Yes" } else { "No" };
                                        ui.label(status);
                                    });
                                    row.col(|ui| {
                                        if ui.small_button("View").on_hover_text("View full asset lock details").clicked() {
                                            app_action = AppAction::AddScreen(
                                                ScreenType::AssetLockDetail(
                                                    wallet.seed_hash(),
                                                    index
                                                ).create_screen(&self.app_context)
                                            );
                                        }
                                        if proof.is_some() {
                                            if ui.small_button("Fund").on_hover_text("Fund a Platform address with this asset lock").clicked() {
                                                open_fund_dialog_for_idx = Some((index, platform_addresses.clone()));
                                            }
                                        }
                                    });
                                });
                            }
                        });
                    });
                    }
                });
        } else {
            ui.label("No wallet selected.");
        }

        // Handle dialog opening outside the borrow
        if let Some((idx, platform_addresses)) = open_fund_dialog_for_idx {
            self.fund_platform_dialog.selected_asset_lock_index = Some(idx);
            self.fund_platform_dialog.is_open = true;
            self.fund_platform_dialog.platform_addresses = platform_addresses;
            self.fund_platform_dialog.selected_platform_address = None;
            self.fund_platform_dialog.status = None;
            self.fund_platform_dialog.is_processing = false;
        }

        // Handle recover asset locks button click - use custom action to check lock status
        if recover_asset_locks_clicked {
            app_action = AppAction::Custom("SearchAssetLocks".to_string());
        }

        app_action
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

    fn dismiss_message(&mut self) {
        self.message = None;
    }

    fn check_message_expiration(&mut self) {
        // Messages no longer auto-expire, they must be dismissed manually
    }

    fn format_dash(amount_duffs: u64) -> String {
        Amount::dash_from_duffs(amount_duffs).to_string()
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
        if tx.is_confirmed() {
            tx.height
                .map(|h| format!("Confirmed @{}", h))
                .unwrap_or_else(|| "Confirmed".to_string())
        } else {
            "Pending".to_string()
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

    fn render_wallet_overview(&self, ui: &mut Ui, wallet: &Wallet) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let total = wallet.total_balance_duffs();
        let platform = Self::platform_balance_duffs(wallet);

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!(
                "Core balance: {}",
                Self::format_dash(total)
            )));
        });
        ui.label(
            RichText::new(format!("Platform balance: {}", Self::format_dash(platform)))
                .color(DashColors::text_primary(dark_mode)),
        );
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
                    self.display_message("Select a wallet first", MessageType::Error);
                }
            }

            if ui
                .button(RichText::new("Receive").color(DashColors::text_primary(dark_mode)))
                .clicked()
            {
                action |= self.open_receive_dialog(ctx);
            }
        });
        action
    }

    fn render_accounts_section(&mut self, ui: &mut Ui, summaries: &[AccountSummary]) {
        ui.add_space(14.0);
        ui.heading("Accounts");
        ui.add_space(6.0);

        if summaries.is_empty() {
            ui.label("No account activity yet.");
            return;
        }

        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Find the currently selected summary
        let selected_summary = self.selected_account.as_ref().and_then(|(cat, idx)| {
            summaries
                .iter()
                .find(|s| &s.category == cat && s.index == *idx)
        });

        // Build the selected text for the dropdown
        let selected_text = selected_summary
            .map(|s| {
                if s.category.is_key_only() {
                    s.label.clone()
                } else if s.category == AccountCategory::PlatformPayment {
                    let credits_as_dash = s.platform_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                    format!("{} - {:.4} DASH", s.label, credits_as_dash)
                } else {
                    format!("{} - {}", s.label, Self::format_dash(s.confirmed_balance))
                }
            })
            .unwrap_or_else(|| "Select an account".to_string());

        // Account dropdown selector
        ComboBox::from_id_salt("account_selector")
            .selected_text(&selected_text)
            .width(ui.available_width() - 16.0)
            .show_ui(ui, |ui| {
                for summary in summaries {
                    let is_selected = self
                        .selected_account
                        .as_ref()
                        .map(|(cat, idx)| cat == &summary.category && *idx == summary.index)
                        .unwrap_or(false);

                    let label = if summary.category.is_key_only() {
                        summary.label.clone()
                    } else if summary.category == AccountCategory::PlatformPayment {
                        let credits_as_dash =
                            summary.platform_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                        format!("{} - {:.4} DASH", summary.label, credits_as_dash)
                    } else {
                        format!(
                            "{} - {}",
                            summary.label,
                            Self::format_dash(summary.confirmed_balance)
                        )
                    };

                    if ui.selectable_label(is_selected, &label).clicked() {
                        self.selected_account = Some((summary.category.clone(), summary.index));
                    }
                }
            });

        // Show description of the selected account below the dropdown
        if let Some(summary) = selected_summary
            && let Some(description) = summary.category.description()
        {
            ui.add_space(4.0);
            ui.label(
                RichText::new(description)
                    .color(DashColors::text_secondary(dark_mode))
                    .italics()
                    .size(12.0),
            );
        }
    }

    fn render_transactions_section(&self, ui: &mut Ui) {
        ui.add_space(10.0);
        ui.heading("Transactions");
        let Some(wallet_arc) = self.selected_wallet.as_ref() else {
            ui.label("Select a wallet to view its transaction history.");
            return;
        };

        let wallet_guard = wallet_arc.read().unwrap();
        if wallet_guard.transactions.is_empty() {
            ui.label("No transactions yet from SPV. Keep your wallet online to sync history.");
            return;
        }

        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let mut order: Vec<usize> = (0..wallet_guard.transactions.len()).collect();
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
        TableBuilder::new(ui)
            .id_salt("transactions_table")
            .striped(true)
            .column(Column::initial(150.0)) // Date
            .column(Column::initial(80.0)) // Type
            .column(Column::initial(120.0)) // Amount
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
                        row.col(|ui| {
                            ui.label(Self::format_transaction_status(tx));
                        });
                        row.col(|ui| {
                            let full_txid = tx.txid.to_string();
                            ui.horizontal(|ui| {
                                let response = ui.label(RichText::new(&full_txid).monospace());
                                response.on_hover_text(&full_txid);
                                if ui
                                    .small_button("Copy")
                                    .on_hover_text("Copy transaction ID")
                                    .clicked()
                                {
                                    let _ = copy_text_to_clipboard(&full_txid);
                                }
                            });
                        });
                    });
                }
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
                        ui.horizontal(|ui| {
                            ui.heading(
                                RichText::new(alias.clone())
                                    .color(DashColors::text_primary(dark_mode))
                                    .size(25.0),
                            );

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.refreshing {
                                        ui.add(egui::Spinner::new().color(DashColors::DASH_BLUE))
                                    } else {
                                        ui.add(egui::Label::new(""))
                                    }
                                },
                            );
                        });

                        let summaries = {
                            let wallet = wallet_arc.read().unwrap();
                            self.render_wallet_overview(ui, &wallet);
                            collect_account_summaries(&wallet)
                        };

                        self.ensure_account_selection(&summaries);
                        action |= self.render_action_buttons(ui, ctx);
                        ui.add_space(10.0);
                        ui.separator();
                        self.render_accounts_section(ui, &summaries);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        let addresses_heading = self
                            .selected_account
                            .as_ref()
                            .map(|(category, index)| {
                                format!("Addresses ({})", category.label(*index))
                            })
                            .unwrap_or_else(|| "Addresses".to_string());
                        ui.heading(
                            RichText::new(addresses_heading)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(8.0);
                        action |= self.render_address_table(ui);

                        // Transactions section - requires SPV which is dev mode only
                        if self.app_context.is_developer_mode() {
                            ui.add_space(10.0);
                            ui.separator();
                            self.render_transactions_section(ui);
                        }

                        ui.add_space(14.0);
                        self.render_bottom_options(ui);

                        ui.add_space(16.0);
                        action |= self.render_wallet_asset_locks(ui);
                    });
            });
        });

        action
    }

    fn render_send_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.send_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.send_dialog.is_open;
        egui::Window::new("Send Dash")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Recipient Address");
                ui.add(egui::TextEdit::singleline(&mut self.send_dialog.address).hint_text("y..."));

                ui.add_space(8.0);

                // Amount input using AmountInput component
                let amount_input = self.send_dialog.amount_input.get_or_insert_with(|| {
                    AmountInput::new(Amount::new_dash(0.0))
                        .with_label("Amount (DASH):")
                        .with_hint_text("Enter amount (e.g., 0.01)")
                        .with_desired_width(150.0)
                });

                let response = amount_input.show(ui);
                response.inner.update(&mut self.send_dialog.amount);

                ui.checkbox(
                    &mut self.send_dialog.subtract_fee,
                    "Subtract fee from amount",
                );

                ui.label("Memo (optional)");
                ui.add(egui::TextEdit::singleline(&mut self.send_dialog.memo));

                if let Some(error) = self.send_dialog.error.clone() {
                    let error_color = Color32::from_rgb(255, 100, 100);
                    Frame::new()
                        .fill(error_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, error_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Error: {}", error)).color(error_color),
                                );
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.send_dialog.error = None;
                                }
                            });
                        });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Send").clicked() {
                        match self.prepare_send_action() {
                            Ok(app_action) => {
                                action = app_action;
                                self.send_dialog = SendDialogState::default();
                            }
                            Err(err) => self.send_dialog.error = Some(err),
                        }
                    }
                });
            });

        self.send_dialog.is_open = open;
        action
    }

    fn render_receive_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.receive_dialog.is_open {
            return AppAction::None;
        }

        let dark_mode = ctx.style().visuals.dark_mode;

        // Determine current address based on selected type
        let current_address = match self.receive_dialog.address_type {
            ReceiveAddressType::Core => self
                .receive_dialog
                .core_addresses
                .get(self.receive_dialog.selected_core_index)
                .map(|(addr, _)| addr.clone()),
            ReceiveAddressType::Platform => self
                .receive_dialog
                .platform_addresses
                .get(self.receive_dialog.selected_platform_index)
                .map(|(addr, _)| addr.clone()),
        };

        // Generate QR texture if needed
        if let Some(address) = current_address.clone() {
            let needs_texture = self.receive_dialog.qr_texture.is_none()
                || self.receive_dialog.qr_address.as_deref() != Some(&address);
            if needs_texture {
                match generate_qr_code_image(&address) {
                    Ok(image) => {
                        let texture = ctx.load_texture(
                            format!("receive_{}", address),
                            image,
                            TextureOptions::LINEAR,
                        );
                        self.receive_dialog.qr_texture = Some(texture);
                        self.receive_dialog.qr_address = Some(address);
                    }
                    Err(err) => {
                        self.receive_dialog.status = Some(err.to_string());
                    }
                }
            }
        }

        let mut open = self.receive_dialog.is_open;

        // Draw dark overlay behind the dialog (only when open)
        if open {
            let screen_rect = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("receive_dialog_overlay"),
            ));
            painter.rect_filled(
                screen_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            );
        }

        egui::Window::new("Receive")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(egui::Frame {
                inner_margin: egui::Margin::same(20),
                outer_margin: egui::Margin::same(0),
                corner_radius: egui::CornerRadius::same(8),
                shadow: egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
                },
                fill: ctx.style().visuals.window_fill,
                stroke: egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                ),
            })
            .show(ctx, |ui| {
                ui.set_min_width(350.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);

                    // Address type selector at the top
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.receive_dialog.address_type,
                            ReceiveAddressType::Core,
                            RichText::new("Core").color(DashColors::text_primary(dark_mode)),
                        );
                        ui.selectable_value(
                            &mut self.receive_dialog.address_type,
                            ReceiveAddressType::Platform,
                            RichText::new("Platform").color(DashColors::text_primary(dark_mode)),
                        );
                    });

                    // Clear QR when switching types
                    let type_label = match self.receive_dialog.address_type {
                        ReceiveAddressType::Core => "Core Address",
                        ReceiveAddressType::Platform => "Platform Address",
                    };

                    ui.add_space(5.0);
                    ui.label(
                        RichText::new(type_label)
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(10.0);

                    // Show QR code
                    if let Some(texture) = &self.receive_dialog.qr_texture {
                        ui.image(SizedTexture::new(texture.id(), egui::vec2(220.0, 220.0)));
                    } else if current_address.is_some() {
                        ui.label("Generating QR code...");
                    }

                    ui.add_space(10.0);

                    match self.receive_dialog.address_type {
                        ReceiveAddressType::Core => {
                            // Core address selector (if multiple addresses)
                            if self.receive_dialog.core_addresses.len() > 1 {
                                ui.horizontal(|ui| {
                                    ui.label("Address:");
                                    ComboBox::from_id_salt("core_addr_selector")
                                        .selected_text(
                                            self.receive_dialog
                                                .core_addresses
                                                .get(self.receive_dialog.selected_core_index)
                                                .map(|(addr, balance)| {
                                                    let balance_dash = *balance as f64 / 1e8;
                                                    format!(
                                                        "{}... ({:.4} DASH)",
                                                        &addr[..12.min(addr.len())],
                                                        balance_dash
                                                    )
                                                })
                                                .unwrap_or_default(),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (idx, (addr, balance)) in
                                                self.receive_dialog.core_addresses.iter().enumerate()
                                            {
                                                let balance_dash = *balance as f64 / 1e8;
                                                let label = format!(
                                                    "{}... ({:.4} DASH)",
                                                    &addr[..12.min(addr.len())],
                                                    balance_dash
                                                );
                                                if ui
                                                    .selectable_label(
                                                        idx == self.receive_dialog.selected_core_index,
                                                        label,
                                                    )
                                                    .clicked()
                                                {
                                                    self.receive_dialog.selected_core_index = idx;
                                                    // Clear QR so it regenerates
                                                    self.receive_dialog.qr_texture = None;
                                                    self.receive_dialog.qr_address = None;
                                                }
                                            }
                                        });
                                });
                                ui.add_space(5.0);
                            }

                            // Show selected Core address
                            if let Some((address, balance)) = self
                                .receive_dialog
                                .core_addresses
                                .get(self.receive_dialog.selected_core_index)
                                .cloned()
                            {
                                ui.label(
                                    RichText::new(&address)
                                        .monospace()
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                let balance_dash = balance as f64 / 1e8;
                                ui.label(
                                    RichText::new(format!("Balance: {:.8} DASH", balance_dash))
                                        .color(DashColors::text_secondary(dark_mode)),
                                );

                                ui.add_space(8.0);

                                let mut copy_status: Option<String> = None;
                                let mut generate_new = false;

                                ui.horizontal(|ui| {
                                    if ui.button("Copy Address").clicked() {
                                        if let Err(err) = copy_text_to_clipboard(&address) {
                                            copy_status = Some(format!("Error: {}", err));
                                        } else {
                                            copy_status = Some("Address copied!".to_string());
                                        }
                                    }

                                    if ui.button("New Address").clicked() {
                                        generate_new = true;
                                    }
                                });

                                if let Some(status) = copy_status {
                                    self.receive_dialog.status = Some(status);
                                }

                                if generate_new
                                    && let Some(wallet) = &self.selected_wallet {
                                        match self.generate_new_core_receive_address(wallet) {
                                            Ok((new_addr, new_balance)) => {
                                                self.receive_dialog.core_addresses.push((new_addr, new_balance));
                                                self.receive_dialog.selected_core_index =
                                                    self.receive_dialog.core_addresses.len() - 1;
                                                self.receive_dialog.qr_texture = None;
                                                self.receive_dialog.qr_address = None;
                                                self.receive_dialog.status = Some("New address generated!".to_string());
                                            }
                                            Err(err) => {
                                                self.receive_dialog.status = Some(err);
                                            }
                                        }
                                    }
                            }

                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("Send Dash to this address to add funds to your wallet.")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(11.0)
                                    .italics(),
                            );
                        }
                        ReceiveAddressType::Platform => {
                            // Platform address selector (if multiple addresses)
                            if self.receive_dialog.platform_addresses.len() > 1 {
                                ui.horizontal(|ui| {
                                    ui.label("Address:");
                                    ComboBox::from_id_salt("platform_addr_selector")
                                        .selected_text(
                                            self.receive_dialog
                                                .platform_addresses
                                                .get(self.receive_dialog.selected_platform_index)
                                                .map(|(addr, balance)| {
                                                    let credits_as_dash =
                                                        *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                                    format!(
                                                        "{}... ({:.4} DASH)",
                                                        &addr[..12.min(addr.len())],
                                                        credits_as_dash
                                                    )
                                                })
                                                .unwrap_or_default(),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (idx, (addr, balance)) in
                                                self.receive_dialog.platform_addresses.iter().enumerate()
                                            {
                                                let credits_as_dash =
                                                    *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                                let label = format!(
                                                    "{}... ({:.4} DASH)",
                                                    &addr[..12.min(addr.len())],
                                                    credits_as_dash
                                                );
                                                if ui
                                                    .selectable_label(
                                                        idx == self.receive_dialog.selected_platform_index,
                                                        label,
                                                    )
                                                    .clicked()
                                                {
                                                    self.receive_dialog.selected_platform_index = idx;
                                                    // Clear QR so it regenerates
                                                    self.receive_dialog.qr_texture = None;
                                                    self.receive_dialog.qr_address = None;
                                                }
                                            }
                                        });
                                });
                                ui.add_space(5.0);
                            }

                            // Show selected Platform address
                            let selected_addr_data = self
                                .receive_dialog
                                .platform_addresses
                                .get(self.receive_dialog.selected_platform_index)
                                .cloned();

                            if let Some((address, balance)) = selected_addr_data {
                                ui.label(
                                    RichText::new(&address)
                                        .monospace()
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                let credits_as_dash = balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.label(
                                    RichText::new(format!("Balance: {:.8} DASH", credits_as_dash))
                                        .color(DashColors::text_secondary(dark_mode)),
                                );

                                ui.add_space(8.0);

                                let mut copy_status: Option<String> = None;
                                let mut new_addr_result: Option<Result<String, String>> = None;

                                ui.horizontal(|ui| {
                                    if ui.button("Copy Address").clicked() {
                                        if let Err(err) = copy_text_to_clipboard(&address) {
                                            copy_status = Some(format!("Error: {}", err));
                                        } else {
                                            copy_status = Some("Address copied!".to_string());
                                        }
                                    }

                                    // Button to add new Platform address
                                    if let Some(wallet) = &self.selected_wallet
                                        && ui.button("New Address").clicked()
                                    {
                                        new_addr_result = Some(self.generate_platform_address(wallet));
                                    }
                                });

                                // Handle copy status after the closure
                                if let Some(status) = copy_status {
                                    self.receive_dialog.status = Some(status);
                                }

                                // Handle new address generation after the closure
                                if let Some(result) = new_addr_result {
                                    match result {
                                        Ok(new_addr) => {
                                            self.receive_dialog.platform_addresses.push((new_addr, 0));
                                            self.receive_dialog.selected_platform_index =
                                                self.receive_dialog.platform_addresses.len() - 1;
                                            self.receive_dialog.qr_texture = None;
                                            self.receive_dialog.qr_address = None;
                                            self.receive_dialog.status =
                                                Some("New address generated!".to_string());
                                        }
                                        Err(err) => {
                                            self.receive_dialog.status = Some(err);
                                        }
                                    }
                                }
                            }

                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(
                                    "Send credits from an identity or another Platform address to fund this address.",
                                )
                                .color(DashColors::text_secondary(dark_mode))
                                .size(11.0)
                                .italics(),
                            );
                        }
                    }

                    if let Some(status) = &self.receive_dialog.status {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(status).color(DashColors::text_secondary(dark_mode)),
                        );
                    }
                });
            });

        self.receive_dialog.is_open = open;
        if !self.receive_dialog.is_open {
            self.receive_dialog = ReceiveDialogState::default();
        }
        AppAction::None
    }

    /// Generate a new Platform address for the wallet.
    /// Returns the address in DIP-18 Bech32m format (e.g., tdashevo1... for testnet)
    fn generate_platform_address(&self, wallet: &Arc<RwLock<Wallet>>) -> Result<String, String> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
        // Pass true to skip known addresses and generate a new one
        let address = wallet_guard
            .platform_receive_address(self.app_context.network, true, Some(&self.app_context))
            .map_err(|e| e.to_string())?;
        // Convert to PlatformAddress and encode as Bech32m per DIP-18
        let platform_addr =
            PlatformAddress::try_from(address).map_err(|e| format!("Invalid address: {}", e))?;
        Ok(platform_addr.to_bech32m_string(self.app_context.network))
    }

    /// Generate a new Core receive address for the wallet
    /// Returns (address_string, balance_duffs)
    fn generate_new_core_receive_address(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<(String, u64), String> {
        let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
        let address = wallet_guard
            .receive_address(self.app_context.network, true, Some(&self.app_context))
            .map_err(|e| e.to_string())?;
        let balance = wallet_guard
            .address_balances
            .get(&address)
            .copied()
            .unwrap_or(0);
        Ok((address.to_string(), balance))
    }

    /// Render the Fund Platform Address from Asset Lock dialog
    fn render_fund_platform_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.fund_platform_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.fund_platform_dialog.is_open;
        let dark_mode = ctx.style().visuals.dark_mode;

        // Draw dark overlay behind the popup
        let screen_rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("fund_platform_dialog_overlay"),
        ));
        painter.rect_filled(
            screen_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
        );

        egui::Window::new("Fund Platform Address from Asset Lock")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(egui::Frame {
                inner_margin: egui::Margin::same(20),
                outer_margin: egui::Margin::same(0),
                corner_radius: egui::CornerRadius::same(8),
                shadow: egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
                },
                fill: ctx.style().visuals.window_fill,
                stroke: egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                ),
            })
            .show(ctx, |ui| {
                ui.set_min_width(400.0);

                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Select a Platform address to fund:")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);

                    // Platform address selector
                    if self.fund_platform_dialog.platform_addresses.is_empty() {
                        ui.label(
                            RichText::new("No Platform addresses found. Generate one first.")
                                .color(DashColors::text_secondary(dark_mode))
                                .italics(),
                        );
                    } else {
                        ComboBox::from_id_salt("fund_platform_addr_selector")
                            .selected_text(
                                self.fund_platform_dialog
                                    .selected_platform_address
                                    .as_deref()
                                    .map(|addr| {
                                        let balance = self
                                            .fund_platform_dialog
                                            .platform_addresses
                                            .iter()
                                            .find(|(a, _)| a == addr)
                                            .map(|(_, b)| *b)
                                            .unwrap_or(0);
                                        let credits_as_dash =
                                            balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                        format!(
                                            "{}... ({:.4} DASH)",
                                            &addr[..12.min(addr.len())],
                                            credits_as_dash
                                        )
                                    })
                                    .unwrap_or_else(|| "Select an address".to_string()),
                            )
                            .show_ui(ui, |ui| {
                                for (addr, balance) in &self.fund_platform_dialog.platform_addresses
                                {
                                    let credits_as_dash =
                                        *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                    let label = format!(
                                        "{}... ({:.4} DASH)",
                                        &addr[..12.min(addr.len())],
                                        credits_as_dash
                                    );
                                    let is_selected = self
                                        .fund_platform_dialog
                                        .selected_platform_address
                                        .as_deref()
                                        == Some(addr.as_str());
                                    if ui.selectable_label(is_selected, label).clicked() {
                                        self.fund_platform_dialog.selected_platform_address =
                                            Some(addr.clone());
                                    }
                                }
                            });
                    }

                    ui.add_space(15.0);

                    // Status message
                    if let Some(status) = &self.fund_platform_dialog.status {
                        let status_color = if self.fund_platform_dialog.status_is_error {
                            egui::Color32::from_rgb(220, 50, 50)
                        } else {
                            DashColors::text_secondary(dark_mode)
                        };
                        ui.label(RichText::new(status).color(status_color));
                        ui.add_space(10.0);
                    }

                    // Buttons
                    ui.horizontal(|ui| {
                        let can_fund = self.fund_platform_dialog.selected_platform_address.is_some()
                            && self.fund_platform_dialog.selected_asset_lock_index.is_some()
                            && !self.fund_platform_dialog.is_processing;

                        // Cancel button
                        let cancel_button = egui::Button::new(
                            RichText::new("Cancel").color(DashColors::text_primary(dark_mode)),
                        )
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.0, DashColors::text_secondary(dark_mode)))
                        .corner_radius(egui::CornerRadius::same(4))
                        .min_size(egui::Vec2::new(80.0, 32.0));

                        if ui.add(cancel_button).clicked() {
                            self.fund_platform_dialog.is_open = false;
                        }

                        ui.add_space(8.0);

                        // Fund button
                        let fund_button = egui::Button::new(
                            RichText::new(if self.fund_platform_dialog.is_processing {
                                "Funding..."
                            } else {
                                "Fund Address"
                            })
                            .color(egui::Color32::WHITE),
                        )
                        .fill(if can_fund {
                            DashColors::DASH_BLUE
                        } else {
                            DashColors::text_secondary(dark_mode)
                        })
                        .corner_radius(egui::CornerRadius::same(4))
                        .min_size(egui::Vec2::new(100.0, 32.0));

                        if ui.add_enabled(can_fund, fund_button).clicked() {
                            // Check if wallet is locked
                            let is_locked = self
                                .selected_wallet
                                .as_ref()
                                .and_then(|w| w.read().ok())
                                .map(|w| !w.is_open())
                                .unwrap_or(false);

                            if is_locked {
                                // Wallet is locked - open unlock popup and set pending flag
                                self.fund_platform_dialog.pending_fund_after_unlock = true;
                                self.wallet_unlock_popup.open();
                            } else {
                                action = self.prepare_fund_platform_action();
                            }
                        }
                    });

                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(
                            "The entire asset lock amount will be used to fund the Platform address.",
                        )
                        .color(DashColors::text_secondary(dark_mode))
                        .size(11.0)
                        .italics(),
                    );
                });
            });

        // Only update from `open` if we didn't manually close via cancel button
        if self.fund_platform_dialog.is_open {
            self.fund_platform_dialog.is_open = open;
        }
        if !self.fund_platform_dialog.is_open {
            self.fund_platform_dialog = FundPlatformAddressDialogState::default();
        }
        action
    }

    /// Render the Private Key dialog
    fn render_private_key_dialog(&mut self, ctx: &Context) {
        if !self.private_key_dialog.is_open {
            return;
        }

        let dark_mode = ctx.style().visuals.dark_mode;
        let mut open = self.private_key_dialog.is_open;

        // Draw dark overlay behind the dialog
        if open {
            let screen_rect = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("private_key_dialog_overlay"),
            ));
            painter.rect_filled(
                screen_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            );
        }

        egui::Window::new("Private Key")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .frame(egui::Frame {
                inner_margin: egui::Margin::same(20),
                outer_margin: egui::Margin::same(0),
                corner_radius: egui::CornerRadius::same(8),
                shadow: egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 16,
                    spread: 0,
                    color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
                },
                fill: ctx.style().visuals.window_fill,
                stroke: egui::Stroke::new(
                    1.0,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                ),
            })
            .show(ctx, |ui| {
                ui.set_min_width(400.0);
                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);

                    // Address label
                    ui.label(
                        RichText::new("Address")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(5.0);

                    // Address value
                    ui.label(
                        RichText::new(&self.private_key_dialog.address)
                            .monospace()
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    ui.add_space(5.0);

                    // Copy address button
                    if ui.button("Copy Address").clicked() {
                        let _ = copy_text_to_clipboard(&self.private_key_dialog.address);
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(15.0);

                    // Private key label
                    ui.label(
                        RichText::new("Private Key (WIF)")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(5.0);

                    // Private key value (hidden by default)
                    if self.private_key_dialog.show_key {
                        ui.label(
                            RichText::new(&self.private_key_dialog.private_key_wif)
                                .monospace()
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    } else {
                        ui.label(
                            RichText::new("••••••••••••••••••••••••••••••••••••••••••••••••••••")
                                .monospace()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    }

                    ui.add_space(10.0);

                    // Show/Hide and Copy buttons
                    ui.horizontal(|ui| {
                        if ui
                            .button(if self.private_key_dialog.show_key {
                                "Hide Key"
                            } else {
                                "Show Key"
                            })
                            .clicked()
                        {
                            self.private_key_dialog.show_key = !self.private_key_dialog.show_key;
                        }

                        if ui.button("Copy Key").clicked() {
                            let _ =
                                copy_text_to_clipboard(&self.private_key_dialog.private_key_wif);
                        }
                    });

                    ui.add_space(15.0);

                    // Warning message
                    ui.label(
                        RichText::new("Keep your private key secure. Never share it with anyone.")
                            .color(DashColors::error_color(dark_mode))
                            .size(11.0)
                            .italics(),
                    );
                });
            });

        self.private_key_dialog.is_open = open;
        if !self.private_key_dialog.is_open {
            self.private_key_dialog = PrivateKeyDialogState::default();
        }
    }

    /// Prepare the backend task for funding a Platform address from asset lock
    fn prepare_fund_platform_action(&mut self) -> AppAction {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use std::collections::BTreeMap;

        let Some(wallet_arc) = &self.selected_wallet else {
            self.fund_platform_dialog.status = Some("No wallet selected".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        let Some(selected_addr) = &self.fund_platform_dialog.selected_platform_address else {
            self.fund_platform_dialog.status = Some("Select a Platform address".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        let Some(asset_lock_idx) = self.fund_platform_dialog.selected_asset_lock_index else {
            self.fund_platform_dialog.status = Some("No asset lock selected".to_string());
            self.fund_platform_dialog.status_is_error = true;
            return AppAction::None;
        };

        // Get the asset lock proof and address from the wallet
        let (seed_hash, asset_lock_proof, asset_lock_address, platform_addr) = {
            let wallet = match wallet_arc.read() {
                Ok(guard) => guard,
                Err(e) => {
                    self.fund_platform_dialog.status = Some(e.to_string());
                    self.fund_platform_dialog.status_is_error = true;
                    return AppAction::None;
                }
            };

            let asset_lock = wallet.unused_asset_locks.get(asset_lock_idx);
            let Some((_, addr, _, _, Some(proof))) = asset_lock else {
                self.fund_platform_dialog.status =
                    Some("Asset lock not found or not ready".to_string());
                self.fund_platform_dialog.status_is_error = true;
                return AppAction::None;
            };

            // Parse the Platform address (Bech32m format: dashevo1.../tdashevo1...)
            use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
            let platform_addr = if selected_addr.starts_with("dashevo1")
                || selected_addr.starts_with("tdashevo1")
            {
                match PlatformAddress::from_bech32m_string(selected_addr) {
                    Ok((addr, _network)) => addr,
                    Err(e) => {
                        self.fund_platform_dialog.status =
                            Some(format!("Invalid Bech32m address: {}", e));
                        self.fund_platform_dialog.status_is_error = true;
                        return AppAction::None;
                    }
                }
            } else {
                // Fall back to base58 parsing for backwards compatibility
                match selected_addr
                    .parse::<Address<NetworkUnchecked>>()
                    .map_err(|e| e.to_string())
                    .and_then(|a| {
                        PlatformAddress::try_from(a.assume_checked())
                            .map_err(|e| format!("Invalid Platform address: {}", e))
                    }) {
                    Ok(addr) => addr,
                    Err(e) => {
                        self.fund_platform_dialog.status = Some(e);
                        self.fund_platform_dialog.status_is_error = true;
                        return AppAction::None;
                    }
                }
            };

            (
                wallet.seed_hash(),
                Box::new(proof.clone()),
                addr.clone(),
                platform_addr,
            )
        };

        // Build outputs - fund the entire asset lock to the selected Platform address
        let mut outputs: BTreeMap<PlatformAddress, Option<u64>> = BTreeMap::new();
        outputs.insert(platform_addr, None); // None = take the full amount

        self.fund_platform_dialog.is_processing = true;
        self.fund_platform_dialog.status = Some("Processing...".to_string());
        self.fund_platform_dialog.status_is_error = false;

        AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromAssetLock {
                seed_hash,
                asset_lock_proof,
                asset_lock_address,
                outputs,
            },
        ))
    }

    fn prepare_send_action(&mut self) -> Result<AppAction, String> {
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or_else(|| "Select a wallet first".to_string())?;

        let amount_duffs = self
            .send_dialog
            .amount
            .as_ref()
            .ok_or_else(|| "Enter an amount".to_string())?
            .dash_to_duffs()?;

        if amount_duffs == 0 {
            return Err("Amount must be greater than 0".to_string());
        }

        {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            if amount_duffs > wallet_guard.confirmed_balance_duffs() {
                return Err("Insufficient confirmed balance".to_string());
            }
        }

        if self.send_dialog.address.trim().is_empty() {
            return Err("Enter a recipient address".to_string());
        }

        let memo = self.send_dialog.memo.trim();
        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: self.send_dialog.address.trim().to_string(),
                amount_duffs,
            }],
            subtract_fee_from_amount: self.send_dialog.subtract_fee,
            memo: if memo.is_empty() {
                None
            } else {
                Some(memo.to_string())
            },
            override_fee: None,
        };

        Ok(AppAction::BackendTask(BackendTask::CoreTask(
            CoreTask::SendWalletPayment {
                wallet: wallet.clone(),
                request,
            },
        )))
    }

    fn open_receive_dialog(&mut self, _ctx: &Context) -> AppAction {
        let Some(wallet) = self.selected_wallet.clone() else {
            self.receive_dialog.status = Some("Select a wallet first".to_string());
            self.receive_dialog.core_addresses.clear();
            self.receive_dialog.platform_addresses.clear();
            self.receive_dialog.qr_texture = None;
            self.receive_dialog.qr_address = None;
            self.receive_dialog.is_open = true;
            return AppAction::None;
        };

        self.receive_dialog.is_open = true;
        self.receive_dialog.qr_texture = None;
        self.receive_dialog.qr_address = None;

        // Load Core addresses (works with locked wallet - uses existing addresses)
        self.load_core_addresses_for_receive(&wallet);

        // Load Platform addresses (works with locked wallet - uses existing addresses)
        self.load_platform_addresses_for_receive(&wallet);

        AppAction::None
    }

    /// Load Core addresses into the receive dialog
    fn load_core_addresses_for_receive(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        let wallet_guard = match wallet.read() {
            Ok(guard) => guard,
            Err(err) => {
                self.receive_dialog.status = Some(err.to_string());
                return;
            }
        };

        // Collect all BIP44 external (receive) addresses with their balances
        let network = self.app_context.network;
        let core_addresses: Vec<(String, u64)> = wallet_guard
            .watched_addresses
            .iter()
            .filter(|(path, _)| path.is_bip44_external(network))
            .map(|(_, info)| {
                let balance = wallet_guard
                    .address_balances
                    .get(&info.address)
                    .copied()
                    .unwrap_or(0);
                (info.address.to_string(), balance)
            })
            .collect();

        drop(wallet_guard);

        if core_addresses.is_empty() {
            // Generate a new Core address if none exists
            match self.generate_new_core_receive_address(wallet) {
                Ok((address, balance)) => {
                    self.receive_dialog.core_addresses = vec![(address, balance)];
                    self.receive_dialog.selected_core_index = 0;
                }
                Err(err) => {
                    self.receive_dialog.status = Some(err);
                    self.receive_dialog.core_addresses.clear();
                }
            }
        } else {
            self.receive_dialog.core_addresses = core_addresses;
            self.receive_dialog.selected_core_index = 0;
        }
    }

    /// Load Platform addresses into the receive dialog
    fn load_platform_addresses_for_receive(&mut self, wallet: &Arc<RwLock<Wallet>>) {
        let wallet_guard = match wallet.read() {
            Ok(guard) => guard,
            Err(err) => {
                self.receive_dialog.status = Some(err.to_string());
                return;
            }
        };

        // Collect Platform addresses with their balances (using DIP-18 Bech32m format)
        let network = self.app_context.network;
        let platform_addresses: Vec<(String, u64)> = wallet_guard
            .platform_address_info
            .iter()
            .filter_map(|(addr, info)| {
                use dash_sdk::dpp::address_funds::PlatformAddress;
                PlatformAddress::try_from(addr.clone())
                    .ok()
                    .map(|pa| (pa.to_bech32m_string(network), info.balance))
            })
            .collect();

        drop(wallet_guard);

        if platform_addresses.is_empty() {
            // Generate a new Platform address if none exists
            match self.generate_platform_address(wallet) {
                Ok(address) => {
                    self.receive_dialog.platform_addresses = vec![(address, 0)];
                    self.receive_dialog.selected_platform_index = 0;
                }
                Err(err) => {
                    self.receive_dialog.status = Some(err);
                    self.receive_dialog.platform_addresses.clear();
                }
            }
        } else {
            self.receive_dialog.platform_addresses = platform_addresses;
            self.receive_dialog.selected_platform_index = 0;
        }
    }

    fn categorize_path(
        path: &DerivationPath,
        reference: DerivationPathReference,
    ) -> (AccountCategory, Option<u32>) {
        let category = AccountCategory::from_reference(reference);
        let index = match category {
            AccountCategory::Bip44 | AccountCategory::Bip32 => path.bip44_account_index(),
            _ => None,
        };
        (category, index)
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

    fn derive_private_key_wif(&self, path: &DerivationPath) -> Result<String, String> {
        let wallet_arc = self
            .selected_wallet
            .clone()
            .ok_or_else(|| "Select a wallet first".to_string())?;
        let wallet = wallet_arc.read().map_err(|e| e.to_string())?;
        if wallet.uses_password && !wallet.is_open() {
            return Err("Unlock this wallet to view private keys.".to_string());
        }
        let private_key = wallet.private_key_at_derivation_path(path, self.app_context.network)?;
        Ok(private_key.to_wif())
    }

    fn lock_selected_wallet(&mut self) {
        let Some(wallet_arc) = self.selected_wallet.clone() else {
            return;
        };

        let locked = {
            let mut wallet = match wallet_arc.write() {
                Ok(guard) => guard,
                Err(err) => {
                    self.display_message(
                        &format!("Failed to lock wallet: {}", err),
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
            self.display_message("Wallet locked", MessageType::Info);
        }
    }

    /// Render the detail view for a selected single key wallet
    fn render_single_key_wallet_view(&mut self, ui: &mut Ui, dark_mode: bool) -> AppAction {
        let mut action = AppAction::None;

        let wallet_arc = match &self.selected_single_key_wallet {
            Some(w) => w.clone(),
            None => return action,
        };

        let wallet = wallet_arc.read().unwrap();
        let address = wallet.address.to_string();
        let alias = wallet
            .alias
            .clone()
            .unwrap_or_else(|| "Unnamed Key".to_string());
        let balance_duffs = wallet.total_balance_duffs();
        let balance_dash = balance_duffs as f64 * 1e-8;
        let utxo_count = wallet.utxos.len();
        let utxos: Vec<_> = wallet.utxos.iter().map(|(o, t)| (*o, t.clone())).collect();
        drop(wallet);

        let text_color = DashColors::text_primary(dark_mode);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(16, 16))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading(RichText::new(&alias).strong().color(text_color));
                    ui.add_space(10.0);

                    // Balance info
                    ui.label(RichText::new(format!("Balance: {:.8} DASH", balance_dash)));
                    ui.add_space(10.0);

                    // Action buttons for SK wallet
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Send").color(text_color).strong())
                            .clicked()
                        {
                            action = AppAction::AddScreen(
                                crate::ui::ScreenType::SingleKeyWalletSendScreen(
                                    wallet_arc.clone(),
                                )
                                .create_screen(&self.app_context),
                            );
                        }

                        if ui
                            .button(RichText::new("Receive").color(text_color))
                            .clicked()
                        {
                            self.receive_dialog.core_addresses =
                                vec![(address.clone(), balance_duffs)];
                            self.receive_dialog.selected_core_index = 0;
                            self.receive_dialog.is_open = true;
                        }
                    });
                    ui.add_space(15.0);

                    // UTXOs section
                    ui.separator();
                    ui.add_space(10.0);
                    ui.heading(RichText::new(format!("UTXOs ({})", utxo_count)).color(text_color));
                    ui.add_space(10.0);

                    if utxos.is_empty() {
                        ui.label("No UTXOs available. Click 'Refresh' to load UTXOs from Core.");
                    } else {
                        const UTXOS_PER_PAGE: usize = 50;
                        let total_pages = utxo_count.div_ceil(UTXOS_PER_PAGE);

                        // Ensure current page is valid
                        if self.utxo_page >= total_pages {
                            self.utxo_page = total_pages.saturating_sub(1);
                        }

                        let start_idx = self.utxo_page * UTXOS_PER_PAGE;
                        let utxos_page: Vec<_> =
                            utxos.iter().skip(start_idx).take(UTXOS_PER_PAGE).collect();

                        // Pagination controls
                        if total_pages > 1 {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(self.utxo_page > 0, egui::Button::new("<< First"))
                                    .clicked()
                                {
                                    self.utxo_page = 0;
                                }
                                if ui
                                    .add_enabled(self.utxo_page > 0, egui::Button::new("< Prev"))
                                    .clicked()
                                {
                                    self.utxo_page = self.utxo_page.saturating_sub(1);
                                }

                                ui.label(format!(
                                    "Page {} of {} ({}-{} of {})",
                                    self.utxo_page + 1,
                                    total_pages,
                                    start_idx + 1,
                                    (start_idx + utxos_page.len()).min(utxo_count),
                                    utxo_count
                                ));

                                if ui
                                    .add_enabled(
                                        self.utxo_page < total_pages - 1,
                                        egui::Button::new("Next >"),
                                    )
                                    .clicked()
                                {
                                    self.utxo_page += 1;
                                }
                                if ui
                                    .add_enabled(
                                        self.utxo_page < total_pages - 1,
                                        egui::Button::new("Last >>"),
                                    )
                                    .clicked()
                                {
                                    self.utxo_page = total_pages - 1;
                                }
                            });
                            ui.add_space(10.0);
                        }

                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for (outpoint, tx_out) in utxos_page {
                                    Frame::group(ui.style())
                                        .fill(DashColors::surface(dark_mode).gamma_multiply(0.9))
                                        .inner_margin(Margin::symmetric(10, 8))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.label("TxID:");
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{}:{}",
                                                                outpoint.txid, outpoint.vout
                                                            ))
                                                            .monospace()
                                                            .size(11.0)
                                                            .color(text_color),
                                                        );
                                                    });
                                                    ui.horizontal(|ui| {
                                                        ui.label("Amount:");
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{:.8} DASH",
                                                                tx_out.value as f64 * 1e-8
                                                            ))
                                                            .strong()
                                                            .color(text_color),
                                                        );
                                                    });
                                                });
                                            });
                                        });
                                    ui.add_space(5.0);
                                }
                            });
                    }
                });
            });

        action
    }

    /// Creates the appropriate refresh action based on the current refresh mode
    fn create_refresh_action(&self, wallet_arc: &Arc<RwLock<Wallet>>) -> AppAction {
        use crate::backend_task::wallet::PlatformSyncMode;

        let seed_hash = wallet_arc
            .read()
            .ok()
            .map(|w| w.seed_hash())
            .unwrap_or_default();

        match self.refresh_mode {
            RefreshMode::All => {
                // Default behavior: Core + Platform (Auto)
                AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(
                    wallet_arc.clone(),
                    Some(PlatformSyncMode::Auto),
                )))
            }
            RefreshMode::CoreOnly => {
                // Core only, no Platform sync
                AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(
                    wallet_arc.clone(),
                    None,
                )))
            }
            RefreshMode::PlatformFull => {
                // Platform only with forced full sync
                AppAction::BackendTask(BackendTask::WalletTask(
                    crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances {
                        seed_hash,
                        sync_mode: PlatformSyncMode::ForceFull,
                    },
                ))
            }
            RefreshMode::PlatformTerminal => {
                // Platform only with terminal sync
                AppAction::BackendTask(BackendTask::WalletTask(
                    crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances {
                        seed_hash,
                        sync_mode: PlatformSyncMode::TerminalOnly,
                    },
                ))
            }
            RefreshMode::CoreAndPlatformFull => {
                // Core + Platform with forced full sync
                AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(
                    wallet_arc.clone(),
                    Some(PlatformSyncMode::ForceFull),
                )))
            }
            RefreshMode::CoreAndPlatformTerminal => {
                // Core + Platform with terminal sync
                AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(
                    wallet_arc.clone(),
                    Some(PlatformSyncMode::TerminalOnly),
                )))
            }
        }
    }

    /// Creates the appropriate refresh action using the pending refresh mode
    fn create_pending_refresh_action(&self, wallet_arc: &Arc<RwLock<Wallet>>) -> AppAction {
        use crate::backend_task::wallet::PlatformSyncMode;

        let seed_hash = wallet_arc
            .read()
            .ok()
            .map(|w| w.seed_hash())
            .unwrap_or_default();

        match self.pending_refresh_mode {
            RefreshMode::All => AppAction::BackendTask(BackendTask::CoreTask(
                CoreTask::RefreshWalletInfo(wallet_arc.clone(), Some(PlatformSyncMode::Auto)),
            )),
            RefreshMode::CoreOnly => AppAction::BackendTask(BackendTask::CoreTask(
                CoreTask::RefreshWalletInfo(wallet_arc.clone(), None),
            )),
            RefreshMode::PlatformFull => AppAction::BackendTask(BackendTask::WalletTask(
                crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances {
                    seed_hash,
                    sync_mode: PlatformSyncMode::ForceFull,
                },
            )),
            RefreshMode::PlatformTerminal => AppAction::BackendTask(BackendTask::WalletTask(
                crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances {
                    seed_hash,
                    sync_mode: PlatformSyncMode::TerminalOnly,
                },
            )),
            RefreshMode::CoreAndPlatformFull => AppAction::BackendTask(BackendTask::CoreTask(
                CoreTask::RefreshWalletInfo(wallet_arc.clone(), Some(PlatformSyncMode::ForceFull)),
            )),
            RefreshMode::CoreAndPlatformTerminal => {
                AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(
                    wallet_arc.clone(),
                    Some(PlatformSyncMode::TerminalOnly),
                )))
            }
        }
    }
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();

        // Check for pending platform balance refresh (triggered after transfers)
        let pending_refresh_action =
            if let Some(seed_hash) = self.pending_platform_balance_refresh.take() {
                AppAction::BackendTask(BackendTask::WalletTask(
                    crate::backend_task::wallet::WalletTask::FetchPlatformAddressBalances {
                        seed_hash,
                        sync_mode: crate::backend_task::wallet::PlatformSyncMode::Auto,
                    },
                ))
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

            // Display messages at the top, outside of scroll area
            let message = self.message.clone();
            if let Some((message, message_type, _timestamp)) = message {
                let message_color = match message_type {
                    MessageType::Error => egui::Color32::from_rgb(255, 100, 100),
                    MessageType::Info => DashColors::text_primary(dark_mode),
                    MessageType::Success => egui::Color32::DARK_GREEN,
                };

                // Display message in a prominent frame with text wrapping
                Frame::new()
                    .fill(message_color.gamma_multiply(0.1))
                    .inner_margin(Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .stroke(egui::Stroke::new(1.0, message_color))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&message).color(message_color),
                                )
                                .wrap(),
                            );
                            ui.add_space(5.0);
                            if ui.small_button("Dismiss").clicked() {
                                self.dismiss_message();
                            }
                        });
                    });
                ui.add_space(10.0);
            }

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
        self.render_private_key_dialog(ctx);

        // Rename dialog
        if self.show_rename_dialog {
            egui::Window::new("Rename Wallet")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.label("Enter new wallet name:");
                        ui.add_space(5.0);

                        let text_edit = egui::TextEdit::singleline(&mut self.rename_input)
                            .hint_text("Enter wallet name")
                            .desired_width(250.0);
                        ui.add(text_edit);

                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
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

                            if ui.button("Cancel").clicked() {
                                self.show_rename_dialog = false;
                                self.rename_input.clear();
                            }
                        });
                    });
                });
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
                                self.display_message(&err, MessageType::Error);
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
                            self.display_message(
                                "Searching for unused asset locks...",
                                MessageType::Info,
                            );
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

                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let mut attempt_unlock = false;

                        ui.horizontal(|ui| {
                            let password_input = ui.add(
                                egui::TextEdit::singleline(&mut self.sk_wallet_password)
                                    .password(!self.sk_show_password)
                                    .hint_text("Enter password")
                                    .desired_width(250.0)
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );

                            if password_input.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                attempt_unlock = true;
                            }
                        });

                        ui.add_space(5.0);

                        ui.checkbox(&mut self.sk_show_password, "Show Password");

                        ui.add_space(10.0);

                        ui.horizontal(|ui| {
                            if ui.button("Unlock").clicked() {
                                attempt_unlock = true;
                            }

                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });

                        if attempt_unlock {
                            if let Some(wallet_arc) = &self.selected_single_key_wallet {
                                let mut wallet = wallet_arc.write().unwrap();
                                let unlock_result = wallet.open(&self.sk_wallet_password);

                                match unlock_result {
                                    Ok(_) => {
                                        self.sk_error_message = None;
                                        close_dialog = true;
                                    }
                                    Err(_) => {
                                        self.sk_error_message =
                                            Some("Incorrect Password".to_string());
                                    }
                                }
                            }
                            self.sk_wallet_password.clear();
                        }

                        // Display error message if the password was incorrect
                        if let Some(error_message) = self.sk_error_message.clone() {
                            ui.add_space(5.0);
                            let error_color = Color32::from_rgb(255, 100, 100);
                            Frame::new()
                                .fill(error_color.gamma_multiply(0.1))
                                .inner_margin(Margin::symmetric(10, 8))
                                .corner_radius(5.0)
                                .stroke(egui::Stroke::new(1.0, error_color))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("Error: {}", error_message)).color(error_color));
                                        ui.add_space(10.0);
                                        if ui.small_button("Dismiss").clicked() {
                                            self.sk_error_message = None;
                                        }
                                    });
                                });
                        }
                    });
                });

            if close_dialog {
                self.show_sk_unlock_dialog = false;
                self.sk_wallet_password.clear();
                self.sk_error_message = None;

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
                    self.display_message("Searching for unused asset locks...", MessageType::Info);
                    action = AppAction::BackendTask(BackendTask::CoreTask(
                        CoreTask::RecoverAssetLocks(wallet_arc),
                    ));
                }
            }
        }

        // Combine with pending refresh action
        action |= pending_refresh_action;
        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.refreshing = false;

            // If the fund platform dialog is processing, show error in the dialog instead
            if self.fund_platform_dialog.is_processing {
                self.fund_platform_dialog.is_processing = false;
                self.fund_platform_dialog.status = Some(message.to_string());
                self.fund_platform_dialog.status_is_error = true;
                return;
            }
        }
        self.message = Some((message.to_string(), message_type, Utc::now()))
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::ui::BackendTaskSuccessResult::RefreshedWallet { warning } => {
                self.refreshing = false;
                if let Some(warn_msg) = warning {
                    self.message = Some((
                        format!("Wallet refreshed with warning: {}", warn_msg),
                        MessageType::Info,
                        Utc::now(),
                    ));
                } else {
                    self.message = Some((
                        "Successfully refreshed wallet".to_string(),
                        MessageType::Success,
                        Utc::now(),
                    ));
                }
            }
            crate::ui::BackendTaskSuccessResult::RecoveredAssetLocks {
                recovered_count,
                total_amount,
            } => {
                let msg = if recovered_count == 0 {
                    "No additional unused asset locks found".to_string()
                } else {
                    format!(
                        "Found {} unused asset lock(s) worth {} Dash",
                        recovered_count,
                        Self::format_dash(total_amount)
                    )
                };
                self.display_message(&msg, MessageType::Success);
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
                self.display_message(&msg, MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address } => {
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
                }
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressWithdrawal { .. } => {
                self.display_message("Platform withdrawal successful. Note: It may take a few minutes for funds to appear on the Core chain.", MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressFunded { .. } => {
                self.fund_platform_dialog.is_processing = false;
                self.fund_platform_dialog.status = Some("Funding successful!".to_string());
                self.fund_platform_dialog.status_is_error = false;
                self.display_message("Platform address funded successfully", MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash } => {
                self.display_message(
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
                self.message = Some((
                    "Successfully synced Platform balances".to_string(),
                    MessageType::Success,
                    Utc::now(),
                ));
            }
            crate::ui::BackendTaskSuccessResult::Message(msg) => {
                self.refreshing = false;
                self.display_message(&msg, MessageType::Success);
            }
            _ => {}
        }
    }

    fn refresh_on_arrival(&mut self) {
        // Check if there's a pending wallet selection (e.g., from wallet creation/import)
        if let Ok(mut pending) = self.app_context.pending_wallet_selection.lock()
            && let Some(seed_hash) = pending.take()
            && let Ok(wallets) = self.app_context.wallets.read()
            && let Some(wallet) = wallets.get(&seed_hash)
        {
            self.selected_wallet = Some(wallet.clone());
            self.selected_single_key_wallet = None; // Clear SK selection
            self.selected_account = None;
            // Persist selection to AppContext and database
            if let Ok(mut guard) = self.app_context.selected_wallet_hash.lock() {
                *guard = Some(seed_hash);
            }
            if let Ok(mut guard) = self.app_context.selected_single_key_hash.lock() {
                *guard = None;
            }
            let _ = self
                .app_context
                .db
                .update_selected_wallet_hash(Some(&seed_hash));
            let _ = self.app_context.db.update_selected_single_key_hash(None);
            return;
        }

        // If no wallet of either type is selected but wallets exist, select the first HD wallet
        if self.selected_wallet.is_none() && self.selected_single_key_wallet.is_none() {
            if let Ok(wallets) = self.app_context.wallets.read()
                && let Some(wallet) = wallets.values().next().cloned()
            {
                self.selected_wallet = Some(wallet);
                return;
            }
            // If no HD wallets, try single key wallets
            if let Ok(wallets) = self.app_context.single_key_wallets.read() {
                self.selected_single_key_wallet = wallets.values().next().cloned();
            }
        }
    }

    fn refresh(&mut self) {}
}
