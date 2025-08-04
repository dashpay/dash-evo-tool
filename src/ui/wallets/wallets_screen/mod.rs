use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::Component;
use crate::ui::components::funding_widget::FundingWidget;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::dashcore::bip32::{ChildNumber, DerivationPath};
use eframe::egui::{self, ComboBox, Context, Ui};
use egui::{Color32, Frame, Margin, RichText};
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
    // Funding widget for top-up
    funding_widget: Option<FundingWidget>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
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
            funding_widget: None,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
        }
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
                    let total_balance = wallet.max_balance();
                    let dash_balance = total_balance as f64 * 1e-8; // Convert to DASH
                    ui.label(
                        RichText::new(format!("Balance: {:.8} DASH", dash_balance))
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
                    .column(Column::initial(150.0)) // Derivation Path
                    .column(Column::remainder()) // Top-up Action
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
                        header.col(|ui| {
                            ui.label("Actions");
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
                                    if data.address_type.eq("Funds")
                                        && ui
                                            .button("💰")
                                            .on_hover_text("Top-up this address")
                                            .clicked()
                                    {
                                        self.init_address_topup_widget(data.address.clone());
                                    }
                                });
                            });
                        }
                    });
            });
        action
    }

    fn render_bottom_options(&mut self, ui: &mut Ui) {
        if self.selected_filters.contains("Funds") {
            ui.add_space(10.0);

            // Check if wallet is unlocked
            let wallet_is_open = if let Some(wallet_guard) = &self.selected_wallet {
                wallet_guard.read().unwrap().is_open()
            } else {
                false
            };

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
                // Show wallet unlock UI
                self.render_wallet_unlock_if_needed(ui);
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

    fn render_top_up_modal(&mut self, ui: &mut Ui) {
        if self.funding_widget.is_none() {
            return;
        };

        let ctx = ui.ctx();
        let screen_rect = ctx.screen_rect();
        let max_height = screen_rect.height() * 0.9; // 90% of screen height

        let mut open = true;

        egui::Window::new("💰 Top-up Address")
            .collapsible(false)
            .resizable(true)
            .max_height(max_height)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .open(&mut open)
            .show(ctx, |ui| {
                self.render_modal_content(ui);
            });

        // Handle close button click
        if !open {
            self.close_funding_modal();
        }
    }

    /// Render the content inside the top-up modal
    fn render_modal_content(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([true; 2])
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    // Render the widget and get response
                    let funding_widget = self.funding_widget.as_mut().expect("Checked above");
                    let response_data = funding_widget.show(ui).inner.on_funded(|_| {
                        self.close_funding_modal();
                    });

                    if let Some(e) = response_data.error {
                        self.display_message(
                            &format!("Funding widget error: {}", e),
                            MessageType::Error,
                        );
                    }

                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(RichText::new("Close").size(14.0)).clicked() {
                                self.close_funding_modal();
                            }
                        });
                    });
                });
            });
    }

    /// Initialize funding widget for address top-up
    fn init_address_topup_widget(&mut self, address: Address) {
        let mut widget = FundingWidget::new(self.app_context.clone())
            .with_address(address)
            .with_default_amount(crate::model::amount::Amount::new_dash(0.1)) // 0.1 DASH
            .with_qr_code(true)
            .with_copy_button(true)
            .with_max_button(false) // Disable max button for address top-up
            .with_ignore_existing_utxos(true); // Enable ignore existing UTXOs for top-up

        if let Some(wallet) = &self.selected_wallet {
            widget = widget.with_wallet(wallet.clone());
        }

        self.funding_widget = Some(widget);
    }

    /// Close the funding modal and reset state
    fn close_funding_modal(&mut self) {
        self.funding_widget = None;
    }

    fn dismiss_message(&mut self) {
        self.message = None;
    }

    fn check_message_expiration(&mut self) {
        // Messages no longer auto-expire, they must be dismissed manually
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
                    MessageType::Success => egui::Color32::from_rgb(100, 255, 100),
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
                .auto_shrink([false; 2])
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
                    ui.separator();
                    ui.add_space(10.0);

                    if self.selected_wallet.is_some() {
                        // Always show the filter selector
                        ui.vertical(|ui| {
                            ui.heading(
                                RichText::new("Addresses")
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            ui.add_space(10.0);

                            // Filter section
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
                            // Render the asset locks section
                            inner_action |= self.render_wallet_asset_locks(ui);
                        }

                        ui.add_space(10.0);
                        self.render_bottom_options(ui);
                        self.render_top_up_modal(ui);
                    } else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(50.0);
                            ui.label(
                                RichText::new("Please select a wallet to view its details")
                                    .size(16.0)
                                    .color(Color32::GRAY),
                            );
                        });
                    }
                });

            inner_action
        });

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
        _backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        // Nothing
        // If we don't include this, messages from the ZMQ listener will keep popping up
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
