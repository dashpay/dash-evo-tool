use crate::app::AppAction;
use crate::backend_task::core::CoreTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::Context;
use egui::{ComboBox, Ui};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use dash_sdk::dpp::identity::TimestampMillis;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;

pub enum UpdateIdentityPayoutStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct UpdateIdentityPayoutScreen {
    pub app_context: Arc<AppContext>,
    pub identity: QualifiedIdentity,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    selected_payout_address: Option<Address>,
    selected_funding_address: Option<Address>,
    update_payout_status: UpdateIdentityPayoutStatus,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl UpdateIdentityPayoutScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = None;
        Self {
            app_context: app_context.clone(),
            identity,
            selected_wallet,
            selected_payout_address: None,
            selected_funding_address: None,
            error_message: None,
            update_payout_status: UpdateIdentityPayoutStatus::NotStarted,
            wallet_password: String::new(),
            show_password: false,
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

    fn render_selected_wallet_payout_addresses(&mut self, ui: &mut Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            // Acquire a read lock
            let wallet = selected_wallet.read().unwrap();
            ui.add_space(20.0);
            ui.heading("Select a Payout Address:");
            ui.add_space(5.0);
            ui.push_id("payout_combo_id", |ui| {
                ComboBox::from_label("")
                    .selected_text(
                        self.selected_payout_address
                            .as_ref() // Get a reference to the Option<Address>
                            .map(|address| address.to_string()) // Convert Address to String
                            .unwrap_or_else(|| "".to_string()), // Use default "" if None
                    )
                    .show_ui(ui, |ui| {
                        for (address, _) in &wallet.known_addresses {
                            if ui
                                .selectable_value(
                                    &mut self.selected_payout_address,
                                    Some(address.clone()),
                                    address.to_string(),
                                )
                                .clicked()
                            {}
                        }
                    });
            });
            ui.add_space(20.0);
            if let Some(selected_address) = &self.selected_payout_address {
                if let Some(value) = wallet.address_balances.get(&selected_address) {
                    ui.label(format!("Selected Address has a balance of {} DASH", value));
                } else {
                    // TODO: Why sometimes balance is not found?
                    //ui.label("Balance NOT FOUND DASH".to_string());
                }
            }
        }
    }

    fn render_selected_wallet_funding_addresses(&mut self, ui: &mut Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            // Acquire a read lock
            let wallet = selected_wallet.read().unwrap();
            ui.add_space(20.0);
            ui.heading("Select a Funding Address:");
            ui.add_space(5.0);
            ui.push_id("funding_combo_id", |ui| {
                ComboBox::from_label("")
                    .selected_text(
                        self.selected_funding_address
                            .as_ref() // Get a reference to the Option<Address>
                            .map(|address| address.to_string()) // Convert Address to String
                            .unwrap_or_else(|| "".to_string()), // Use default "" if None
                    )
                    .show_ui(ui, |ui| {
                        for (address, _) in &wallet.known_addresses {
                            if ui
                                .selectable_value(
                                    &mut self.selected_funding_address,
                                    Some(address.clone()),
                                    address.to_string(),
                                )
                                .clicked()
                            {}
                        }
                    });
            });
            ui.add_space(20.0);
            if let Some(selected_address) = &self.selected_funding_address {
                if let Some(value) = wallet.address_balances.get(&selected_address) {
                    ui.label(format!("Selected Address has a balance of {} DASH", value));
                } else {
                    // TODO: Why sometimes balance is not found?
                    //ui.label("Balance NOT FOUND DASH".to_string());
                }
            }
        }
    }
}

impl ScreenLike for UpdateIdentityPayoutScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.update_payout_status = UpdateIdentityPayoutStatus::Complete;
            }
            MessageType::Info => {}
            MessageType::Error => {
                self.update_payout_status = UpdateIdentityPayoutStatus::ErrorMessage(message.to_string());
            }
        }
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
            if self.identity.identity_type == IdentityType::User {
                ui.heading(
                    "Updating Payout Address for User identities is not allowed.".to_string(),
                );
                return;
            }
            if !self.app_context.has_wallet.load(Ordering::Relaxed) {
                ui.heading("Load a Wallet in order to continue.".to_string());
                return;
            }
            ui.heading("Update Payout Address".to_string());
            ui.add_space(20.0);

            ui.heading("Load Address from wallet".to_string());
            self.render_wallet_selection(&mut ui);

            let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

            if needed_unlock && !just_unlocked {
                return;
            }

            if self.selected_wallet.is_some() {
                self.render_selected_wallet_payout_addresses(&mut ui);
                if self.selected_payout_address.is_some() {
                    self.render_selected_wallet_funding_addresses(&mut ui);
                    if self.selected_funding_address.is_some() {
                        ui.add_space(20.0);
                        ui.colored_label(
                            egui::Color32::ORANGE,
                            "The owner key of the Masternode/Evonode must be known to your wallet.",
                        );
                        ui.add_space(20.0);
                        if ui.button("Update Payout Address").clicked() {
                            // Set the status to waiting and capture the current time
                            let now = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .expect("Time went backwards")
                                .as_secs();
                            self.update_payout_status = UpdateIdentityPayoutStatus::WaitingForResult(now);
                            action |= AppAction::BackendTask(BackendTask::CoreTask(
                                CoreTask::ProRegUpdateTx(
                                    self.identity.identity.id().to_string(Encoding::Hex),
                                    self.selected_payout_address.clone().unwrap(),
                                    self.selected_funding_address.clone().unwrap(),
                                ),
                            ));
                        }

                        // Handle registration status messages
                        match &self.update_payout_status {
                            UpdateIdentityPayoutStatus::NotStarted => {
                                // Do nothing
                            }
                            UpdateIdentityPayoutStatus::WaitingForResult(start_time) => {
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .expect("Time went backwards")
                                    .as_secs();
                                let elapsed_seconds = now - start_time;

                                let display_time = if elapsed_seconds < 60 {
                                    format!(
                                        "{} second{}",
                                        elapsed_seconds,
                                        if elapsed_seconds == 1 { "" } else { "s" }
                                    )
                                } else {
                                    let minutes = elapsed_seconds / 60;
                                    let seconds = elapsed_seconds % 60;
                                    format!(
                                        "{} minute{} and {} second{}",
                                        minutes,
                                        if minutes == 1 { "" } else { "s" },
                                        seconds,
                                        if seconds == 1 { "" } else { "s" }
                                    )
                                };

                                ui.add_space(20.0);
                                ui.label(format!(
                                    "Waiting... Time taken so far: {}",
                                    display_time
                                ));
                            }
                            UpdateIdentityPayoutStatus::ErrorMessage(msg) => {
                                ui.add_space(20.0);
                                ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                            }
                            UpdateIdentityPayoutStatus::Complete => {
                                action = AppAction::PopScreenAndRefresh;
                            }
                        }
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for UpdateIdentityPayoutScreen {
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
