use crate::app::AppAction;
use crate::model::wallet::{DerivationPathHelpers, DerivationPathReference};
use crate::ui::wallets::account_summary::AccountCategory;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use eframe::egui::{self, Ui};
use egui_extras::{Column, TableBuilder};

use super::WalletsBalancesScreen;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SortColumn {
    Address,
    Balance,
    UTXOs,
    TotalReceived,
    Type,
    Index,
    DerivationPath,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SortOrder {
    Ascending,
    Descending,
}

pub(super) struct AddressData {
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

impl AddressData {
    /// Returns the address formatted for display.
    /// Platform Payment addresses are shown in DIP-18 Bech32m format (e.g., tdash1k...).
    fn display_address(&self, network: Network) -> String {
        if self.account_category == AccountCategory::PlatformPayment {
            use dash_sdk::dpp::address_funds::PlatformAddress;
            PlatformAddress::try_from(self.address.clone())
                .map(|pa| pa.to_bech32m_string(network))
                .unwrap_or_else(|_| self.address.to_string())
        } else {
            self.address.to_string()
        }
    }
}

impl WalletsBalancesScreen {
    pub(super) fn toggle_sort(&mut self, column: SortColumn) {
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

    pub(super) fn categorize_path(
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

    pub(super) fn render_address_table(&mut self, ui: &mut Ui) -> AppAction {
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

        let account_address_count = address_data.len();

        if !self.show_zero_balance_addresses {
            address_data.retain(|data| {
                let is_platform_payment = data.account_category == AccountCategory::PlatformPayment;
                if data.account_category.is_key_only() {
                    true
                } else if is_platform_payment {
                    data.platform_credits > 0
                } else {
                    data.balance > 0
                }
            });
        }

        let show_empty_due_to_balance_filter = !self.show_zero_balance_addresses
            && account_address_count > 0
            && address_data.is_empty();

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
                        let is_key_only = data.account_category.is_key_only();
                        let is_platform_payment =
                            data.account_category == AccountCategory::PlatformPayment;

                        row.col(|ui| {
                            ui.label(data.display_address(network));
                        });
                        row.col(|ui| {
                            if is_key_only {
                                ui.label("N/A");
                            } else if is_platform_payment {
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
                            // Key-only addresses and Platform addresses don't hold UTXOs
                            if is_key_only || is_platform_payment {
                                ui.label("N/A");
                            } else {
                                ui.label(format!("{}", data.utxo_count));
                            }
                        });
                        row.col(|ui| {
                            // These address types don't track historical received amounts
                            if is_key_only || is_platform_payment {
                                ui.label("N/A");
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

                                let display_address = data.display_address(network);

                                if wallet_locked {
                                    // Store pending info and show unlock popup
                                    self.private_key_dialog.pending_derivation_path =
                                        Some(data.derivation_path.clone());
                                    self.private_key_dialog.pending_address = Some(display_address);
                                    self.wallet_unlock_popup.open();
                                } else {
                                    match self.derive_private_key_wif(&data.derivation_path) {
                                        Ok(key) => {
                                            self.private_key_dialog.is_open = true;
                                            self.private_key_dialog.address = display_address;
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

        if show_empty_due_to_balance_filter {
            ui.add_space(8.0);
            ui.label("No addresses with balance. Enable \"Show zero-balance addresses\" to view all addresses.");
        }
        action
    }
}
