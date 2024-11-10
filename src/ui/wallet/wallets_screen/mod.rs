use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::core::CoreTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::dashcore::bip32::{ChildNumber, DerivationPath};
use eframe::egui::{self, ComboBox, Context, Ui};
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
    error_message: Option<(String, MessageType)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    selected_filters: HashSet<String>,
}

pub trait DerivationPathHelpers {
    fn is_bip44(&self, network: Network) -> bool;
    fn is_bip44_external(&self, network: Network) -> bool;
    fn is_bip44_change(&self, network: Network) -> bool;
    fn is_asset_lock_funding(&self, network: Network) -> bool;
}
impl DerivationPathHelpers for DerivationPath {
    fn is_bip44(&self, network: Network) -> bool {
        // BIP44 external paths have the form m/44'/coin_type'/account'/0/...
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        let components = self.as_ref();
        components.len() == 5
            && components[0] == ChildNumber::Hardened { index: 44 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
    }

    fn is_bip44_external(&self, network: Network) -> bool {
        // BIP44 external paths have the form m/44'/coin_type'/account'/0/...
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        let components = self.as_ref();
        components.len() == 5
            && components[0] == ChildNumber::Hardened { index: 44 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
            && components[3] == ChildNumber::Normal { index: 0 }
    }

    fn is_bip44_change(&self, network: Network) -> bool {
        // BIP44 change paths have the form m/44'/coin_type'/account'/1/...
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        let components = self.as_ref();
        components.len() >= 5
            && components[0] == ChildNumber::Hardened { index: 44 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
            && components[3] == ChildNumber::Normal { index: 1 }
    }

    fn is_asset_lock_funding(&self, network: Network) -> bool {
        // BIP44 change paths have the form m/44'/coin_type'/account'/1/...
        let coin_type = match network {
            Network::Dash => 5,
            _ => 1,
        };
        // Asset lock funding paths have the form m/9'/coin_type'/5'/1'/x
        let components = self.as_ref();
        components.len() == 5
            && components[0] == ChildNumber::Hardened { index: 9 }
            && components[1] == ChildNumber::Hardened { index: coin_type }
            && components[2] == ChildNumber::Hardened { index: 5 }
            && components[3] == ChildNumber::Hardened { index: 1 }
    }
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

impl WalletsBalancesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = app_context.wallets.read().unwrap().first().cloned();
        let mut selected_filters = HashSet::new();
        selected_filters.insert("Funds".to_string()); // "Funds" selected by default
        Self {
            selected_wallet,
            app_context: app_context.clone(),
            error_message: None,
            sort_column: SortColumn::Index,
            sort_order: SortOrder::Ascending,
            selected_filters,
        }
    }

    fn add_receiving_address(&mut self) {
        if let Some(wallet) = &self.selected_wallet {
            let result = {
                let mut wallet = wallet.write().unwrap();
                wallet.receive_address(self.app_context.network, true, Some(&self.app_context))
            };

            // Now the immutable borrow of `wallet` is dropped, and we can use `self` mutably
            if let Err(e) = result {
                self.display_message(&e, MessageType::Error);
            }
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
        ui.horizontal(|ui| {
            let filter_options = ["Funds", "Identity Creation", "System", "Asset Locks"];

            for filter_option in &filter_options {
                let is_selected = self.selected_filters.contains(*filter_option);

                // Create RichText with a larger font size
                let text = egui::RichText::new(*filter_option).size(14.0);

                let button = egui::SelectableLabel::new(is_selected, text);

                // Set the desired button size
                let button_size = egui::Vec2::new(100.0, 30.0);

                if ui.add_sized(button_size, button).clicked() {
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
        ui.horizontal(|ui| {
            if self.app_context.has_wallet.load(Ordering::Relaxed) {
                let wallets = &self.app_context.wallets.read().unwrap();
                let wallet_aliases: Vec<String> = wallets
                    .iter()
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
                    .unwrap_or_else(|| "Select".to_string());

                // Display the ComboBox for wallet selection
                ComboBox::from_label("")
                    .selected_text(selected_wallet_alias.clone())
                    .show_ui(ui, |ui| {
                        for (idx, wallet) in wallets.iter().enumerate() {
                            let wallet_alias = wallet_aliases[idx].clone();

                            let is_selected = self
                                .selected_wallet
                                .as_ref()
                                .map_or(false, |selected| Arc::ptr_eq(selected, wallet));

                            if ui
                                .selectable_label(is_selected, wallet_alias.clone())
                                .clicked()
                            {
                                // Update the selected wallet
                                self.selected_wallet = Some(wallet.clone());
                            }
                        }
                    });

                ui.add_space(20.0);

                // Text input for renaming the wallet
                if let Some(selected_wallet) = &self.selected_wallet {
                    {
                        let mut wallet = selected_wallet.write().unwrap();
                        let mut alias = wallet.alias.clone().unwrap_or_default();

                        // Limit the alias length to 64 characters
                        if alias.len() > 64 {
                            alias.truncate(64);
                        }

                        // Render a text field with a placeholder for the wallet alias
                        let text_edit = egui::TextEdit::singleline(&mut alias)
                            .hint_text("Enter wallet alias (max 64 chars)");

                        // Render a text field to modify the wallet alias
                        if ui.add(text_edit).changed() {
                            // Update the wallet alias when the text field is modified
                            wallet.alias = Some(alias.clone());

                            // Update the alias in the database
                            let seed_hash = wallet.seed_hash();
                            self.app_context
                                .db
                                .set_wallet_alias(&seed_hash, Some(alias.clone()))
                                .ok();
                        }
                    }

                    ui.add_space(20.0);

                    // Display total wallet balance next to the selector
                    if let Some(selected_wallet) = &self.selected_wallet {
                        let wallet = selected_wallet.read().unwrap();
                        let total_balance = wallet.max_balance();
                        let dash_balance = total_balance as f64 * 1e-8; // Convert to DASH
                        ui.label(format!("Total Balance: {:.8} DASH", dash_balance));
                    }
                }
            } else {
                ui.label("No wallets available.");
            }
        });
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

        // Render the table
        egui::ScrollArea::vertical()
            .id_salt("address_table")
            .show(ui, |ui| {
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill)
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
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
            });
        action
    }

    fn render_bottom_options(&mut self, ui: &mut Ui) {
        if self.selected_filters.contains("Funds") {
            // Add the button to add a receiving address
            if ui.button("Add Receiving Address").clicked() {
                self.add_receiving_address();
            }
        }
    }

    fn render_wallet_asset_locks(&mut self, ui: &mut Ui) {
        if let Some(wallet) = &self.selected_wallet {
            let wallet = wallet.read().unwrap();

            if wallet.unused_asset_locks.is_empty() {
                ui.label("No asset locks available.");
                return;
            }

            ui.label("Asset Locks:");
            egui::ScrollArea::vertical()
                .id_salt("asset_locks_table")
                .show(ui, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
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
        } else {
            ui.label("No wallet selected.");
        }
    }
}

impl ScreenLike for WalletsBalancesScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let right_buttons = if let Some(wallet) = self.selected_wallet.as_ref() {
            vec![
                (
                    "Refresh",
                    DesiredAppAction::BackendTask(BackendTask::CoreTask(
                        CoreTask::RefreshWalletInfo(wallet.clone()),
                    )),
                ),
                (
                    "Add Wallet",
                    DesiredAppAction::AddScreenType(ScreenType::AddNewWallet),
                ),
            ]
        } else {
            vec![(
                "Add Wallet",
                DesiredAppAction::AddScreenType(ScreenType::AddNewWallet),
            )]
        };
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            right_buttons,
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            self.render_wallet_selection(ui);
            ui.add_space(10.0);

            ui.add_space(20.0);

            // Render the address table
            if self.selected_wallet.is_some() {
                self.render_filter_selector(ui);

                ui.add_space(20.0);

                if !(self.selected_filters.contains("Asset Locks")
                    && self.selected_filters.len() == 1)
                {
                    action |= self.render_address_table(ui);
                }

                ui.add_space(20.0);

                if self.selected_filters.contains("Asset Locks") {
                    // Render the asset locks section
                    self.render_wallet_asset_locks(ui);
                }

                ui.add_space(15.0);

                self.render_bottom_options(ui);
            } else {
                ui.label("No wallet selected.");
            }

            // Display error message if any
            if let Some((message, message_type)) = &self.error_message {
                ui.add_space(10.0);
                let color = match message_type {
                    MessageType::Error => egui::Color32::RED,
                    MessageType::Info => egui::Color32::BLACK,
                    MessageType::Success => egui::Color32::GREEN,
                };
                ui.colored_label(color, message);
            }
        });

        action
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        println!("{:?}", backend_task_success_result)
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some((message.to_string(), message_type));
    }

    fn refresh_on_arrival(&mut self) {
        // Optionally implement if needed
    }

    fn refresh(&mut self) {}
}
