use std::collections::BTreeMap;

use crate::app::AppAction;
use crate::model::wallet::{DerivationPathHelpers, DerivationPathReference};
use crate::ui::state::account_summary::{AccountCategory, categorize_account_path};
use crate::wallet_backend::poison::RwLockRecover;
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
    /// Platform address nonce (for Platform Payment addresses)
    nonce: u32,
    address_type: String,
    index: u32,
    derivation_path: DerivationPath,
    account_category: AccountCategory,
    account_index: Option<u32>,
}

/// The authoritative set of addresses to list for a wallet, each paired with
/// its derivation path.
///
/// Unions DET's bootstrapped `known_addresses` (a fixed gap-limit window derived
/// once at load) with every address the display snapshot reports: the generated
/// `snapshot_address_paths` (addresses upstream derived past DET's bootstrap
/// window) and any funded address in `snapshot_address_balances` outside both.
///
/// Listing from the same snapshot the header total and tab labels read keeps the
/// per-address rows reconciled with them. Iterating `known_addresses` alone
/// dropped funds held on addresses derived past the bootstrap window (e.g. BIP44
/// receive index ≥ 32): they counted in the headline total but never appeared in
/// the list. Addresses present in both keep their `known_addresses` path (which
/// equals the snapshot path for the same address).
pub(super) fn combined_address_paths(
    known_addresses: &BTreeMap<Address, DerivationPath>,
    snapshot_address_paths: &BTreeMap<Address, DerivationPath>,
    snapshot_address_balances: &BTreeMap<Address, u64>,
) -> BTreeMap<Address, DerivationPath> {
    let mut combined = known_addresses.clone();
    for (address, path) in snapshot_address_paths {
        combined
            .entry(address.clone())
            .or_insert_with(|| path.clone());
    }
    // Funded addresses the snapshot reports outside both sets carry no known
    // derivation path; surface them (empty path ⇒ `Other` category) so no real
    // funds are ever invisible in the list.
    for (address, &balance) in snapshot_address_balances {
        if balance > 0 {
            combined
                .entry(address.clone())
                .or_insert_with(|| DerivationPath::from(Vec::new()));
        }
    }
    combined
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

    pub(super) fn render_address_table(
        &mut self,
        ui: &mut Ui,
        account_filter: (AccountCategory, Option<u32>),
    ) -> AppAction {
        let action = AppAction::None;

        // Per-address UTXO counts + balances come from the display-only
        // `WalletBackend` snapshot (P4a). Cumulative historical receipts have no
        // upstream source post-migration, so no "Total Received" column is shown.
        let seed_hash = self
            .selected_wallet
            .as_ref()
            .and_then(|w| w.read().ok())
            .map(|g| g.seed_hash());
        let snap_address_balances = seed_hash
            .map(|sh| self.snapshot_address_balances(&sh))
            .unwrap_or_default();
        let snap_address_paths = seed_hash
            .map(|sh| self.snapshot_address_paths(&sh))
            .unwrap_or_default();
        let snap_utxo_counts: std::collections::HashMap<Address, usize> = seed_hash
            .map(|sh| {
                let mut counts: std::collections::HashMap<Address, usize> =
                    std::collections::HashMap::new();
                if let Ok(wb) = self.app_context.wallet_backend() {
                    for u in wb.utxos(&sh) {
                        *counts.entry(u.address).or_insert(0) += 1;
                    }
                }
                counts
            })
            .unwrap_or_default();

        // Without a selected wallet there is no address data to render.
        let Some(selected_wallet) = self.selected_wallet.as_ref() else {
            return action;
        };

        // Move the data preparation into its own scope
        let mut address_data = {
            let wallet = selected_wallet.read_recover();

            // List every address the display snapshot knows about, not just DET's
            // fixed bootstrap window, so funds derived past it stay visible and the
            // list reconciles with the header total and tab labels.
            let listed_addresses = combined_address_paths(
                &wallet.known_addresses,
                &snap_address_paths,
                &snap_address_balances,
            );

            // Prepare data for the table
            listed_addresses
                .iter()
                .map(|(address, derivation_path)| {
                    let utxo_count = snap_utxo_counts.get(address).copied().unwrap_or(0);

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
                    let (account_category, account_index) = Self::categorize_path(
                        derivation_path,
                        path_reference,
                        self.app_context.network,
                    );

                    // Get Platform credits balance and nonce for Platform Payment addresses only.
                    // Skip the lookup for non-platform addresses to avoid unnecessary linear
                    // scans in get_platform_address_info()'s fallback path.
                    let (platform_credits, nonce) =
                        if account_category == AccountCategory::PlatformPayment {
                            let platform_info = wallet.get_platform_address_info(address);
                            (
                                platform_info.map(|info| info.balance).unwrap_or_default(),
                                platform_info.map(|info| info.nonce).unwrap_or_default(),
                            )
                        } else {
                            (Default::default(), Default::default())
                        };

                    AddressData {
                        address: address.clone(),
                        balance: snap_address_balances
                            .get(address)
                            .copied()
                            .unwrap_or_default(),
                        platform_credits,
                        utxo_count,
                        nonce,
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

        // Zero-balance filter treats key-only addresses like any other: a
        // key-only account with no balance carries no actionable information.
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
        if is_platform_account && matches!(self.sort_column, SortColumn::UTXOs) {
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
        } else {
            builder.column(Column::initial(70.0)) // UTXOs
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
                                    self.queue_view_key_request(
                                        &data.derivation_path,
                                        display_address,
                                    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::PublicKey;
    use dash_sdk::dpp::dashcore::secp256k1::{Secp256k1, SecretKey};

    /// A distinct testnet p2pkh address keyed off `n` (derived from a valid
    /// secret key so the pubkey is a real curve point).
    fn addr(n: u8) -> Address {
        let mut sk_bytes = [1u8; 32];
        sk_bytes[31] = n.max(1);
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&sk_bytes).unwrap();
        let pubkey = PublicKey::new(sk.public_key(&secp));
        Address::p2pkh(&pubkey, Network::Testnet)
    }

    /// A BIP-44 external (receive) path `m/44'/1'/0'/0/index` on testnet.
    fn bip44_external(index: u32) -> DerivationPath {
        DerivationPath::from(vec![
            ChildNumber::Hardened { index: 44 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Normal { index: 0 },
            ChildNumber::Normal { index },
        ])
    }

    /// Regression: funds held on a BIP-44 address derived past DET's fixed
    /// bootstrap window (index ≥ 32) must still be listed. The header total and
    /// tab labels read the snapshot, which sees the address; the address table
    /// used to iterate only `known_addresses` (the 48-address bootstrap window),
    /// so such funds counted in the total but vanished from the list. The
    /// authoritative address set must union the snapshot's generated addresses
    /// in, and the buggy `known_addresses`-only set must NOT contain it.
    #[test]
    fn funds_past_bootstrap_window_are_listed() {
        // `known_addresses`: the bootstrap window — a couple of low indices.
        let mut known = BTreeMap::new();
        known.insert(addr(1), bip44_external(0));
        known.insert(addr(2), bip44_external(3));

        // The snapshot has derived further (index 40, past the window) and it is
        // funded — this is the address the old list dropped.
        let past_window = addr(40);
        let mut snap_paths = BTreeMap::new();
        snap_paths.insert(addr(1), bip44_external(0));
        snap_paths.insert(addr(2), bip44_external(3));
        snap_paths.insert(past_window.clone(), bip44_external(40));

        let mut snap_balances = BTreeMap::new();
        snap_balances.insert(past_window.clone(), 1_500_000_000u64);

        // The pre-fix source (known_addresses alone) drops the funded address.
        assert!(
            !known.contains_key(&past_window),
            "precondition: the bootstrap window does not cover the past-window address"
        );

        let combined = combined_address_paths(&known, &snap_paths, &snap_balances);

        assert!(
            combined.contains_key(&past_window),
            "a funded address derived past the bootstrap window must be listed"
        );
        // It carries the snapshot's real BIP-44 path, so it categorizes into the
        // Dash Core (BIP44 account 0) tab and reconciles with that tab's label.
        assert_eq!(combined.get(&past_window), Some(&bip44_external(40)));
        // The bootstrap-window addresses are still present.
        assert!(combined.contains_key(&addr(1)));
        assert!(combined.contains_key(&addr(2)));
    }

    /// A funded address the snapshot reports but neither `known_addresses` nor
    /// the snapshot's `address_paths` cover is still surfaced (with an empty
    /// path ⇒ `Other` category), so no real funds are ever invisible. An
    /// unfunded such address is not fabricated into a row.
    #[test]
    fn stray_funded_address_is_surfaced_unfunded_is_not() {
        let known = BTreeMap::new();
        let snap_paths = BTreeMap::new();

        let funded_stray = addr(50);
        let zero_stray = addr(51);
        let mut snap_balances = BTreeMap::new();
        snap_balances.insert(funded_stray.clone(), 7_000_000u64);
        snap_balances.insert(zero_stray.clone(), 0u64);

        let combined = combined_address_paths(&known, &snap_paths, &snap_balances);

        assert!(
            combined.contains_key(&funded_stray),
            "a funded address with no known path must still be listed"
        );
        assert_eq!(
            combined.get(&funded_stray),
            Some(&DerivationPath::from(Vec::new())),
            "a pathless funded address carries an empty path (Other category)"
        );
        assert!(
            !combined.contains_key(&zero_stray),
            "a zero-balance pathless address is not fabricated into a row"
        );
    }
}
