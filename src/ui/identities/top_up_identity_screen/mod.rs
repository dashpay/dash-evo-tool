mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::components::funding_widget::FundingWidget;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::{OutPoint, Transaction, TxOut};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::Context;
use egui::{Color32, ScrollArea};
use std::sync::{Arc, RwLock};

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    wallet: Option<Arc<RwLock<Wallet>>>,
    funding_address: Option<Address>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    error_message: Option<String>,
    show_password: bool,
    wallet_password: String,
    show_pop_up_info: Option<String>,
    pub app_context: Arc<AppContext>,
    funding_widget: Option<FundingWidget>,
}

impl TopUpIdentityScreen {
    pub fn new(qualified_identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            identity: qualified_identity,
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            wallet: None,
            funding_address: None,
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "".to_string(),
            funding_amount_exact: None,
            funding_utxo: None,
            error_message: None,
            show_password: false,
            wallet_password: "".to_string(),
            show_pop_up_info: None,
            app_context: app_context.clone(),
            funding_widget: None,
        }
    }

    fn top_up_identity_clicked(&mut self, funding_method: FundingMethod) -> AppAction {
        let Some(selected_wallet) = &self.wallet else {
            return AppAction::None;
        };
        match funding_method {
            FundingMethod::UseUnusedAssetLock => {
                if let Some((tx, funding_asset_lock, address)) = self.funding_asset_lock.clone() {
                    let identity_input = IdentityTopUpInfo {
                        qualified_identity: self.identity.clone(),
                        wallet: Arc::clone(selected_wallet),
                        identity_funding_method: TopUpIdentityFundingMethod::UseAssetLock(
                            address,
                            Box::new(funding_asset_lock),
                            Box::new(tx),
                        ),
                    };

                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;

                    AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                        identity_input,
                    )))
                } else {
                    AppAction::None
                }
            }
            FundingMethod::UseWalletBalance => {
                // Parse the funding amount or fall back to the default value
                let amount = self.funding_amount_exact.unwrap_or_else(|| {
                    (self.funding_amount.parse::<f64>().unwrap_or(0.0) * 1e8) as u64
                });

                if amount == 0 {
                    return AppAction::None;
                }
                let identity_input = IdentityTopUpInfo {
                    qualified_identity: self.identity.clone(),
                    wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                    identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                        amount,
                        self.identity.wallet_index.unwrap_or(u32::MAX >> 1),
                        self.identity
                            .top_ups
                            .keys()
                            .max()
                            .cloned()
                            .map(|i| i + 1)
                            .unwrap_or_default(),
                    ),
                };

                let mut step = self.step.write().unwrap();
                *step = WalletFundedScreenStep::WaitingForAssetLock;

                // Create the backend task to top_up the identity
                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                    identity_input,
                )))
            }
            FundingMethod::AddressWithQRCode => {
                // For address with QR code, we need to wait for funds to arrive
                // This is similar to UseWalletBalance but uses external funding
                let Some(funding_utxo) = &self.funding_utxo else {
                    // Start waiting for funds
                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::WaitingOnFunds;
                    return AppAction::None;
                };

                let (outpoint, tx_out, address) = funding_utxo.clone();
                let identity_input = IdentityTopUpInfo {
                    qualified_identity: self.identity.clone(),
                    wallet: Arc::clone(selected_wallet),
                    identity_funding_method: TopUpIdentityFundingMethod::FundWithUtxo(
                        outpoint,
                        tx_out,
                        address,
                        self.identity.wallet_index.unwrap_or(u32::MAX >> 1),
                        self.identity
                            .top_ups
                            .keys()
                            .max()
                            .cloned()
                            .map(|i| i + 1)
                            .unwrap_or_default(),
                    ),
                };

                let mut step = self.step.write().unwrap();
                *step = WalletFundedScreenStep::WaitingForAssetLock;

                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                    identity_input,
                )))
            }
            _ => AppAction::None,
        }
    }

    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        // Ensure a wallet is selected
        let Some(selected_wallet) = self.wallet.clone() else {
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
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                });

                ui.add_space(5.0); // Add space between each entry
            }
        });
    }
}

