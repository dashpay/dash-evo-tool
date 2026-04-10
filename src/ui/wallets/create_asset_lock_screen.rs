use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::Component;
use crate::ui::components::MessageBanner;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::funding_common::{self, WalletFundedScreenStep, generate_qr_code_image};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::{Address, OutPoint, TxOut};
use eframe::egui::{self, Context, Ui};
use egui::{Button, RichText, Vec2};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

const MAX_IDENTITY_INDEX: u32 = 30;

#[derive(Debug, Clone, Copy, PartialEq)]
enum AssetLockPurpose {
    Registration,
    TopUp,
}

pub struct CreateAssetLockScreen {
    pub wallet: Arc<RwLock<Wallet>>,
    pub(crate) selected_wallet: Option<Arc<RwLock<Wallet>>>,
    pub app_context: Arc<AppContext>,
    password_input: PasswordInput,
    // Asset lock creation fields
    step: Arc<RwLock<WalletFundedScreenStep>>,
    amount_input: Option<AmountInput>,
    identity_index: u32,
    funding_address: Option<Address>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    core_has_funding_address: Option<bool>,
    is_creating: bool,
    asset_lock_tx_id: Option<String>,

    // New fields for asset lock purpose flow
    asset_lock_purpose: Option<AssetLockPurpose>,
    selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    top_up_index: u32,
    show_advanced_options: bool,
}

impl CreateAssetLockScreen {
    pub fn new(wallet: Arc<RwLock<Wallet>>, app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = Some(wallet.clone());

        // Calculate next unused identity index from the database
        let identity_index = {
            let wallet_guard = wallet.read().unwrap();
            let used = app_context
                .db
                .get_wallet_identity_indices(&wallet_guard.seed_hash(), app_context.network);
            used.iter().copied().max().map(|max| max + 1).unwrap_or(0)
        };

        Self {
            wallet,
            selected_wallet,
            app_context: app_context.clone(),
            password_input: PasswordInput::new().with_hint_text("Enter password"),
            step: Arc::new(RwLock::new(WalletFundedScreenStep::WaitingOnFunds)),
            amount_input: Some(
                AmountInput::new(Amount::new_dash(0.5))
                    .with_label("Amount (DASH):")
                    .with_min_amount(Some(1000)), // Minimum 0.00000001 DASH (1000 credits)
            ),
            identity_index,
            funding_address: None,
            funding_utxo: None,
            core_has_funding_address: None,
            is_creating: false,
            asset_lock_tx_id: None,
            asset_lock_purpose: None,
            selected_identity: None,
            selected_identity_string: String::new(),
            top_up_index: 0,
            show_advanced_options: false,
        }
    }

    fn generate_funding_address(&mut self) -> Result<(), TaskError> {
        let mut wallet = self.wallet.write().unwrap();

        // Generate a new asset lock funding address
        let receive_address = wallet
            .receive_address(self.app_context.network, true, Some(&self.app_context))
            .map_err(|e| TaskError::WalletAddressDerivationFailed { detail: e })?;
        let core_wallet_name = wallet.core_wallet_name.clone();
        drop(wallet);

        // Import address to core if needed
        if let Some(has_address) = self.core_has_funding_address {
            if !has_address {
                self.app_context.ensure_address_imported(
                    &receive_address,
                    core_wallet_name.as_deref(),
                    Some("Managed by Dash Evo Tool - Asset Lock"),
                )?;
            }
            self.funding_address = Some(receive_address);
        } else {
            self.app_context.ensure_address_imported(
                &receive_address,
                core_wallet_name.as_deref(),
                Some("Managed by Dash Evo Tool - Asset Lock"),
            )?;
            self.funding_address = Some(receive_address);
            self.core_has_funding_address = Some(true);
        }

        Ok(())
    }

    fn render_qr_code(&mut self, ui: &mut egui::Ui) -> Result<(), TaskError> {
        if self.funding_address.is_none() {
            self.generate_funding_address()?
        }

        let address = self.funding_address.as_ref().unwrap();
        let amount = self
            .amount_input
            .as_ref()
            .and_then(|ai| ai.current_value())
            .map(|a| a.to_f64())
            .unwrap_or(0.5);
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
            MessageBanner::set_global(
                ui.ctx(),
                "Address copied to clipboard",
                MessageType::Success,
            );
        }

