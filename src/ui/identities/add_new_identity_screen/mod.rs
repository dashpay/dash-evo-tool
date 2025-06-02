mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod by_wallet_qr_code;
mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::identity::{
    IdentityKeys, IdentityRegistrationInfo, IdentityTask, RegisterIdentityFundingMethod,
};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::secp256k1::hashes::hex::DisplayHex;
use dash_sdk::dpp::dashcore::{OutPoint, PrivateKey, Transaction, TxOut};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::platform::Identifier;
use eframe::egui::Context;
use egui::ahash::HashSet;
use egui::{Color32, ComboBox, ScrollArea, Ui};
use std::cmp::PartialEq;
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum FundingMethod {
    NoSelection,
    UseUnusedAssetLock,
    UseWalletBalance,
    AddressWithQRCode,
}

impl fmt::Display for FundingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = match self {
            FundingMethod::NoSelection => "Select funding method",
            FundingMethod::AddressWithQRCode => "Address with QR Code",
            FundingMethod::UseWalletBalance => "Use Wallet Balance",
            FundingMethod::UseUnusedAssetLock => "Use Unused Asset Lock (recommended)",
        };
        write!(f, "{}", output)
    }
}

pub struct AddNewIdentityScreen {
    identity_id_number: u32,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    core_has_funding_address: Option<bool>,
    funding_address: Option<Address>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    alias_input: String,
    copied_to_clipboard: Option<Option<String>>,
    identity_keys: IdentityKeys,
    error_message: Option<String>,
    show_password: bool,
    wallet_password: String,
    show_pop_up_info: Option<String>,
    in_key_selection_advanced_mode: bool,
    pub app_context: Arc<AppContext>,
    successful_qualified_identity_id: Option<Identifier>,
}

