use crate::app::AppAction;
use crate::backend_task::identity::{IdentityResult, IdentityTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::Wallet;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey as RPCPrivateKey;
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::sign_message::signed_msg_hash;
use dash_sdk::dpp::dashcore::{Address, PrivateKey, PubkeyHash, ScriptHash};
use dash_sdk::dpp::identity::KeyType::BIP13_SCRIPT_HASH;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context};
use egui::{Color32, Frame, Margin, RichText, ScrollArea};
use std::sync::{Arc, RwLock};

pub struct KeyInfoScreen {
    pub identity: QualifiedIdentity,
    pub key: IdentityPublicKey,
    pub private_key_data: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
    pub decrypted_private_key: Option<RPCPrivateKey>,
    pub app_context: Arc<AppContext>,
    private_key_input: String,
    error_message: Option<String>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    message_input: String,
    signed_message: Option<String>,
    sign_error_message: Option<String>,
    view_wallet_unlock: bool,
    wallet_open: bool,
    view_private_key_even_if_encrypted_or_in_wallet: bool,
    show_pop_up_info: Option<String>,
    show_confirm_remove_private_key: bool,
    show_confirm_disable_key: bool,
    disable_key_submitted: bool,
    show_confirm_replace_key: bool,
    replace_key_submitted: bool,
    replace_key_type: KeyType,
    replace_key_private_hex: String,
    success_message: Option<String>,
}

// /// The prefix for signed messages using Dash's message signing protocol.
// pub const DASH_SIGNED_MSG_PREFIX: &[u8] = b"\x19Dash Signed Message:\n";
//
// pub fn signed_msg_hash(msg: &str) -> sha256d::Hash {
//     let mut engine = sha256d::Hash::engine();
//     engine.input(DASH_SIGNED_MSG_PREFIX);
//     let msg_len = encode::VarInt(msg.len() as u64);
//     msg_len.consensus_encode(&mut engine).expect("engines don't error");
//     engine.input(msg.as_bytes());
//     sha256d::Hash::from_engine(engine)
// }

