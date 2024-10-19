use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::platform::identity::{IdentityKeys, IdentityRegistrationInfo, IdentityTask};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreenStep::{
    ChooseFundingMethod, FundsReceived, ReadyToCreate,
};
use crate::ui::ScreenLike;
use arboard::Clipboard;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::PrivateKey;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::Context;
use egui::{Color32, ColorImage, ComboBox, TextureHandle};
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
        };
        write!(f, "{}", output)
    }
}

#[derive(Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub enum AddNewIdentityScreenStep {
    ChooseFundingMethod,
    FundsReceived,
    ReadyToCreate,
}

pub struct AddNewIdentityScreen {
    identity_id_number: u32,
    step: Arc<RwLock<AddNewIdentityScreenStep>>,
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
            step: Arc::new(RwLock::new(AddNewIdentityScreenStep::ChooseFundingMethod)),
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
                        if let Some(mut wallet_guard) = selected_wallet.as_ref() {
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
                            *step.write().unwrap() = AddNewIdentityScreenStep::FundsReceived;
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
                        if ui.selectable_value(&mut self.identity_id_number, i, format!("{}", i)).clicked() {
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

    fn render_wallet_selection(&mut self, ui: &mut egui::Ui) {

        if self.app_context.wallets.len() > 1 {
            ComboBox::from_label("Select Wallet")
                .selected_text(
                    self.selected_wallet // Read lock on selected_wallet
                        .as_ref()
                        .and_then(|wallet| wallet.read().unwrap().alias.as_ref().map(|s| s.as_str()))
                        .unwrap_or("Select"),
                )
                .show_ui(ui, |ui| {
                    for wallet in self.app_context.wallets.iter() {
                        let a = self.selected_wallet
                            .as_ref();
                        let mut selected_wallet = self.selected_wallet.write().unwrap(); // Write lock on selected_wallet
                        if ui
                            .selectable_value(
                                &mut *selected_wallet,
                                Some(wallet.clone()),
                                wallet.alias.as_deref().unwrap_or("Unnamed Wallet"),
                            )
                            .clicked()
                        {
                            *selected_wallet = Some(wallet.clone());
                        }
                    }
                });
        } else if let Some(wallet) = wallets.first() {
            // Automatically select the only available wallet
            *self.selected_wallet.write().unwrap() = Some(wallet.clone());
        }
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let Some(selected_wallet) = self.selected_wallet.as_ref() else {
            return;
        };
        let mut funding_method = self.funding_method.write().unwrap(); // Write lock on funding_method

        ComboBox::from_label("Funding Method")
            .selected_text(format!("{}", *funding_method))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::NoSelection,
                    "Please select funding method",
                );
                if selected_wallet.read().unwrap().has_balance() {
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
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::AttachedCoreWallet,
                    "Attached Core Wallet",
                );
            });
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];

        for (i, (key, key_type)) in self.identity_keys.keys_input.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Key {}:", i + 1));
                ui.label(key.to_wif());

                ComboBox::from_label("Key Type")
                    .selected_text(format!("{:?}", key_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(key_type, KeyType::ECDSA_SECP256K1, "ECDSA_SECP256K1");
                        ui.selectable_value(key_type, KeyType::BLS12_381, "BLS12_381");
                        ui.selectable_value(key_type, KeyType::ECDSA_HASH160, "ECDSA_HASH160");
                        ui.selectable_value(
                            key_type,
                            KeyType::BIP13_SCRIPT_HASH,
                            "BIP13_SCRIPT_HASH",
                        );
                        ui.selectable_value(
                            key_type,
                            KeyType::EDDSA_25519_HASH160,
                            "EDDSA_25519_HASH160",
                        );
                    });

                if ui.button("-").clicked() {
                    keys_to_remove.push(i);
                }
            });
        }

        for i in keys_to_remove.iter().rev() {
            self.identity_keys.keys_input.remove(*i);
        }

        if ui.button("+ Add Key").clicked() {
            self.add_identity_key();
        }
    }

    fn register_identity_clicked(&mut self) -> AppAction {
        if self.identity_keys.master_private_key.is_some()
        {
            let amount = self.funding_amount_exact.unwrap_or_else(|| {
                self.funding_amount.parse().unwrap_or_default()
            });
            let identity_input = IdentityRegistrationInfo {
                alias_input: self.alias_input.clone(),
                amount,
                keys: self.identity_keys.clone(),
                identity_index: 0,
                wallet: self.selected_wallet,
            };

            AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                identity_input,
            )))
        } else {
            AppAction::None
        }
    }

    fn render_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.funding_method.read().unwrap(); // Read lock on funding_method
        let selected_wallet = self.selected_wallet.read().unwrap(); // Read lock on selected_wallet

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
                if let Err(_) = self.funding_amount.parse::<f64>() {
                    ui.label("Invalid amount. Please enter a valid number.");
                }
            }

            // Check if the funding method is `UseWalletBalance`
            if *funding_method == FundingMethod::UseWalletBalance {
                // Add a button to quickly set the amount to the wallet's max balance (if applicable)
                if let Some(wallet) = selected_wallet.as_ref() {
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
            self.identity_keys = IdentityKeys {
                master_private_key: Some(wallet.identity_authentication_ecdsa_private_key(self.app_context.network, identity_index, 0)),
                master_private_key_type: self.identity_keys.master_private_key_type,
                keys_input: self.identity_keys.keys_input.iter().enumerate().map(|(key_index, (_, key_type))|
                (wallet.identity_authentication_ecdsa_private_key(self.app_context.network, identity_index, key_index as u32 + 1), *key_type)
                ).collect(),
            }
        }
    }

    fn add_identity_key(&mut self) {
        if let Some(wallet_guard) = self.selected_wallet.as_ref() {
            let wallet = wallet_guard.read().unwrap();
            self.identity_keys.keys_input.push(( wallet.identity_authentication_ecdsa_private_key(self.app_context.network, self.identity_id_number, self.identity_keys.keys_input.len() as u32 + 1), KeyType::ECDSA_HASH160))
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

            self.render_wallet_selection(ui);

            ui.heading("1. Choose your funding method.");

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

                self.render_funding_amount_input(ui);
            }

            let Ok(amount_dash) = self.funding_amount.parse::<f64>() else {
                return;
            };

            // Extract the step from the RwLock to minimize borrow scope
            let step = self.step.read().unwrap().clone();

            if step == ChooseFundingMethod && funding_method == FundingMethod::AddressWithQRCode {
                if let Err(e) = self.render_qr_code(ui, amount_dash) {
                    eprintln!("Error: {:?}", e);
                }
            }

            if step < FundsReceived && funding_method != FundingMethod::UseWalletBalance {
                ui.add_space(20.0);
                ui.heading("...Waiting for funds to continue...");
                return;
            }

            ui.heading(
                "3. Choose an identity index. Leave this 0 if this is your first identity for this wallet."
            );

            self.render_identity_index_input(ui);

            ui.add_space(10.0);

            if let Some(key) = self.identity_keys.master_private_key {
                self.render_master_key(ui, key);
            }

            self.render_keys_input(ui);

            if step == ReadyToCreate || funding_method == FundingMethod::UseWalletBalance {
                if ui.button("Create Identity").clicked() {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    action = self.register_identity_clicked();
                }
            }
        });

        action
    }
}