impl AddNewIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let mut selected_wallet = None;

        if app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = &app_context.wallets.read().unwrap();
            if let Some(wallet) = wallets.values().next() {
                // Automatically select the only available wallet
                selected_wallet = Some(wallet.clone());
            }
        }

        let mut created = Self {
            identity_id_number: 0, // updated later
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            selected_wallet: None, // updated later
            core_has_funding_address: None,
            funding_address: None,
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "0.5".to_string(),
            funding_amount_exact: None,
            funding_utxo: None,
            alias_input: String::new(),
            copied_to_clipboard: None,
            // updated later
            identity_keys: IdentityKeys {
                master_private_key: None,
                master_private_key_type: KeyType::ECDSA_HASH160,
                keys_input: vec![],
            },
            error_message: None,
            show_password: false,
            wallet_password: "".to_string(),
            show_pop_up_info: None,
            in_key_selection_advanced_mode: false,
            app_context: app_context.clone(),
            successful_qualified_identity_id: None,
        };

        if let Some(wallet) = selected_wallet {
            created.update_wallet(wallet);
        };

        created
    }

    /// Ensure that identity keys are correctly set up and generated.
    ///
    /// If the master key is not set, it generates a new master key and derives other keys from it.
    /// Otherwise, it updates the existing keys based on the current wallet and identity index.
    ///
    /// ## Return value
    ///
    /// * Ok(()) when the keys are correctly set up.
    /// * Err(String) if there was an error during the process, e.g. wallet not open
    pub fn ensure_correct_identity_keys(&mut self) -> Result<(), String> {
        if self.identity_keys.master_private_key.is_some() {
            return match self.update_identity_key() {
                Ok(true) => Ok(()),
                Ok(false) => Err("failed to update identity keys".to_string()),
                Err(e) => Err(format!("failed to update identity keys: {}", e)),
            };
        }

        if let Some(wallet_lock) = &self.selected_wallet {
            // sanity checks
            {
                let wallet = wallet_lock.read().unwrap();
                if !wallet.is_open() {
                    return Err(format!(
                        "wallet {} is not open",
                        wallet
                            .alias
                            .as_ref()
                            .unwrap_or(&wallet.seed_hash().to_lower_hex_string())
                    ));
                }
            }

            let app_context = &self.app_context;
            let identity_id_number = self.next_identity_id(); // note: this grabs rlock on the wallet

            const DEFAULT_KEY_TYPES: [(KeyType, Purpose, SecurityLevel); 3] = [
                (
                    KeyType::ECDSA_HASH160,
                    Purpose::AUTHENTICATION,
                    SecurityLevel::CRITICAL,
                ),
                (
                    KeyType::ECDSA_HASH160,
                    Purpose::AUTHENTICATION,
                    SecurityLevel::HIGH,
                ),
                (
                    KeyType::ECDSA_HASH160,
                    Purpose::TRANSFER,
                    SecurityLevel::CRITICAL,
                ),
            ];
            let mut wallet = wallet_lock.write().expect("wallet lock failed");
            let master_key = wallet.identity_authentication_ecdsa_private_key(
                app_context.network,
                identity_id_number,
                0,
                Some(&app_context),
            )?;

            let other_keys = DEFAULT_KEY_TYPES
                .into_iter()
                .enumerate()
                .map(|(i, (key_type, purpose, security_level))| {
                    Ok((
                        wallet.identity_authentication_ecdsa_private_key(
                            app_context.network,
                            identity_id_number,
                            (i + 1).try_into().expect("key index must fit u32"), // key index 0 is the master key
                            Some(&app_context),
                        )?,
                        key_type,
                        purpose,
                        security_level,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;

            self.identity_keys = IdentityKeys {
                master_private_key: Some(master_key),
                master_private_key_type: KeyType::ECDSA_HASH160,
                keys_input: other_keys,
            };

            Ok(())
        } else {
            Err("no wallet selected".to_string())
        }
    }

    fn render_identity_index_input(&mut self, ui: &mut egui::Ui) {
        let mut index_changed = false; // Track if the index has changed

        ui.horizontal(|ui| {
            ui.label("Identity Index:");

            // Check if we have access to the selected wallet
            if let Some(wallet_guard) = self.selected_wallet.as_ref() {
                let wallet = wallet_guard.read().unwrap();
                let used_indices: HashSet<u32> = wallet.identities.keys().cloned().collect();

                // Modify the selected text to include "(used)" if the current index is used
                let selected_text = {
                    let is_used = used_indices.contains(&self.identity_id_number);
                    if is_used {
                        format!("{} (used)", self.identity_id_number)
                    } else {
                        format!("{}", self.identity_id_number)
                    }
                };

                // Render a ComboBox to select the identity index
                ComboBox::from_id_salt("identity_index")
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        // Provide up to 30 entries for selection (0 to 29)
                        for i in 0..30 {
                            let is_used = used_indices.contains(&i);
                            let label = if is_used {
                                format!("{} (used)", i)
                            } else {
                                format!("{}", i)
                            };

                            let is_selected = self.identity_id_number == i;

                            // Enable the option if it's not used or if it's the currently selected index
                            let enabled = !is_used || is_selected;

                            // Use `add_enabled` to disable used indices
                            let response = ui.add_enabled(
                                enabled,
                                egui::SelectableLabel::new(is_selected, label),
                            );

                            // Only allow selection if the index is not used
                            if response.clicked() && !is_used {
                                self.identity_id_number = i;
                                index_changed = true;
                            }
                        }
                    });
            } else {
                ui.label("No wallet selected");
            }
        });

        // If the index has changed, update the identity key
        if index_changed {
            self.ensure_correct_identity_keys()
                .expect("failed to update identity key");
        }
    }

    // fn render_wallet_unlock(&mut self, ui: &mut Ui) -> bool {
    //     if let Some(wallet_guard) = self.selected_wallet.as_ref() {
    //         let mut wallet = wallet_guard.write().unwrap();
    //
    //         // Only render the unlock prompt if the wallet requires a password and is locked
    //         if wallet.uses_password && !wallet.is_open() {
    //             ui.add_space(10.0);
    //             ui.label("This wallet is locked. Please enter the password to unlock it:");
    //
    //             let mut unlocked = false;
    //             ui.horizontal(|ui| {
    //                 let password_input = ui.add(
    //                     egui::TextEdit::singleline(&mut self.wallet_password)
    //                         .password(!self.show_password)
    //                         .hint_text("Enter password"),
    //                 );
    //
    //                 ui.checkbox(&mut self.show_password, "Show Password");
    //
    //                 unlocked = if password_input.lost_focus()
    //                     && ui.input(|i| i.key_pressed(egui::Key::Enter))
    //                 {
    //                     let unlocked = match wallet.wallet_seed.open(&self.wallet_password) {
    //                         Ok(_) => {
    //                             self.error_message = None; // Clear any previous error
    //                             true
    //                         }
    //                         Err(_) => {
    //                             if let Some(hint) = wallet.password_hint() {
    //                                 self.error_message = Some(format!(
    //                                     "Incorrect Password, password hint is {}",
    //                                     hint
    //                                 ));
    //                             } else {
    //                                 self.error_message = Some("Incorrect Password".to_string());
    //                             }
    //                             false
    //                         }
    //                     };
    //                     // Clear the password field after submission
    //                     self.wallet_password.zeroize();
    //                     unlocked
    //                 } else {
    //                     false
    //                 };
    //             });
    //
    //             // Display error message if the password was incorrect
    //             if let Some(error_message) = &self.error_message {
    //                 ui.add_space(5.0);
    //                 ui.colored_label(Color32::RED, error_message);
    //             }
    //
    //             return unlocked;
    //         }
    //     }
    //     false
    // }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        let mut selected_wallet = None;
        let rendered = if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = &self.app_context.wallets.read().unwrap();
            if wallets.len() > 1 {
                // Retrieve the alias of the currently selected wallet, if any
                let selected_wallet_alias = self
                    .selected_wallet
                    .as_ref()
                    .and_then(|wallet| wallet.read().ok()?.alias.clone())
                    .unwrap_or_else(|| "Select".to_string());

                ui.heading(
                    "1. Choose the wallet to use in which this identities keys will come from.",
                );

                // Display the ComboBox for wallet selection
                ComboBox::from_id_salt("select_wallet")
                    .selected_text(selected_wallet_alias)
                    .show_ui(ui, |ui| {
                        for wallet in wallets.values() {
                            let wallet_alias = wallet
                                .read()
                                .ok()
                                .and_then(|w| w.alias.clone())
                                .unwrap_or_else(|| "Unnamed Wallet".to_string());

                            let is_selected = self
                                .selected_wallet
                                .as_ref()
                                .map_or(false, |selected| Arc::ptr_eq(selected, wallet));

                            if ui.selectable_label(is_selected, wallet_alias).clicked() {
                                // Update the selected wallet
                                selected_wallet = Some(wallet.clone());
                            }
                        }
                    });
                true
            } else if let Some(wallet) = wallets.values().next() {
                if self.selected_wallet.is_none() {
                    // Automatically select the only available wallet
                    selected_wallet = Some(wallet.clone());
                }
                false
            } else {
                false
            }
        } else {
            false
        };

        if let Some(wallet) = selected_wallet {
            self.update_wallet(wallet);
        }

        rendered
    }

    /// Update selected wallet and trigger all dependent actions, like updating identity keys
    /// and identity index.
    ///
    /// This function is called whenever a wallet was changed in the UI or unlocked
    fn update_wallet(&mut self, wallet: Arc<RwLock<Wallet>>) {
        let is_open = wallet.read().expect("wallet lock poisoned").is_open();

        self.selected_wallet = Some(wallet);
        self.identity_id_number = self.next_identity_id();

        if is_open {
            self.ensure_correct_identity_keys()
                .expect("failed to initialize keys")
        }
    }

    /// Generate next identity ID that can be used for the new identity.
    ///
    /// TODO: This function is not working in a reliable way, because it relies on the
    /// `identities` map in the wallet, which may not be up to date (user can remove
    /// identities from the wallet while they still are stored on the Platform).
    fn next_identity_id(&self) -> u32 {
        self.selected_wallet
            .as_ref()
            .unwrap()
            .read()
            .unwrap()
            .identities
            .keys()
            .copied()
            .max()
            .map(|max| max + 1)
            .unwrap_or_default()
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            return;
        };
        let funding_method_arc = self.funding_method.clone();
        let mut funding_method = funding_method_arc.write().unwrap(); // Write lock on funding_method

        ComboBox::from_id_salt("funding_method")
            .selected_text(format!("{}", *funding_method))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::NoSelection,
                        "Please select funding method",
                    )
                    .changed()
                {
                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::ChooseFundingMethod;
                    self.funding_amount = "0.5".to_string();
                }

                let (has_unused_asset_lock, has_balance) = {
                    let wallet = selected_wallet.read().unwrap();
                    (wallet.has_unused_asset_lock(), wallet.has_balance())
                };

                if has_unused_asset_lock {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseUnusedAssetLock,
                            "Use Unused Evo Funding Locks (recommended)",
                        )
                        .changed()
                    {
                        self.ensure_correct_identity_keys()
                            .expect("failed to initialize keys");
                        let mut step = self.step.write().unwrap();
                        *step = WalletFundedScreenStep::ReadyToCreate;
                        self.funding_amount = "0.5".to_string();
                    }
                }
                if has_balance {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseWalletBalance,
                            "Use Wallet Balance",
                        )
                        .changed()
                    {
                        if let Some(wallet) = &self.selected_wallet {
                            let wallet = wallet.read().unwrap();
                            let max_amount = wallet.max_balance();
                            self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        }
                        let mut step = self.step.write().unwrap(); // Write lock on step
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                }
                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::AddressWithQRCode,
                        "Address with QR Code",
                    )
                    .changed()
                {
                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::WaitingOnFunds;
                    self.funding_amount = "0.5".to_string();
                }
            });
    }

    // Function to render the key selection mode (Default or Advanced)
    fn render_key_selection(&mut self, ui: &mut egui::Ui) {
        // Provide the selection toggle for Default or Advanced mode
        ui.horizontal(|ui| {
            ui.label("Key Selection Mode:");

            ComboBox::from_id_salt("key_selection_mode")
                .selected_text(if self.in_key_selection_advanced_mode {
                    "Advanced"
                } else {
                    "Default"
                })
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            !self.in_key_selection_advanced_mode,
                            "Default (Recommended)",
                        )
                        .clicked()
                    {
                        self.in_key_selection_advanced_mode = false;
                    }
                    if ui
                        .selectable_label(self.in_key_selection_advanced_mode, "Advanced")
                        .clicked()
                    {
                        self.in_key_selection_advanced_mode = true;
                    }
                });
        });

        ui.add_space(10.0);

        // Render additional key options only if "Advanced" mode is selected
        if self.in_key_selection_advanced_mode {
            // Render the master key input
            if let Some((master_key, _)) = self.identity_keys.master_private_key {
                self.render_master_key(ui, master_key);
            }

            // Render additional keys input (if any) and allow adding more keys
            self.render_keys_input(ui);
        } else {
            ui.colored_label(Color32::DARK_GREEN, "Default allows for most operations on Platform: updating the identity, interacting with data contracts, transferring credits to other identities, and withdrawing to the Core payment chain. More keys can always be added later.".to_string());
        }
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];

        for (i, ((key, _), key_type, purpose, security_level)) in
            self.identity_keys.keys_input.iter_mut().enumerate()
        {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label(format!(" • Key {}:", i + 1));
                ui.label(key.to_wif());

                // Purpose selection
                ComboBox::from_id_salt(format!("purpose_combo_{}", i))
                    .selected_text(format!("{:?}", purpose))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(purpose, Purpose::AUTHENTICATION, "AUTHENTICATION");
                        ui.selectable_value(purpose, Purpose::TRANSFER, "TRANSFER");
                    });

                // Key Type selection with conditional filtering
                ComboBox::from_id_salt(format!("key_type_combo_{}", i))
                    .selected_text(format!("{:?}", key_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(key_type, KeyType::ECDSA_HASH160, "ECDSA_HASH160");
                        ui.selectable_value(key_type, KeyType::ECDSA_SECP256K1, "ECDSA_SECP256K1");
                        // ui.selectable_value(key_type, KeyType::BLS12_381, "BLS12_381");
                        // ui.selectable_value(
                        //     key_type,
                        //     KeyType::EDDSA_25519_HASH160,
                        //     "EDDSA_25519_HASH160",
                        // );
                    });

                // Security Level selection with conditional filtering
                ComboBox::from_id_salt(format!("security_level_combo_{}", i))
                    .selected_text(format!("{:?}", security_level))
                    .show_ui(ui, |ui| {
                        if *purpose == Purpose::TRANSFER {
                            // For TRANSFER purpose, security level is locked to CRITICAL
                            *security_level = SecurityLevel::CRITICAL;
                            ui.label("Locked to CRITICAL");
                        } else {
                            // For AUTHENTICATION, allow all except MASTER
                            ui.selectable_value(
                                security_level,
                                SecurityLevel::CRITICAL,
                                "CRITICAL",
                            );
                            ui.selectable_value(security_level, SecurityLevel::HIGH, "HIGH");
                            ui.selectable_value(security_level, SecurityLevel::MEDIUM, "MEDIUM");
                        }
                    });

                if ui.button("-").clicked() {
                    keys_to_remove.push(i);
                }
            });
        }

        // Remove keys marked for deletion
        for i in keys_to_remove.iter().rev() {
            self.identity_keys.keys_input.remove(*i);
        }

        // Add new key input entry
        ui.add_space(15.0);
        if ui.button("+ Add Key").clicked() {
            self.add_identity_key(
                KeyType::ECDSA_HASH160,  // Default key type
                Purpose::AUTHENTICATION, // Default purpose
                SecurityLevel::HIGH,     // Default security level
            );
        }
    }

    fn register_identity_clicked(&mut self, funding_method: FundingMethod) -> AppAction {
        let Some(selected_wallet) = &self.selected_wallet else {
            return AppAction::None;
        };
        if self.identity_keys.master_private_key.is_none() {
            return AppAction::None;
        };
        match funding_method {
            FundingMethod::UseUnusedAssetLock => {
                if let Some((tx, funding_asset_lock, address)) = self.funding_asset_lock.clone() {
                    let identity_input = IdentityRegistrationInfo {
                        alias_input: self.alias_input.clone(),
                        keys: self.identity_keys.clone(),
                        wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                        wallet_identity_index: self.identity_id_number,
                        identity_funding_method: RegisterIdentityFundingMethod::UseAssetLock(
                            address,
                            funding_asset_lock,
                            tx,
                        ),
                    };

                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;

                    AppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::RegisterIdentity(identity_input),
                    ))
                } else {
                    AppAction::None
                }
            }
            FundingMethod::UseWalletBalance => {
                // Parse the funding amount or fall back to the default value
                let amount = self.funding_amount_exact.unwrap_or_else(|| {
                    (self.funding_amount.parse::<f64>().unwrap_or_else(|_| 0.0) * 1e8) as u64
                });

                if amount == 0 {
                    return AppAction::None;
                }

                let seed = selected_wallet.read().unwrap().wallet_seed.clone();
                tracing::debug!(selected_wallet = ?selected_wallet,?seed, "funding with wallet balance");
                let identity_input = IdentityRegistrationInfo {
                    alias_input: self.alias_input.clone(),
                    keys: self.identity_keys.clone(),
                    wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                    wallet_identity_index: self.identity_id_number,
                    identity_funding_method: RegisterIdentityFundingMethod::FundWithWallet(
                        amount,
                        self.identity_id_number,
                    ),
                };

                let mut step = self.step.write().unwrap();
                *step = WalletFundedScreenStep::WaitingForAssetLock;

                // Create the backend task to register the identity
                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                    identity_input,
                )))
            }
            _ => AppAction::None,
        }
    }

    fn render_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.funding_method.read().unwrap();

        ui.horizontal(|ui| {
            ui.label("Amount (DASH):");

            // Render the text input field for the funding amount
            let amount_input = ui
                .add(
                    egui::TextEdit::singleline(&mut self.funding_amount)
                        .hint_text("Enter amount (e.g., 0.1234)")
                        .desired_width(100.0),
                )
                .lost_focus();

            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            if amount_input && enter_pressed {
                // Optional: Validate the input when Enter is pressed
                if self.funding_amount.parse::<f64>().is_err() {
                    ui.label("Invalid amount. Please enter a valid number.");
                }
            }

            // Check if the funding method is `UseWalletBalance`
            if *funding_method == FundingMethod::UseWalletBalance {
                // Safely access the selected wallet
                if let Some(wallet) = &self.selected_wallet {
                    let wallet = wallet.read().unwrap(); // Read lock on the wallet
                    if ui.button("Max").clicked() {
                        let max_amount = wallet.max_balance();
                        self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        self.funding_amount_exact = Some(max_amount);
                    }
                }
            }

            if self.funding_amount.parse::<f64>().is_err()
                || self.funding_amount.parse::<f64>().unwrap_or_default() <= 0.0
            {
                ui.colored_label(Color32::DARK_RED, "Invalid amount");
            }
        });

        ui.add_space(10.0);
    }

    /// Update existing identity keys based on the current wallet and identity index.
    ///
    /// When the wallet is updated, we need to ensure that all the private keys are
    /// generated with the correct parameters (seed, derivation path, etc.).
    ///
    /// If the master key is not set, this function is a no-op and returns Ok(false).
    fn update_identity_key(&mut self) -> Result<bool, String> {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let mut wallet = wallet_guard.write().unwrap();
            let identity_index = self.identity_id_number;

            // Update the master private key and keys input from the wallet
            self.identity_keys.master_private_key =
                Some(wallet.identity_authentication_ecdsa_private_key(
                    self.app_context.network,
                    identity_index,
                    0,
                    Some(&self.app_context),
                )?);

            // Update the additional keys input
            self.identity_keys.keys_input = self
                .identity_keys
                .keys_input
                .iter()
                .enumerate()
                .map(|(key_index, (_, key_type, purpose, security_level))| {
                    Ok((
                        wallet.identity_authentication_ecdsa_private_key(
                            self.app_context.network,
                            identity_index,
                            key_index as u32 + 1,
                            Some(&self.app_context),
                        )?,
                        *key_type,
                        *purpose,
                        *security_level,
                    ))
                })
                .collect::<Result<_, String>>()?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn add_identity_key(
        &mut self,
        key_type: KeyType,
        purpose: Purpose,
        security_level: SecurityLevel,
    ) {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let mut wallet = wallet_guard.write().unwrap();
            let new_key_index = self.identity_keys.keys_input.len() as u32 + 1;

            // Add a new key with default parameters
            self.identity_keys.keys_input.push((
                wallet
                    .identity_authentication_ecdsa_private_key(
                        self.app_context.network,
                        self.identity_id_number,
                        new_key_index,
                        Some(&self.app_context),
                    )
                    .expect("expected to have decrypted wallet"),
                key_type, // Default key type
                purpose,
                security_level,
            ));
        }
    }

    fn render_master_key(&mut self, ui: &mut egui::Ui, key: PrivateKey) {
        ui.horizontal(|ui| {
            ui.label(" • Master Private Key:");
            ui.label(key.to_wif());

            ComboBox::from_id_salt("master_key_type")
                .selected_text(format!("{:?}", self.identity_keys.master_private_key_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.identity_keys.master_private_key_type,
                        KeyType::ECDSA_SECP256K1,
                        "ECDSA_SECP256K1",
                    );
                    ui.selectable_value(
                        &mut self.identity_keys.master_private_key_type,
                        KeyType::ECDSA_HASH160,
                        "ECDSA_HASH160",
                    );
                });
        });
    }
}