impl ScreenLike for KeyInfoScreen {
    fn refresh(&mut self) {}

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Error => {
                self.error_message = Some(message.to_string());
                self.disable_key_submitted = false;
                self.replace_key_submitted = false;
            }
            MessageType::Success => {
                self.success_message = Some(message.to_string());
            }
            _ => {}
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::Identity(identity_result) = backend_task_success_result {
            match identity_result {
                IdentityResult::DisabledKeys(updated_identity, _fee_result) => {
                    self.identity = updated_identity;
                    let key_id = self.key.id();
                    if let Some(updated_key) = self.identity.identity.public_keys().get(&key_id) {
                        self.key = IdentityPublicKey::clone(updated_key);
                    }
                    self.disable_key_submitted = false;
                    self.success_message = Some("Key has been disabled on Platform.".to_string());
                }
                IdentityResult::ReplacedKey(updated_identity, _fee_result) => {
                    self.identity = updated_identity;
                    let key_id = self.key.id();
                    if let Some(updated_key) = self.identity.identity.public_keys().get(&key_id) {
                        self.key = IdentityPublicKey::clone(updated_key);
                    }
                    self.replace_key_submitted = false;
                    self.replace_key_private_hex.clear();
                    self.success_message = Some(
                        "Master key has been replaced on Platform. The old key is now disabled."
                            .to_string(),
                    );
                }
                _ => {}
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Key Info", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            ScrollArea::vertical().show(ui, |ui| {
                let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
                ui.heading(RichText::new("Key Information").color(text_primary));
                ui.add_space(10.0);

                egui::Grid::new("key_info_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .striped(false)
                    .show(ui, |ui| {
                        // Key ID
                        ui.label(RichText::new("Key ID:").strong().color(text_primary));
                        ui.label(RichText::new(format!("{}", self.key.id())).color(text_primary));
                        ui.end_row();

                        // Purpose
                        ui.label(RichText::new("Purpose:").strong().color(text_primary));
                        ui.label(
                            RichText::new(format!("{:?}", self.key.purpose())).color(text_primary),
                        );
                        ui.end_row();

                        // Security Level
                        ui.label(
                            RichText::new("Security Level:")
                                .strong()
                                .color(text_primary),
                        );
                        ui.label(
                            RichText::new(format!("{:?}", self.key.security_level()))
                                .color(text_primary),
                        );
                        ui.end_row();

                        // Type
                        ui.label(RichText::new("Type:").strong().color(text_primary));
                        ui.label(
                            RichText::new(format!("{:?}", self.key.key_type())).color(text_primary),
                        );
                        ui.end_row();

                        // Read Only
                        ui.label(RichText::new("Read Only:").strong().color(text_primary));
                        ui.label(
                            RichText::new(format!("{}", self.key.read_only())).color(text_primary),
                        );
                        ui.end_row();

                        // Disabled
                        ui.label(
                            RichText::new("Active/Disabled:")
                                .strong()
                                .color(text_primary),
                        );
                        if !self.key.is_disabled() {
                            ui.label(RichText::new("Active").color(text_primary));
                        } else {
                            ui.label(RichText::new("Disabled").color(text_primary));
                        }
                        ui.end_row();

                        if let Some((_, Some(wallet_derivation_path))) =
                            self.private_key_data.as_ref()
                        {
                            // Disabled
                            ui.label(
                                RichText::new("In local Wallet")
                                    .strong()
                                    .color(text_primary),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "At derivation path {}",
                                    wallet_derivation_path.derivation_path
                                ))
                                .strong()
                                .color(text_primary),
                            );
                            ui.end_row();
                        }

                        // Contract Bounds
                        if let Some(contract_bounds) = self.key.contract_bounds() {
                            ui.label(
                                RichText::new("Contract Bounds:")
                                    .strong()
                                    .color(text_primary),
                            );
                            match contract_bounds {
                                ContractBounds::SingleContract { id } => {
                                    ui.label(
                                        RichText::new(format!("Contract ID: {}", id))
                                            .color(text_primary),
                                    );
                                }
                                ContractBounds::SingleContractDocumentType {
                                    id,
                                    document_type_name,
                                } => {
                                    ui.label(
                                        RichText::new(format!(
                                            "Contract ID: {}\nDocument Type: {}",
                                            id, document_type_name
                                        ))
                                        .color(text_primary),
                                    );
                                }
                            }
                            ui.end_row();
                        }

                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display the public key information
                ui.heading(RichText::new("Public Key Information").color(text_primary));
                ui.add_space(10.0);

                egui::Grid::new("public_key_info_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .striped(false)
                    .show(ui, |ui| {
                        match self.key.key_type() {
                            KeyType::ECDSA_SECP256K1 | KeyType::BLS12_381 => {
                                // Public Key Hex
                                ui.label(
                                    RichText::new("Public Key (Hex):")
                                        .strong()
                                        .color(text_primary),
                                );
                                ui.label(
                                    RichText::new(self.key.data().to_string(Encoding::Hex))
                                        .color(text_primary),
                                );
                                ui.end_row();

                                // Public Key Hex
                                ui.label(
                                    RichText::new("Public Key (Base64):")
                                        .strong()
                                        .color(text_primary),
                                );
                                ui.label(
                                    RichText::new(self.key.data().to_string(Encoding::Base64))
                                        .color(text_primary),
                                );
                                ui.end_row();
                            }
                            _ => {}
                        }

                        // Public Key Hash
                        ui.label(
                            RichText::new("Public Key Hash:")
                                .strong()
                                .color(text_primary),
                        );
                        match self.key.public_key_hash() {
                            Ok(hash) => {
                                let hash_hex = hex::encode(hash);
                                ui.label(RichText::new(hash_hex).color(text_primary));
                            }
                            Err(e) => {
                                ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
                            }
                        }

                        if self.key.key_type().is_core_address_key_type() {
                            // Public Key Hash
                            ui.label(RichText::new("Address:").strong().color(text_primary));
                            match self.key.public_key_hash() {
                                Ok(hash) => {
                                    let address = if self.key.key_type() == BIP13_SCRIPT_HASH {
                                        Address::new(
                                            self.app_context.network,
                                            Payload::ScriptHash(ScriptHash::from_byte_array(hash)),
                                        )
                                    } else {
                                        Address::new(
                                            self.app_context.network,
                                            Payload::PubkeyHash(PubkeyHash::from_byte_array(hash)),
                                        )
                                    };
                                    ui.label(
                                        RichText::new(address.to_string()).color(text_primary),
                                    );
                                }
                                Err(e) => {
                                    ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
                                }
                            }
                        }

                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display the private key if available
                if let Some((private_key, _)) = self.private_key_data.as_mut() {
                    ui.heading(RichText::new("Private Key").color(text_primary));
                    ui.add_space(10.0);

                    match private_key {
                        PrivateKeyData::Clear(clear) | PrivateKeyData::AlwaysClear(clear) => {
                            egui::Grid::new("private_key_grid")
                                .num_columns(2)
                                .spacing([10.0, 10.0])
                                .show(ui, |ui| {
                                    if let Ok(secret_key) = SecretKey::from_slice(clear) {
                                        let private_key =
                                            PrivateKey::new(secret_key, self.app_context.network);
                                        ui.label(
                                            RichText::new("Private Key (WIF):")
                                                .strong()
                                                .color(ui.visuals().text_color()),
                                        );
                                        ui.label(
                                            RichText::new(private_key.to_wif())
                                                .color(ui.visuals().text_color()),
                                        );
                                        ui.end_row();
                                    }

                                    ui.label(
                                        RichText::new("Private Key (Hex):")
                                            .strong()
                                            .color(ui.visuals().text_color()),
                                    );
                                    let private_key_hex = hex::encode(clear);
                                    ui.label(
                                        RichText::new(private_key_hex)
                                            .color(ui.visuals().text_color()),
                                    );
                                    ui.end_row();
                                });
                            ui.add_space(10.0);
                            if ui.button("Remove private key from DET").clicked() {
                                self.show_confirm_remove_private_key = true;
                            }
                            self.render_sign_input(ui);
                        }
                        PrivateKeyData::Encrypted(_) => {
                            ui.label(RichText::new("Key is encrypted").color(text_primary));
                            ui.add_space(10.0);

                            //todo decrypt key
                        }
                        PrivateKeyData::AtWalletDerivationPath(derivation_path) => {
                            if self.wallet_open
                                && self.view_private_key_even_if_encrypted_or_in_wallet
                                && self.selected_wallet.is_some()
                            {
                                if let Some(private_key) = self.decrypted_private_key {
                                    egui::Grid::new("private_key_grid_wallet")
                                        .num_columns(2)
                                        .spacing([10.0, 10.0])
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new("Private Key (WIF):")
                                                    .strong()
                                                    .color(ui.visuals().text_color()),
                                            );
                                            let private_key_wif = private_key.to_wif();
                                            ui.label(
                                                RichText::new(private_key_wif)
                                                    .color(ui.visuals().text_color()),
                                            );
                                            ui.end_row();

                                            ui.label(
                                                RichText::new("Private Key (Hex):")
                                                    .strong()
                                                    .color(ui.visuals().text_color()),
                                            );
                                            let private_key_hex =
                                                hex::encode(private_key.inner.secret_bytes());
                                            ui.label(
                                                RichText::new(private_key_hex)
                                                    .color(ui.visuals().text_color()),
                                            );
                                            ui.end_row();
                                        });
                                } else {
                                    let wallet =
                                        self.selected_wallet.as_ref().unwrap().read_or_recover();
                                    match wallet.private_key_at_derivation_path(
                                        &derivation_path.derivation_path,
                                        self.app_context.network,
                                    ) {
                                        Ok(private_key) => {
                                            egui::Grid::new("private_key_grid_wallet2")
                                                .num_columns(2)
                                                .spacing([10.0, 10.0])
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new("Private Key (WIF):")
                                                            .strong()
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    let private_key_wif = private_key.to_wif();
                                                    ui.label(
                                                        RichText::new(private_key_wif)
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    ui.end_row();

                                                    ui.label(
                                                        RichText::new("Private Key (Hex):")
                                                            .strong()
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    let private_key_hex = hex::encode(
                                                        private_key.inner.secret_bytes(),
                                                    );
                                                    ui.label(
                                                        RichText::new(private_key_hex)
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    ui.end_row();
                                                });

                                            self.decrypted_private_key = Some(private_key);
                                        }
                                        Err(e) => {
                                            ui.label(format!("Error: {}", e));
                                            return;
                                        }
                                    }
                                }
                                self.render_sign_input(ui);
                            } else if self.wallet_open {
                                ui.colored_label(Color32::DARK_RED, "Key is in encrypted wallet");
                                ui.add_space(10.0);

                                if ui.button("View Private Key").clicked() {
                                    self.view_private_key_even_if_encrypted_or_in_wallet = true;
                                    self.view_wallet_unlock = true;
                                }
                                if self.decrypted_private_key.is_none() {
                                    let wallet =
                                        self.selected_wallet.as_ref().unwrap().read_or_recover();
                                    match wallet.private_key_at_derivation_path(
                                        &derivation_path.derivation_path,
                                        self.app_context.network,
                                    ) {
                                        Ok(private_key) => {
                                            egui::Grid::new("private_key_grid_wallet2")
                                                .num_columns(2)
                                                .spacing([10.0, 10.0])
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new("Private Key (WIF):")
                                                            .strong()
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    let private_key_wif = private_key.to_wif();
                                                    ui.label(
                                                        RichText::new(private_key_wif)
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    ui.end_row();

                                                    ui.label(
                                                        RichText::new("Private Key (Hex):")
                                                            .strong()
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    let private_key_hex = hex::encode(
                                                        private_key.inner.secret_bytes(),
                                                    );
                                                    ui.label(
                                                        RichText::new(private_key_hex)
                                                            .color(ui.visuals().text_color()),
                                                    );
                                                    ui.end_row();
                                                });

                                            self.decrypted_private_key = Some(private_key);
                                        }
                                        Err(e) => {
                                            ui.label(format!("Error: {}", e));
                                            return;
                                        }
                                    }
                                }
                                self.render_sign_input(ui);
                            } else {
                                ui.colored_label(Color32::DARK_RED, "Key is in encrypted wallet");
                                ui.add_space(10.0);

                                if ui.button("View Private Key").clicked() {
                                    self.view_private_key_even_if_encrypted_or_in_wallet = true;
                                    self.view_wallet_unlock = true;
                                }

                                if ui.button("Sign Message").clicked() {
                                    self.view_wallet_unlock = true;
                                }
                            }
                        }
                    }
                } else {
                    ui.label(RichText::new("Enter Private Key:").color(text_primary));
                    ui.text_edit_singleline(&mut self.private_key_input);

                    if ui.button("Add Private Key").clicked() {
                        self.validate_and_store_private_key();
                    }

                    // Display error message if validation fails
                    if let Some(error_message) = self.error_message.clone() {
                        let error_color = DashColors::ERROR;
                        Frame::new()
                            .fill(error_color.gamma_multiply(0.1))
                            .inner_margin(Margin::symmetric(10, 8))
                            .corner_radius(5.0)
                            .stroke(egui::Stroke::new(1.0, error_color))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Error: {}", error_message))
                                            .color(error_color),
                                    );
                                    ui.add_space(10.0);
                                    if ui.small_button("Dismiss").clicked() {
                                        self.error_message = None;
                                    }
                                });
                            });
                    }
                }

                if self.view_wallet_unlock
                    && let Some(wallet) = &self.selected_wallet
                {
                    if let Err(e) = try_open_wallet_no_password(wallet) {
                        self.error_message = Some(e);
                    }
                    if wallet_needs_unlock(wallet) {
                        ui.add_space(10.0);
                        ui.colored_label(
                            DashColors::WARNING_ORANGE,
                            "Wallet is locked. Please unlock to continue.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                    } else {
                        self.wallet_open = true;
                    }
                }

                // Show the remove private key confirmation popup
                if self.show_confirm_remove_private_key {
                    self.render_remove_private_key_confirm(ui);
                }

                // Show success message
                if let Some(success_msg) = self.success_message.clone() {
                    ui.add_space(10.0);
                    let success_color = DashColors::SUCCESS;
                    Frame::new()
                        .fill(success_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, success_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&success_msg).color(success_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.success_message = None;
                                }
                            });
                        });
                }

                // Disable Key on Platform section
                // Only show for non-master, non-disabled keys that we can sign for
                if !self.key.is_disabled()
                    && self.key.security_level() != SecurityLevel::MASTER
                    && self.identity.can_sign_with_master_key().is_some()
                {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading(RichText::new("Key Management").color(text_primary));
                    ui.add_space(5.0);

                    if self.disable_key_submitted {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Disabling key on Platform...").color(text_primary),
                            );
                        });
                    } else if ui.button("Disable Key on Platform").clicked() {
                        self.show_confirm_disable_key = true;
                    }
                }

                // Show the disable key confirmation popup
                if self.show_confirm_disable_key {
                    inner_action |= self.render_disable_key_confirm(ui);
                }

                // Replace Master Key section
                // Only show for master keys that are not disabled and where we can sign
                if !self.key.is_disabled()
                    && self.key.security_level() == SecurityLevel::MASTER
                    && self.identity.can_sign_with_master_key().is_some()
                {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    ui.heading(RichText::new("Key Management").color(text_primary));
                    ui.add_space(5.0);

                    if self.replace_key_submitted {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("Replacing master key on Platform...")
                                    .color(text_primary),
                            );
                        });
                    } else if ui.button("Replace Master Key").clicked() {
                        self.generate_replace_key();
                        self.show_confirm_replace_key = true;
                    }
                }

                // Show the replace key confirmation popup
                if self.show_confirm_replace_key {
                    inner_action |= self.render_replace_key_confirm(ui);
                }

                ui.add_space(10.0);
            });

            inner_action
        });

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut popup = InfoPopup::new("Sign Message Info", &show_pop_up_info_text);
                    if popup.show(ui).inner {
                        self.show_pop_up_info = None;
                    }
                });
        }

        action
    }
}

