use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, WalletPaymentRequest};
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
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use eframe::egui::{self, ComboBox, Context, Ui};
use eframe::epaint::TextureHandle;
use egui::load::SizedTexture;
use egui::{Color32, Frame, Margin, RichText, TextureOptions};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, RwLock};

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
    pub(crate) app_context: Arc<AppContext>,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    refreshing: bool,
    show_rename_dialog: bool,
    rename_input: String,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
    remove_wallet_dialog: Option<ConfirmationDialog>,
    pending_wallet_removal: Option<WalletSeedHash>,
    pending_wallet_removal_alias: Option<String>,
    send_dialog: SendDialogState,
    receive_dialog: ReceiveDialogState,
    selected_account: Option<(AccountCategory, Option<u32>)>,
}

// Define a struct to hold the address data
struct AddressData {
    address: Address,
    balance: u64,
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

impl WalletsBalancesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = app_context.wallets.read().unwrap().values().next().cloned();
        Self {
            selected_wallet,
            app_context: app_context.clone(),
            message: None,
            sort_column: SortColumn::Index,
            sort_order: SortOrder::Ascending,
            refreshing: false,
            show_rename_dialog: false,
            rename_input: String::new(),
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
            remove_wallet_dialog: None,
            pending_wallet_removal: None,
            pending_wallet_removal_alias: None,
            send_dialog: SendDialogState::default(),
            receive_dialog: ReceiveDialogState::default(),
            selected_account: None,
        }
    }

    pub(crate) fn update_selected_wallet_for_network(&mut self) {
        let selected_seed = self
            .selected_wallet
            .as_ref()
            .and_then(|wallet| wallet.read().ok().map(|wallet| wallet.seed_hash()));

        let wallets = match self.app_context.wallets.read() {
            Ok(guard) => guard,
            Err(_) => {
                self.selected_wallet = None;
                return;
            }
        };

        if let Some(seed_hash) = selected_seed
            && let Some(wallet) = wallets.get(&seed_hash)
        {
            self.selected_wallet = Some(wallet.clone());
            self.selected_account = None;
            return;
        }

        self.selected_wallet = wallets.values().next().cloned();
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

    fn render_wallet_selection(&mut self, ui: &mut Ui) {
        let wallets_guard = self.app_context.wallets.read().unwrap();
        if wallets_guard.is_empty() {
            self.render_no_wallets_view(ui);
            return;
        }

        let items: Vec<(String, Arc<RwLock<Wallet>>)> = wallets_guard
            .values()
            .map(|wallet| {
                let guard = wallet.read().unwrap();
                let balance_dash = guard.total_balance_duffs() as f64 * 1e-8;
                let label = format!(
                    "{} ({:.2})",
                    guard
                        .alias
                        .clone()
                        .unwrap_or_else(|| "Unnamed Wallet".to_string()),
                    balance_dash
                );
                (label, wallet.clone())
            })
            .collect();

        drop(wallets_guard);

        let selected_label = self
            .selected_wallet
            .as_ref()
            .and_then(|wallet| {
                wallet.read().ok().map(|guard| {
                    guard
                        .alias
                        .clone()
                        .unwrap_or_else(|| "Unnamed Wallet".to_string())
                        .to_string()
                })
            })
            .unwrap_or_else(|| "Select a wallet".to_string());

        ui.with_layout(
            egui::Layout::left_to_right(egui::Align::TOP).with_main_justify(true),
            |ui| {
                ui.horizontal(|ui| {
                    ComboBox::from_id_salt("wallet_selector")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for (label, wallet) in &items {
                                let is_selected = self
                                    .selected_wallet
                                    .as_ref()
                                    .is_some_and(|selected| Arc::ptr_eq(selected, wallet));
                                if ui.selectable_label(is_selected, label).clicked() {
                                    self.selected_wallet = Some(wallet.clone());
                                    self.selected_account = None;
                                }
                            }
                        });

                    ui.colored_label(
                        DashColors::text_primary(ui.ctx().style().visuals.dark_mode),
                        format!(
                            " Balance: {}",
                            match &self.selected_wallet {
                                Some(wallet) => {
                                    let guard = wallet.read().unwrap();
                                    Self::format_dash(guard.total_balance_duffs())
                                }
                                None => "N/A".to_string(),
                            }
                        ),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    self.render_remove_wallet_button(ui);
                    ui.add_space(8.0);
                    let mut should_lock_wallet = false;
                    if let Some(wallet_arc) = &self.selected_wallet
                        && let Ok(wallet) = wallet_arc.read()
                        && wallet.uses_password
                    {
                        if wallet.is_open() {
                            if ui.button("Lock").clicked() {
                                should_lock_wallet = true;
                            }
                        } else {
                            ui.add_enabled(false, egui::Button::new("Locked"));
                        }
                    }
                    if should_lock_wallet {
                        self.lock_selected_wallet();
                    }
                    ui.add_space(8.0);
                    if let Some(wallet_arc) = &self.selected_wallet
                        && ui.button("Rename").clicked()
                        && let Ok(wallet) = wallet_arc.read()
                    {
                        self.show_rename_dialog = true;
                        self.rename_input = wallet.alias.clone().unwrap_or_default();
                    }
                });
            },
        );
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

                    // Calculate total received by summing UTXO values
                    let total_received = utxo_info
                        .map(|outpoints| outpoints.values().map(|txout| txout.value).sum::<u64>())
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
                    AddressData {
                        address: address.clone(),
                        balance: wallet
                            .address_balances
                            .get(address)
                            .cloned()
                            .unwrap_or_default(),
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
                for data in &address_data {
                    body.row(25.0, |mut row| {
                        row.col(|ui| {
                            ui.label(data.address.to_string());
                        });
                        row.col(|ui| {
                            let dash_balance = data.balance as f64 * 1e-8;
                            ui.label(format!("{:.8}", dash_balance));
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

        if wallet_is_open {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("➕ Add Receiving Address").size(14.0))
                    .clicked()
                {
                    self.add_receiving_address();
                }
            });
        } else {
            ui.add_space(10.0);
            self.render_wallet_unlock_if_needed(ui);
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

                self.selected_wallet = next_wallet;

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
                            ui.label(RichText::new("Asset locks are special transactions that can be used to create identities").color(Color32::GRAY).size(12.0));
                            ui.add_space(15.0);
                            if ui.button("Search for asset locks").clicked() {
                                app_action = AppAction::BackendTask(BackendTask::CoreTask(
                                    CoreTask::RefreshWalletInfo(arc_wallet.clone()),
                                ))
                            };
                            ui.add_space(20.0);
                        });
                    } else {
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
                        })
                        .body(|mut body| {
                            for (tx, address, amount, islock, proof) in &wallet.unused_asset_locks {
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
                                });
                            }
                        });
                    });
                    }
                });
        } else {
            ui.label("No wallet selected.");
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

    fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
        let amount = Amount::parse(input, DASH_DECIMAL_PLACES)?.with_unit_name("DASH");
        amount.dash_to_duffs()
    }

    fn platform_balance_duffs(wallet: &Wallet) -> u64 {
        wallet
            .identities
            .values()
            .map(|identity| identity.balance() / CREDITS_PER_DUFF)
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
                if self.selected_wallet.is_some() {
                    self.send_dialog.is_open = true;
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
                            ui.label(
                                RichText::new(Self::format_dash(summary.confirmed_balance))
                                    .strong()
                                    .color(DashColors::text_primary(
                                        ui.ctx().style().visuals.dark_mode,
                                    )),
                            );
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
            to_address: self.send_dialog.address.trim().to_string(),
            amount_duffs: amount,
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
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();
        let mut right_buttons = vec![
            (
                "Import Wallet",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportWallet)),
            ),
            (
                "Create Wallet",
                DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
            ),
        ];

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
                    if self.app_context.wallets.read().unwrap().is_empty() {
                        self.render_no_wallets_view(ui);
                        return;
                    }

                    Frame::group(ui.style())
                        .fill(DashColors::surface(dark_mode))
                        .inner_margin(Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            self.render_wallet_selection(ui);
                        });

                    ui.add_space(10.0);
                    inner_action |= self.render_wallet_detail_panel(ui, ctx);
                });

            inner_action
        });

        action |= self.render_send_dialog(ctx);
        action |= self.render_receive_dialog(ctx);

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
                                if let Some(selected_wallet) = &self.selected_wallet {
                                    let mut wallet = selected_wallet.write().unwrap();

                                    // Limit the alias length to 64 characters
                                    if self.rename_input.len() > 64 {
                                        self.rename_input.truncate(64);
                                    }

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

        if let AppAction::BackendTask(BackendTask::CoreTask(CoreTask::RefreshWalletInfo(_))) =
            action
        {
            self.refreshing = true;
        }

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message.contains("Successfully refreshed wallet")
            || message.contains("Error refreshing wallet")
            || message.contains("Wallet refreshed from SPV")
        {
            self.refreshing = false;
        }
        self.message = Some((message.to_string(), message_type, Utc::now()))
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        if let crate::ui::BackendTaskSuccessResult::WalletPayment {
            txid,
            address,
            amount,
        } = backend_task_success_result
        {
            let msg = format!(
                "Sent {} to {}\nTxID: {}",
                Self::format_dash(amount),
                address,
                txid
            );
            self.display_message(&msg, MessageType::Success);
            return;
        }

        if let crate::ui::BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address } =
            backend_task_success_result
        {
            if let Some(selected) = &self.selected_wallet
                && let Ok(wallet) = selected.read()
            {
                if wallet.seed_hash() == seed_hash {
                    self.receive_dialog.address = Some(address.clone());
                    self.receive_dialog.qr_texture = None;
                    self.receive_dialog.qr_address = None;
                    self.receive_dialog.status = None;
                }
            }
        }
    }

    fn refresh_on_arrival(&mut self) {}

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
