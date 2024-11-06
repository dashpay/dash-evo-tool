use crate::app::AppAction;
use crate::backend_task::identity::{
    IdentityKeys, IdentityRegistrationInfo, IdentityRegistrationMethod, IdentityTask,
};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityWalletFundedScreenStep::{
    ChooseFundingMethod, FundsReceived, ReadyToCreate,
};
use crate::ui::{MessageType, ScreenLike};
use arboard::Clipboard;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::{PrivateKey, Transaction};
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::Context;
use egui::{Color32, ColorImage, ComboBox, TextureHandle, Ui};
use image::Luma;
use qrcode::QrCode;
use serde::Deserialize;
use std::cmp::PartialEq;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fmt, thread};

#[derive(Debug, Clone, Deserialize)]
struct KeyInfo {
    address: String,
    #[serde(rename = "private_key")]
    private_key: String,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum FundingMethod {
    NoSelection,
    UseUnusedAssetLock,
    UseWalletBalance,
    AddressWithQRCode,
    AttachedCoreWallet,
}

impl fmt::Display for FundingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = match self {
            FundingMethod::NoSelection => "Select funding method",
            FundingMethod::AddressWithQRCode => "Address with QR Code",
            FundingMethod::AttachedCoreWallet => "Attached Core Wallet",
            FundingMethod::UseWalletBalance => "Use Wallet Balance",
            FundingMethod::UseUnusedAssetLock => "Use Unused Asset Lock (recommended)",
        };
        write!(f, "{}", output)
    }
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub enum AddNewIdentityWalletFundedScreenStep {
    ChooseFundingMethod,
    FundsReceived,
    ReadyToCreate,
    WaitingForAssetLock,
    WaitingForPlatformAcceptance,
}

pub struct AddNewIdentityScreen {
    identity_id_number: u32,
    step: Arc<RwLock<AddNewIdentityWalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    core_has_funding_address: Option<bool>,
    funding_address: Option<Address>,
    funding_address_balance: Arc<RwLock<Option<Duffs>>>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    alias_input: String,
    copied_to_clipboard: Option<Option<String>>,
    identity_keys: IdentityKeys,
    balance_check_handle: Option<(Arc<AtomicBool>, thread::JoinHandle<()>)>,
    error_message: Option<String>,
    pub app_context: Arc<AppContext>,
}

// Function to generate a QR code image from the address
fn generate_qr_code_image(pay_uri: &str) -> Result<ColorImage, qrcode::types::QrError> {
    // Generate the QR code
    let code = QrCode::new(pay_uri.as_bytes())?;

    // Render the QR code into an image buffer
    let image = code.render::<Luma<u8>>().build();

    // Convert the image buffer to ColorImage
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let pixels: Vec<Color32> = pixels
        .into_iter()
        .map(|p| {
            let color = 255 - p; // Invert colors for better visibility
            Color32::from_rgba_unmultiplied(color, color, color, 255)
        })
        .collect();

    Ok(ColorImage { size, pixels })
}

pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

