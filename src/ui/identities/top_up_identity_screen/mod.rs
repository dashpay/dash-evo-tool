mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod by_wallet_qr_code;
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
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::{OutPoint, Transaction, TxOut};
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::Context;
use egui::{ComboBox, ScrollArea, Ui};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    wallet: Option<Arc<RwLock<Wallet>>>,
    core_has_funding_address: Option<bool>,
    funding_address: Option<Address>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    copied_to_clipboard: Option<Option<String>>,
    error_message: Option<String>,
    show_password: bool,
    wallet_password: String,
    show_pop_up_info: Option<String>,
    pub app_context: Arc<AppContext>,
}

impl TopUpIdentityScreen {
    pub fn new(qualified_identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            identity: qualified_identity,
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            wallet: None,
            core_has_funding_address: None,
            funding_address: None,
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "0.5".to_string(),
            funding_amount_exact: None,
            funding_utxo: None,
            copied_to_clipboard: None,
            error_message: None,
            show_password: false,
            wallet_password: "".to_string(),
            show_pop_up_info: None,
            app_context: app_context.clone(),
        }
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = self.app_context.wallets.read().unwrap();
            if wallets.len() > 1 {
                // Retrieve the alias of the currently selected wallet, if any
                let selected_wallet_alias = self
                    .wallet
                    .as_ref()
                    .and_then(|wallet| wallet.read().ok()?.alias.clone())
                    .unwrap_or_else(|| "Select".to_string());

                // Display the ComboBox for wallet selection
                ComboBox::from_id_salt("select_wallet")
                    .selected_text(selected_wallet_alias)
                    .show_ui(ui, |ui| {
                        for wallet in wallets.values() {
                            let wallet_alias = wallet
                                .read()
                                .ok()
                                .and_then(|w| w.alias.clone())
                                .unwrap_or_else(|| "Unnamed Wallet".to_string());

                            let is_selected = self
                                .wallet
                                .as_ref()
                                .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                            if ui.selectable_label(is_selected, wallet_alias).clicked() {
                                // Update the selected wallet from app_context
                                self.wallet = Some(wallet.clone());
                                
                                // If UseWalletBalance is selected and we just set the wallet, update the max amount
                                let funding_method = self.funding_method.read().unwrap();
                                if *funding_method == FundingMethod::UseWalletBalance {
                                    let wallet = wallet.read().unwrap();
                                    let max_amount = wallet.max_balance();
                                    self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                                    self.funding_amount_exact = Some(max_amount);
                                }
                            }
                        }
                    });
                true
            } else if let Some(wallet) = wallets.values().next() {
                if self.wallet.is_none() {
                    // Automatically select the only available wallet from app_context
                    self.wallet = Some(wallet.clone());
                    
                    // If UseWalletBalance is selected and we just set the wallet, update the max amount
                    let funding_method = self.funding_method.read().unwrap();
                    if *funding_method == FundingMethod::UseWalletBalance {
                        let wallet = wallet.read().unwrap();
                        let max_amount = wallet.max_balance();
                        self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        self.funding_amount_exact = Some(max_amount);
                    }
                }
                false
            } else {
                false
            }
        } else {
            false
        }
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let funding_method_arc = self.funding_method.clone();
        let mut funding_method = funding_method_arc.write().unwrap();

        ComboBox::from_id_salt("funding_method")
            .selected_text(format!("{}", *funding_method))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::NoSelection,
                    "Please select funding method",
                );

                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::UseUnusedAssetLock,
                        "Use Unused Evo Funding Locks (recommended)",
                    )
                    .changed()
                {
                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::ReadyToCreate;
                }

                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::UseWalletBalance,
                        "Use Wallet Balance",
                    )
                    .changed()
                {
                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::ReadyToCreate;
                }

                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::AddressWithQRCode,
                        "Address with QR Code",
                    )
                    .changed()
                {
                    let mut step = self.step.write().unwrap();
                    *step = WalletFundedScreenStep::WaitingOnFunds;
                }
            });
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
            _ => AppAction::None,
        }
    }

    fn top_up_funding_amount_input(&mut self, ui: &mut egui::Ui) {
        let funding_method = self.funding_method.read().unwrap(); // Read lock on funding_method

        ui.horizontal(|ui| {
            ui.label("Amount (DASH):");

            // Render the text input field for the funding amount
            let amount_input = ui
                .add(
                    egui::TextEdit::singleline(&mut self.funding_amount)
                        .hint_text("Enter amount (e.g., 0.1234)")
                        .desired_width(100.0),
                )
                .lost_focus();

            self.funding_amount_exact = self.funding_amount.parse::<f64>().ok().map(|f| {
                (f * 1e8) as u64 // Convert the amount to Duffs
            });

            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            if amount_input && enter_pressed {
                // Optional: Validate the input when Enter is pressed
                if self.funding_amount.parse::<f64>().is_err() {
                    ui.label("Invalid amount. Please enter a valid number.");
                }
            }

            // Check if the funding method is `UseWalletBalance`
            if *funding_method == FundingMethod::UseWalletBalance {
                // Safely access the selected wallet
                if let Some(wallet) = &self.wallet {
                    let wallet = wallet.read().unwrap();
                    if ui.button("Max").clicked() {
                        let max_amount = wallet.max_balance();
                        self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        self.funding_amount_exact = Some(max_amount);
                    }
                }
            }
        });

        ui.add_space(10.0);
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
                ui.heading("Follow these steps to top up your identity:");
                ui.add_space(15.0);

                let mut step_number = 1;
                ui.heading(format!("{}. Choose your funding method.", step_number).as_str());
                step_number += 1;
                ui.add_space(10.0);

                self.render_funding_method(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Extract the funding method from the RwLock to minimize borrow scope
                let funding_method = *self.funding_method.read().unwrap();
                if funding_method == FundingMethod::NoSelection {
                    return;
                }

                if funding_method == FundingMethod::UseWalletBalance
                    || funding_method == FundingMethod::UseUnusedAssetLock
                {
                    ui.heading(format!(
                        "{}. Choose the wallet to use to top up this identity.",
                        step_number
                    ));
                    step_number += 1;

                    ui.add_space(10.0);

                    self.render_wallet_selection(ui);

                    if self.wallet.is_none() {
                        return;
                    };

                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return;
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                match funding_method {
                    FundingMethod::NoSelection => (),
                    FundingMethod::UseUnusedAssetLock => {
                        inner_action |= self.render_ui_by_using_unused_asset_lock(ui, step_number);
                    }
                    FundingMethod::UseWalletBalance => {
                        inner_action |= self.render_ui_by_using_unused_balance(ui, step_number);
                    }
                    FundingMethod::AddressWithQRCode => {
                        inner_action |= self.render_ui_by_wallet_qr_code(ui, step_number)
                    }
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
