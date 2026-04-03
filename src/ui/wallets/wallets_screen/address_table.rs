use crate::app::AppAction;
use crate::model::wallet::{DerivationPathHelpers, DerivationPathReference};
use crate::platform_wallet_bridge::CoreAddressInfo;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::wallets::account_summary::{AccountCategory, categorize_account_path};
use dash_sdk::dashcore_rpc::dashcore::{Address, Network};
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
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
    /// Platform address nonce (for Platform Payment addresses)
    nonce: u32,
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
        network: Network,
    ) -> (AccountCategory, Option<u32>) {
        categorize_account_path(path, network, reference)
    }

    /// Build `AddressData` from the cached `CoreAddressInfo` snapshot.
    fn address_data_from_cache(cached: &[CoreAddressInfo], network: Network) -> Vec<AddressData> {
        cached
            .iter()
            .map(|info| {
                let derivation_path = &info.derivation_path;
                let address_type = if derivation_path.is_bip44_external(network) {
                    "Funds".to_string()
                } else if derivation_path.is_bip44_change(network) {
                    "Change".to_string()
                } else if derivation_path.is_asset_lock_funding(network) {
                    "Identity Creation".to_string()
                } else if derivation_path.is_platform_payment(network) {
                    "Platform".to_string()
                } else {
                    "System".to_string()
                };

                // Use Unknown reference for cached data; categorize_account_path
                // derives the category from the derivation path structure.
                let (account_category, _category_account_index) = Self::categorize_path(
                    derivation_path,
                    DerivationPathReference::Unknown,
                    network,
                );

                AddressData {
                    address: info.address.clone(),
                    balance: info.balance,
                    platform_credits: 0,
                    utxo_count: info.utxo_count,
                    total_received: info.total_received,
                    nonce: 0,
                    address_type,
                    index: info.index,
                    derivation_path: derivation_path.clone(),
                    account_category,
                    account_index: info.account_index,
                }
            })
            .collect()
    }

    /// Build `AddressData` directly from PlatformWallet's `ManagedWalletInfo`
    /// (fallback when the async cache hasn't been populated yet).
    fn address_data_from_platform_wallet(screen: &Self) -> Vec<AddressData> {
        let pw = screen
            .selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok())
            .and_then(|g| g.platform_wallet.clone());
        let Some(pw) = pw else {
            return Vec::new();
        };
        let info = pw.core().blocking_wallet_info();
        let cached = CoreAddressInfo::all_from_wallet_info(&info);
        Self::address_data_from_cache(&cached, screen.app_context.network)
    }

    pub(super) fn render_address_table(
        &mut self,
        ui: &mut Ui,
        account_filter: (AccountCategory, Option<u32>),
    ) -> AppAction {
        let action = AppAction::None;

        // Build address data from cached CoreAddressInfo if available,
        // otherwise fall back to the old direct-wallet-access path.
        let mut address_data = if let Some(cached) = &self.cached_address_info {
            Self::address_data_from_cache(cached, self.app_context.network)
        } else {
            Self::address_data_from_platform_wallet(self)
        };

        // Now you can use `self` mutably without conflict
        // Sort the data
        self.sort_address_data(&mut address_data);

        {
            let (ref category, ref index) = account_filter;
            address_data
                .retain(|data| data.account_category == *category && data.account_index == *index);
        }

        let account_address_count = address_data.len();

        // Auto-show zero-balance addresses when the wallet is nearly empty:
        // fewer than 5 addresses total and none have a balance. This prevents
        // new/empty wallets from showing a blank address list.
        let all_zero_balance = !address_data.iter().any(|d| {
            if d.account_category == AccountCategory::PlatformPayment {
                d.platform_credits > 0
            } else {
                d.balance > 0
            }
        });
        let auto_show = account_address_count < 5 && all_zero_balance;

        // INTENTIONAL(CMT-002): Zero-balance filter treats key-only addresses the same as all
        // others. The old exception (always showing key-only addresses) was removed intentionally
        // to reduce UI clutter — key-only accounts with no balance carry no actionable information.
        if !self.show_zero_balance_addresses && !auto_show {
            address_data.retain(|data| {
                if data.account_category == AccountCategory::PlatformPayment {
                    data.platform_credits > 0
                } else {
                    data.balance > 0
                }
            });
        }

        let hidden_by_balance_filter_count =
            account_address_count.saturating_sub(address_data.len());
        let show_balance_filter_hint =
            !self.show_zero_balance_addresses && hidden_by_balance_filter_count > 0;

        // Space allocation for UI elements is handled by the layout system

        let is_platform_account = account_filter.0 == AccountCategory::PlatformPayment;

        // Reset sort column if it refers to a column not visible for the current account type
        if is_platform_account
            && matches!(
                self.sort_column,
                SortColumn::UTXOs | SortColumn::TotalReceived
            )
        {
            self.sort_column = SortColumn::Balance;
            self.sort_order = SortOrder::Descending;
        }

        // Render the table
        let mut builder = TableBuilder::new(ui)
            .id_salt("addresses_table")
            .striped(false)
            .resizable(true)
            .vscroll(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto()) // Address
            .column(Column::initial(140.0)); // Balance

        builder = if is_platform_account {
            builder.column(Column::initial(80.0)) // Nonce (replaces UTXOs)
        // Total Received column omitted
        } else {
            builder
                .column(Column::initial(70.0)) // UTXOs
                .column(Column::initial(150.0)) // Total Received
        };

        builder
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
                if is_platform_account {
                    header.col(|ui| {
                        ui.label("Nonce");
                    });
                } else {
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
                };
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
                        let is_platform_payment =
                            data.account_category == AccountCategory::PlatformPayment;

                        row.col(|ui| {
                            ui.label(data.display_address(network));
                        });
                        row.col(|ui| {
                            if is_platform_payment {
                                let dash_balance =
                                    data.platform_credits as f64 / CREDITS_PER_DUFF as f64 / 1e8;
                                ui.label(format!("{:.8}", dash_balance));
                            } else {
                                let dash_balance = data.balance as f64 * 1e-8;
                                ui.label(format!("{:.8}", dash_balance));
                            }
                        });
                        if is_platform_account {
                            row.col(|ui| {
                                ui.label(format!("{}", data.nonce));
                            });
                        } else {
                            row.col(|ui| {
                                ui.label(format!("{}", data.utxo_count));
                            });
                            row.col(|ui| {
                                let dash_received = data.total_received as f64 * 1e-8;
                                ui.label(format!("{:.8}", dash_received));
                            });
                        };
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
                                        Err(err) => {
                                            MessageBanner::set_global(
                                                self.app_context.egui_ctx(),
                                                &err,
                                                MessageType::Error,
                                            );
                                        }
                                    }
                                }
                            }
                        });
                    });
                }
            });

        if show_balance_filter_hint {
            ui.add_space(8.0);
            let address_label = if hidden_by_balance_filter_count == 1 {
                "address"
            } else {
                "addresses"
            };
            ui.label(format!(
                "{} {} hidden by zero-balance filter. Enable \"Show zero-balance addresses\" to view all addresses.",
                hidden_by_balance_filter_count, address_label
            ));
        }
        action
    }
}
