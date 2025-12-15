use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use crate::backend_task::wallet::WalletTask;
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::wallet::{
    DerivationPathHelpers, DerivationPathReference, Wallet, WalletSeedHash, WalletTransaction,
};
use crate::spv::CoreBackendMode;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
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
    show_unlock_dialog: bool,
    show_sk_unlock_dialog: bool,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
    remove_wallet_dialog: Option<ConfirmationDialog>,
    pending_wallet_removal: Option<WalletSeedHash>,
    pending_wallet_removal_alias: Option<String>,
    send_dialog: SendDialogState,
    receive_dialog: ReceiveDialogState,
    platform_receive_dialog: PlatformReceiveDialogState,
    fund_platform_dialog: FundPlatformAddressDialogState,
    withdraw_platform_dialog: WithdrawPlatformDialogState,
    selected_account: Option<(AccountCategory, Option<u32>)>,
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
    amount: String,
    subtract_fee: bool,
    memo: String,
    error: Option<String>,
}

#[derive(Default)]
struct ReceiveDialogState {
    is_open: bool,
    address: Option<String>,
    qr_texture: Option<TextureHandle>,
    qr_address: Option<String>,
    status: Option<String>,
}

/// State for the Platform address receive dialog
#[derive(Default)]
struct PlatformReceiveDialogState {
    is_open: bool,
    /// List of Platform addresses with their balances: (display_address, balance_credits)
    addresses: Vec<(String, u64)>,
    /// Currently selected address index
    selected_index: usize,
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
    is_processing: bool,
}

/// State for the Withdraw from Platform Address dialog
#[derive(Default)]
struct WithdrawPlatformDialogState {
    is_open: bool,
    /// Selected Platform address to withdraw from
    selected_platform_address: Option<String>,
    /// Platform addresses with balances
    platform_addresses: Vec<(String, u64)>,
    /// Withdrawal amount input
    amount_input: String,
    /// Destination Core address
    destination_address: String,
    status: Option<String>,
    is_processing: bool,
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
            if let Some(sk_hash) = selected_sk_hash {
                if let Ok(sk_wallets) = app_context.single_key_wallets.read() {
                    if let Some(wallet) = sk_wallets.get(&sk_hash) {
                        return Self::create_with_selection(
                            app_context,
                            None,
                            Some(wallet.clone()),
                        );
                    }
                }
            }

