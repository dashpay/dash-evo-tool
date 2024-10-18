use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::platform::identity::{IdentityRegistrationInfo, IdentityTask};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_new_identity_screen::AddNewIdentityScreenStep::{
    ChooseFundingMethod, FundsReceived, ReadyToCreate,
};
use crate::ui::identities::add_new_identity_screen::FundingMethod::{
    AddressWithQRCode, NoSelection,
};
use crate::ui::ScreenLike;
use arboard::Clipboard;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::Bytes32;
use dash_sdk::platform::Identifier;
use eframe::egui::Context;
use egui::{Color32, ColorImage, ComboBox, TextureHandle};
use image::Luma;
use qrcode::QrCode;
use serde::Deserialize;
use std::cmp::PartialEq;
use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
struct KeyInfo {
    address: String,
    #[serde(rename = "private_key")]
    private_key: String,
}

#[derive(Debug, PartialEq, Eq)]
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

#[derive(Eq, PartialEq, Ord, PartialOrd)]
pub enum AddNewIdentityScreenStep {
    ChooseFundingMethod,
    FundsReceived,
    ReadyToCreate,
}

pub struct AddNewIdentityScreen {
    identity_id_number: u32,
    step: AddNewIdentityScreenStep,
    selected_wallet: Option<Wallet>,
    identity_id: Option<Identifier>,
    funding_method: FundingMethod,
    funding_amount: String,
    alias_input: String,
    copied_to_clipboard: Option<Option<String>>,
    master_private_key: Option<Bytes32>,
    master_private_key_type: KeyType,
    keys_input: Vec<(String, KeyType)>,
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
            step: AddNewIdentityScreenStep::ChooseFundingMethod,
            selected_wallet: None,
            identity_id: None,
            funding_method: FundingMethod::NoSelection,
            funding_amount: "0.2".to_string(),
            alias_input: String::new(),
            copied_to_clipboard: None,
            master_private_key: None,
            master_private_key_type: KeyType::ECDSA_HASH160,
            keys_input: vec![(String::new(), KeyType::ECDSA_HASH160)],
            app_context: app_context.clone(),
        }
    }

    fn render_qr_code(&mut self, ui: &mut egui::Ui, amount: f64) {
        if let Some(wallet) = self.selected_wallet.as_ref() {
            // Get the receive address
            let address = wallet.receive_address(self.app_context.network);

            let pay_uri = format!("{}?amount={:.4}", address.to_qr_uri(), amount);

            // Generate the QR code image
            if let Ok(qr_image) = generate_qr_code_image(&pay_uri) {
                // Convert the image to egui's TextureHandle
                let texture: TextureHandle =
                    ui.ctx()
                        .load_texture("qr_code", qr_image, egui::TextureOptions::LINEAR);

                // Display the QR code image
                ui.image(&texture);
            } else {
                ui.label("Failed to generate QR code.");
            }

            ui.add_space(10.0);

            // Show the address underneath
            ui.label(pay_uri);

            // Add a button to copy the address
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
        }
    }

    fn render_wallet_selection(&mut self, ui: &mut egui::Ui) {
        let wallets = self.app_context.wallets.read().unwrap(); // Read lock

        if wallets.len() > 1 {
            ComboBox::from_label("Select Wallet")
                .selected_text(
                    self.selected_wallet
                        .as_ref()
                        .and_then(|wallet| wallet.alias.as_ref().map(|s| s.as_str()))
                        .unwrap_or("Select"),
                )
                .show_ui(ui, |ui| {
                    for wallet in wallets.iter() {
                        if ui
                            .selectable_value(
                                &mut self.selected_wallet,
                                Some(wallet.clone()),
                                wallet.alias.as_deref().unwrap_or("Unnamed Wallet"),
                            )
                            .clicked()
                        {
                            self.selected_wallet = Some(wallet.clone());
                        }
                    }
                });
        } else {
            // If there's only one wallet, automatically select it
            self.selected_wallet = wallets.first().cloned();
        }
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        ComboBox::from_label("Funding Method")
            .selected_text(format!("{}", self.funding_method))
            .show_ui(ui, |ui| {
                if let Some(wallet) = self.selected_wallet.as_ref() {
                    if wallet.has_balance() {
                        ui.selectable_value(
                            &mut self.funding_method,
                            FundingMethod::UseWalletBalance,
                            "Use wallet balance",
                        );
                    }
                }
                ui.selectable_value(
                    &mut self.funding_method,
                    FundingMethod::NoSelection,
                    "Please select funding method",
                );
                ui.selectable_value(
                    &mut self.funding_method,
                    FundingMethod::AddressWithQRCode,
                    "Address with QR Code",
                );
                ui.selectable_value(
                    &mut self.funding_method,
                    FundingMethod::AttachedCoreWallet,
                    "Attached Core Wallet",
                );
            });
    }

    fn render_keys_input(&mut self, ui: &mut egui::Ui) {
        let mut keys_to_remove = vec![];

        for (i, (key, key_type)) in self.keys_input.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Key {}:", i + 1));
                ui.text_edit_singleline(key);

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
            self.keys_input.remove(*i);
        }

        if ui.button("+ Add Key").clicked() {
            self.keys_input
                .push((String::new(), KeyType::ECDSA_HASH160));
        }
    }

    fn register_identity_clicked(&mut self) -> AppAction {
        if let Some((identifier, master_private_key)) = self
            .identity_id
            .and_then(|s| self.master_private_key.map(|k| (s, k)))
        {
            let identity_input = IdentityRegistrationInfo {
                identity_id: identifier,
                alias_input: self.alias_input.clone(),
                master_private_key,
                master_private_key_type: self.master_private_key_type,
                keys_input: self.keys_input.clone(),
            };

            AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
                identity_input,
            )))
        } else {
            AppAction::None
        }
    }

    fn render_funding_amount_input(&mut self, ui: &mut egui::Ui) {
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

            if self.funding_method == FundingMethod::UseWalletBalance {
                // Add a button to quickly set the amount to the wallet's max balance (optional)
                if let Some(wallet) = self.selected_wallet.as_ref() {
                    if ui.button("Max").clicked() {
                        let max_amount = wallet.max_balance(self.app_context.network);
                        self.funding_amount = format!("{:.4}", max_amount);
                    }
                }
            }
        });
    }

    fn render_master_key(&mut self, ui: &mut egui::Ui, key: Bytes32) {
        ui.horizontal(|ui| {
            ui.label("Master Private Key:");
            ui.label(key.to_string(Encoding::Hex));

            ComboBox::from_label("Master Key Type")
                .selected_text(format!("{:?}", self.master_private_key_type))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::ECDSA_SECP256K1,
                        "ECDSA_SECP256K1",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
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

            if self.funding_method == NoSelection {
                return;
            }

            ui.add_space(20.0);

            ui.heading("2. Choose much would you like to transfer to your new identity?");

            self.render_funding_amount_input(ui);

            let Ok(amount_dash) = self.funding_amount.parse::<f64>() else {
                return;
            };

            if self.step == ChooseFundingMethod && self.funding_method == AddressWithQRCode {
                self.render_qr_code(ui, amount_dash);
            }

            if self.step < FundsReceived {
                ui.add_space(20.0);
                ui.heading("...Waiting for funds to continue...");
                return;
            }

            ui.heading("2. Leave 0 if this is the first identity for your wallet. If you have made an identity already bump this number");

            // todo add a selector for the identity id number, it should have up to 30 entries

            ui.add_space(10.0);
            if let Some(key) = self.master_private_key {
                self.render_master_key(ui, key);
            }

            ui.horizontal(|ui| {
                ui.label("Identity ID (Hex or Base58):");
                ui.label(self.identity_id.map(|id| id.to_string(Encoding::Base58)).unwrap_or("None".to_string()));
            });

            self.render_keys_input(ui);

            if self.step == ReadyToCreate {
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