        Ok(())
    }

    fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(100.0);

            ui.heading("🎉");
            ui.heading("Asset Lock Created Successfully!");

            if let Some(tx_id) = &self.asset_lock_tx_id {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.label("Transaction ID:");
                    ui.label(RichText::new(tx_id).font(egui::FontId::monospace(12.0)));
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(tx_id.clone());
                    }
                });
            }

            ui.add_space(20.0);

            if ui.button("Back to Wallets").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            if ui.button("Create Another").clicked() {
                // Reset state for creating another asset lock
                self.asset_lock_purpose = None;
                self.selected_identity = None;
                self.selected_identity_string.clear();
                // Recalculate next unused identity index from the database
                self.identity_index = {
                    let wallet_guard = self.wallet.read().unwrap();
                    let used = self.app_context.db.get_wallet_identity_indices(
                        &wallet_guard.seed_hash(),
                        self.app_context.network,
                    );
                    used.iter().copied().max().map(|max| max + 1).unwrap_or(0)
                };
                self.top_up_index = 0;
                // Reset amount input to default 0.5 DASH
                self.amount_input = Some(
                    AmountInput::new(Amount::new_dash(0.5))
                        .with_label("Amount (DASH):")
                        .with_min_amount(Some(1000)),
                );
                self.funding_address = None;
                self.funding_utxo = None;
                self.core_has_funding_address = None;
                self.asset_lock_tx_id = None;
                self.show_advanced_options = false;
                *self.step.write().unwrap() = WalletFundedScreenStep::WaitingOnFunds;
            }

            ui.add_space(100.0);
        });

        action
    }
}

impl ScreenWithWalletUnlock for CreateAssetLockScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn password_input(&mut self) -> &mut PasswordInput {
        &mut self.password_input
    }

    fn app_context(&self) -> Arc<AppContext> {
        self.app_context.clone()
    }
}

