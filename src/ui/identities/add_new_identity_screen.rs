use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::platform::identity::{IdentityRegistrationInfo, IdentityTask};
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use copypasta::{ClipboardContext, ClipboardProvider};
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::identity::KeyType;
use eframe::egui::Context;
use egui::{Color32, ColorImage, ComboBox, TextureHandle};
use image::Luma;
use qrcode::QrCode;
use serde::Deserialize;
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
    UseWalletBalance,
    AddressWithQRCode,
    AttachedCoreWallet,
}

impl fmt::Display for FundingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let output = match self {
            FundingMethod::AddressWithQRCode => "Address with QR Code",
            FundingMethod::AttachedCoreWallet => "Attached Core Wallet",
            FundingMethod::UseWalletBalance => "Use Wallet Balance",
        };
        write!(f, "{}", output)
    }
}

pub struct AddNewIdentityScreen {
    identity_id_input: String,
    selected_wallet: Option<Wallet>,
    funding_method: FundingMethod,
    alias_input: String,
    master_private_key_input: String,
    master_private_key_type: KeyType,
    keys_input: Vec<(String, KeyType)>,
    pub app_context: Arc<AppContext>,
}

// Function to generate a QR code image from the address
fn generate_qr_code_image(
    address: &str,
    amount: Duffs,
) -> Result<ColorImage, qrcode::types::QrError> {
    // Generate the QR code
    let code = QrCode::new(address.as_bytes())?;

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

// Function to copy text to the clipboard
pub fn copy_to_clipboard(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    // // Use `dyn` to indicate dynamic dispatch for the trait object
    // let mut ctx: ClipboardContext = ClipboardProvider::new()?.into();
    // ClipboardProvider::.set_contents(text.to_owned())?;
    Ok(())
}

impl AddNewIdentityScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            identity_id_input: String::new(),
            selected_wallet: None,
            funding_method: FundingMethod::AddressWithQRCode,
            alias_input: String::new(),
            master_private_key_input: String::new(),
            master_private_key_type: KeyType::ECDSA_HASH160,
            keys_input: vec![(String::new(), KeyType::ECDSA_HASH160)],
            app_context: app_context.clone(),
        }
    }

    fn render_qr_code(&mut self, ui: &mut egui::Ui) {
        if let Some(wallet) = self.selected_wallet.as_ref() {
            // Get the receive address
            let address = wallet.receive_address(self.app_context.network);

            // Generate the QR code image
            if let Ok(qr_image) = generate_qr_code_image(&address.to_qr_uri(), 10) {
                // // Convert the image to egui's TextureHandle
                // let texture: TextureHandle =
                //     ui.ctx()
                //         .load_texture("qr_code", qr_image, egui::TextureFilter::Linear);
                //
                // // Display the QR code image
                // ui.image(&texture);
            } else {
                ui.label("Failed to generate QR code.");
            }

            // Show the address underneath
            ui.label(&address.to_qr_uri());

            // Add a button to copy the address
            if ui.button("Copy Address").clicked() {
                if let Err(e) = copy_to_clipboard(&address.to_qr_uri()) {
                    ui.label(format!("Failed to copy to clipboard: {}", e));
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
        let identity_input = IdentityRegistrationInfo {
            identity_id_input: self.identity_id_input.trim().to_string(),
            alias_input: self.alias_input.clone(),
            master_private_key_input: self.master_private_key_input.clone(),
            master_private_key_type: self.master_private_key_type,
            keys_input: self.keys_input.clone(),
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
            identity_input,
        )))
    }

    fn render_master_key_input(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Master Private Key:");
            ui.text_edit_singleline(&mut self.master_private_key_input);

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
                        KeyType::BLS12_381,
                        "BLS12_381",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::ECDSA_HASH160,
                        "ECDSA_HASH160",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::BIP13_SCRIPT_HASH,
                        "BIP13_SCRIPT_HASH",
                    );
                    ui.selectable_value(
                        &mut self.master_private_key_type,
                        KeyType::EDDSA_25519_HASH160,
                        "EDDSA_25519_HASH160",
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

            ui.add_space(10.0);
            self.render_funding_method(ui);

            ui.add_space(10.0);
            self.render_master_key_input(ui);

            ui.horizontal(|ui| {
                ui.label("Identity ID (Hex or Base58):");
                ui.text_edit_singleline(&mut self.identity_id_input);
            });

            ui.horizontal(|ui| {
                ui.label("Alias:");
                ui.text_edit_singleline(&mut self.alias_input);
            });

            self.render_keys_input(ui);

            if ui.button("Create Identity").clicked() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                action = self.register_identity_clicked();
            }
        });

        action
    }
}
