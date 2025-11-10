use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::{CoreTask, WalletPaymentRequest};
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathHelpers, Wallet, WalletSeedHash};
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::copy_text_to_clipboard;
use crate::ui::identities::funding_common::generate_qr_code_image;
use crate::ui::theme::DashColors;
use crate::ui::wallets::account_summary::{AccountSummary, collect_account_summaries};
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use eframe::egui::{self, ComboBox, Context, Ui};
use eframe::epaint::TextureHandle;
use egui::load::SizedTexture;
use egui::{Color32, Frame, Margin, RichText, TextureOptions};
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
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
    selected_filters: HashSet<String>,
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
    status: Option<String>,
}

impl WalletsBalancesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = app_context.wallets.read().unwrap().values().next().cloned();
        let mut selected_filters = HashSet::new();
        selected_filters.insert("Funds".to_string()); // "Funds" selected by default
        Self {
            selected_wallet,
            app_context: app_context.clone(),
            message: None,
            sort_column: SortColumn::Index,
            sort_order: SortOrder::Ascending,
            selected_filters,
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
            return;
        }

        self.selected_wallet = wallets.values().next().cloned();
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

    fn render_filter_selector(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let filter_options = [
            ("Funds", "Show receiving and change addresses"),
            (
                "Identity Creation",
                "Show addresses used for identity creation",
            ),
            ("System", "Show system-related addresses"),
            (
                "Unused Asset Locks",
                "Show available asset locks for identity creation",
            ),
        ];

        // Single row layout
        ui.horizontal(|ui| {
            for (filter_option, description) in filter_options.iter() {
                let is_selected = self.selected_filters.contains(*filter_option);

                // Create a button with distinct styling
                let button = if is_selected {
                    egui::Button::new(
                        RichText::new(*filter_option)
                            .color(Color32::WHITE)
                            .size(12.0),
                    )
                    .fill(egui::Color32::from_rgb(0, 128, 255))
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(3.0)
                    .min_size(egui::vec2(0.0, 22.0))
                } else {
                    egui::Button::new(
                        RichText::new(*filter_option)
                            .color(DashColors::text_primary(dark_mode))
                            .size(12.0),
                    )
                    .fill(DashColors::glass_white(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border(dark_mode)))
                    .corner_radius(3.0)
                    .min_size(egui::vec2(0.0, 22.0))
                };

                if ui
                    .add(button)
                    .on_hover_text(format!("{} (Shift+click for multiple)", description))
                    .clicked()
                {
                    let shift_held = ui.input(|i| i.modifiers.shift_only());

                    if shift_held {
                        // If Shift is held, toggle the filter
                        if is_selected {
                            self.selected_filters.remove(*filter_option);
                        } else {
                            self.selected_filters.insert((*filter_option).to_string());
                        }
                    } else {
                        // Without Shift, replace the selection
                        self.selected_filters.clear();
                        self.selected_filters.insert((*filter_option).to_string());
                    }
                }
            }
        });
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = &self.app_context.wallets.read().unwrap();
            let wallet_aliases: Vec<String> = wallets
                .values()
                .map(|wallet| {
                    wallet
                        .read()
                        .unwrap()
                        .alias
                        .clone()
                        .unwrap_or_else(|| "Unnamed Wallet".to_string())
                })
                .collect();

            let selected_wallet_alias = self
                .selected_wallet
                .as_ref()
                .and_then(|wallet| wallet.read().ok()?.alias.clone())
                .unwrap_or_else(|| "Select a wallet".to_string());

            // Compact horizontal layout
            ui.horizontal(|ui| {
                // Display the ComboBox for wallet selection
                ComboBox::from_label("")
                    .selected_text(selected_wallet_alias.clone())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for (idx, wallet) in wallets.values().enumerate() {
                            let wallet_alias = wallet_aliases[idx].clone();

                            let is_selected = self
                                .selected_wallet
                                .as_ref()
                                .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                            if ui
                                .selectable_label(is_selected, wallet_alias.clone())
                                .clicked()
                            {
                                // Update the selected wallet
                                self.selected_wallet = Some(wallet.clone());
                            }
                        }
                    });

                if let Some(selected_wallet) = &self.selected_wallet {
                    let wallet = selected_wallet.read().unwrap();

                    if ui.button("Rename").clicked() {
                        self.show_rename_dialog = true;
                        self.rename_input = wallet.alias.clone().unwrap_or_default();
                    }
                }

                // Balance and rename button on same row
                if let Some(selected_wallet) = &self.selected_wallet {
                    ui.separator();

                    let wallet = selected_wallet.read().unwrap();
                    let total_balance = wallet.confirmed_balance_duffs();
                    ui.label(
                        RichText::new(format!("Balance: {}", Self::format_dash(total_balance)))
                            .strong()
                            .color(DashColors::success_color(dark_mode)),
                    );
                }
            });
        } else {
            ui.label("No wallets available.");
        }
    }

    fn render_address_table(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        let mut included_address_types = HashSet::new();

        for filter in &self.selected_filters {
            match filter.as_str() {
                "Funds" => {
                    included_address_types.insert("Funds".to_string());
                    included_address_types.insert("Change".to_string());
                }
                other => {
                    included_address_types.insert(other.to_string());
                }
            }
        }

        // Move the data preparation into its own scope
        let mut address_data = {
            let wallet = self.selected_wallet.as_ref().unwrap().read().unwrap();

            // Prepare data for the table
            wallet
                .known_addresses
                .iter()
                .filter_map(|(address, derivation_path)| {
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

                    if included_address_types.contains(address_type.as_str()) {
                        Some(AddressData {
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
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<AddressData>>()
        }; // The borrow of `wallet` ends here

        // Now you can use `self` mutably without conflict
        // Sort the data
        self.sort_address_data(&mut address_data);

        // Space allocation for UI elements is handled by the layout system

        // Render the table
        egui::ScrollArea::both()
            .id_salt("address_table")
            .show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(false)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::auto()) // Address
                    .column(Column::initial(100.0)) // Balance
                    .column(Column::initial(60.0)) // UTXOs
                    .column(Column::initial(150.0)) // Total Received
                    .column(Column::initial(100.0)) // Type
                    .column(Column::initial(60.0)) // Index
                    .column(Column::remainder()) // Derivation Path
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
                                    SortOrder::Ascending => "Total Received (DASH) ^",
                                    SortOrder::Descending => "Total Received (DASH) v",
                                }
                            } else {
                                "Total Received (DASH)"
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
                                    SortOrder::Ascending => "Balance (DASH) ^",
                                    SortOrder::Descending => "Balance (DASH) v",
                                }
                            } else {
                                "Balance (DASH)"
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
                            });
                        }
                    });
            });
        action
    }

    fn render_bottom_options(&mut self, ui: &mut Ui) {
        let wallet_is_open = self
            .selected_wallet
            .as_ref()
            .is_some_and(|wallet_guard| wallet_guard.read().unwrap().is_open());

        if self.selected_filters.contains("Funds") {
            ui.add_space(10.0);

            if wallet_is_open {
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("➕ Add Receiving Address").size(14.0))
                        .clicked()
                    {
                        self.add_receiving_address();
                    }
                });
            } else {
                // Show wallet unlock UI for locked wallets when Funds filter is active
                self.render_wallet_unlock_if_needed(ui);
            }
        }

        if self.selected_wallet.is_some() {
            ui.add_space(16.0);
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            let remove_button = egui::Button::new(
                RichText::new("🗑 Remove Wallet")
                    .color(Color32::WHITE)
                    .size(14.0),
            )
            .min_size(egui::vec2(0.0, 28.0))
            .fill(DashColors::error_color(!dark_mode))
            .stroke(egui::Stroke::NONE)
            .corner_radius(4.0);

            if ui.add(remove_button).clicked()
                && let Some(selected_wallet) = &self.selected_wallet
            {
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

                if self.selected_wallet.is_none() {
                    self.selected_filters.clear();
                    self.selected_filters.insert("Funds".to_string());
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

    fn format_dash(amount: u64) -> String {
        let dash = amount as f64 / 100_000_000.0;
        if dash >= 1.0 {
            format!("{:.4} DASH", dash)
        } else {
            let trimmed = format!("{:.8}", dash)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string();
            format!("{} DASH", trimmed)
        }
    }

    fn parse_amount_to_duffs(input: &str) -> Result<u64, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Enter an amount".to_string());
        }
        let value = trimmed
            .parse::<f64>()
            .map_err(|_| "Invalid amount".to_string())?;
        if !value.is_finite() || value <= 0.0 {
            return Err("Amount must be greater than zero".to_string());
        }
        let duffs = (value * 100_000_000.0).round();
        if duffs <= 0.0 || duffs > u64::MAX as f64 {
            return Err("Amount is out of range".to_string());
        }
        Ok(duffs as u64)
    }

    fn platform_balance_duffs(wallet: &Wallet) -> u64 {
        wallet
            .identities
            .values()
            .map(|identity| identity.balance())
            .sum()
    }

    fn render_wallet_overview(&self, ui: &mut Ui, wallet: &Wallet) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let total = wallet.total_balance_duffs();
        let confirmed = wallet.confirmed_balance_duffs();
        let unconfirmed = wallet.unconfirmed_balance_duffs();
        let platform = Self::platform_balance_duffs(wallet);

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Network: {}", self.app_context.network))
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    if self.refreshing {
                        ui.spinner();
                    }
                });

                ui.add_space(6.0);
                ui.heading(RichText::new(Self::format_dash(total)).size(28.0));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Confirmed: {}", Self::format_dash(confirmed)))
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    if unconfirmed > 0 {
                        ui.label(
                            RichText::new(format!("Pending: {}", Self::format_dash(unconfirmed)))
                                .color(Color32::from_rgb(255, 200, 0)),
                        );
                    }
                    if platform > 0 {
                        ui.label(
                            RichText::new(format!("Platform: {}", Self::format_dash(platform)))
                                .color(Color32::from_rgb(0, 170, 255)),
                        );
                    }
                });
            });
    }

    fn render_action_buttons(&mut self, ui: &mut Ui, ctx: &Context) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .button(RichText::new("Send").color(Color32::WHITE).strong())
                .clicked()
            {
                if self.selected_wallet.is_some() {
                    self.send_dialog.is_open = true;
                } else {
                    self.display_message("Select a wallet first", MessageType::Error);
                }
            }

            if ui
                .button(RichText::new("Receive").color(Color32::WHITE))
                .clicked()
            {
                self.open_receive_dialog(ctx);
            }
        });
    }

    fn render_accounts_section(&mut self, ui: &mut Ui, summaries: Vec<AccountSummary>) {
        ui.add_space(14.0);
        ui.heading("Accounts");
        ui.add_space(6.0);

        if summaries.is_empty() {
            ui.label("No account activity yet.");
            return;
        }

        for summary in summaries {
            Frame::group(ui.style())
                .fill(ui.visuals().extreme_bg_color)
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
                                    .strong(),
                            );
                        });
                    });

                    ui.label(format!(
                        "Addresses: {} ({} receive / {} change)",
                        summary.total_addresses,
                        summary.external_addresses,
                        summary.internal_addresses
                    ));

                    if let Some(next) = &summary.next_receive_hint {
                        ui.horizontal(|ui| {
                            ui.monospace(format!("Next: {}", next));
                            if ui.small_button("Copy").clicked() {
                                if let Err(err) = copy_text_to_clipboard(next) {
                                    self.display_message(&err, MessageType::Error);
                                } else {
                                    self.display_message("Address copied", MessageType::Success);
                                }
                            }
                        });
                    }
                });

            ui.add_space(6.0);
        }
    }

    fn render_transactions_section(&self, ui: &mut Ui) {
        ui.add_space(10.0);
        ui.heading("Transactions");
        ui.label("Transaction history will appear here once SPV exposes it.");
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

                    if ui.button("Cancel").clicked() {
                        self.send_dialog = SendDialogState::default();
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

        let mut open = self.receive_dialog.is_open;
        egui::Window::new("Receive Dash")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(texture) = &self.receive_dialog.qr_texture {
                    ui.image(SizedTexture::new(texture.id(), egui::vec2(220.0, 220.0)));
                }

                if let Some(address) = &self.receive_dialog.address {
                    ui.add_space(6.0);
                    ui.monospace(address);
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

                if ui.button("Close").clicked() {
                    self.receive_dialog = ReceiveDialogState::default();
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

    fn open_receive_dialog(&mut self, ctx: &Context) {
        let Some(wallet) = self.selected_wallet.clone() else {
            self.receive_dialog.status = Some("Select a wallet first".to_string());
            self.receive_dialog.is_open = true;
            return;
        };

        match self.prepare_receive_dialog(ctx, &wallet) {
            Ok(()) => self.receive_dialog.status = None,
            Err(err) => self.receive_dialog.status = Some(err),
        }

        self.receive_dialog.is_open = true;
    }

    fn prepare_receive_dialog(
        &mut self,
        ctx: &Context,
        wallet: &Arc<RwLock<Wallet>>,
    ) -> Result<(), String> {
        let address = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            wallet_guard.receive_address(
                self.app_context.network,
                false,
                Some(&self.app_context),
            )?
        };

        let address_str = address.to_string();
        let qr_image = generate_qr_code_image(&address_str).map_err(|e| e.to_string())?;
        let texture = ctx.load_texture(
            format!("wallet_receive_{}", address_str),
            qr_image,
            TextureOptions::LINEAR,
        );

        self.receive_dialog.address = Some(address_str);
        self.receive_dialog.qr_texture = Some(texture);
        Ok(())
    }
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();
        let right_buttons = if let Some(wallet) = self.selected_wallet.as_ref() {
            match self.refreshing {
                true => vec![
                    ("Refreshing...", DesiredAppAction::None),
                    (
                        "Import Wallet",
                        DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportWallet)),
                    ),
                    (
                        "Create Wallet",
                        DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
                    ),
                ],
                false => vec![
                    (
                        "Refresh",
                        DesiredAppAction::BackendTask(Box::new(BackendTask::CoreTask(
                            CoreTask::RefreshWalletInfo(wallet.clone()),
                        ))),
                    ),
                    (
                        "Import Wallet",
                        DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportWallet)),
                    ),
                    (
                        "Create Wallet",
                        DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
                    ),
                ],
            }
        } else {
            vec![
                (
                    "Import Wallet",
                    DesiredAppAction::AddScreenType(Box::new(ScreenType::ImportWallet)),
                ),
                (
                    "Create Wallet",
                    DesiredAppAction::AddScreenType(Box::new(ScreenType::AddNewWallet)),
                ),
            ]
        };
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

                    // Wallet Information Panel (fit content)
                    ui.vertical(|ui| {
                        ui.heading(
                            RichText::new("Wallets").color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            Frame::new()
                                .fill(DashColors::surface(dark_mode))
                                .corner_radius(5.0)
                                .inner_margin(Margin::symmetric(15, 10))
                                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                                .show(ui, |ui| {
                                    self.render_wallet_selection(ui);
                                });
                        });
                    });

                    ui.add_space(10.0);

                    if let Some(wallet_arc) = &self.selected_wallet {
                        let summaries = {
                            let wallet = wallet_arc.read().unwrap();
                            self.render_wallet_overview(ui, &wallet);
                            collect_account_summaries(&wallet, self.app_context.network)
                        };

                        self.render_action_buttons(ui, ctx);
                        self.render_accounts_section(ui, summaries);
                        self.render_transactions_section(ui);

                        ui.separator();
                        ui.add_space(10.0);

                        ui.vertical(|ui| {
                            ui.heading(
                                RichText::new("Addresses")
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            ui.add_space(10.0);
                            self.render_filter_selector(ui);
                            ui.add_space(5.0);
                            ui.label(
                                RichText::new("Tip: Hold Shift to select multiple filters")
                                    .color(Color32::GRAY)
                                    .size(10.0)
                                    .italics(),
                            );
                        });

                        ui.add_space(10.0);

                        if !(self.selected_filters.contains("Unused Asset Locks")
                            && self.selected_filters.len() == 1)
                        {
                            inner_action |= self.render_address_table(ui);
                        }

                        if self.selected_filters.contains("Unused Asset Locks") {
                            ui.add_space(15.0);
                            inner_action |= self.render_wallet_asset_locks(ui);
                        }

                        ui.add_space(10.0);
                        self.render_bottom_options(ui);
                    }
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
}
