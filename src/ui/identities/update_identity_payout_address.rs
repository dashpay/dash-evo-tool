use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use eframe::egui::Context;
use std::sync::{Arc, RwLock};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey};
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use egui::{ComboBox, TextBuffer, Ui};
use tracing_subscriber::fmt::format;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::model::wallet::Wallet;
use crate::ui::identities::add_existing_identity_screen::AddIdentityStatus;

pub struct UpdateIdentityPayoutScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    payout_address_private_key_input: String,
    error_message: Option<String>,
    //selected_address: String,
    selected_address: Option<Address>,
}

impl UpdateIdentityPayoutScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = None;
        Self {
            app_context: app_context.clone(),
            identity,
            selected_wallet,
            //selected_address: String::default(),
            selected_address: None,
            payout_address_private_key_input: String::new(),
            error_message: None,
        }
    }

    fn verify_key_input(
        untrimmed_private_key: String,
        type_key: &str,
    ) -> Result<Option<[u8; 32]>, String> {
        let private_key = untrimmed_private_key.trim().to_string();
        match private_key.len() {
            64 => {
                // hex
                match hex::decode(private_key.as_str()) {
                    Ok(decoded) => Ok(Some(decoded.try_into().unwrap())),
                    Err(_) => Err(format!(
                        "{} key is the size of a hex key but isn't hex",
                        type_key
                    )),
                }
            }
            51 | 52 => {
                // wif
                match PrivateKey::from_wif(private_key.as_str()) {
                    Ok(key) => Ok(Some(key.inner.secret_bytes())),
                    Err(_) => Err(format!(
                        "{} key is the length of a WIF key but is invalid",
                        type_key
                    )),
                }
            }
            0 => Ok(None),
            _ => Err(format!("{} key is of incorrect size", type_key)),
        }
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
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
                    .unwrap_or_else(|| "Select".to_string());

                // Display the ComboBox for wallet selection
                ComboBox::from_label("")
                    .selected_text(selected_wallet_alias.clone())
                    .show_ui(ui, |ui| {
                        for (idx, wallet) in wallets.values().enumerate() {
                            let wallet_alias = wallet_aliases[idx].clone();

                            let is_selected = self
                                .selected_wallet
                                .as_ref()
                                .map_or(false, |selected| Arc::ptr_eq(selected, wallet));

                            if ui
                                .selectable_label(is_selected, wallet_alias.clone())
                                .clicked()
                            {
                                // Update the selected wallet
                                self.selected_wallet = Some(wallet.clone());
                            }
                        }
                    });

                ui.add_space(20.0);
            } else {
                ui.label("No wallets available.");
            }
        });
    }

    fn render_selected_wallet_addresses(&mut self, ctx: &Context, ui: &mut Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            // Acquire a read lock
            let wallet = selected_wallet.read().unwrap();
            ui.label("Select an Address:");
            ComboBox::from_label("")
                .selected_text(
                    self.selected_address
                        .as_ref() // Get a reference to the Option<Address>
                        .map(|address| address.to_string()) // Convert Address to String
                        .unwrap_or_else(|| "".to_string()), // Use default "" if None
                )
                .show_ui(ui, |ui| {
                    for (_, address_info) in &wallet.watched_addresses {
                        if ui.selectable_value(&mut self.selected_address, Some(address_info.clone().address), address_info.clone().address.to_string()).clicked() {
                        }
                    }
                });
            if let Some(selected_address) = &self.selected_address {
                ui.label(format!("Selected Address: {} with ", selected_address.to_string()));
                if let Some(value) = wallet.address_balances.get(&selected_address) {
                    ui.label(format!("Balance {} DASH", value));
                } else {
                    ui.label("Balance NOT FOUND DASH".to_string());
                }
            }
        }
    }
}

impl ScreenLike for UpdateIdentityPayoutScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {

    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Update Payout Address", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |mut ui| {
            if (self.identity.identity_type == IdentityType::User) {
                ui.heading("Updating Payout Address for User identities is not allowed.".to_string());
            }
            else {
                ui.heading("Update Payout Address".to_string());
            }

            let loaded_wallet = self.app_context.has_wallet.load(Ordering::Relaxed);
            if ( loaded_wallet) {
                //println!("Loaded wallet");
                self.render_wallet_selection(&mut ui);

                if self.selected_wallet.is_some() {
                    self.render_selected_wallet_addresses(ctx, &mut ui);
                }
            }
            else {
                //print!("No loaded wallet");
            }
/*
            if ui.button("Update Payout Address").clicked() {
                match Self::verify_key_input(payout_address_private_key_input.clone(), "test".as_str()) {
                    Ok(value) => {


                    }
                    Err(error) => {
                        eprintln!("Error: {}", error);
                    }
                }
            }

 */
        });
        
        action
    }
}
