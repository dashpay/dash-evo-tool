mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::funding_widget::{FundingWidget, FundingWidgetMethod};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::Context;
use egui::{Color32, ScrollArea};
use std::sync::{Arc, RwLock};

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    wallet: Option<Arc<RwLock<Wallet>>>,
    error_message: Option<String>,
    show_password: bool,
    wallet_password: String,
    show_pop_up_info: Option<String>,
    pub app_context: Arc<AppContext>,
    funding_widget: Option<FundingWidget>,
    funding_amount: String,
    funding: Option<FundingWidgetMethod>,
}

impl TopUpIdentityScreen {
    pub fn new(qualified_identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            identity: qualified_identity,
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            wallet: None,
            funding_amount: "".to_string(),
            funding: None,
            error_message: None,
            show_password: false,
            wallet_password: "".to_string(),
            show_pop_up_info: None,
            app_context: app_context.clone(),
            funding_widget: None,
        }
    }

    fn top_up_identity_clicked(&mut self, funding_method: FundingWidgetMethod) -> AppAction {
        let Some(selected_wallet) = &self.wallet else {
            return AppAction::None;
        };
        match funding_method {
            FundingWidgetMethod::UseAssetLock(address, funding_asset_lock, tx) => {
                let txid = tx.txid().to_hex();
                let identity_input = IdentityTopUpInfo {
                    qualified_identity: self.identity.clone(),
                    wallet: Arc::clone(selected_wallet),
                    identity_funding_method: TopUpIdentityFundingMethod::UseAssetLock(
                        address,
                        funding_asset_lock,
                        tx,
                    ),
                };
                tracing::debug!("Using asset lock for identity top-up: {:?}", txid);
                let mut step = self.step.write().unwrap();
                *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;

                AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::TopUpIdentity(
                    identity_input,
                )))
            }
            FundingWidgetMethod::FundWithWallet(amount) => {
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
            FundingWidgetMethod::FundWithUtxo(outpoint, tx_out, address) => {
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
        }
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
                }

                // Render the funding widget
                if let Some(ref mut widget) = self.funding_widget {
                    // Disable balance checks if operation is in progress
                    let step = {*self.step.read().unwrap()};
                    widget.set_validation_hints(!step.is_processing());

                    ui.heading(format!("{}. Configure your top-up", step_number));
                    ui.add_space(10.0);

                    let widget_response = widget.show(ui);
                    let response_data = widget_response.inner;

                    // Handle widget responses
                    if let Some(wallet) = &response_data.wallet_changed {
                        self.wallet = Some(wallet.clone());
                        // Clear funding asset lock when wallet changes
                        self.funding = None;
                    }

                    if let Some(method) = &response_data.funding_secured {
                        self.funding = Some(method.clone());
                    }

                    if let Some(amount) = &response_data.amount_changed {
                        self.funding_amount = amount.clone();
                    }

                    if let Some(error) = &response_data.error {
                        self.error_message = Some(error.clone());
                    }

                    let funding_secured = response_data.funded() || step.is_processing();

                    if funding_secured {
                        // for wallet funding, we need a button to top up the identity
                        // others can be done automatically
                        if let Some(funding_method) = self.funding.clone() {
                            if matches!(funding_method, FundingWidgetMethod::FundWithWallet(_)) {
                                ui.add_space(15.0);
                                ui.separator();
                                ui.add_space(10.0);
                                if ui.button("Top Up Identity").clicked() {
                                    inner_action |= self.top_up_identity_clicked(funding_method);
                                }
                            } else if !step.is_processing() {
                                inner_action |= self.top_up_identity_clicked(funding_method);
                            }
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
