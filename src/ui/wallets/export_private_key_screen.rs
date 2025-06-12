use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::{Address, PrivateKey};
use dash_sdk::dpp::dashcore::bip32::DerivationPath;
use eframe::egui::{self, Context, RichText, TextEdit};
use egui::Color32;
use std::sync::{Arc, RwLock};

pub struct ExportPrivateKeyScreen {
    pub address: Address,
    pub derivation_path: DerivationPath,
    pub wallet: Arc<RwLock<Wallet>>,
    pub app_context: Arc<AppContext>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
    private_key: Option<PrivateKey>,
    show_private_key: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
}

impl ExportPrivateKeyScreen {
    pub fn new(
        address: Address,
        derivation_path: DerivationPath,
        wallet: Arc<RwLock<Wallet>>,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            address,
            derivation_path,
            wallet: wallet.clone(),
            app_context: app_context.clone(),
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
            private_key: None,
            show_private_key: false,
            selected_wallet: Some(wallet),
        }
    }

    fn export_private_key(&mut self) {
        match self.wallet.write() {
            Ok(wallet) => {
                match wallet.private_key_at_derivation_path(&self.derivation_path) {
                    Ok(private_key) => {
                        self.private_key = Some(private_key);
                        self.show_private_key = true;
                        self.error_message = None;
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to export private key: {}", e));
                    }
                }
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to lock wallet: {}", e));
            }
        }
    }
}

impl ScreenLike for ExportPrivateKeyScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Wallets", AppAction::GoToMainScreen),
                ("Export Private Key", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            ui.heading(RichText::new("Export Private Key").color(Color32::BLACK));
            ui.add_space(20.0);

            // Show address info
            ui.horizontal(|ui| {
                ui.label(RichText::new("Address:").strong().color(Color32::BLACK));
                ui.label(RichText::new(&self.address.to_string()).color(Color32::BLACK));
            });
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("Derivation Path:").strong().color(Color32::BLACK));
                ui.label(RichText::new(&self.derivation_path.to_string()).color(Color32::BLACK));
            });
            ui.add_space(20.0);

            // Wallet unlock section
            let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);
            
            if needed_unlock && !just_unlocked {
                return inner_action;
            }

            if just_unlocked && self.private_key.is_none() {
                self.export_private_key();
            }

            // Show export button if wallet is unlocked but key not yet exported
            if !needed_unlock && self.private_key.is_none() {
                if ui.button("Export Private Key").clicked() {
                    self.export_private_key();
                }
            }

            // Show private key if exported
            if let Some(private_key) = &self.private_key {
                ui.add_space(20.0);
                ui.separator();
                ui.add_space(20.0);

                if self.show_private_key {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Private Key (WIF):").strong().color(Color32::BLACK));
                        if ui.button("Hide").clicked() {
                            self.show_private_key = false;
                        }
                    });
                    ui.add_space(10.0);

                    let wif = private_key.to_wif();
                    ui.add(
                        TextEdit::singleline(&mut wif.as_str().to_owned())
                            .desired_width(400.0)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button("Copy to Clipboard").clicked() {
                            ui.ctx().copy_text(wif);
                        }
                    });

                    ui.add_space(20.0);
                    ui.colored_label(
                        Color32::from_rgb(200, 0, 0),
                        "⚠️ Warning: Keep this private key secure! Anyone with access to it can spend your funds.",
                    );
                } else {
                    if ui.button("Show Private Key").clicked() {
                        self.show_private_key = true;
                    }
                }
            }

            // Show error if any
            if let Some(error) = &self.error_message {
                ui.add_space(10.0);
                ui.colored_label(Color32::from_rgb(200, 0, 0), error);
            }

            ui.add_space(20.0);
            if ui.button("Back").clicked() {
                inner_action = AppAction::PopScreen;
            }

            inner_action
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Error => {
                self.error_message = Some(message.to_string());
            }
            _ => {}
        }
    }
}

impl ScreenWithWalletUnlock for ExportPrivateKeyScreen {
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