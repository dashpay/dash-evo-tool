mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod by_wallet_qr_code;
mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::CoreItem;
use crate::backend_task::identity::{
    IdentityKeys, IdentityTask, IdentityTopUpInfo, RegisterIdentityFundingMethod,
    TopUpIdentityFundingMethod,
};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::balances::credits::Duffs;
use dash_sdk::dpp::dashcore::{OutPoint, Transaction, TxOut};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::Context;
use egui::{ComboBox, ScrollArea, Ui};
use std::cmp::PartialEq;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    wallet: Option<Arc<RwLock<Wallet>>>,
    core_has_funding_address: Option<bool>,
    funding_address: Option<Address>,
    funding_address_balance: Arc<RwLock<Option<Duffs>>>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    alias_input: String,
    copied_to_clipboard: Option<Option<String>>,
    error_message: Option<String>,
    show_password: bool,
    wallet_password: String,
    show_pop_up_info: Option<String>,
    in_key_selection_advanced_mode: bool,
    pub app_context: Arc<AppContext>,
}

impl TopUpIdentityScreen {
    pub fn new(qualified_identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let selected_wallet = qualified_identity
            .associated_wallets
            .first_key_value()
            .map(|(_, wallet)| wallet.clone());

        Self {
            identity: qualified_identity,
            step: Arc::new(RwLock::new(WalletFundedScreenStep::ChooseFundingMethod)),
            funding_asset_lock: None,
            wallet: selected_wallet,
            core_has_funding_address: None,
            funding_address: None,
            funding_address_balance: Arc::new(RwLock::new(None)),
            funding_method: Arc::new(RwLock::new(FundingMethod::NoSelection)),
            funding_amount: "0.5".to_string(),
            funding_amount_exact: None,
            funding_utxo: None,
            alias_input: String::new(),
            copied_to_clipboard: None,
            error_message: None,
            show_password: false,
            wallet_password: "".to_string(),
            show_pop_up_info: None,
            in_key_selection_advanced_mode: false,
            app_context: app_context.clone(),
        }
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets = &self.identity.associated_wallets;
            if wallets.len() > 1 {
                // Retrieve the alias of the currently selected wallet, if any
                let selected_wallet_alias = self
                    .wallet
                    .as_ref()
                    .and_then(|wallet| wallet.read().ok()?.alias.clone())
                    .unwrap_or_else(|| "Select".to_string());

                ui.heading(
                    "1. Choose the wallet to use in which this identities keys will come from.",
                );

                ui.add_space(10.0);

                // Display the ComboBox for wallet selection
                ComboBox::from_label("Select Wallet")
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
                                .map_or(false, |selected| Arc::ptr_eq(selected, wallet));

                            if ui.selectable_label(is_selected, wallet_alias).clicked() {
                                // Update the selected wallet
                                self.wallet = Some(wallet.clone());
                            }
                        }
                    });
                ui.add_space(10.0);
                true
            } else if let Some(wallet) = wallets.values().next() {
                if self.wallet.is_none() {
                    // Automatically select the only available wallet
                    self.wallet = Some(wallet.clone());
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
        let Some(selected_wallet) = self.wallet.clone() else {
            return;
        };
        let funding_method_arc = self.funding_method.clone();
        let mut funding_method = funding_method_arc.write().unwrap(); // Write lock on funding_method

        ComboBox::from_label("Funding Method")
            .selected_text(format!("{}", *funding_method))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::NoSelection,
                    "Please select funding method",
                );

                let (has_unused_asset_lock, has_balance) = {
                    let wallet = selected_wallet.read().unwrap();
                    (wallet.has_unused_asset_lock(), wallet.has_balance())
                };

                if has_unused_asset_lock {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseUnusedAssetLock,
                            "Use Unused Evo Funding Locks (recommended)",
                        )
                        .changed()
                    {
                        self.update_identity_key();
                        let mut step = self.step.write().unwrap(); // Write lock on step
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                }
                if has_balance {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseWalletBalance,
                            "Use Wallet Balance",
                        )
                        .changed()
                    {
                        if let Some(wallet) = &self.wallet {
                            let wallet = wallet.read().unwrap(); // Read lock on the wallet
                            let max_amount = wallet.max_balance();
                            self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        }
                        let mut step = self.step.write().unwrap(); // Write lock on step
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                }
                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::AddressWithQRCode,
                        "Address with QR Code",
                    )
                    .changed()
                {
                    let mut step = self.step.write().unwrap(); // Write lock on step
                    *step = WalletFundedScreenStep::WaitingOnFunds;
                }

                // Uncomment this if AttachedCoreWallet is available in the future
                // ui.selectable_value(
                //     &mut *funding_method,
                //     FundingMethod::AttachedCoreWallet,
                //     "Attached Core Wallet",
                // );
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
                        wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                        identity_funding_method: TopUpIdentityFundingMethod::UseAssetLock(
                            address,
                            funding_asset_lock,
                            tx,
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
                    (self.funding_amount.parse::<f64>().unwrap_or_else(|_| 0.0) * 1e8) as u64
                });

                if amount == 0 {
                    return AppAction::None;
                }
                let identity_input = IdentityTopUpInfo {
                    qualified_identity: self.identity.clone(),
                    wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                    identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                        amount,
                        self.identity_id_number,
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
            ui.label("Funding Amount (DASH):");

            // Render the text input field for the funding amount
            let amount_input = ui
                .add(
                    egui::TextEdit::singleline(&mut self.funding_amount)
                        .hint_text("Enter amount (e.g., 0.1234)")
                        .desired_width(100.0),
                )
                .lost_focus();

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
                    let wallet = wallet.read().unwrap(); // Read lock on the wallet
                    if ui.button("Max").clicked() {
                        let max_amount = wallet.max_balance();
                        self.funding_amount = format!("{:.4}", max_amount as f64 * 1e-8);
                        self.funding_amount_exact = Some(max_amount);
                    }
                }
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
            self.error_message = Some(format!("Error top_uping identity: {}", message));
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
                        if asset_lock_payload
                            .credit_outputs
                            .iter()
                            .find(|tx_out| {
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
                            })
                            .is_some()
                        {
                            *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
                        }
                    }
                }
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                if let BackendTaskSuccessResult::ToppedUpIdentity(qualified_identity) =
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

        egui::CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                let step = {self.step.read().unwrap().clone()};
                if step == WalletFundedScreenStep::Success {
                    action |= self.show_success(ui);
                    return;
                }
                ui.add_space(10.0);
                ui.heading("Follow these steps to top up your identity!");
                ui.add_space(15.0);

                let mut step_number = 1;

                if self.render_wallet_selection(ui) {
                    // We had more than 1 wallet
                    step_number += 1;
                }

                if self.wallet.is_none() {
                    return;
                };

                // Display the heading with an info icon that shows a tooltip on hover
                ui.horizontal(|ui| {
                    let wallet_guard = self.wallet.as_ref().unwrap();
                    let wallet = wallet_guard.read().unwrap();
                    if wallet.identities.is_empty() {
                        ui.heading(format!(
                            "{}. Choose an identity index. Leave this 0 if this is your first identity for this wallet.",
                            step_number
                        ));
                    } else {
                        ui.heading(format!(
                            "{}. Choose an identity index. Leaving this {} is recommended.",
                            step_number,
                            wallet.identities.keys().cloned().max().map(|max| max + 1).unwrap_or_default()
                        ));
                    }


                    // Create a label with click sense and tooltip
                    let info_icon = egui::Label::new("ℹ").sense(egui::Sense::click());
                    let response = ui.add(info_icon)
                        .on_hover_text("The identity index is an internal reference within the wallet. The wallet’s seed phrase can always be used to recover any identity, including this one, by using the same index.");

                    // Check if the label was clicked
                    if response.clicked() {
                        self.show_pop_up_info = Some("The identity index is an internal reference within the wallet. The wallet’s seed phrase can always be used to recover any identity, including this one, by using the same index.".to_string());
                    }
                });

                step_number += 1;

                ui.add_space(8.0);

                self.render_identity_index_input(ui);

                ui.add_space(10.0);

                // Display the heading with an info icon that shows a tooltip on hover
                ui.horizontal(|ui| {
                    ui.heading(format!(
                        "{}. Choose what keys you want to add to this new identity.",
                        step_number
                    ));

                    // Create a label with click sense and tooltip
                    let info_icon = egui::Label::new("ℹ").sense(egui::Sense::click());
                    let response = ui.add(info_icon)
                        .on_hover_text("Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.");

                    // Check if the label was clicked
                    if response.clicked() {
                        self.show_pop_up_info = Some("Keys allow an identity to perform actions on the Blockchain. They are contained in your wallet and allow you to prove that the action you are making is really coming from yourself.".to_string());
                    }
                });

                step_number += 1;

                ui.add_space(8.0);

                self.render_key_selection(ui);

                ui.add_space(10.0);

                ui.heading(
                    format!("{}. Choose your funding method.", step_number).as_str()
                );
                step_number += 1;

                ui.add_space(10.0);
                self.render_funding_method(ui);

                // Extract the funding method from the RwLock to minimize borrow scope
                let funding_method = self.funding_method.read().unwrap().clone();

                if funding_method == FundingMethod::NoSelection {
                    return;
                }

                match funding_method {
                    FundingMethod::NoSelection => return,
                    FundingMethod::UseUnusedAssetLock => {
                        action |= self.render_ui_by_using_unused_asset_lock(ui, step_number);
                    },
                    FundingMethod::UseWalletBalance => {
                        action |= self.render_ui_by_using_unused_balance(ui, step_number);
                    },
                    FundingMethod::AddressWithQRCode => {
                        action |= self.render_ui_by_wallet_qr_code(ui, step_number)
                    },
                    FundingMethod::AttachedCoreWallet => return,
                }
            });
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