            // If we have a persisted HD wallet selection, try to find it
            if let Some(hd_hash) = selected_hd_hash {
                if let Ok(wallets) = app_context.wallets.read() {
                    if let Some(wallet) = wallets.get(&hd_hash) {
                        return Self::create_with_selection(
                            app_context,
                            Some(wallet.clone()),
                            None,
                        );
                    }
                }
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
            show_unlock_dialog: false,
            show_sk_unlock_dialog: false,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
            remove_wallet_dialog: None,
            pending_wallet_removal: None,
            pending_wallet_removal_alias: None,
            send_dialog: SendDialogState::default(),
            receive_dialog: ReceiveDialogState::default(),
            platform_receive_dialog: PlatformReceiveDialogState::default(),
            fund_platform_dialog: FundPlatformAddressDialogState::default(),
            withdraw_platform_dialog: WithdrawPlatformDialogState::default(),
            selected_account: None,
        }
    }

    pub(crate) fn update_selected_wallet_for_network(&mut self) {
        // Check if HD wallet selection is still valid
        if let Some(wallet_arc) = &self.selected_wallet {
            let seed_hash = wallet_arc.read().ok().map(|w| w.seed_hash());
            if let Some(hash) = seed_hash {
                if let Ok(wallets) = self.app_context.wallets.read() {
                    if wallets.contains_key(&hash) {
                        self.selected_account = None;
                        return;
                    }
                }
            }
            // HD wallet no longer valid
            self.selected_wallet = None;
        }

        // Check if single key wallet selection is still valid
        if let Some(wallet_arc) = &self.selected_single_key_wallet {
            let key_hash = wallet_arc.read().ok().map(|w| w.key_hash());
            if let Some(hash) = key_hash {
                if let Ok(wallets) = self.app_context.single_key_wallets.read() {
                    if wallets.contains_key(&hash) {
                        self.selected_account = None;
                        return;
                    }
                }
            }
            // Single key wallet no longer valid
            self.selected_single_key_wallet = None;
        }

        // No valid selection, pick a new one (HD wallet first, then single key)
        if let Ok(wallets) = self.app_context.wallets.read() {
            if let Some(wallet) = wallets.values().next().cloned() {
                self.selected_wallet = Some(wallet);
                self.selected_single_key_wallet = None;
                self.selected_account = None;
                return;
            }
        }

        if let Ok(wallets) = self.app_context.single_key_wallets.read() {
            if let Some(wallet) = wallets.values().next().cloned() {
                self.selected_single_key_wallet = Some(wallet);
                self.selected_wallet = None;
                self.selected_account = None;
                return;
            }
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
                                            // Persist selection to AppContext
                                            if let Ok(hash) = w.read().map(|g| g.seed_hash()) {
                                                if let Ok(mut guard) =
                                                    self.app_context.selected_wallet_hash.lock()
                                                {
                                                    *guard = Some(hash);
                                                }
                                            }
                                            if let Ok(mut guard) =
                                                self.app_context.selected_single_key_hash.lock()
                                            {
                                                *guard = None;
                                            }
                                        }
                                        WalletItem::SingleKey(w) => {
                                            self.selected_single_key_wallet = Some(w.clone());
                                            self.selected_wallet = None;
                                            // Persist selection to AppContext
                                            if let Ok(hash) = w.read().map(|g| g.key_hash) {
                                                if let Ok(mut guard) =
                                                    self.app_context.selected_single_key_hash.lock()
                                                {
                                                    *guard = Some(hash);
                                                }
                                            }
                                            if let Ok(mut guard) =
                                                self.app_context.selected_wallet_hash.lock()
                                            {
                                                *guard = None;
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
                                self.show_unlock_dialog = true;
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
                                // Clear persisted selection
                                if let Ok(mut guard) =
                                    self.app_context.selected_single_key_hash.lock()
                                {
                                    *guard = None;
                                }
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
                        if should_lock_sk_wallet {
                            if let Ok(mut wallet) = wallet_arc.write() {
                                wallet.private_key_data.close();
                            }
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
                    let platform_credits = wallet
                        .platform_address_info
                        .get(address)
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
                            // For Platform addresses, show credits balance; for others, show Core balance
                            if data.account_category == AccountCategory::PlatformPayment {
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
                            ui.label(format!("{}", data.utxo_count));
                        });
                        row.col(|ui| {
                            let dash_received = data.total_received as f64 * 1e-8;
                            ui.label(format!("{:.8}", dash_received));
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
                                match self.derive_private_key_wif(&data.derivation_path) {
                                    Ok(key) => self.display_message(
                                        &format!("{}\n{}", data.address, key),
                                        MessageType::Info,
                                    ),
                                    Err(err) => self.display_message(&err, MessageType::Error),
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

                // Update persisted selection
                if let Ok(mut guard) = self.app_context.selected_wallet_hash.lock() {
                    *guard = next_wallet
                        .as_ref()
                        .and_then(|w| w.read().ok().map(|g| g.seed_hash()));
                }

                self.show_rename_dialog = false;
                self.rename_input.clear();
                self.wallet_password.clear();
                self.show_password = false;
                self.error_message = None;
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
                    ui.heading(RichText::new("Asset Locks").color(DashColors::text_primary(dark_mode)));
                    ui.add_space(10.0);

                    if wallet.unused_asset_locks.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("No asset locks found").color(Color32::GRAY).size(14.0));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Asset locks are special transactions that can be used to create identities or fund Platform addresses").color(Color32::GRAY).size(12.0));
                            ui.add_space(15.0);
                            if ui.button("Search for asset locks").clicked() {
                                app_action = AppAction::BackendTask(BackendTask::CoreTask(
                                    CoreTask::RefreshWalletInfo(arc_wallet.clone()),
                                ))
                            };
                            ui.add_space(20.0);
                        });
                    } else {
                        // Collect Platform addresses for the fund dialog (using DIP-18 Bech32m format)
                        let network = self.app_context.network;
                        let platform_addresses: Vec<(String, u64)> = wallet
                            .platform_address_info
                            .iter()
                            .filter_map(|(addr, info)| {
                                use dash_sdk::dpp::address_funds::PlatformAddress;
                                PlatformAddress::try_from(addr.clone())
                                    .ok()
                                    .map(|pa| (pa.to_bech32m_string(network), info.balance))
                            })
                            .collect();

                        egui::ScrollArea::both()
                            .id_salt("asset_locks_table")
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
                        .column(Column::initial(150.0)) // Actions
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
                            for (idx, (tx, address, amount, islock, proof)) in wallet.unused_asset_locks.iter().enumerate() {
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
                                        if proof.is_some() {
                                            if ui.small_button("Fund Platform Addr").on_hover_text("Fund a Platform address with this asset lock").clicked() {
                                                open_fund_dialog_for_idx = Some((idx, platform_addresses.clone()));
                                            }
                                        } else {
                                            ui.label(RichText::new("Not ready").color(Color32::GRAY).size(11.0));
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
                         on \"Import Mnemonic\" at the top right, or",
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

    fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
        let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
        amount.dash_to_duffs()
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
        let confirmed = wallet.confirmed_balance_duffs();
        let _unconfirmed = wallet.unconfirmed_balance_duffs();
        let platform = Self::platform_balance_duffs(wallet);

        ui.horizontal(|ui| {
            ui.label(RichText::new(format!(
                "Core balance: {}",
                Self::format_dash(total)
            )));
            ui.label(
                RichText::new(format!("(Confirmed: {})", Self::format_dash(confirmed)))
                    .color(DashColors::text_secondary(dark_mode)),
            );
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

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Platform address receive button
            if ui
                .button(
                    RichText::new("Receive Platform Credits")
                        .color(DashColors::text_primary(dark_mode)),
                )
                .on_hover_text("Show Platform address to receive credits ")
                .clicked()
            {
                action |= self.open_platform_receive_dialog(ctx);
            }

            // Withdraw from Platform address button
            if ui
                .button(
                    RichText::new("Withdraw Platform Credits")
                        .color(DashColors::text_primary(dark_mode)),
                )
                .on_hover_text("Withdraw credits from Platform address to Core ")
                .clicked()
            {
                action |= self.open_withdraw_platform_dialog();
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

        for summary in summaries {
            let is_selected = self
                .selected_account
                .as_ref()
                .map(|(cat, idx)| cat == &summary.category && *idx == summary.index)
                .unwrap_or(false);

            let dark_mode = ui.ctx().style().visuals.dark_mode;
            let fill = if is_selected {
                DashColors::glass_blue(dark_mode)
            } else {
                ui.visuals().extreme_bg_color
            };
            let stroke_color = if is_selected {
                DashColors::DASH_BLUE
            } else {
                DashColors::border_light(dark_mode)
            };

            let response = Frame::group(ui.style())
                .fill(fill)
                .stroke(egui::Stroke::new(
                    if is_selected { 2.0 } else { 1.0 },
                    stroke_color,
                ))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&summary.label)
                                .strong()
                                .color(DashColors::text_primary(ui.ctx().style().visuals.dark_mode))
                                .size(16.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Show Platform credits for Platform Payment accounts
                            if summary.category == AccountCategory::PlatformPayment {
                                // Display Platform credits (convert to DASH equivalent)
                                let credits_as_dash =
                                    summary.platform_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.label(
                                    RichText::new(format!("{:.8} DASH", credits_as_dash))
                                        .strong()
                                        .color(DashColors::text_primary(
                                            ui.ctx().style().visuals.dark_mode,
                                        )),
                                );
                            } else {
                                ui.label(
                                    RichText::new(Self::format_dash(summary.confirmed_balance))
                                        .strong()
                                        .color(DashColors::text_primary(
                                            ui.ctx().style().visuals.dark_mode,
                                        )),
                                );
                            }
                        });
                    });

                    if let Some(description) = summary.category.description() {
                        ui.label(
                            RichText::new(description)
                                .color(DashColors::text_secondary(dark_mode))
                                .italics(),
                        );
                    }

                    ui.label(format!(
                        "Addresses: {} ({} receive / {} change)",
                        summary.total_addresses,
                        summary.external_addresses,
                        summary.internal_addresses
                    ));
                })
                .response
                .interact(egui::Sense::click());

            if response.clicked() {
                self.selected_account = Some((summary.category.clone(), summary.index));
            }

            ui.add_space(6.0);
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
                            collect_account_summaries(&wallet, self.app_context.network)
                        };

                        self.ensure_account_selection(&summaries);
                        action |= self.render_action_buttons(ui, ctx);
                        ui.add_space(10.0);
                        ui.separator();
                        self.render_transactions_section(ui);
                        ui.add_space(10.0);
                        ui.separator();
                        self.render_accounts_section(ui, &summaries);
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.heading(
                            RichText::new("Addresses").color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(8.0);
                        action |= self.render_address_table(ui);

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

                ui.label("Amount (DASH)");
                ui.add(egui::TextEdit::singleline(&mut self.send_dialog.amount).hint_text("0.01"));

                ui.checkbox(
                    &mut self.send_dialog.subtract_fee,
                    "Subtract fee from amount",
                );

                ui.label("Memo (optional)");
                ui.add(egui::TextEdit::singleline(&mut self.send_dialog.memo));

                if let Some(error) = &self.send_dialog.error {
                    ui.colored_label(Color32::DARK_RED, error);
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

        if let Some(address) = self.receive_dialog.address.clone() {
            let needs_texture = self.receive_dialog.qr_texture.is_none()
                || self.receive_dialog.qr_address.as_deref() != Some(&address);
            if needs_texture {
                match generate_qr_code_image(&address) {
                    Ok(image) => {
                        let texture = ctx.load_texture(
                            format!("wallet_receive_{}", address),
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
        egui::Window::new("Receive Dash")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(texture) = &self.receive_dialog.qr_texture {
                    ui.image(SizedTexture::new(texture.id(), egui::vec2(220.0, 220.0)));
                } else if self.receive_dialog.address.is_some() {
                    ui.label("Preparing QR code...");
                }

                if let Some(address) = &self.receive_dialog.address {
                    ui.add_space(6.0);
                    ui.label(address);
                    if ui.button("Copy Address").clicked() {
                        if let Err(err) = copy_text_to_clipboard(address) {
                            self.receive_dialog.status = Some(err);
                        } else {
                            self.receive_dialog.status = Some("Address copied".to_string());
                        }
                    }
                }

                if let Some(status) = &self.receive_dialog.status {
                    ui.label(status);
                }
            });

        self.receive_dialog.is_open = open;
        if !self.receive_dialog.is_open {
            self.receive_dialog = ReceiveDialogState::default();
        }
        AppAction::None
    }

    /// Open the Platform address receive dialog
    fn open_platform_receive_dialog(&mut self, _ctx: &Context) -> AppAction {
        let Some(wallet) = self.selected_wallet.clone() else {
            self.platform_receive_dialog.status = Some("Select a wallet first".to_string());
            self.platform_receive_dialog.is_open = true;
            return AppAction::None;
        };

        // Check if wallet is locked
        {
            let wallet_guard = match wallet.read() {
                Ok(guard) => guard,
                Err(err) => {
                    self.platform_receive_dialog.status = Some(err.to_string());
                    self.platform_receive_dialog.is_open = true;
                    return AppAction::None;
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

            if platform_addresses.is_empty() {
                // Generate a new Platform address if none exists
                self.platform_receive_dialog.status =
                    Some("No Platform addresses found. Generating...".to_string());
                self.platform_receive_dialog.addresses.clear();
            } else {
                self.platform_receive_dialog.addresses = platform_addresses;
                self.platform_receive_dialog.selected_index = 0;
                self.platform_receive_dialog.status = None;
            }
        }

        // If no Platform addresses, try to generate one
        if self.platform_receive_dialog.addresses.is_empty() {
            match self.generate_platform_address(&wallet) {
                Ok(address) => {
                    self.platform_receive_dialog.addresses = vec![(address, 0)];
                    self.platform_receive_dialog.selected_index = 0;
                    self.platform_receive_dialog.status = None;
                }
                Err(err) => {
                    self.platform_receive_dialog.status = Some(err);
                }
            }
        }

        self.platform_receive_dialog.is_open = true;
        self.platform_receive_dialog.qr_texture = None;
        self.platform_receive_dialog.qr_address = None;

        AppAction::None
    }

    /// Generate a new Platform address for the wallet (or return existing one with zero balance)
    /// Returns the address in DIP-18 Bech32m format (e.g., tdashevo1... for testnet)
    fn generate_platform_address(&self, wallet: &Arc<RwLock<Wallet>>) -> Result<String, String> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
        let address = wallet_guard
            .platform_receive_address(self.app_context.network, false, Some(&self.app_context))
            .map_err(|e| e.to_string())?;
        // Convert to PlatformAddress and encode as Bech32m per DIP-18
        let platform_addr =
            PlatformAddress::try_from(address).map_err(|e| format!("Invalid address: {}", e))?;
        Ok(platform_addr.to_bech32m_string(self.app_context.network))
    }

    /// Render the Platform address receive dialog
    fn render_platform_receive_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.platform_receive_dialog.is_open {
            return AppAction::None;
        }

        // Get current selected address for QR code
        let current_address = self
            .platform_receive_dialog
            .addresses
            .get(self.platform_receive_dialog.selected_index)
            .map(|(addr, _)| addr.clone());

        // Generate QR texture if needed
        if let Some(address) = current_address.clone() {
            let needs_texture = self.platform_receive_dialog.qr_texture.is_none()
                || self.platform_receive_dialog.qr_address.as_deref() != Some(&address);
            if needs_texture {
                match generate_qr_code_image(&address) {
                    Ok(image) => {
                        let texture = ctx.load_texture(
                            format!("platform_receive_{}", address),
                            image,
                            TextureOptions::LINEAR,
                        );
                        self.platform_receive_dialog.qr_texture = Some(texture);
                        self.platform_receive_dialog.qr_address = Some(address);
                    }
                    Err(err) => {
                        self.platform_receive_dialog.status = Some(err.to_string());
                    }
                }
            }
        }

        let mut open = self.platform_receive_dialog.is_open;
        let dark_mode = ctx.style().visuals.dark_mode;

        egui::Window::new("Receive Platform Credits")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Platform Address ")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(12.0),
                    );
                    ui.add_space(10.0);

                    // Show QR code
                    if let Some(texture) = &self.platform_receive_dialog.qr_texture {
                        ui.image(SizedTexture::new(texture.id(), egui::vec2(220.0, 220.0)));
                    } else if !self.platform_receive_dialog.addresses.is_empty() {
                        ui.label("Generating QR code...");
                    }

                    ui.add_space(10.0);

                    // Address selector (if multiple addresses)
                    if self.platform_receive_dialog.addresses.len() > 1 {
                        ui.horizontal(|ui| {
                            ui.label("Address:");
                            ComboBox::from_id_salt("platform_addr_selector")
                                .selected_text(
                                    self.platform_receive_dialog
                                        .addresses
                                        .get(self.platform_receive_dialog.selected_index)
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
                                        self.platform_receive_dialog.addresses.iter().enumerate()
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
                                                idx == self.platform_receive_dialog.selected_index,
                                                label,
                                            )
                                            .clicked()
                                        {
                                            self.platform_receive_dialog.selected_index = idx;
                                            // Clear QR so it regenerates
                                            self.platform_receive_dialog.qr_texture = None;
                                            self.platform_receive_dialog.qr_address = None;
                                        }
                                    }
                                });
                        });
                        ui.add_space(5.0);
                    }

                    // Show selected address - clone values to avoid borrow issues
                    let selected_addr_data = self
                        .platform_receive_dialog
                        .addresses
                        .get(self.platform_receive_dialog.selected_index)
                        .cloned();

                    if let Some((address, balance)) = selected_addr_data {
                        ui.add_space(6.0);
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
                                && ui.button("New Address").clicked() {
                                    new_addr_result = Some(self.generate_platform_address(wallet));
                                }
                        });

                        // Handle copy status after the closure
                        if let Some(status) = copy_status {
                            self.platform_receive_dialog.status = Some(status);
                        }

                        // Handle new address generation after the closure
                        if let Some(result) = new_addr_result {
                            match result {
                                Ok(new_addr) => {
                                    self.platform_receive_dialog.addresses.push((new_addr, 0));
                                    self.platform_receive_dialog.selected_index =
                                        self.platform_receive_dialog.addresses.len() - 1;
                                    self.platform_receive_dialog.qr_texture = None;
                                    self.platform_receive_dialog.qr_address = None;
                                    self.platform_receive_dialog.status =
                                        Some("New address generated!".to_string());
                                }
                                Err(err) => {
                                    self.platform_receive_dialog.status = Some(err);
                                }
                            }
                        }
                    }

                    if let Some(status) = &self.platform_receive_dialog.status {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(status).color(DashColors::text_secondary(dark_mode)),
                        );
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
                });
            });

        self.platform_receive_dialog.is_open = open;
        if !self.platform_receive_dialog.is_open {
            self.platform_receive_dialog = PlatformReceiveDialogState::default();
        }
        AppAction::None
    }

    /// Render the Fund Platform Address from Asset Lock dialog
    fn render_fund_platform_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.fund_platform_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.fund_platform_dialog.is_open;
        let dark_mode = ctx.style().visuals.dark_mode;

        egui::Window::new("Fund Platform Address from Asset Lock")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(5.0);
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
                        ui.label(RichText::new(status).color(DashColors::text_secondary(dark_mode)));
                        ui.add_space(10.0);
                    }

                    // Buttons
                    ui.horizontal(|ui| {
                        let can_fund = self.fund_platform_dialog.selected_platform_address.is_some()
                            && self.fund_platform_dialog.selected_asset_lock_index.is_some()
                            && !self.fund_platform_dialog.is_processing;

                        if ui
                            .add_enabled(
                                can_fund,
                                egui::Button::new(if self.fund_platform_dialog.is_processing {
                                    "Funding..."
                                } else {
                                    "Fund Address"
                                }),
                            )
                            .clicked()
                        {
                            action = self.prepare_fund_platform_action();
                        }

                        if ui.button("Cancel").clicked() {
                            self.fund_platform_dialog = FundPlatformAddressDialogState::default();
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

        self.fund_platform_dialog.is_open = open;
        if !self.fund_platform_dialog.is_open {
            self.fund_platform_dialog = FundPlatformAddressDialogState::default();
        }
        action
    }

    /// Prepare the backend task for funding a Platform address from asset lock
    fn prepare_fund_platform_action(&mut self) -> AppAction {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use std::collections::BTreeMap;

        let Some(wallet_arc) = &self.selected_wallet else {
            self.fund_platform_dialog.status = Some("No wallet selected".to_string());
            return AppAction::None;
        };

        let Some(selected_addr) = &self.fund_platform_dialog.selected_platform_address else {
            self.fund_platform_dialog.status = Some("Select a Platform address".to_string());
            return AppAction::None;
        };

        let Some(asset_lock_idx) = self.fund_platform_dialog.selected_asset_lock_index else {
            self.fund_platform_dialog.status = Some("No asset lock selected".to_string());
            return AppAction::None;
        };

        // Get the asset lock proof and address from the wallet
        let (seed_hash, asset_lock_proof, asset_lock_address, platform_addr) = {
            let wallet = match wallet_arc.read() {
                Ok(guard) => guard,
                Err(e) => {
                    self.fund_platform_dialog.status = Some(e.to_string());
                    return AppAction::None;
                }
            };

            let asset_lock = wallet.unused_asset_locks.get(asset_lock_idx);
            let Some((_, addr, _, _, Some(proof))) = asset_lock else {
                self.fund_platform_dialog.status =
                    Some("Asset lock not found or not ready".to_string());
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

        AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::FundPlatformAddressFromAssetLock {
                seed_hash,
                asset_lock_proof,
                asset_lock_address,
                outputs,
            },
        ))
    }

    /// Render the Withdraw from Platform Address dialog
    fn render_withdraw_platform_dialog(&mut self, ctx: &Context) -> AppAction {
        if !self.withdraw_platform_dialog.is_open {
            return AppAction::None;
        }

        let mut action = AppAction::None;
        let mut open = self.withdraw_platform_dialog.is_open;
        let dark_mode = ctx.style().visuals.dark_mode;

        egui::Window::new("Withdraw from Platform Address")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Withdraw credits from Platform to Core")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);

                    // Platform address selector (source)
                    ui.label("From Platform address:");
                    if self.withdraw_platform_dialog.platform_addresses.is_empty() {
                        ui.label(
                            RichText::new("No Platform addresses with balance found.")
                                .color(DashColors::text_secondary(dark_mode))
                                .italics(),
                        );
                    } else {
                        ComboBox::from_id_salt("withdraw_platform_addr_selector")
                            .selected_text(
                                self.withdraw_platform_dialog
                                    .selected_platform_address
                                    .as_deref()
                                    .map(|addr| {
                                        let balance = self
                                            .withdraw_platform_dialog
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
                                for (addr, balance) in
                                    &self.withdraw_platform_dialog.platform_addresses
                                {
                                    if *balance == 0 {
                                        continue; // Skip addresses with no balance
                                    }
                                    let credits_as_dash =
                                        *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                    let label = format!(
                                        "{}... ({:.4} DASH)",
                                        &addr[..12.min(addr.len())],
                                        credits_as_dash
                                    );
                                    let is_selected = self
                                        .withdraw_platform_dialog
                                        .selected_platform_address
                                        .as_deref()
                                        == Some(addr.as_str());
                                    if ui.selectable_label(is_selected, label).clicked() {
                                        self.withdraw_platform_dialog.selected_platform_address =
                                            Some(addr.clone());
                                    }
                                }
                            });
                    }

                    ui.add_space(10.0);

                    // Amount input
                    ui.label("Amount (DASH):");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut self.withdraw_platform_dialog.amount_input,
                            )
                            .hint_text("0.001")
                            .desired_width(150.0),
                        );

                        // Max button
                        if let Some(selected) =
                            &self.withdraw_platform_dialog.selected_platform_address
                            && let Some((_, balance)) = self
                                .withdraw_platform_dialog
                                .platform_addresses
                                .iter()
                                .find(|(a, _)| a == selected)
                            && ui.small_button("Max").clicked()
                        {
                            let max_dash = *balance as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                            self.withdraw_platform_dialog.amount_input = format!("{:.8}", max_dash);
                        }
                    });

                    ui.add_space(10.0);

                    // Destination Core address
                    ui.label("To Core address:");
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut self.withdraw_platform_dialog.destination_address,
                        )
                        .hint_text("y...")
                        .desired_width(350.0),
                    );

                    ui.add_space(15.0);

                    // Status message
                    if let Some(status) = &self.withdraw_platform_dialog.status {
                        ui.label(
                            RichText::new(status).color(DashColors::text_secondary(dark_mode)),
                        );
                        ui.add_space(10.0);
                    }

                    // Buttons
                    ui.horizontal(|ui| {
                        let can_withdraw = self
                            .withdraw_platform_dialog
                            .selected_platform_address
                            .is_some()
                            && !self.withdraw_platform_dialog.amount_input.is_empty()
                            && !self.withdraw_platform_dialog.destination_address.is_empty()
                            && !self.withdraw_platform_dialog.is_processing;

                        if ui
                            .add_enabled(
                                can_withdraw,
                                egui::Button::new(if self.withdraw_platform_dialog.is_processing {
                                    "Withdrawing..."
                                } else {
                                    "Withdraw"
                                }),
                            )
                            .clicked()
                        {
                            action = self.prepare_withdraw_platform_action();
                        }

                        if ui.button("Cancel").clicked() {
                            self.withdraw_platform_dialog = WithdrawPlatformDialogState::default();
                        }
                    });

                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Note: Withdrawals require waiting for chain confirmations.")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(11.0)
                            .italics(),
                    );
                });
            });

        self.withdraw_platform_dialog.is_open = open;
        if !self.withdraw_platform_dialog.is_open {
            self.withdraw_platform_dialog = WithdrawPlatformDialogState::default();
        }
        action
    }

    /// Prepare the backend task for withdrawing from a Platform address
    fn prepare_withdraw_platform_action(&mut self) -> AppAction {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::dpp::identity::core_script::CoreScript;
        use std::collections::BTreeMap;

        let Some(wallet_arc) = &self.selected_wallet else {
            self.withdraw_platform_dialog.status = Some("No wallet selected".to_string());
            return AppAction::None;
        };

        let Some(selected_addr) = &self.withdraw_platform_dialog.selected_platform_address else {
            self.withdraw_platform_dialog.status = Some("Select a Platform address".to_string());
            return AppAction::None;
        };

        // Parse amount
        let amount_dash: f64 = match self.withdraw_platform_dialog.amount_input.parse() {
            Ok(v) => v,
            Err(_) => {
                self.withdraw_platform_dialog.status = Some("Invalid amount".to_string());
                return AppAction::None;
            }
        };
        if amount_dash <= 0.0 {
            self.withdraw_platform_dialog.status = Some("Amount must be positive".to_string());
            return AppAction::None;
        }
        let amount_credits = (amount_dash * 1e8 * CREDITS_PER_DUFF as f64) as u64;

        // Parse destination address and create CoreScript
        use dash_sdk::dashcore_rpc::dashcore::address::NetworkUnchecked;
        let dest_addr_str = self.withdraw_platform_dialog.destination_address.trim();
        let output_script = match dest_addr_str.parse::<Address<NetworkUnchecked>>() {
            Ok(addr) => {
                let script_pubkey = addr.assume_checked().script_pubkey();
                CoreScript::new(script_pubkey)
            }
            Err(e) => {
                self.withdraw_platform_dialog.status =
                    Some(format!("Invalid destination address: {}", e));
                return AppAction::None;
            }
        };

        // Parse Platform address (Bech32m format: dashevo1.../tdashevo1...)
        let platform_addr =
            if selected_addr.starts_with("dashevo1") || selected_addr.starts_with("tdashevo1") {
                match PlatformAddress::from_bech32m_string(selected_addr) {
                    Ok((addr, _network)) => addr,
                    Err(e) => {
                        self.withdraw_platform_dialog.status =
                            Some(format!("Invalid Bech32m address: {}", e));
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
                        self.withdraw_platform_dialog.status = Some(e);
                        return AppAction::None;
                    }
                }
            };

        let seed_hash = {
            let wallet = match wallet_arc.read() {
                Ok(guard) => guard,
                Err(e) => {
                    self.withdraw_platform_dialog.status = Some(e.to_string());
                    return AppAction::None;
                }
            };
            wallet.seed_hash()
        };

        // Build inputs
        let mut inputs: BTreeMap<PlatformAddress, u64> = BTreeMap::new();
        inputs.insert(platform_addr, amount_credits);

        self.withdraw_platform_dialog.is_processing = true;
        self.withdraw_platform_dialog.status = Some("Processing withdrawal...".to_string());

        AppAction::BackendTask(BackendTask::WalletTask(
            WalletTask::WithdrawFromPlatformAddress {
                seed_hash,
                inputs,
                output_script,
                core_fee_per_byte: 1, // Default fee rate
            },
        ))
    }

    /// Open the Withdraw Platform dialog
    fn open_withdraw_platform_dialog(&mut self) -> AppAction {
        let Some(wallet) = self.selected_wallet.clone() else {
            self.withdraw_platform_dialog.status = Some("Select a wallet first".to_string());
            self.withdraw_platform_dialog.is_open = true;
            return AppAction::None;
        };

        // Collect Platform addresses with balances
        let platform_addresses: Vec<(String, u64)> = {
            let wallet_guard = match wallet.read() {
                Ok(guard) => guard,
                Err(e) => {
                    self.withdraw_platform_dialog.status = Some(e.to_string());
                    self.withdraw_platform_dialog.is_open = true;
                    return AppAction::None;
                }
            };

            let network = self.app_context.network;
            wallet_guard
                .platform_address_info
                .iter()
                .filter(|(_, info)| info.balance > 0)
                .filter_map(|(addr, info)| {
                    use dash_sdk::dpp::address_funds::PlatformAddress;
                    PlatformAddress::try_from(addr.clone())
                        .ok()
                        .map(|pa| (pa.to_bech32m_string(network), info.balance))
                })
                .collect()
        };

        self.withdraw_platform_dialog.platform_addresses = platform_addresses;
        self.withdraw_platform_dialog.selected_platform_address = None;
        self.withdraw_platform_dialog.amount_input = String::new();
        self.withdraw_platform_dialog.destination_address = String::new();
        self.withdraw_platform_dialog.status = None;
        self.withdraw_platform_dialog.is_processing = false;
        self.withdraw_platform_dialog.is_open = true;

        AppAction::None
    }

    fn prepare_send_action(&mut self) -> Result<AppAction, String> {
        let wallet = self
            .selected_wallet
            .as_ref()
            .ok_or_else(|| "Select a wallet first".to_string())?;

        let amount = Self::parse_amount_to_duffs(&self.send_dialog.amount)?;

        {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            if amount > wallet_guard.confirmed_balance_duffs() {
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
                amount_duffs: amount,
            }],
            subtract_fee_from_amount: self.send_dialog.subtract_fee,
            memo: if memo.is_empty() {
                None
            } else {
                Some(memo.to_string())
            },
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
            self.receive_dialog.address = None;
            self.receive_dialog.qr_texture = None;
            self.receive_dialog.qr_address = None;
            self.receive_dialog.is_open = true;
            return AppAction::None;
        };

        self.receive_dialog.is_open = true;

        if self.app_context.core_backend_mode() == CoreBackendMode::Spv {
            let seed_hash = match wallet.read() {
                Ok(guard) => guard.seed_hash(),
                Err(err) => {
                    self.receive_dialog.status = Some(err.to_string());
                    return AppAction::None;
                }
            };

            self.receive_dialog.address = None;
            self.receive_dialog.qr_texture = None;
            self.receive_dialog.qr_address = None;
            self.receive_dialog.status = Some("Requesting new address...".to_string());

            return AppAction::BackendTask(BackendTask::WalletTask(
                WalletTask::GenerateReceiveAddress { seed_hash },
            ));
        }

        match self.prepare_receive_dialog(&wallet) {
            Ok(()) => self.receive_dialog.status = None,
            Err(err) => self.receive_dialog.status = Some(err),
        }

        AppAction::None
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

    fn prepare_receive_dialog(&mut self, wallet: &Arc<RwLock<Wallet>>) -> Result<(), String> {
        let address = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            wallet_guard.receive_address(
                self.app_context.network,
                false,
                Some(&self.app_context),
            )?
        };

        let address_str = address.to_string();
        self.receive_dialog.address = Some(address_str);
        self.receive_dialog.qr_texture = None;
        self.receive_dialog.qr_address = None;
        Ok(())
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
        let utxos: Vec<_> = wallet
            .utxos
            .iter()
            .map(|(o, t)| (o.clone(), t.clone()))
            .collect();
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
                            self.receive_dialog.address = Some(address.clone());
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
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for (outpoint, tx_out) in &utxos {
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
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();
        let mut right_buttons = vec![
            (
                "Import Mnemonic",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportMnemonic)),
            ),
            (
                "Import Private Key",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportPrivateKey)),
            ),
            (
                "Create Wallet",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
            ),
        ];

        // Add Refresh button for HD wallet
        if !self.refreshing
            && self.app_context.core_backend_mode() == CoreBackendMode::Rpc
            && let Some(wallet_arc) = self.selected_wallet.clone()
        {
            right_buttons.push((
                "Refresh",
                DesiredAppAction::BackendTask(Box::new(BackendTask::CoreTask(
                    CoreTask::RefreshWalletInfo(wallet_arc),
                ))),
            ));
        }

        // Add Refresh button for single key wallet
        if !self.refreshing
            && self.app_context.core_backend_mode() == CoreBackendMode::Rpc
            && let Some(wallet_arc) = self.selected_single_key_wallet.clone()
        {
            right_buttons.push((
                "Refresh",
                DesiredAppAction::BackendTask(Box::new(BackendTask::CoreTask(
                    CoreTask::RefreshSingleKeyWalletInfo(wallet_arc),
                ))),
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

                // Display message in a prominent frame
                ui.horizontal(|ui| {
                    Frame::new()
                        .fill(message_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, message_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(message).color(message_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.dismiss_message();
                                }
                            });
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
        action |= self.render_platform_receive_dialog(ctx);
        action |= self.render_fund_platform_dialog(ctx);
        action |= self.render_withdraw_platform_dialog(ctx);

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

        // Unlock dialog
        if self.show_unlock_dialog {
            let mut close_dialog = false;
            egui::Window::new("Unlock Wallet")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.vertical(|ui| {
                        if let Some(wallet_arc) = &self.selected_wallet {
                            if let Ok(wallet) = wallet_arc.read() {
                                if let Some(alias) = &wallet.alias {
                                    ui.label(format!(
                                        "Wallet \"{}\" is locked. Please enter the password to unlock it:",
                                        alias
                                    ));
                                } else {
                                    ui.label("This wallet is locked. Please enter the password to unlock it:");
                                }
                            }
                        }

                        ui.add_space(10.0);

                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let mut attempt_unlock = false;

                        ui.horizontal(|ui| {
                            let password_input = ui.add(
                                egui::TextEdit::singleline(&mut self.wallet_password)
                                    .password(!self.show_password)
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

                        ui.checkbox(&mut self.show_password, "Show Password");

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
                            if let Some(wallet_arc) = &self.selected_wallet {
                                let mut wallet = wallet_arc.write().unwrap();
                                let unlock_result =
                                    wallet.wallet_seed.open(&self.wallet_password);

                                match unlock_result {
                                    Ok(_) => {
                                        self.error_message = None;
                                        close_dialog = true;
                                        // Trigger wallet unlocked handling
                                        drop(wallet);
                                        self.app_context.handle_wallet_unlocked(wallet_arc);
                                    }
                                    Err(_) => {
                                        if let Some(hint) = wallet.password_hint() {
                                            self.error_message = Some(format!(
                                                "Incorrect Password, password hint is {}",
                                                hint
                                            ));
                                        } else {
                                            self.error_message =
                                                Some("Incorrect Password".to_string());
                                        }
                                    }
                                }
                            }
                            self.wallet_password.clear();
                        }

                        // Display error message if the password was incorrect
                        if let Some(error_message) = &self.error_message {
                            ui.add_space(5.0);
                            ui.colored_label(Color32::RED, error_message);
                        }
                    });
                });

            if close_dialog {
                self.show_unlock_dialog = false;
                self.wallet_password.clear();
                self.error_message = None;
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
                        if let Some(wallet_arc) = &self.selected_single_key_wallet {
                            if let Ok(wallet) = wallet_arc.read() {
                                if let Some(alias) = &wallet.alias {
                                    ui.label(format!(
                                        "Wallet \"{}\" is locked. Please enter the password to unlock it:",
                                        alias
                                    ));
                                } else {
                                    ui.label("This wallet is locked. Please enter the password to unlock it:");
                                }
                            }
                        }

                        ui.add_space(10.0);

                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let mut attempt_unlock = false;

                        ui.horizontal(|ui| {
                            let password_input = ui.add(
                                egui::TextEdit::singleline(&mut self.wallet_password)
                                    .password(!self.show_password)
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

                        ui.checkbox(&mut self.show_password, "Show Password");

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
                                let unlock_result = wallet.open(&self.wallet_password);

                                match unlock_result {
                                    Ok(_) => {
                                        self.error_message = None;
                                        close_dialog = true;
                                    }
                                    Err(_) => {
                                        self.error_message =
                                            Some("Incorrect Password".to_string());
                                    }
                                }
                            }
                            self.wallet_password.clear();
                        }

                        // Display error message if the password was incorrect
                        if let Some(error_message) = &self.error_message {
                            ui.add_space(5.0);
                            ui.colored_label(Color32::RED, error_message);
                        }
                    });
                });

            if close_dialog {
                self.show_sk_unlock_dialog = false;
                self.wallet_password.clear();
                self.error_message = None;
            }
        }

        if let AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(_))) =
            action
        {
            self.refreshing = true;
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.refreshing = false;
        }
        self.message = Some((message.to_string(), message_type, Utc::now()))
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::ui::BackendTaskSuccessResult::RefreshedWallet => {
                self.refreshing = false;
                self.message = Some((
                    "Successfully refreshed wallet".to_string(),
                    MessageType::Success,
                    Utc::now(),
                ));
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
                    self.receive_dialog.address = Some(address.clone());
                    self.receive_dialog.qr_texture = None;
                    self.receive_dialog.qr_address = None;
                    self.receive_dialog.status = None;
                }
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressWithdrawal { .. } => {
                self.withdraw_platform_dialog.is_processing = false;
                self.withdraw_platform_dialog.status = Some("Withdrawal successful!".to_string());
                self.display_message("Platform withdrawal successful", MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressFunded { .. } => {
                self.fund_platform_dialog.is_processing = false;
                self.fund_platform_dialog.status = Some("Funding successful!".to_string());
                self.display_message("Platform address funded successfully", MessageType::Success);
            }
            crate::ui::BackendTaskSuccessResult::PlatformCreditsTransferred { .. } => {
                self.display_message(
                    "Platform credits transferred successfully",
                    MessageType::Success,
                );
            }
            crate::ui::BackendTaskSuccessResult::PlatformAddressBalances {
                seed_hash,
                balances,
            } => {
                // Update wallet's platform_address_info if this is for the selected wallet
                if let Some(selected) = &self.selected_wallet
                    && let Ok(mut wallet) = selected.write()
                    && wallet.seed_hash() == seed_hash
                {
                    // Update balances in the wallet
                    for (addr_str, (balance, nonce)) in balances {
                        // Find the address that matches the string
                        if let Some((addr, _)) = wallet
                            .platform_address_info
                            .iter()
                            .find(|(a, _)| a.to_string() == addr_str)
                        {
                            let addr = addr.clone();
                            wallet.set_platform_address_info(addr, balance, nonce);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn refresh_on_arrival(&mut self) {
        // Check if there's a pending wallet selection (e.g., from wallet creation/import)
        if let Ok(mut pending) = self.app_context.pending_wallet_selection.lock() {
            if let Some(seed_hash) = pending.take() {
                if let Ok(wallets) = self.app_context.wallets.read() {
                    if let Some(wallet) = wallets.get(&seed_hash) {
                        self.selected_wallet = Some(wallet.clone());
                        self.selected_single_key_wallet = None; // Clear SK selection
                        self.selected_account = None;
                        return;
                    }
                }
            }
        }

        // If no wallet of either type is selected but wallets exist, select the first HD wallet
        if self.selected_wallet.is_none() && self.selected_single_key_wallet.is_none() {
            if let Ok(wallets) = self.app_context.wallets.read() {
                if let Some(wallet) = wallets.values().next().cloned() {
                    self.selected_wallet = Some(wallet);
                    return;
                }
            }
            // If no HD wallets, try single key wallets
            if let Ok(wallets) = self.app_context.single_key_wallets.read() {
                self.selected_single_key_wallet = wallets.values().next().cloned();
            }
        }
    }

    fn refresh(&mut self) {}
}

impl ScreenWithWalletUnlock for WalletsBalancesScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }

    fn app_context(&self) -> Arc<AppContext> {
        self.app_context.clone()
    }
}
