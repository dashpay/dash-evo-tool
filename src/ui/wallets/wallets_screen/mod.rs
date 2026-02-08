mod address_table;
mod asset_locks;
mod dialogs;
mod single_key_view;

use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::wallet::{Wallet, WalletSeedHash, WalletTransaction};
use crate::spv::CoreBackendMode;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{WalletUnlockPopup, WalletUnlockResult};
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::theme::DashColors;
use crate::ui::wallets::account_summary::{
    AccountCategory, AccountSummary, collect_account_summaries,
};
use crate::ui::wallets::send_utils::format_dash;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use eframe::egui::{self, ComboBox, Context, Ui};
use egui::{Color32, Frame, Margin, RichText};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, RwLock};
use zeroize::Zeroize;

use crate::model::wallet::single_key::SingleKeyWallet;
use address_table::{SortColumn, SortOrder};
use dialogs::{
    FundPlatformAddressDialogState, PrivateKeyDialogState, ReceiveDialogState, SendDialogState,
};

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
    /// Whether to hide addresses with zero balance in the address table
    hide_zero_balances: bool,
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
            let hd_wallet = app_context
                .wallets
                .read_or_recover()
                .values()
                .next()
                .cloned();
            let sk_wallet = if hd_wallet.is_none() {
                app_context
                    .single_key_wallets
                    .read_or_recover()
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
            hide_zero_balances: true,
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

    fn select_hd_wallet(&mut self, wallet: Arc<RwLock<Wallet>>) {
        self.selected_wallet = Some(wallet.clone());
        self.selected_single_key_wallet = None;
        self.selected_account = None;

        if let Ok(hash) = wallet.read().map(|g| g.seed_hash()) {
            self.persist_selected_wallet_hash(Some(hash));
        }
        self.persist_selected_single_key_hash(None);
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
                let mut wallet = wallet.write_or_recover();
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
                let guard = wallet.read_or_recover();
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
                let guard = wallet.read_or_recover();
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
                        format!(" Balance: {}", format_dash(current_balance)),
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
                                self.persist_selected_single_key_hash(None);
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

    fn render_bottom_options(&mut self, ui: &mut Ui) {
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|wallet_guard| wallet_guard.read_or_recover().is_open());

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
                let wallet = selected_wallet.read_or_recover();
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
                self.persist_selected_wallet_hash(new_hash);

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

    fn set_message(&mut self, message: String, message_type: MessageType) {
        self.message = Some((message, message_type, Utc::now()));
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
        let amount = format_dash(tx.amount_abs());
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
                format_dash(total)
            )));
        });
        ui.label(
            RichText::new(format!("Platform balance: {}", format_dash(platform)))
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
                    format!("{} - {}", s.label, format_dash(s.confirmed_balance))
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
                            format_dash(summary.confirmed_balance)
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

        let wallet_guard = wallet_arc.read_or_recover();
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
            let wallet = wallet_arc.read_or_recover();
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
                            let wallet = wallet_arc.read_or_recover();
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
        use crate::backend_task::wallet::PlatformSyncMode;

        let seed_hash = wallet_arc
            .read()
            .ok()
            .map(|w| w.seed_hash())
            .unwrap_or_default();

        match mode {
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

    fn render_rename_dialog(&mut self, ctx: &Context) {
        if !self.show_rename_dialog {
            return;
        }
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
                                let mut wallet = selected_wallet.write_or_recover();
                                wallet.alias = Some(self.rename_input.clone());

                                // Update the alias in the database
                                let seed_hash = wallet.seed_hash();
                                self.app_context
                                    .db
                                    .set_wallet_alias(&seed_hash, Some(self.rename_input.clone()))
                                    .ok();
                            }
                            // Handle single key wallet rename
                            else if let Some(selected_sk_wallet) =
                                &self.selected_single_key_wallet
                            {
                                let mut wallet = selected_sk_wallet.write_or_recover();
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

    fn handle_hd_unlock_result(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        let Some(wallet_arc) = self.selected_wallet.clone() else {
            return action;
        };

        let result = self
            .wallet_unlock_popup
            .show(ctx, &wallet_arc, &self.app_context);
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

        action
    }

    fn render_sk_unlock_dialog(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        if !self.show_sk_unlock_dialog {
            return action;
        }

        let mut close_dialog = false;
        egui::Window::new("Unlock Wallet")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    if let Some(wallet_arc) = &self.selected_single_key_wallet
                        && let Ok(wallet) = wallet_arc.read()
                    {
                        if let Some(alias) = &wallet.alias {
                            ui.label(format!(
                                "Wallet \"{}\" is locked. Please enter the password to unlock it:",
                                alias
                            ));
                        } else {
                            ui.label(
                                "This wallet is locked. Please enter the password to unlock it:",
                            );
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
                            let mut wallet = wallet_arc.write_or_recover();
                            let unlock_result = wallet.open(&self.sk_wallet_password);

                            match unlock_result {
                                Ok(_) => {
                                    self.sk_error_message = None;
                                    close_dialog = true;
                                }
                                Err(_) => {
                                    self.sk_error_message = Some("Incorrect Password".to_string());
                                }
                            }
                        }
                        self.sk_wallet_password.zeroize();
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
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!("Error: {}", error_message))
                                                .color(error_color),
                                        )
                                        .wrap(),
                                    );
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
            self.sk_wallet_password.zeroize();
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

        action
    }

    fn handle_custom_actions(&mut self, action: &mut AppAction) {
        let AppAction::Custom(ref cmd) = *action else {
            return;
        };

        if cmd == "RefreshHDWallet" {
            if let Some(wallet_arc) = &self.selected_wallet {
                let is_locked = wallet_arc.read().map(|w| !w.is_open()).unwrap_or(true);
                if is_locked {
                    // Wallet is locked - open unlock popup and store the refresh mode
                    self.pending_refresh_after_unlock = true;
                    self.pending_refresh_mode = self.refresh_mode;
                    self.wallet_unlock_popup.open();
                    *action = AppAction::None;
                } else {
                    // Wallet is unlocked - proceed with refresh using selected mode
                    self.refreshing = true;
                    *action = self.create_refresh_action(wallet_arc);
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
                *action = AppAction::None;
            } else {
                // SK wallet is unlocked - proceed with refresh
                self.refreshing = true;
                *action = AppAction::BackendTask(BackendTask::CoreTask(
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
                *action = AppAction::None;
            } else {
                // Wallet is unlocked - proceed with search
                self.display_message("Searching for unused asset locks...", MessageType::Info);
                *action = AppAction::BackendTask(BackendTask::CoreTask(
                    CoreTask::RecoverAssetLocks(wallet_arc),
                ));
            }
        }
    }
}

impl Drop for WalletsBalancesScreen {
    fn drop(&mut self) {
        self.sk_wallet_password.zeroize();
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
                    let has_hd_wallets = !self.app_context.wallets.read_or_recover().is_empty();
                    let has_single_key_wallets = !self
                        .app_context
                        .single_key_wallets
                        .read_or_recover()
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

        self.render_rename_dialog(ctx);
        action |= self.handle_hd_unlock_result(ctx);
        action |= self.render_sk_unlock_dialog(ctx);

        if let AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(_, _))) =
            action
        {
            self.refreshing = true;
        }

        self.handle_custom_actions(&mut action);

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
        self.set_message(message.to_string(), message_type);
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::ui::BackendTaskSuccessResult::Wallet(wallet_result) => {
                use crate::backend_task::wallet::WalletResult;
                match wallet_result {
                    WalletResult::Refreshed { warning } => {
                        self.refreshing = false;
                        if let Some(warn_msg) = warning {
                            self.set_message(
                                format!("Wallet refreshed with warning: {}", warn_msg),
                                MessageType::Info,
                            );
                        } else {
                            self.set_message(
                                "Successfully refreshed wallet".to_string(),
                                MessageType::Success,
                            );
                        }
                    }
                    WalletResult::RecoveredAssetLocks {
                        recovered_count,
                        total_amount,
                    } => {
                        let msg = if recovered_count == 0 {
                            "No additional unused asset locks found".to_string()
                        } else {
                            format!(
                                "Found {} unused asset lock(s) worth {} Dash",
                                recovered_count,
                                format_dash(total_amount)
                            )
                        };
                        self.display_message(&msg, MessageType::Success);
                    }
                    WalletResult::Payment {
                        txid,
                        recipients,
                        total_amount,
                    } => {
                        let msg = if recipients.len() == 1 {
                            let (address, amount) = &recipients[0];
                            format!(
                                "Sent {} to {}\nTxID: {}",
                                format_dash(*amount),
                                address,
                                txid
                            )
                        } else {
                            format!(
                                "Sent {} total to {} recipients\nTxID: {}",
                                format_dash(total_amount),
                                recipients.len(),
                                txid
                            )
                        };
                        self.display_message(&msg, MessageType::Success);
                    }
                    WalletResult::GeneratedReceiveAddress { seed_hash, address } => {
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
                    WalletResult::PlatformAddressWithdrawal { .. } => {
                        self.display_message("Platform withdrawal successful. Note: It may take a few minutes for funds to appear on the Core chain.", MessageType::Success);
                    }
                    WalletResult::PlatformAddressFunded { .. } => {
                        self.fund_platform_dialog.is_processing = false;
                        self.fund_platform_dialog.status = Some("Funding successful!".to_string());
                        self.fund_platform_dialog.status_is_error = false;
                        self.display_message(
                            "Platform address funded successfully",
                            MessageType::Success,
                        );
                    }
                    WalletResult::PlatformCreditsTransferred { seed_hash } => {
                        self.display_message(
                            "Platform credits transferred successfully",
                            MessageType::Success,
                        );
                        // Schedule a refresh of platform address balances to update the UI
                        self.pending_platform_balance_refresh = Some(seed_hash);
                    }
                    WalletResult::PlatformAddressBalances {
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
                        self.set_message(
                            "Successfully synced Platform balances".to_string(),
                            MessageType::Success,
                        );
                    }
                }
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