impl AddNewIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            identity_id_number: 0,
            step: Arc::new(RwLock::new(
                AddNewIdentityWalletFundedScreenStep::ChooseFundingMethod,
            )),
            funding_asset_lock: None,
            selected_wallet: None,
            core_has_funding_address: None,
            funding_address: None,
            funding_address_balance: Arc::new(RwLock::new(None)),
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "0.2".to_string(),
            funding_amount_exact: None,
            alias_input: String::new(),
            copied_to_clipboard: None,
            identity_keys: IdentityKeys {
                master_private_key: None,
                master_private_key_type: KeyType::ECDSA_HASH160,
                keys_input: vec![],
            },
            balance_check_handle: None,
            error_message: None,
            app_context: app_context.clone(),
        }
    }

    // Start the balance checking process
    pub fn start_balance_check(&mut self, check_address: &Address, ui_context: &Context) {
        let app_context = self.app_context.clone();
        let balance_state = Arc::clone(&self.funding_address_balance);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_clone = Arc::clone(&stop_flag);
        let ctx = ui_context.clone();

        let selected_wallet = self.selected_wallet.clone();
        let funding_method = Arc::clone(&self.funding_method);
        let step = Arc::clone(&self.step);

        let starting_balance = balance_state.read().unwrap().unwrap_or_default();
        let expected_balance = starting_balance + self.funding_amount.parse::<Duffs>().unwrap_or(1);

        let address = check_address.clone();

        // Spawn a new thread to monitor the balance.
        let handle = thread::spawn(move || {
            while !stop_flag_clone.load(Ordering::Relaxed) {
                match app_context
                    .core_client
                    .get_received_by_address(&address, Some(1))
                {
                    Ok(new_balance) => {
                        // Update wallet balance if it has changed.
                        if let Some(wallet_guard) = selected_wallet.as_ref() {
                            let mut wallet = wallet_guard.write().unwrap();
                            wallet
                                .update_address_balance(
                                    &address,
                                    new_balance.to_sat(),
                                    &app_context,
                                )
                                .ok();
                        }

                        // Write the new balance into the RwLock.
                        if let Ok(mut balance) = balance_state.write() {
                            *balance = Some(new_balance.to_sat());
                        }

                        // Trigger UI redraw.
                        ctx.request_repaint();

                        // Check if expected balance is reached and update funding method and step.
                        if new_balance.to_sat() >= expected_balance {
                            *funding_method.write().unwrap() = FundingMethod::UseWalletBalance;
                            *step.write().unwrap() =
                                AddNewIdentityWalletFundedScreenStep::FundsReceived;
                            break;
                        }
                    }
                    Err(e) => {
                        // Get the current time
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards");
                        eprintln!("[{:?}] Error fetching balance: {:?}", now, e);
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
        });

        // Save the handle and stop flag to allow stopping the thread later.
        self.balance_check_handle = Some((stop_flag, handle));
    }

    // Stop the balance checking process
    fn stop_balance_check(&mut self) {
        if let Some((stop_flag, handle)) = self.balance_check_handle.take() {
            // Set the atomic flag to stop the thread
            stop_flag.store(true, Ordering::Relaxed);
            // Wait for the thread to finish
            if let Err(e) = handle.join() {
                eprintln!("Failed to join balance check thread: {:?}", e);
            }
        }
    }

    fn render_qr_code(&mut self, ui: &mut egui::Ui, amount: f64) -> Result<(), String> {
        let (address, should_check_balance) = {
            // Scope the write lock to ensure it's dropped before calling `start_balance_check`.

            if let Some(wallet_guard) = self.selected_wallet.as_ref() {
                // Get the receive address
                if self.funding_address.is_none() {
                    let mut wallet = wallet_guard.write().unwrap();
                    let receive_address = wallet
                        .receive_address(self.app_context.network, Some(&self.app_context))?;

                    if let Some(has_address) = self.core_has_funding_address {
                        if !has_address {
                            self.app_context
                                .core_client
                                .import_address(
                                    &receive_address,
                                    Some("Managed by Dash Evo Tool"),
                                    Some(false),
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        self.funding_address = Some(receive_address);
                    } else {
                        let info = self
                            .app_context
                            .core_client
                            .get_address_info(&receive_address)
                            .map_err(|e| e.to_string())?;

                        if !(info.is_watchonly || info.is_mine) {
                            self.app_context
                                .core_client
                                .import_address(
                                    &receive_address,
                                    Some("Managed by Dash Evo Tool"),
                                    Some(false),
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        self.funding_address = Some(receive_address);
                        self.core_has_funding_address = Some(true);
                    }

                    // Extract the address to return it outside this scope
                    (self.funding_address.as_ref().unwrap().clone(), true)
                } else {
                    (self.funding_address.as_ref().unwrap().clone(), false)
                }
            } else {
                return Err("No wallet selected".to_string());
            }
        };

        if should_check_balance {
            // Now `address` is available, and all previous borrows are dropped.
            self.start_balance_check(&address, ui.ctx());
        }

        let pay_uri = format!("{}?amount={:.4}", address.to_qr_uri(), amount);

        // Generate the QR code image
        if let Ok(qr_image) = generate_qr_code_image(&pay_uri) {
            let texture: TextureHandle =
                ui.ctx()
                    .load_texture("qr_code", qr_image, egui::TextureOptions::LINEAR);
            ui.image(&texture);
        } else {
            ui.label("Failed to generate QR code.");
        }

        ui.add_space(10.0);
        ui.label(pay_uri);

        if ui.button("Copy Address").clicked() {
            if let Err(e) = copy_to_clipboard(&address.to_qr_uri()) {
                self.copied_to_clipboard = Some(Some(e));
            } else {
                self.copied_to_clipboard = Some(None);
            }
        }

        if let Some(error) = self.copied_to_clipboard.as_ref() {
            if let Some(error) = error {
                ui.label(format!("Failed to copy to clipboard: {}", error));
            } else {
                ui.label("Address copied to clipboard.");
            }
        }

        Ok(())
    }

    fn render_identity_index_input(&mut self, ui: &mut egui::Ui) {
        let mut index_changed = false; // Track if the index has changed

        ui.horizontal(|ui| {
            ui.label("Identity Index:");

            // Render a ComboBox to select the identity index
            ComboBox::from_id_salt("identity_index")
                .selected_text(format!("{}", self.identity_id_number))
                .show_ui(ui, |ui| {
                    // Provide up to 30 entries for selection (0 to 29)
                    for i in 0..30 {
                        if ui
                            .selectable_value(&mut self.identity_id_number, i, format!("{}", i))
                            .clicked()
                        {
                            self.identity_id_number = i;
                            index_changed = true;
                        }
                    }
                });
        });

        // If the index has changed, call update_identity_key
        if index_changed {
            self.update_identity_key();
        }
    }

    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            let wallet = selected_wallet.read().unwrap(); // Read lock on the wallet

            let total_balance: u64 = wallet.max_balance(); // Sum up all the balances

            let dash_balance = total_balance as f64 * 1e-8; // Convert to DASH units

            ui.horizontal(|ui| {
                ui.label(format!("Wallet Balance: {:.8} DASH", dash_balance));
            });
        } else {
            ui.label("No wallet selected");
        }
    }
    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        if self.app_context.has_wallet.load(Ordering::Relaxed) {
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

                ui.add_space(10.0);

                // Display the ComboBox for wallet selection
                ComboBox::from_label("Select Wallet")
                    .selected_text(selected_wallet_alias)
                    .show_ui(ui, |ui| {
                        for wallet in wallets.iter() {
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
                                self.selected_wallet = Some(wallet.clone());
                            }
                        }
                    });
                ui.add_space(10.0);
                true
            } else if let Some(wallet) = wallets.first() {
                // Automatically select the only available wallet
                self.selected_wallet = Some(wallet.clone());
                false
            } else {
                false
            }
        } else {
            false
        }
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            return;
        };
        let funding_method_arc = self.funding_method.clone();
        let mut funding_method = funding_method_arc.write().unwrap(); // Write lock on funding_method

        ComboBox::from_label("Funding Method")
            .selected_text(format!("{}", *funding_method))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::NoSelection,
                    "Please select funding method",
                );
                let wallet = selected_wallet.read().unwrap();
                if wallet.has_unused_asset_lock() {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseUnusedAssetLock,
                            "Use Unused Evo Funding Locks (recommended)",
                        )
                        .changed()
                    {
                        self.update_identity_key();
                    }
                }
                if wallet.has_balance() {
                    ui.selectable_value(
                        &mut *funding_method,
                        FundingMethod::UseWalletBalance,
                        "Use Wallet Balance",
                    );
                }
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::AddressWithQRCode,
                    "Address with QR Code",
                );

                // ui.selectable_value(
                //     &mut *funding_method,
                //     FundingMethod::AttachedCoreWallet,
                //     "Attached Core Wallet",
                // );
            });
    }

    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        // Ensure a wallet is selected
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            ui.label("No wallet selected.");
            return;
        };

        // Read the wallet to access unused asset locks
        let wallet = selected_wallet.read().unwrap();

        if wallet.unused_asset_locks.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");

        // Track the index of the currently selected asset lock (if any)
        let selected_index = self.funding_asset_lock.as_ref().and_then(|(_, proof, _)| {
            wallet
                .unused_asset_locks
                .iter()
                .position(|(_, _, _, _, p)| p.as_ref() == Some(proof))
        });

        // Display the asset locks in a scrollable area
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, (tx, address, amount, islock, proof)) in
                wallet.unused_asset_locks.iter().enumerate()
            {
                ui.horizontal(|ui| {
                    let tx_id = tx.txid().to_string();
                    let lock_amount = *amount as f64 * 1e-8; // Convert to DASH
                    let is_locked = if islock.is_some() { "Yes" } else { "No" };

                    // Display asset lock information with "Selected" if this one is selected
                    let selected_text = if Some(index) == selected_index {
                        " (Selected)"
                    } else {
                        ""
                    };

                    ui.label(format!(
                        "TxID: {}, Address: {}, Amount: {:.8} DASH, InstantLock: {}{}",
                        tx_id, address, lock_amount, is_locked, selected_text
                    ));

                    // Button to select this asset lock
                    if ui.button("Select").clicked() {
                        // Update the selected asset lock
                        self.funding_asset_lock = Some((
                            tx.clone(),
                            proof.clone().expect("Asset lock proof is required"),
                            address.clone(),
                        ));

                        // Update the step to ready to create identity
                        let mut step = self.step.write().unwrap();
                        *step = AddNewIdentityWalletFundedScreenStep::ReadyToCreate;
                    }
                });

                ui.add_space(5.0); // Add space between each entry
            }
        });
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];

        for (i, (key, key_type, purpose, security_level)) in
            self.identity_keys.keys_input.iter_mut().enumerate()
        {
            ui.horizontal(|ui| {
                ui.label(format!("Key {}:", i + 1));
                ui.label(key.to_wif());

                // Purpose selection
                ComboBox::from_label("Purpose")
                    .selected_text(format!("{:?}", purpose))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(purpose, Purpose::AUTHENTICATION, "AUTHENTICATION");
                        ui.selectable_value(purpose, Purpose::TRANSFER, "TRANSFER");
                    });

                // Key Type selection with conditional filtering
                ComboBox::from_label("Key Type")
                    .selected_text(format!("{:?}", key_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(key_type, KeyType::ECDSA_HASH160, "ECDSA_HASH160");
                        ui.selectable_value(key_type, KeyType::ECDSA_SECP256K1, "ECDSA_SECP256K1");
                        ui.selectable_value(key_type, KeyType::BLS12_381, "BLS12_381");
                        ui.selectable_value(
                            key_type,
                            KeyType::EDDSA_25519_HASH160,
                            "EDDSA_25519_HASH160",
                        );
                    });

                // Security Level selection with conditional filtering
                ComboBox::from_label("Security Level")
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
        if ui.button("+ Add Key").clicked() {
            self.add_identity_key();
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
                        identity_registration_method: IdentityRegistrationMethod::UseAssetLock(
                            address,
                            funding_asset_lock,
                            tx,
                        ),
                    };

                    let mut step = self.step.write().unwrap();
                    *step = AddNewIdentityWalletFundedScreenStep::WaitingForPlatformAcceptance;

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
                let identity_input = IdentityRegistrationInfo {
                    alias_input: self.alias_input.clone(),
                    keys: self.identity_keys.clone(),
                    wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                    identity_registration_method: IdentityRegistrationMethod::FundWithWallet(
                        amount,
                        self.identity_id_number,
                    ),
                };

                let mut step = self.step.write().unwrap();
                *step = AddNewIdentityWalletFundedScreenStep::WaitingForAssetLock;

                // Create the backend task to register the identity
                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                    identity_input,
                )))
            }
            _ => AppAction::None,
        }
    }

    fn render_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.funding_method.read().unwrap(); // Read lock on funding_method

        ui.horizontal(|ui| {
            ui.label("Funding Amount (DASH):");

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
        });
    }
    fn update_identity_key(&mut self) {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let wallet = wallet_guard.read().unwrap();
            let identity_index = self.identity_id_number;

            // Update the master private key and keys input from the wallet
            self.identity_keys.master_private_key =
                Some(wallet.identity_authentication_ecdsa_private_key(
                    self.app_context.network,
                    identity_index,
                    0,
                ));

            // Update the additional keys input
            self.identity_keys.keys_input = self
                .identity_keys
                .keys_input
                .iter()
                .enumerate()
                .map(|(key_index, (_, key_type, purpose, security_level))| {
                    (
                        wallet.identity_authentication_ecdsa_private_key(
                            self.app_context.network,
                            identity_index,
                            key_index as u32 + 1,
                        ),
                        *key_type,
                        *purpose,
                        *security_level,
                    )
                })
                .collect();
        }
    }

    fn add_identity_key(&mut self) {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let wallet = wallet_guard.read().unwrap();
            let new_key_index = self.identity_keys.keys_input.len() as u32 + 1;

            // Add a new key with default parameters
            self.identity_keys.keys_input.push((
                wallet.identity_authentication_ecdsa_private_key(
                    self.app_context.network,
                    self.identity_id_number,
                    new_key_index,
                ),
                KeyType::ECDSA_HASH160,  // Default key type
                Purpose::AUTHENTICATION, // Default purpose
                SecurityLevel::HIGH,     // Default security level
            ));
        }
    }

    fn render_master_key(&mut self, ui: &mut egui::Ui, key: PrivateKey) {
        ui.horizontal(|ui| {
            ui.label("Master Private Key:");
            ui.label(key.to_wif());

            ComboBox::from_label("Master Key Type")
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

impl ScreenLike for AddNewIdentityScreen {
    fn display_message(&mut self, message: &str, _message_type: MessageType) {
        self.error_message = Some(message.to_string());
    }
    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {
        let mut step = self.step.write().unwrap();
        *step = AddNewIdentityWalletFundedScreenStep::WaitingForPlatformAcceptance;
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

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.heading("Follow these steps to create your identity!");
            ui.add_space(5.0);

            let mut step_number = 1;

            if self.render_wallet_selection(ui) {
                // We had more than 1 wallet
                step_number += 1;
            }

            if self.selected_wallet.is_none() {
                return;
            };

            ui.heading(
                format!("{}. Choose your funding method.", step_number).as_str()
            );
            step_number += 1;

            ui.add_space(10.0);
            self.render_funding_method(ui);

            // Extract the funding method from the RwLock to minimize borrow scope
            let funding_method = self.funding_method.read().unwrap().clone();

            if funding_method == FundingMethod::NoSelection {
                return;
            }

            ui.add_space(20.0);

            if funding_method == FundingMethod::AddressWithQRCode {
                ui.heading("2. Choose how much you would like to transfer to your new identity?");
                step_number += 1;

                self.render_funding_amount_input(ui);
            }

            if funding_method == FundingMethod::UseWalletBalance {
                self.show_wallet_balance(ui);

                step_number += 1;

                ui.heading("2. How much of your wallet balance would you like to transfer?");
                step_number += 1;

                self.render_funding_amount_input(ui);
            }

            // Extract the step from the RwLock to minimize borrow scope
            let step = self.step.read().unwrap().clone();

            if funding_method != FundingMethod::UseUnusedAssetLock {
                let Ok(amount_dash) = self.funding_amount.parse::<f64>() else {
                    return;
                };

                if step == ChooseFundingMethod && funding_method == FundingMethod::AddressWithQRCode {
                    if let Err(e) = self.render_qr_code(ui, amount_dash) {
                        eprintln!("Error: {:?}", e);
                    }
                }
            }

            if step < FundsReceived && funding_method == FundingMethod::AddressWithQRCode {
                ui.add_space(20.0);
                ui.heading("...Waiting for funds to continue...");
                return;
            }

            if funding_method == FundingMethod::UseUnusedAssetLock {
                ui.heading(
                    format!("{}. Choose the unused asset lock that you would like to use.", step_number).as_str()
                );
                ui.add_space(10.0);
                self.render_choose_funding_asset_lock(ui);
                step_number += 1;
            }

            ui.heading(
                format!("{}. Choose an identity index. Leave this 0 if this is your first identity for this wallet.", step_number).as_str()
            );

            self.render_identity_index_input(ui);

            ui.add_space(10.0);

            if let Some(key) = self.identity_keys.master_private_key {
                self.render_master_key(ui, key);
            }

            self.render_keys_input(ui);

            if step == ReadyToCreate || funding_method == FundingMethod::UseWalletBalance || funding_method == FundingMethod::UseUnusedAssetLock {
                if ui.button("Create Identity").clicked() {
                    action = self.register_identity_clicked(funding_method);
                }
            }

            if step == AddNewIdentityWalletFundedScreenStep::WaitingForAssetLock {
                ui.heading("Waiting for Asset Lock");
            }

            if let Some(error_message) = self.error_message.as_ref() {
                ui.heading(error_message);
            }

            if step == AddNewIdentityWalletFundedScreenStep::WaitingForPlatformAcceptance {
                ui.heading("Waiting for Platform Acknowledgement");
            }
        });

        action
    }
}