impl KeyInfoScreen {
    pub fn new(
        identity: QualifiedIdentity,
        key: IdentityPublicKey,
        private_key_data: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let selected_wallet =
            if let Some((_, Some(wallet_derivation_path))) = private_key_data.as_ref() {
                let wallets = app_context.wallets.read_or_recover();
                wallets
                    .get(&wallet_derivation_path.wallet_seed_hash)
                    .cloned()
            } else {
                None
            };
        Self {
            identity,
            key,
            private_key_data,
            decrypted_private_key: None,
            app_context: app_context.clone(),
            private_key_input: String::new(),
            error_message: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            message_input: "".to_string(),
            signed_message: None,
            sign_error_message: None,
            view_wallet_unlock: false,
            wallet_open: false,
            view_private_key_even_if_encrypted_or_in_wallet: false,
            show_pop_up_info: None,
            show_confirm_remove_private_key: false,
            show_confirm_disable_key: false,
            disable_key_submitted: false,
            show_confirm_replace_key: false,
            replace_key_submitted: false,
            replace_key_type: KeyType::ECDSA_SECP256K1,
            replace_key_private_hex: String::new(),
            success_message: None,
        }
    }

    fn validate_and_store_private_key(&mut self) {
        // Convert the input string to bytes (hex decoding)
        let private_key_bytes = match hex::decode(&self.private_key_input) {
            Ok(private_key_bytes_vec) if private_key_bytes_vec.len() == 32 => {
                private_key_bytes_vec.try_into().unwrap()
            }
            Ok(_) => {
                self.error_message = Some("Private key not 32 bytes".to_string());
                return;
            }
            Err(_) => match PrivateKey::from_wif(&self.private_key_input) {
                Ok(key) => key.inner.secret_bytes(),
                Err(_) => {
                    self.error_message =
                        Some("Invalid hex string or WIF for private key.".to_string());
                    return;
                }
            },
        };

        let validation_result = self
            .key
            .validate_private_key_bytes(&private_key_bytes, self.app_context.network);
        if let Err(err) = validation_result {
            self.error_message = Some(format!("Issue verifying private key {}", err));
        } else if validation_result.unwrap() {
            // If valid, store the private key in the context and reset the input field
            self.private_key_data = Some((PrivateKeyData::Clear(private_key_bytes), None));
            self.identity.private_keys.insert_non_encrypted(
                (self.key.purpose().into(), self.key.id()),
                (self.key.clone().into(), private_key_bytes),
            );
            match self
                .app_context
                .update_local_qualified_identity(&self.identity)
            {
                Ok(_) => {
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("Issue saving: {}", e));
                }
            }
        } else {
            self.error_message = Some("Private key does not match the public key.".to_string());
        }
    }

    fn render_sign_input(&mut self, ui: &mut egui::Ui) {
        let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.heading(RichText::new("Sign").color(text_primary));

            // Create an info icon button
            let response = crate::ui::helpers::info_icon_button(ui, "Enter a message and click Sign to encrypt it with your private key. You can send the encrypted message to someone and they can decrypt it using your public key. This is useful for proving you own the private key.");

            // Check if the label was clicked
            if response.clicked() {
                self.show_pop_up_info = Some("Enter a message click Sign to encrypt it with your private key. You can can send the encrypted message to someone and they can decrypt it using your public key. This is useful for proving you own the private key.".to_string());
            }
        });
        ui.add_space(5.0);

        ui.label(RichText::new("Enter message to sign:").color(text_primary));
        ui.add_space(5.0);
        ui.add(
            egui::TextEdit::multiline(&mut self.message_input)
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );
        ui.add_space(5.0);

        if ui.button("Sign Message").clicked() {
            // Attempt to sign the message
            self.sign_message();
        }

        if let Some(error_message) = self.sign_error_message.clone() {
            let error_color = DashColors::ERROR;
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Error: {}", error_message)).color(error_color),
                        );
                        ui.add_space(10.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.sign_error_message = None;
                        }
                    });
                });
        }

        if let Some(signed_message) = &self.signed_message {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.label(RichText::new("Signed Message (Base64):").color(text_primary));
            ui.add_space(5.0);
            ui.add(
                egui::TextEdit::multiline(&mut signed_message.as_str().to_owned())
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
        }
    }

    fn sign_message(&mut self) {
        // Check that we have a private key
        if let Some((private_key_data, _)) = &self.private_key_data {
            let private_key_bytes = match (private_key_data, self.decrypted_private_key.as_ref()) {
                (PrivateKeyData::Clear(bytes), _) | (PrivateKeyData::AlwaysClear(bytes), _) => {
                    *bytes
                }
                (_, Some(private_key)) => private_key.inner.secret_bytes(),
                // Other cases may not have the private key directly
                _ => {
                    self.sign_error_message = Some("Private key is not available.".to_string());
                    return;
                }
            };

            // Use the key type to determine how to sign
            match self.key.key_type() {
                KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                    // Sign the message using ECDSA
                    let secp = Secp256k1::new();

                    let message_hash = signed_msg_hash(self.message_input.as_str());
                    let message = Message::from_digest(*message_hash.as_byte_array());

                    let secret_key = SecretKey::from_byte_array(&private_key_bytes).unwrap();

                    let signature = secp.sign_ecdsa(&message, &secret_key);

                    // Serialize the signature
                    let mut serialized_signature = signature.serialize_compact().to_vec();
                    serialized_signature.insert(0, 32);

                    // Encode to Base64
                    let signature_base64 = STANDARD.encode(serialized_signature);

                    self.signed_message = Some(signature_base64);
                    self.sign_error_message = None;
                }
                _ => {
                    self.sign_error_message = Some("Unsupported key type for signing.".to_string());
                }
            }
        } else {
            self.sign_error_message = Some("Private key is not available.".to_string());
        }
    }

    fn render_disable_key_confirm(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;
        let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
        egui::Window::new("Disable Key on Platform")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(
                    RichText::new(
                        "Are you sure you want to disable this key on Platform?\n\n\
                         This action is irreversible. The key will be permanently \
                         disabled and can no longer be used for signing transactions.",
                    )
                    .color(text_primary),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!(
                        "Key ID: {}\nPurpose: {:?}\nSecurity Level: {:?}",
                        self.key.id(),
                        self.key.purpose(),
                        self.key.security_level()
                    ))
                    .color(text_primary),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.show_confirm_disable_key = false;
                    }
                    ui.add_space(3.0);
                    if ui
                        .button(RichText::new("Disable Key").color(DashColors::ERROR))
                        .clicked()
                    {
                        self.show_confirm_disable_key = false;
                        self.disable_key_submitted = true;
                        action = AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::DisableKeys(self.identity.clone(), vec![self.key.id()]),
                        ));
                    }
                });
            });
        action
    }

    fn generate_replace_key(&mut self) {
        use bip39::rand::{SeedableRng, rngs::StdRng};
        let mut rng = StdRng::from_entropy();
        match self
            .replace_key_type
            .random_public_and_private_key_data(&mut rng, self.app_context.platform_version())
        {
            Ok((_, private_key_bytes)) => {
                self.replace_key_private_hex = hex::encode(private_key_bytes);
            }
            Err(_) => {
                self.error_message = Some("Failed to generate a random private key.".to_string());
                self.show_confirm_replace_key = false;
            }
        }
    }

    fn render_replace_key_confirm(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;
        let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
        egui::Window::new("Replace Master Key")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label(
                    RichText::new(
                        "This will generate a new master key and disable the current one \
                         in a single atomic transition.\n\n\
                         Make sure to save the new private key! You will need it to sign \
                         future identity updates.",
                    )
                    .color(text_primary),
                );
                ui.add_space(10.0);

                ui.label(
                    RichText::new(format!(
                        "Old Key ID: {}\nKey Type: {:?}",
                        self.key.id(),
                        self.key.key_type()
                    ))
                    .color(text_primary),
                );
                ui.add_space(10.0);

                // Key type selector for the new key
                ui.horizontal(|ui| {
                    ui.label(RichText::new("New Key Type:").color(text_primary));
                    let prev_type = self.replace_key_type;
                    egui::ComboBox::from_id_salt("replace_key_type_selector")
                        .selected_text(format!("{:?}", self.replace_key_type))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.replace_key_type,
                                KeyType::ECDSA_SECP256K1,
                                "ECDSA_SECP256K1",
                            );
                            ui.selectable_value(
                                &mut self.replace_key_type,
                                KeyType::BLS12_381,
                                "BLS12_381",
                            );
                            ui.selectable_value(
                                &mut self.replace_key_type,
                                KeyType::ECDSA_HASH160,
                                "ECDSA_HASH160",
                            );
                            ui.selectable_value(
                                &mut self.replace_key_type,
                                KeyType::EDDSA_25519_HASH160,
                                "EDDSA_25519_HASH160",
                            );
                        });
                    // Regenerate if key type changed
                    if prev_type != self.replace_key_type {
                        self.generate_replace_key();
                    }
                });
                ui.add_space(5.0);

                // Show the generated private key (read-only, copyable)
                ui.horizontal(|ui| {
                    ui.label(RichText::new("New Private Key (hex):").color(text_primary));
                });
                let mut key_display = self.replace_key_private_hex.clone();
                ui.add(
                    egui::TextEdit::singleline(&mut key_display)
                        .desired_width(ui.available_width()),
                );
                ui.add_space(5.0);
                if ui.small_button("Regenerate").clicked() {
                    self.generate_replace_key();
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.show_confirm_replace_key = false;
                        self.replace_key_private_hex.clear();
                    }
                    ui.add_space(3.0);
                    if ui
                        .button(RichText::new("Replace Master Key").color(DashColors::WARNING))
                        .clicked()
                    {
                        self.show_confirm_replace_key = false;
                        self.replace_key_submitted = true;
                        action = self.submit_replace_key();
                    }
                });
            });
        action
    }

    fn submit_replace_key(&mut self) -> AppAction {
        let private_key_bytes: [u8; 32] = match hex::decode(&self.replace_key_private_hex) {
            Ok(bytes) if bytes.len() == 32 => bytes.try_into().unwrap(),
            _ => {
                self.error_message = Some("Invalid private key for replacement.".to_string());
                self.replace_key_submitted = false;
                return AppAction::None;
            }
        };

        // Generate the public key from the private key
        let public_key_data = match self
            .replace_key_type
            .public_key_data_from_private_key_data(&private_key_bytes, self.app_context.network)
        {
            Ok(data) => data,
            Err(e) => {
                self.error_message = Some(format!("Failed to derive public key: {}", e));
                self.replace_key_submitted = false;
                return AppAction::None;
            }
        };

        let new_key = IdentityPublicKeyV0 {
            id: 0, // Will be set by backend task
            key_type: self.replace_key_type,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::MASTER,
            data: public_key_data.into(),
            read_only: false,
            disabled_at: None,
            contract_bounds: None,
        };

        let new_qualified_key = QualifiedIdentityPublicKey {
            identity_public_key: new_key.into(),
            in_wallet_at_derivation_path: None,
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::ReplaceKey(
            self.identity.clone(),
            self.key.id(),
            new_qualified_key,
            private_key_bytes,
        )))
    }

    fn render_remove_private_key_confirm(&mut self, ui: &mut egui::Ui) {
        let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
        egui::Window::new("Remove Private Key")
            .collapsible(false) // Prevent collapsing
            .resizable(false) // Prevent resizing
            .show(ui.ctx(), |ui| {
                ui.label(
                    RichText::new("Are you sure you want to remove the private key?")
                        .color(text_primary),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.show_confirm_remove_private_key = false;
                    }
                    ui.add_space(3.0);
                    if ui.button("Remove").clicked() {
                        self.private_key_data = None;
                        self.identity
                            .private_keys
                            .private_keys
                            .remove(&(self.key.purpose().into(), self.key.id()));
                        match self
                            .app_context
                            .update_local_qualified_identity(&self.identity)
                        {
                            Ok(_) => {
                                self.error_message = None;
                            }
                            Err(e) => {
                                self.error_message = Some(format!("Issue saving: {}", e));
                            }
                        }
                        self.show_confirm_remove_private_key = false;
                    }
                });
            });
    }
}
