use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::funding_common::{WalletFundedScreenStep, generate_qr_code_image};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::dashcore::{Address, OutPoint, TxOut};
use dash_sdk::dpp::balances::credits::Credits;
use eframe::egui::{self, Context, Ui};
use egui::{RichText, Vec2};
use std::sync::{Arc, RwLock};

pub struct CreateAssetLockScreen {
    pub wallet: Arc<RwLock<Wallet>>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    pub app_context: Arc<AppContext>,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,

    // Asset lock creation fields
    step: Arc<RwLock<WalletFundedScreenStep>>,
    amount_input: String,
    amount_credits: Option<Credits>,
    funding_address: Option<Address>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    core_has_funding_address: Option<bool>,
    is_creating: bool,
    asset_lock_tx_id: Option<String>,
}

impl CreateAssetLockScreen {
    pub fn new(wallet: Arc<RwLock<Wallet>>, app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = Some(wallet.clone());
        Self {
            wallet,
            selected_wallet,
            app_context: app_context.clone(),
            message: None,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
            step: Arc::new(RwLock::new(WalletFundedScreenStep::WaitingOnFunds)),
            amount_input: "0.5".to_string(), // Default to 0.5 DASH
            amount_credits: Some(50000000),  // 0.5 DASH in credits
            funding_address: None,
            funding_utxo: None,
            core_has_funding_address: None,
            is_creating: false,
            asset_lock_tx_id: None,
        }
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Amount (DASH):").color(DashColors::text_primary(dark_mode)));

            let response = ui.text_edit_singleline(&mut self.amount_input);

            if response.changed() {
                // Parse the input as DASH and convert to credits
                if let Ok(dash_amount) = self.amount_input.parse::<f64>() {
                    if dash_amount >= 0.0 {
                        let credits = (dash_amount * 100_000_000.0) as u64;
                        self.amount_credits = Some(credits);
                    } else {
                        self.amount_credits = None;
                    }
                } else {
                    self.amount_credits = None;
                }
            }
        });

        ui.add_space(5.0);

        // Show amount in credits if valid
        if let Some(credits) = self.amount_credits {
            ui.label(
                RichText::new(format!("= {} credits", credits))
                    .size(12.0)
                    .color(DashColors::text_secondary(dark_mode)),
            );
        }
    }

    fn generate_funding_address(&mut self) -> Result<(), String> {
        let mut wallet = self.wallet.write().unwrap();

        // Generate a new asset lock funding address
        let receive_address =
            wallet.receive_address(self.app_context.network, false, Some(&self.app_context))?;

        // Import address to core if needed
        if let Some(has_address) = self.core_has_funding_address {
            if !has_address {
                self.app_context
                    .core_client
                    .read()
                    .expect("Core client lock was poisoned")
                    .import_address(
                        &receive_address,
                        Some("Managed by Dash Evo Tool - Asset Lock"),
                        Some(false),
                    )
                    .map_err(|e| e.to_string())?;
            }
            self.funding_address = Some(receive_address);
        } else {
            let info = self
                .app_context
                .core_client
                .read()
                .expect("Core client lock was poisoned")
                .get_address_info(&receive_address)
                .map_err(|e| e.to_string())?;

            if !(info.is_watchonly || info.is_mine) {
                self.app_context
                    .core_client
                    .read()
                    .expect("Core client lock was poisoned")
                    .import_address(
                        &receive_address,
                        Some("Managed by Dash Evo Tool - Asset Lock"),
                        Some(false),
                    )
                    .map_err(|e| e.to_string())?;
            }
            self.funding_address = Some(receive_address);
            self.core_has_funding_address = Some(true);
        }

        Ok(())
    }

    fn render_qr_code(&mut self, ui: &mut egui::Ui) -> Result<(), String> {
        if self.funding_address.is_none() {
            self.generate_funding_address()?
        }

        let address = self.funding_address.as_ref().unwrap();
        let amount = self.amount_input.parse::<f64>().unwrap_or(0.5);
        let dash_uri = format!("dash:{}?amount={:.4}", address, amount);

        // Generate the QR code image
        if let Ok(qr_image) = generate_qr_code_image(&dash_uri) {
            let texture = ui
                .ctx()
                .load_texture("qr_code", qr_image, egui::TextureOptions::LINEAR);
            ui.image((texture.id(), Vec2::new(200.0, 200.0)));
        } else {
            ui.label("Failed to generate QR code.");
        }

        ui.add_space(10.0);
        ui.label(&dash_uri);
        ui.add_space(5.0);

        if ui.button("Copy Address").clicked() {
            ui.ctx().copy_text(dash_uri.clone());
            self.display_message("Address copied to clipboard", MessageType::Success);
        }

        Ok(())
    }

    fn check_message_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            if elapsed.num_seconds() >= 10 {
                self.message = None;
            }
        }
    }

    fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading(RichText::new("🎉").size(48.0));
            ui.heading(
                RichText::new("Success!")
                    .size(32.0)
                    .color(DashColors::success_color(dark_mode)),
            );

            ui.add_space(20.0);

            ui.label(
                RichText::new("Asset lock created successfully!")
                    .size(18.0)
                    .color(DashColors::text_primary(dark_mode)),
            );

            ui.add_space(10.0);

            if let Some(tx_id) = &self.asset_lock_tx_id {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Transaction ID:")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(RichText::new(tx_id).font(egui::FontId::monospace(12.0)));
                    if ui.button("📋").on_hover_text("Copy to clipboard").clicked() {
                        ui.ctx().copy_text(tx_id.clone());
                    }
                });
            }

            ui.add_space(30.0);

            // Display the "Back to Wallets" button
            if ui
                .button(RichText::new("Back to Wallets").size(16.0))
                .clicked()
            {
                action = AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for CreateAssetLockScreen {
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

impl ScreenLike for CreateAssetLockScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();

        let wallet_name = self
            .wallet
            .read()
            .ok()
            .and_then(|w| w.alias.clone())
            .unwrap_or_else(|| "Unknown Wallet".to_string());

        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                (
                    "Wallets",
                    AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenWalletsBalances,
                    ),
                ),
                (
                    &format!("{} / Create Asset Lock", wallet_name),
                    AppAction::None,
                ),
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
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.heading(
                        RichText::new("Create Asset Lock")
                            .color(DashColors::text_primary(dark_mode))
                            .size(24.0)
                    );
                    ui.add_space(10.0);

                    ui.label(
                        RichText::new("Follow these steps to create an asset lock")
                            .color(DashColors::text_secondary(dark_mode))
                    );

                    ui.add_space(20.0);

                    // Wallet unlock section
                    let (needs_unlock, unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if !needs_unlock || unlocked {
                        let step = *self.step.read().unwrap();

                        // Step 1: Amount selection
                        ui.heading("1. Select how much you would like to transfer?");
                        ui.add_space(10.0);

                        self.render_amount_input(ui);
                        ui.add_space(20.0);

                        // Step 2: QR Code and address
                        let amount_valid = self.amount_input.parse::<f64>().map(|a| a > 0.0).unwrap_or(false);
                        if amount_valid {
                            let layout_action = ui.with_layout(
                                egui::Layout::top_down(egui::Align::Min).with_cross_align(egui::Align::Center),
                                |ui| {
                                    if let Err(e) = self.render_qr_code(ui) {
                                        self.error_message = Some(e);
                                    }

                                    ui.add_space(20.0);

                                    if let Some(error_message) = self.error_message.as_ref() {
                                        ui.colored_label(egui::Color32::DARK_RED, error_message);
                                        ui.add_space(20.0);
                                    }

                                    match step {
                                        WalletFundedScreenStep::WaitingOnFunds => {
                                            ui.heading("=> Waiting for funds. <=");
                                            AppAction::None
                                        }
                                        WalletFundedScreenStep::FundsReceived => {
                                            ui.heading("Funds received! Creating asset lock...");

                                            // Trigger asset lock creation
                                            if self.is_creating {
                                                self.is_creating = false;
                                                if let Some(credits) = self.amount_credits {
                                                    AppAction::BackendTask(BackendTask::CoreTask(
                                                        CoreTask::CreateAssetLock(self.wallet.clone(), credits)
                                                    ))
                                                } else {
                                                    AppAction::None
                                                }
                                            } else {
                                                AppAction::None
                                            }
                                        }
                                        WalletFundedScreenStep::WaitingForAssetLock => {
                                            ui.heading("=> Waiting for Core Chain to produce proof of asset lock. <=");
                                            AppAction::None
                                        }
                                        WalletFundedScreenStep::Success => {
                                            // Success screen will be shown below
                                            AppAction::None
                                        }
                                        _ => AppAction::None
                                    }
                                }
                            );

                            inner_action |= layout_action.inner;
                        }

                        // Show success screen
                        if *self.step.read().unwrap() == WalletFundedScreenStep::Success {
                            inner_action |= self.show_success(ui);
                        }
                    }
                });

            // Display messages
            if let Some((message, message_type, timestamp)) = &self.message {
                let message_color = match message_type {
                    MessageType::Error => egui::Color32::DARK_RED,
                    MessageType::Info => DashColors::text_primary(dark_mode),
                    MessageType::Success => egui::Color32::DARK_GREEN,
                };

                ui.add_space(25.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    let now = Utc::now();
                    let elapsed = now.signed_duration_since(*timestamp);
                    let remaining = (10 - elapsed.num_seconds()).max(0);

                    let full_msg = format!("{} ({}s)", message, remaining);
                    ui.label(egui::RichText::new(full_msg).color(message_color));
                });
                ui.add_space(2.0);
            }

            inner_action
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn refresh_on_arrival(&mut self) {
        self.is_creating = false;
    }

    fn refresh(&mut self) {}

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        let current_step = *self.step.read().unwrap();

        match current_step {
            WalletFundedScreenStep::WaitingOnFunds => {
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(_, outpoints_with_addresses),
                ) = result
                {
                    for utxo in outpoints_with_addresses {
                        let (_, _, address) = &utxo;
                        if let Some(funding_address) = &self.funding_address {
                            if funding_address == address {
                                let mut step = self.step.write().unwrap();
                                *step = WalletFundedScreenStep::FundsReceived;
                                self.funding_utxo = Some(utxo);
                                drop(step); // Release the lock before creating new action

                                // Refresh wallet to create the asset lock
                                self.is_creating = true;
                                return;
                            }
                        }
                    }
                }
            }
            WalletFundedScreenStep::FundsReceived => {
                // Asset lock creation was triggered
                match &result {
                    BackendTaskSuccessResult::Message(msg) => {
                        if msg.contains("Asset lock transaction broadcast successfully") {
                            // Extract TX ID from message
                            if let Some(tx_id_start) = msg.find("TX ID: ") {
                                let tx_id = msg[tx_id_start + 7..].trim().to_string();
                                self.asset_lock_tx_id = Some(tx_id);
                            }

                            let mut step = self.step.write().unwrap();
                            *step = WalletFundedScreenStep::Success;
                            drop(step);
                            self.display_message(
                                "Asset lock created successfully!",
                                MessageType::Success,
                            );
                        }
                    }
                    BackendTaskSuccessResult::CoreItem(
                        CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                    ) => {
                        // This is the asset lock transaction from ZMQ
                        if tx.special_transaction_payload.is_some() {
                            self.asset_lock_tx_id = Some(tx.txid().to_string());
                            let mut step = self.step.write().unwrap();
                            *step = WalletFundedScreenStep::Success;
                            drop(step);
                            self.display_message(
                                "Asset lock created successfully!",
                                MessageType::Success,
                            );
                        }
                    }
                    _ => {}
                }
            }
            WalletFundedScreenStep::WaitingForAssetLock => {
                // Check if we received an asset lock transaction
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                ) = result
                {
                    if tx.special_transaction_payload.is_some() {
                        let mut step = self.step.write().unwrap();
                        *step = WalletFundedScreenStep::Success;
                        drop(step);
                        self.display_message(
                            "Asset lock created successfully!",
                            MessageType::Success,
                        );
                    }
                }
            }
            _ => {}
        }

        self.is_creating = false;
    }
}