impl ScreenWithWalletUnlock for AddNewIdentityScreen {
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

impl ScreenLike for AddNewIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Error {
            self.error_message = Some(format!("Error registering identity: {}", message));
        } else {
            self.error_message = Some(message.to_string());
        }
    }
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        let mut step = self.step.write().unwrap();
        match *step {
            WalletFundedScreenStep::ChooseFundingMethod => {}
            WalletFundedScreenStep::WaitingOnFunds => {
                if let Some(funding_address) = self.funding_address.as_ref() {
                    if let BackendTaskSuccessResult::CoreItem(
                        CoreItem::ReceivedAvailableUTXOTransaction(_, outpoints_with_addresses),
                    ) = backend_task_success_result
                    {
                        for (outpoint, tx_out, address) in outpoints_with_addresses {
                            if funding_address == &address {
                                *step = WalletFundedScreenStep::FundsReceived;
                                self.funding_utxo = Some((outpoint, tx_out, address))
                            }
                        }
                    }
                }
            }
            WalletFundedScreenStep::FundsReceived => {}
            WalletFundedScreenStep::ReadyToCreate => {}
            WalletFundedScreenStep::WaitingForAssetLock => {
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                ) = backend_task_success_result
                {
                    if let Some(TransactionPayload::AssetLockPayloadType(asset_lock_payload)) =
                        tx.special_transaction_payload
                    {
                        if asset_lock_payload
                            .credit_outputs
                            .iter()
                            .find(|tx_out| {
                                let Ok(address) = Address::from_script(
                                    &tx_out.script_pubkey,
                                    self.app_context.network,
                                ) else {
                                    return false;
                                };
                                if let Some(wallet) = &self.selected_wallet {
                                    let wallet = wallet.read().unwrap();
                                    wallet.known_addresses.contains_key(&address)
                                } else {
                                    false
                                }
                            })
                            .is_some()
                        {
                            *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
                        }
                    }
                }
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                if let BackendTaskSuccessResult::RegisteredIdentity(qualified_identity) =
                    backend_task_success_result
                {
                    self.successful_qualified_identity_id = Some(qualified_identity.identity.id());
                    *step = WalletFundedScreenStep::Success;
                }
            }
            WalletFundedScreenStep::Success => {}
        }
    }
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Create Identity", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                let step = {self.step.read().unwrap().clone()};
                if step == WalletFundedScreenStep::Success {
                    action |= self.show_success(ui);
                    return;
                }
                ui.add_space(10.0);
                ui.heading("Follow these steps to create your identity!");
                ui.add_space(15.0);

                let mut step_number = 1;

                if self.render_wallet_selection(ui) {
                    // We had more than 1 wallet
                    step_number += 1;
                }

                if self.selected_wallet.is_none() {
                    return;
                };

                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                if needed_unlock {
                    if just_unlocked {
                        // Select wallet will properly update all dependencies
                        self.update_wallet(self.selected_wallet.clone().expect("we just checked selected_wallet set above"));
                    } else {
                        return;
                    }
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display the heading with an info icon that shows a tooltip on hover
                ui.horizontal(|ui| {
                    let wallet_guard = self.selected_wallet.as_ref().unwrap();
                    let wallet = wallet_guard.read().unwrap();
                    if wallet.identities.is_empty() {
                        ui.heading(format!(
                            "{}. Choose an identity index. Leave this 0 if this is your first identity for this wallet.",
                            step_number
                        ));
                    } else {
                        ui.heading(format!(
                            "{}. Choose an identity index. Leaving this {} is recommended.",
                            step_number,
                            self.next_identity_id(),
                        ));
                    }


                    // Create a label with click sense and tooltip
                    let info_icon = egui::Label::new("ℹ").sense(egui::Sense::click());
                    let response = ui.add(info_icon)
                        .on_hover_text("The identity index is an internal reference within the wallet. The wallet’s seed phrase can always be used to recover any identity, including this one, by using the same index.");

                    // Check if the label was clicked
                    if response.clicked() {
                        self.show_pop_up_info = Some("The identity index is an internal reference within the wallet. The wallet’s seed phrase can always be used to recover any identity, including this one, by using the same index.".to_string());
                    }
                });

                step_number += 1;

                ui.add_space(8.0);

                self.render_identity_index_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display the heading with an info icon that shows a tooltip on hover
                ui.horizontal(|ui| {
                    ui.heading(format!(
                        "{}. Choose what keys you want to add to this new identity.",
                        step_number
                    ));

                    // Create a label with click sense and tooltip
                    let info_icon = egui::Label::new("ℹ").sense(egui::Sense::click());
                    let response = ui.add(info_icon)
                        .on_hover_text("Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.");

                    // Check if the label was clicked
                    if response.clicked() {
                        self.show_pop_up_info = Some("Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.".to_string());
                    }
                });

                step_number += 1;

                ui.add_space(8.0);

                self.render_key_selection(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.heading(
                    format!("{}. Choose your funding method.", step_number).as_str()
                );
                step_number += 1;

                ui.add_space(10.0);
                self.render_funding_method(ui);
                ui.add_space(10.0);
                ui.separator();

                // Extract the funding method from the RwLock to minimize borrow scope
                let funding_method = self.funding_method.read().unwrap().clone();

                if funding_method == FundingMethod::NoSelection {
                    return;
                }

                match funding_method {
                    FundingMethod::NoSelection => return,
                    FundingMethod::UseUnusedAssetLock => {
                        action |= self.render_ui_by_using_unused_asset_lock(ui, step_number);
                    },
                    FundingMethod::UseWalletBalance => {
                        action |= self.render_ui_by_using_unused_balance(ui, step_number);
                    },
                    FundingMethod::AddressWithQRCode => {
                        action |= self.render_ui_by_wallet_qr_code(ui, step_number)
                    },
                }
            });
        });

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::Window::new("Identity Index Information")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(show_pop_up_info_text);

                    // Add a close button to dismiss the popup
                    if ui.button("Close").clicked() {
                        self.show_pop_up_info = None
                    }
                });
        }

        action
    }
}