impl ScreenWithWalletUnlock for TopUpIdentityScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.wallet
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

impl ScreenLike for TopUpIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Error {
            self.error_message = Some(format!("Error topping up identity: {}", message));
        } else {
            self.error_message = Some(message.to_string());
        }
    }
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        let mut step = self.step.write().unwrap();
        match *step {
            WalletFundedScreenStep::ChooseFundingMethod => {}
            WalletFundedScreenStep::WaitingOnFunds => {
                if let Some(funding_address) = self.funding_address.as_ref() {
                    if let BackendTaskSuccessResult::CoreItem(
                        CoreItem::ReceivedAvailableUTXOTransaction(_, outpoints_with_addresses),
                    ) = backend_task_success_result
                    {
                        for (outpoint, tx_out, address) in outpoints_with_addresses {
                            if funding_address == &address {
                                *step = WalletFundedScreenStep::FundsReceived;
                                self.funding_utxo = Some((outpoint, tx_out, address))
                            }
                        }
                    }
                }
            }
            WalletFundedScreenStep::FundsReceived => {}
            WalletFundedScreenStep::ReadyToCreate => {}
            WalletFundedScreenStep::WaitingForAssetLock => {
                if let BackendTaskSuccessResult::CoreItem(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                ) = backend_task_success_result
                {
                    if let Some(TransactionPayload::AssetLockPayloadType(asset_lock_payload)) =
                        tx.special_transaction_payload
                    {
                        if asset_lock_payload.credit_outputs.iter().any(|tx_out| {
                            let Ok(address) = Address::from_script(
                                &tx_out.script_pubkey,
                                self.app_context.network,
                            ) else {
                                return false;
                            };
                            if let Some(wallet) = &self.wallet {
                                let wallet = wallet.read().unwrap();
                                wallet.known_addresses.contains_key(&address)
                            } else {
                                false
                            }
                        }) {
                            *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
                        }
                    }
                }
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                if let BackendTaskSuccessResult::ToppedUpIdentity(_qualified_identity) =
                    backend_task_success_result
                {
                    *step = WalletFundedScreenStep::Success;
                }
            }
            WalletFundedScreenStep::Success => {}
        }
    }
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Top Up Identity", AppAction::None),
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
                let step = { *self.step.read().unwrap() };
                if step == WalletFundedScreenStep::Success {
                    inner_action |= self.show_success(ui);
                    return;
                }

                ui.add_space(10.0);

                // Display identity info
                ui.horizontal(|ui| {
                    ui.label("Identity:");

                    // Show alias if available, otherwise show ID
                    if let Some(alias) = &self.identity.alias {
                        ui.label(alias);
                    } else {
                        ui.label(self.identity.identity.id().to_string(Encoding::Base58));
                    }
                });

                // Show current balance
                ui.horizontal(|ui| {
                    ui.label("Balance:");
                    let balance_dash = self.identity.identity.balance() as f64 * 1e-11;
                    ui.label(format!("{:.4} DASH", balance_dash));
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.heading("Follow these steps to top up your identity:");
                ui.add_space(15.0);

                let step_number = 1;
                
                // Initialize funding widget if needed
                if self.funding_widget.is_none() {
                    let mut widget = FundingWidget::new(self.app_context.clone())
                        .with_amount_label("Top-up Amount (DASH):")
                        .with_default_amount("0.5");

                    // Set wallet if already selected
                    if let Some(wallet) = &self.wallet {
                        widget = widget.with_wallet(wallet.clone());
                    }

                    self.funding_widget = Some(widget);
                    
                    // Initialize funding_amount_exact with the default amount
                    self.funding_amount = "0.5".to_string();
                    self.funding_amount_exact = Some((0.5 * 1e8) as u64);
                }

                // Render the funding widget
                if let Some(ref mut widget) = self.funding_widget {
                    ui.heading(format!("{}. Configure your top-up", step_number));
                    ui.add_space(10.0);

                    let widget_response = widget.show(ui);
                    let response_data = widget_response.inner;

                    // Handle widget responses
                    if let Some(wallet) = response_data.wallet_changed {
                        self.wallet = Some(wallet);
                        // Clear funding asset lock when wallet changes
                        self.funding_asset_lock = None;
                    }

                    if let Some(method) = response_data.funding_method_changed {
                        let mut funding_method_guard = self.funding_method.write().unwrap();
                        *funding_method_guard = method;
                        // Clear funding asset lock when method changes
                        self.funding_asset_lock = None;
                    }

                    if let Some(amount) = response_data.amount_changed {
                        self.funding_amount = amount.clone();
                        self.funding_amount_exact = amount.parse::<f64>().ok().map(|f| {
                            (f * 1e8) as u64 // Convert to Duffs
                        });
                    }

                    if let Some(address) = response_data.address_changed {
                        self.funding_address = Some(address);
                    }

                    if let Some(error) = response_data.error {
                        self.error_message = Some(error);
                    }

                    // Get the current funding method from the widget
                    let funding_method = widget.funding_method();
                    
                    // Additional UI for UseUnusedAssetLock method
                    if funding_method == FundingMethod::UseUnusedAssetLock {
                        ui.add_space(15.0);
                        ui.separator();
                        ui.add_space(10.0);
                        
                        self.render_choose_funding_asset_lock(ui);
                    }

                    // Show top-up button when ready
                    let can_top_up = match funding_method {
                        FundingMethod::UseUnusedAssetLock => {
                            // Check if we have a funding asset lock selected
                            self.funding_asset_lock.is_some()
                        }
                        FundingMethod::UseWalletBalance => {
                            // Check if amount is valid
                            let has_amount = self.funding_amount_exact.is_some();
                            let amount_valid = has_amount && self.funding_amount_exact.unwrap_or_default() > 0;
                            
                            // Debug output
                            if !has_amount {
                                ui.label("Debug: No funding_amount_exact set");
                            } else if !amount_valid {
                                ui.label(format!("Debug: Amount not valid: {:?}", self.funding_amount_exact));
                            }
                            
                            amount_valid
                        }
                        FundingMethod::AddressWithQRCode => {
                            // For QR code method, always show the button
                            // The backend will handle the fund waiting logic
                            self.funding_address.is_some() &&
                            self.funding_amount_exact.is_some() && 
                            self.funding_amount_exact.unwrap() > 0
                        }
                        FundingMethod::NoSelection => false,
                    };

                    if can_top_up {
                        ui.add_space(15.0);
                        ui.separator();
                        ui.add_space(10.0);

                        if ui.button("Top Up Identity").clicked() {
                            inner_action |= self.top_up_identity_clicked(funding_method);
                        }
                    }

                    // Show error message if any
                    if let Some(error_message) = self.error_message.as_ref() {
                        ui.add_space(10.0);
                        ui.colored_label(Color32::DARK_RED, error_message);
                    }

                    // Show step status
                    let step = *self.step.read().unwrap();
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| match step {
                        WalletFundedScreenStep::WaitingOnFunds => {
                            ui.heading("=> Waiting for funds <=");
                            ui.add_space(10.0);
                            ui.label("Send the specified amount to the address above.");
                        }
                        WalletFundedScreenStep::WaitingForAssetLock => {
                            ui.heading("=> Creating asset lock transaction <=");
                        }
                        WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                            ui.heading("=> Waiting for Platform acknowledgement <=");
                            ui.add_space(10.0);
                            ui.label("NOTE: If this gets stuck, the funds were likely either transferred to the wallet or asset locked,\nand you can use the funding method selector in step 1 to change the method and use those funds to complete the process.");
                        }
                        WalletFundedScreenStep::Success => {
                            ui.heading("...Success...");
                        }
                        _ => {}
                    });
                }
            });

            inner_action
        });

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::Window::new("Identity Index Information")
                .collapsible(false) // Prevent collapsing
                .resizable(false) // Prevent resizing
                .show(ctx, |ui| {
                    ui.label(show_pop_up_info_text);

                    // Add a close button to dismiss the popup
                    if ui.button("Close").clicked() {
                        self.show_pop_up_info = None
                    }
                });
        }

        action
    }
}