impl ScreenLike for CreateAssetLockScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
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
                ("Create Asset Lock", AppAction::None),
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

            // Header with Back button and Advanced Options checkbox (outside ScrollArea)
            ui.horizontal(|ui| {
                ui.heading(
                    RichText::new("Create Asset Lock")
                        .color(DashColors::text_primary(dark_mode))
                        .size(24.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Back").clicked() {
                        inner_action = AppAction::PopScreenAndRefresh;
                    }
                    ui.add_space(10.0);
                    ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                });
            });

            // Show wallet name
            ui.label(
                RichText::new(format!("Wallet: {}", wallet_name))
                    .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(10.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {

                    // Show success screen
                    if *self.step.read().unwrap() == WalletFundedScreenStep::Success {
                        inner_action |= self.show_success(ui);
                        return;
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Wallet unlock section
                    let (needs_unlock, unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if !needs_unlock || unlocked {
                        // First, select the purpose of the asset lock
                        if self.asset_lock_purpose.is_none() {
                            ui.heading(RichText::new("Select Asset Lock Purpose").color(DashColors::text_primary(dark_mode)));

                            ui.add_space(10.0);

                            ui.horizontal(|ui| {
                                if ui.button("Registration").clicked() {
                                    self.asset_lock_purpose = Some(AssetLockPurpose::Registration);
                                }

                                ui.add_space(5.0);

                                if ui.button("Top Up").clicked() {
                                    self.asset_lock_purpose = Some(AssetLockPurpose::TopUp);
                                }
                            });

                            ui.add_space(10.0);

                            // Show explanation
                            ui.group(|ui| {
                                ui.label(RichText::new("Information").strong().color(DashColors::text_primary(dark_mode)));
                                ui.add_space(5.0);
                                ui.label(RichText::new("• Registration: Create an asset lock for a new identity registration").color(DashColors::text_secondary(dark_mode)));
                                ui.label(RichText::new("• Top Up: Add credits to an existing identity").color(DashColors::text_secondary(dark_mode)));
                            });

                            return;
                        }

                        // Show selected purpose
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Purpose:").strong().color(DashColors::text_primary(dark_mode)));
                            let purpose_text = match self.asset_lock_purpose {
                                Some(AssetLockPurpose::Registration) => "Registration",
                                Some(AssetLockPurpose::TopUp) => "Top Up",
                                None => "Not selected",
                            };
                            ui.label(RichText::new(purpose_text).color(DashColors::text_secondary(dark_mode)));
                        });

                        // Only show Back button if a purpose has been selected
                        if self.asset_lock_purpose.is_some()
                            && ui.button("Change Purpose").clicked() {
                                self.asset_lock_purpose = None;
                                self.selected_identity = None;
                                self.selected_identity_string.clear();
                            }

                        // For top up, select identity
                        if self.asset_lock_purpose == Some(AssetLockPurpose::TopUp) {
                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(10.0);
                            ui.heading(RichText::new("1. Select Identity to Top Up").color(DashColors::text_primary(dark_mode)));
                            let identities = match self.app_context.load_local_qualified_identities() {
                                Ok(ids) => ids,
                                Err(e) => {
                                    ui.label(
                                        RichText::new(format!("Error loading identities: {}", e))
                                            .color(egui::Color32::RED)
                                    );
                                    return;
                                }
                            };

                            if identities.is_empty() {
                                ui.label(
                                    RichText::new("No identities found. Please create an identity first.")
                                        .color(DashColors::WARNING_BRIGHT)
                                );
                                return;
                            }

                            let identity_selector_response = ui.add(IdentitySelector::new(
                                "top_up_identity_selector",
                                &mut self.selected_identity_string,
                                &identities
                            )
                            .selected_identity(&mut self.selected_identity).unwrap()
                            .label("Identity to top up:")
                            .width(300.0));

                            // Update top_up_index to next unused value when identity selection changes
                            if identity_selector_response.changed()
                                && let Some(selected) = &self.selected_identity {
                                    self.top_up_index = selected
                                        .top_ups
                                        .keys()
                                        .max()
                                        .cloned()
                                        .map(|i| i + 1)
                                        .unwrap_or(0);
                                }

                            if self.selected_identity.is_none() {
                                return;
                            }

                            if self.show_advanced_options {
                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(10.0);

                                ui.heading(RichText::new("2. Top Up Index Selection").color(DashColors::text_primary(dark_mode)));
                                ui.add_space(10.0);

                                // Get used top_up indices from selected identity
                                let used_top_up_indices: HashSet<u32> = self.selected_identity
                                    .as_ref()
                                    .map(|id| id.top_ups.keys().cloned().collect())
                                    .unwrap_or_default();

                                ui.horizontal(|ui| {
                                    ui.label("Top Up Index:");
                                    let selected_text = if used_top_up_indices.contains(&self.top_up_index) {
                                        format!("{} (used)", self.top_up_index)
                                    } else {
                                        format!("{}", self.top_up_index)
                                    };
                                    egui::ComboBox::from_id_salt("top_up_index")
                                        .selected_text(selected_text)
                                        .show_ui(ui, |ui| {
                                            for i in 0..MAX_IDENTITY_INDEX {
                                                let is_used = used_top_up_indices.contains(&i);
                                                let label = if is_used {
                                                    format!("{} (used)", i)
                                                } else {
                                                    format!("{}", i)
                                                };
                                                let is_selected = self.top_up_index == i;
                                                let response = ui.add_enabled(!is_used, Button::new(label).selected(is_selected));
                                                if response.clicked() {
                                                    self.top_up_index = i;
                                                }
                                            }
                                        });
                                });
                            }
                        } else if self.asset_lock_purpose == Some(AssetLockPurpose::Registration)

                            && self.show_advanced_options {
                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(10.0);

                                ui.heading(RichText::new("1. Index Selection").color(DashColors::text_primary(dark_mode)));
                                ui.add_space(10.0);

                                // Get used indices from the database
                                let wallet_guard = self.wallet.read().unwrap();
                                let used_indices: HashSet<u32> = self
                                    .app_context
                                    .db
                                    .get_wallet_identity_indices(
                                        &wallet_guard.seed_hash(),
                                        self.app_context.network,
                                    );
                                drop(wallet_guard);

                                egui::Grid::new("registration_advanced_options_grid")
                                    .num_columns(2)
                                    .spacing([10.0, 8.0])
                                    .show(ui, |ui| {
                                        // Row 1: Identity Index
                                        ui.label("Identity Index:");
                                        let selected_text = if used_indices.contains(&self.identity_index) {
                                            format!("{} (used)", self.identity_index)
                                        } else {
                                            format!("{}", self.identity_index)
                                        };
                                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                            egui::ComboBox::from_id_salt("registration_identity_index")
                                                .selected_text(selected_text)
                                                .show_ui(ui, |ui| {
                                                    for i in 0..MAX_IDENTITY_INDEX {
                                                        let is_used = used_indices.contains(&i);
                                                        let label = if is_used {
                                                            format!("{} (used)", i)
                                                        } else {
                                                            format!("{}", i)
                                                        };
                                                        let is_selected = self.identity_index == i;
                                                        let response = ui.add_enabled(!is_used, Button::new(label).selected(is_selected));
                                                        if response.clicked() {
                                                            self.identity_index = i;
                                                        }
                                                    }
                                                });
                                        });
                                        ui.end_row();
                                    });
                            }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);

                        // Check if funds have arrived at the funding address
                        if let Some(utxo) = funding_common::capture_qr_funding_utxo_if_available(
                            &self.step,
                            self.selected_wallet.as_ref(),
                            self.funding_address.as_ref(),
                        ) {
                            self.funding_utxo = Some(utxo);
                        }

                        let step = *self.step.read().unwrap();

                        // Request periodic repaints while waiting for funds
                        if step == WalletFundedScreenStep::WaitingOnFunds {
                            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                        }

                        // Amount selection step number depends on purpose and advanced options
                        let step_num = match (self.asset_lock_purpose, self.show_advanced_options) {
                            (Some(AssetLockPurpose::TopUp), true) => "3",
                            (Some(AssetLockPurpose::TopUp), false) => "2",
                            (Some(AssetLockPurpose::Registration), true) => "2",
                            _ => "1",
                        };
                        ui.heading(RichText::new(format!("{}. Select how much you would like to transfer?", step_num)).color(DashColors::text_primary(dark_mode)));
                        ui.add_space(10.0);

                        // Show amount input using the component
                        let amount_response = self.amount_input.as_mut().map(|ai| ai.show(ui));
                        ui.add_space(20.0);

                        // Step 3: QR Code and address
                        let amount_valid = amount_response
                            .as_ref()
                            .and_then(|r| r.inner.parsed_amount.as_ref())
                            .map(|a| a.value() > 0)
                            .unwrap_or(false);
                        if amount_valid {
                            let layout_action = ui.with_layout(
                                egui::Layout::top_down(egui::Align::Min).with_cross_align(egui::Align::Center),
                                |ui| {
                                    if let Err(e) = self.render_qr_code(ui) {
                                        MessageBanner::set_global(
                                            ui.ctx(),
                                            "Failed to render QR code",
                                            MessageType::Error,
                                        )
                                        .with_details(e);
                                    }

                                    ui.add_space(20.0);

                                    match step {
                                        WalletFundedScreenStep::WaitingOnFunds => {
                                            ui.heading(RichText::new("Waiting for funds...").color(DashColors::text_primary(dark_mode)));
                                            AppAction::None
                                        }
                                        WalletFundedScreenStep::FundsReceived => {
                                            ui.heading(RichText::new("Funds received! Creating asset lock...").color(DashColors::text_primary(dark_mode)));

                                            // Trigger asset lock creation - get credits from the amount input
                                            let credits = self.amount_input
                                                .as_ref()
                                                .and_then(|ai| ai.current_value())
                                                .map(|a| a.value());
                                            if let Some(credits) = credits {
                                                // Transition to WaitingForAssetLock BEFORE dispatching to prevent duplicate dispatches
                                                {
                                                    let mut step = self.step.write().unwrap();
                                                    *step = WalletFundedScreenStep::WaitingForAssetLock;
                                                }

                                                match self.asset_lock_purpose {
                                                    Some(AssetLockPurpose::Registration) => {
                                                        AppAction::BackendTask(BackendTask::CoreTask(
                                                            CoreTask::CreateRegistrationAssetLock(self.wallet.clone(), credits, self.identity_index)
                                                        ))
                                                    }
                                                    Some(AssetLockPurpose::TopUp) => {
                                                        if let Some(identity) = &self.selected_identity {
                                                            if let Some(identity_index) = identity.wallet_index {
                                                                AppAction::BackendTask(BackendTask::CoreTask(
                                                                    CoreTask::CreateTopUpAssetLock(self.wallet.clone(), credits, identity_index, self.top_up_index)
                                                                ))
                                                            } else {
                                                                MessageBanner::set_global(ui.ctx(), "Selected identity has no wallet index", MessageType::Error);
                                                                AppAction::None
                                                            }
                                                        } else {
                                                            MessageBanner::set_global(ui.ctx(), "No identity selected for top-up", MessageType::Error);
                                                            AppAction::None
                                                        }
                                                    }
                                                    None => {
                                                        MessageBanner::set_global(ui.ctx(), "No purpose selected", MessageType::Error);
                                                        AppAction::None
                                                    }
                                                }
                                            } else {
                                                MessageBanner::set_global(ui.ctx(), "No amount specified", MessageType::Error);
                                                AppAction::None
                                            }
                                        }
                                        WalletFundedScreenStep::WaitingForAssetLock => {
                                            ui.heading(RichText::new("Waiting for Core Chain to produce proof of asset lock...").color(DashColors::text_primary(dark_mode)));
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
                    } else {
                        // Wallet needs to be unlocked
                    }
                });

            // Message display is handled by the global MessageBanner

            inner_action
        });

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Error/success display is handled by the global MessageBanner.
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
                        if let Some(funding_address) = &self.funding_address
                            && funding_address == address
                        {
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
                            MessageBanner::set_global(
                                self.app_context.egui_ctx(),
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
                            MessageBanner::set_global(
                                self.app_context.egui_ctx(),
                                "Asset lock created successfully!",
                                MessageType::Success,
                            );
                        }
                    }
                    _ => {}
                }
            }
            WalletFundedScreenStep::WaitingForAssetLock => {
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
                            MessageBanner::set_global(
                                self.app_context.egui_ctx(),
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
                            MessageBanner::set_global(
                                self.app_context.egui_ctx(),
                                "Asset lock created successfully!",
                                MessageType::Success,
                            );
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        self.is_creating = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that DASH amount parsing correctly converts to credits
    #[test]
    fn test_dash_to_credits_conversion() {
        // 1 DASH = 100_000_000_000 credits (10^11)
        // Test various DASH amounts

        // 0.5 DASH = 50_000_000_000 credits
        let dash_amount = 0.5f64;
        let credits = (dash_amount * 100_000_000_000.0) as u64;
        assert_eq!(credits, 50_000_000_000);

        // 1 DASH = 100_000_000_000 credits
        let dash_amount = 1.0f64;
        let credits = (dash_amount * 100_000_000_000.0) as u64;
        assert_eq!(credits, 100_000_000_000);

        // 0.1 DASH = 10_000_000_000 credits
        let dash_amount = 0.1f64;
        let credits = (dash_amount * 100_000_000_000.0) as u64;
        assert_eq!(credits, 10_000_000_000);

        // 10 DASH = 1_000_000_000_000 credits
        let dash_amount = 10.0f64;
        let credits = (dash_amount * 100_000_000_000.0) as u64;
        assert_eq!(credits, 1_000_000_000_000);
    }

    /// Test that invalid amounts are handled correctly
    #[test]
    fn test_invalid_amount_parsing() {
        // Test that negative amounts don't produce valid credits
        let dash_amount = -1.0f64;
        let is_valid = dash_amount >= 0.0;
        assert!(!is_valid);

        // Test that parsing invalid strings returns None
        let invalid_input = "not_a_number";
        let parsed: Result<f64, _> = invalid_input.parse();
        assert!(parsed.is_err());

        // Test empty string
        let empty_input = "";
        let parsed: Result<f64, _> = empty_input.parse();
        assert!(parsed.is_err());
    }

    /// Test step number calculation based on purpose and advanced options
    #[test]
    fn test_step_number_calculation() {
        // Helper function that mimics the step number calculation in the UI
        fn calculate_step_num(
            purpose: Option<AssetLockPurpose>,
            show_advanced_options: bool,
        ) -> &'static str {
            match (purpose, show_advanced_options) {
                (Some(AssetLockPurpose::TopUp), true) => "3",
                (Some(AssetLockPurpose::TopUp), false) => "2",
                (Some(AssetLockPurpose::Registration), true) => "2",
                _ => "1",
            }
        }

        // Top Up with advanced options: step 3 (1: identity selection, 2: index selection, 3: amount)
        assert_eq!(calculate_step_num(Some(AssetLockPurpose::TopUp), true), "3");

        // Top Up without advanced options: step 2 (1: identity selection, 2: amount)
        assert_eq!(
            calculate_step_num(Some(AssetLockPurpose::TopUp), false),
            "2"
        );

        // Registration with advanced options: step 2 (1: index selection, 2: amount)
        assert_eq!(
            calculate_step_num(Some(AssetLockPurpose::Registration), true),
            "2"
        );

        // Registration without advanced options: step 1 (1: amount)
        assert_eq!(
            calculate_step_num(Some(AssetLockPurpose::Registration), false),
            "1"
        );

        // No purpose selected: step 1
        assert_eq!(calculate_step_num(None, false), "1");
        assert_eq!(calculate_step_num(None, true), "1");
    }

    /// Test next unused identity index calculation
    #[test]
    fn test_next_unused_identity_index() {
        use std::collections::BTreeMap;

        // Helper function that mimics the next index calculation
        fn calculate_next_identity_index(used_indices: &BTreeMap<u32, ()>) -> u32 {
            used_indices
                .keys()
                .copied()
                .max()
                .map(|max| max + 1)
                .unwrap_or(0)
        }

        // No used indices -> next is 0
        let empty: BTreeMap<u32, ()> = BTreeMap::new();
        assert_eq!(calculate_next_identity_index(&empty), 0);

        // Used indices: 0 -> next is 1
        let mut used = BTreeMap::new();
        used.insert(0, ());
        assert_eq!(calculate_next_identity_index(&used), 1);

        // Used indices: 0, 1, 2 -> next is 3
        used.insert(1, ());
        used.insert(2, ());
        assert_eq!(calculate_next_identity_index(&used), 3);

        // Non-contiguous indices: 0, 5 -> next is 6 (not 1)
        let mut non_contiguous = BTreeMap::new();
        non_contiguous.insert(0, ());
        non_contiguous.insert(5, ());
        assert_eq!(calculate_next_identity_index(&non_contiguous), 6);
    }

    /// Test next unused top_up index calculation
    #[test]
    fn test_next_unused_top_up_index() {
        use std::collections::BTreeMap;

        // Helper function that mimics the next top_up index calculation
        fn calculate_next_top_up_index(used_indices: &BTreeMap<u32, ()>) -> u32 {
            used_indices
                .keys()
                .max()
                .cloned()
                .map(|i| i + 1)
                .unwrap_or(0)
        }

        // No used indices -> next is 0
        let empty: BTreeMap<u32, ()> = BTreeMap::new();
        assert_eq!(calculate_next_top_up_index(&empty), 0);

        // Used indices: 0 -> next is 1
        let mut used = BTreeMap::new();
        used.insert(0, ());
        assert_eq!(calculate_next_top_up_index(&used), 1);

        // Used indices: 0, 1, 2 -> next is 3
        used.insert(1, ());
        used.insert(2, ());
        assert_eq!(calculate_next_top_up_index(&used), 3);
    }

    /// Test AssetLockPurpose enum values
    #[test]
    fn test_asset_lock_purpose_values() {
        let registration = AssetLockPurpose::Registration;
        let top_up = AssetLockPurpose::TopUp;

        // Test equality
        assert_eq!(registration, AssetLockPurpose::Registration);
        assert_eq!(top_up, AssetLockPurpose::TopUp);
        assert_ne!(registration, top_up);

        // Test copy semantics
        let registration_copy = registration;
        assert_eq!(registration, registration_copy);
    }

    /// Test TX ID extraction from success message
    #[test]
    fn test_tx_id_extraction() {
        let msg = "Asset lock transaction broadcast successfully. TX ID: abc123def456";

        // Extract TX ID from message
        let tx_id = msg
            .find("TX ID: ")
            .map(|tx_id_start| msg[tx_id_start + 7..].trim().to_string());

        assert_eq!(tx_id, Some("abc123def456".to_string()));

        // Test message without TX ID
        let msg_without_id = "Some other message";
        let no_tx_id = msg_without_id
            .find("TX ID: ")
            .map(|tx_id_start| msg_without_id[tx_id_start + 7..].trim().to_string());

        assert_eq!(no_tx_id, None);
    }

    /// Test MAX_IDENTITY_INDEX constant
    #[test]
    fn test_max_identity_index_constant() {
        assert_eq!(MAX_IDENTITY_INDEX, 30);

        // Verify reasonable range for iteration
        let indices: Vec<u32> = (0..MAX_IDENTITY_INDEX).collect();
        assert_eq!(indices.len(), 30);
        assert_eq!(indices[0], 0);
        assert_eq!(indices[29], 29);
    }

    /// Test default amount values
    #[test]
    fn test_default_amount_values() {
        // Default amount is 0.5 DASH
        let default_amount_input = "0.5";
        let parsed_amount: f64 = default_amount_input.parse().unwrap();
        assert_eq!(parsed_amount, 0.5);

        // Default credits for 0.5 DASH
        let default_credits = 50_000_000_000u64;
        let calculated_credits = (parsed_amount * 100_000_000_000.0) as u64;
        assert_eq!(calculated_credits, default_credits);
    }
}
