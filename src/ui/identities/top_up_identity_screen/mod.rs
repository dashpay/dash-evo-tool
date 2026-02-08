mod by_platform_address;
mod by_using_unused_asset_lock;
mod by_using_unused_balance;
mod by_wallet_qr_code;
mod success_screen;

use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreResult};
use crate::backend_task::identity::{
    IdentityResult, IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod,
};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::lock_helper::RwLockExt;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::funding_common::WalletFundedScreenStep;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dashcore_rpc::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::{Credits, Duffs};
use dash_sdk::dpp::dashcore::{OutPoint, Transaction, TxOut};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::Context;
use egui::{ComboBox, ScrollArea, Ui};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

const WALLET_SELECTION_TOOLTIP: &str = "This wallet will provide the address for receiving funds \
and create the asset lock transaction to top up your identity.";

pub struct TopUpIdentityScreen {
    pub identity: QualifiedIdentity,
    step: Arc<RwLock<WalletFundedScreenStep>>,
    funding_asset_lock: Option<(Transaction, AssetLockProof, Address)>,
    wallet: Option<Arc<RwLock<Wallet>>>,
    funding_address: Option<Address>,
    funding_method: Arc<RwLock<FundingMethod>>,
    funding_amount: String,
    funding_amount_exact: Option<Duffs>,
    funding_amount_input: Option<AmountInput>,
    funding_utxo: Option<(OutPoint, TxOut, Address)>,
    copied_to_clipboard: Option<Option<String>>,
    error_message: Option<String>,
    wallet_unlock_popup: WalletUnlockPopup,
    show_pop_up_info: Option<String>,
    pub app_context: Arc<AppContext>,
    // Platform address fields
    selected_platform_address: Option<(Address, PlatformAddress, Credits)>,
    platform_top_up_amount: Option<Amount>,
    platform_top_up_amount_input: Option<AmountInput>,
    /// Fee result from completed top-up
    completed_fee_result: Option<FeeResult>,
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
            funding_amount_input: None,
            funding_utxo: None,
            copied_to_clipboard: None,
            error_message: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_pop_up_info: None,
            app_context: app_context.clone(),
            selected_platform_address: None,
            platform_top_up_amount: None,
            platform_top_up_amount_input: None,
            completed_fee_result: None,
        }
    }

    fn render_wallet_selection(&mut self, ui: &mut Ui) -> bool {
        let mut selected_wallet_update: Option<Arc<RwLock<Wallet>>> = None;
        let mut step_update_method: Option<FundingMethod> = None;

        let rendered = if self.app_context.has_wallet.load(Ordering::Relaxed) {
            let wallets_guard = self.app_context.wallets.read_or_recover();
            let wallets = &*wallets_guard;

            if wallets.len() > 1 {
                // Cache current funding method to avoid holding the lock across UI callbacks
                let funding_method = *self.funding_method.read_or_recover();

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
                            let (wallet_alias, has_required_resources) = {
                                let wallet_read = wallet.read_or_recover();
                                let alias = wallet_read
                                    .alias
                                    .clone()
                                    .unwrap_or_else(|| "Unnamed Wallet".to_string());

                                let has_resources = match funding_method {
                                    FundingMethod::UseWalletBalance => wallet_read.has_balance(),
                                    FundingMethod::UseUnusedAssetLock => {
                                        wallet_read.has_unused_asset_lock()
                                    }
                                    _ => true,
                                };

                                (alias, has_resources)
                            };

                            let is_selected = self
                                .wallet
                                .as_ref()
                                .is_some_and(|selected| Arc::ptr_eq(selected, wallet));

                            ui.add_enabled_ui(has_required_resources, |ui| {
                                if ui.selectable_label(is_selected, wallet_alias).clicked() {
                                    selected_wallet_update = Some(wallet.clone());
                                    step_update_method = Some(funding_method);
                                }
                            });
                        }
                    });
                true
            } else if let Some(wallet) = wallets.values().next() {
                if self.wallet.is_none() {
                    // Cache current funding method to avoid holding the lock across updates
                    let funding_method = *self.funding_method.read_or_recover();

                    // Check if the wallet has the required resources
                    let has_required_resources = {
                        let wallet_read = wallet.read_or_recover();
                        match funding_method {
                            FundingMethod::UseWalletBalance => wallet_read.has_balance(),
                            FundingMethod::UseUnusedAssetLock => {
                                wallet_read.has_unused_asset_lock()
                            }
                            _ => true,
                        }
                    };

                    if has_required_resources {
                        // Automatically select the only available wallet from app_context
                        selected_wallet_update = Some(wallet.clone());
                        step_update_method = Some(funding_method);
                    }
                }
                false
            } else {
                false
            }
        } else {
            false
        };

        if let Some(wallet) = selected_wallet_update {
            self.wallet = Some(wallet);
            self.funding_address = None;
            self.funding_asset_lock = None;
            self.funding_utxo = None;
            self.funding_amount_input = None;
            self.copied_to_clipboard = None;

            if let Some(method) = step_update_method {
                self.update_step_after_wallet_change(method);
            } else {
                let mut step = self.step.write_or_recover();
                *step = WalletFundedScreenStep::ChooseFundingMethod;
            }
        }

        rendered
    }

    /// Adjust the current step to match the funding method after a wallet switch.
    fn update_step_after_wallet_change(&mut self, funding_method: FundingMethod) {
        let mut step = self.step.write_or_recover();
        *step = match funding_method {
            FundingMethod::AddressWithQRCode => WalletFundedScreenStep::WaitingOnFunds,
            FundingMethod::UseUnusedAssetLock
            | FundingMethod::UseWalletBalance
            | FundingMethod::UsePlatformAddress => WalletFundedScreenStep::ReadyToCreate,
            FundingMethod::NoSelection => WalletFundedScreenStep::ChooseFundingMethod,
        };
    }

    fn render_funding_method(&mut self, ui: &mut egui::Ui) {
        let funding_method_arc = self.funding_method.clone();
        let mut funding_method = funding_method_arc.write_or_recover();

        // Check if any wallet has unused asset locks, balance, or Platform address balance
        let (has_any_unused_asset_lock, has_any_balance, has_any_platform_balance) = {
            let wallets = self.app_context.wallets.read_or_recover();
            let mut has_unused_asset_lock = false;
            let mut has_balance = false;
            let mut has_platform_balance = false;

            for wallet in wallets.values() {
                let wallet = wallet.read_or_recover();
                if wallet.has_unused_asset_lock() {
                    has_unused_asset_lock = true;
                }
                if wallet.has_balance() {
                    has_balance = true;
                }
                if wallet.total_platform_balance() > 0 {
                    has_platform_balance = true;
                }
                if has_unused_asset_lock && has_balance && has_platform_balance {
                    break; // No need to check further
                }
            }

            (has_unused_asset_lock, has_balance, has_platform_balance)
        };

        ComboBox::from_id_salt("funding_method")
            .selected_text(format!("{}", *funding_method))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut *funding_method,
                    FundingMethod::NoSelection,
                    "Please select funding method",
                );

                ui.add_enabled_ui(has_any_unused_asset_lock, |ui| {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseUnusedAssetLock,
                            "Unused Asset Locks",
                        )
                        .changed()
                    {
                        let mut step = self.step.write_or_recover();
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                });

                ui.add_enabled_ui(has_any_balance, |ui| {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UseWalletBalance,
                            "Wallet Balance",
                        )
                        .changed()
                    {
                        let mut step = self.step.write_or_recover();
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                });

                ui.add_enabled_ui(has_any_platform_balance, |ui| {
                    if ui
                        .selectable_value(
                            &mut *funding_method,
                            FundingMethod::UsePlatformAddress,
                            "Platform Address",
                        )
                        .changed()
                    {
                        let mut step = self.step.write_or_recover();
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                });

                if ui
                    .selectable_value(
                        &mut *funding_method,
                        FundingMethod::AddressWithQRCode,
                        "Address with QR Code",
                    )
                    .changed()
                {
                    let mut step = self.step.write_or_recover();
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

                    let mut step = self.step.write_or_recover();
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

                let mut step = self.step.write_or_recover();
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
        let funding_method = *self.funding_method.read_or_recover();

        // Only apply max amount restriction when using wallet balance
        // For QR code funding, funds come from external source so no max applies
        let (max_amount, show_max_button, fee_hint) =
            if funding_method == FundingMethod::UseWalletBalance {
                let max_amount_duffs = self
                    .wallet
                    .as_ref()
                    .map(|w| w.read_or_recover().total_balance_duffs())
                    .unwrap_or(0);
                // Convert Duffs to Credits (1 Duff = 1000 Credits)
                let total_credits = max_amount_duffs * 1000;
                // Reserve estimated fees so "Max" doesn't exceed spendable amount
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_identity_topup();
                let max_with_fee_reserved = total_credits.saturating_sub(estimated_fee);
                (
                    Some(max_with_fee_reserved),
                    true,
                    Some(format!(
                        "~{} reserved for fees",
                        format_credits_as_dash(estimated_fee)
                    )),
                )
            } else {
                (None, false, None)
            };

        // Lazy initialization of the AmountInput component
        let amount_input = self.funding_amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::new_dash(0.0))
                .with_label("Amount:")
                .with_max_button(show_max_button)
                .with_max_amount(max_amount)
        });

        // Update max amount and button visibility in case funding method or wallet balance changed
        amount_input.set_max_amount(max_amount);
        amount_input.set_show_max_button(show_max_button);
        amount_input.set_max_exceeded_hint(fee_hint);

        let response = amount_input.show(ui);

        // Update the funding_amount_exact from the parsed amount
        if let Some(amount) = response.inner.parsed_amount {
            // Amount.value() returns credits, convert to duffs (divide by 1000)
            self.funding_amount_exact = Some(amount.value() / 1000);
            // Keep the string in sync for backward compatibility
            self.funding_amount = format!("{}", amount.value() as f64 / 100_000_000_000.0);
        } else {
            self.funding_amount_exact = None;
        }

        ui.add_space(10.0);
    }
}

impl ScreenLike for TopUpIdentityScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Error {
            self.error_message = Some(format!("Error topping up identity: {}", message));
            // Reset step so UI is not stuck on waiting messages
            let mut step = self.step.write_or_recover();
            if *step == WalletFundedScreenStep::WaitingForPlatformAcceptance
                || *step == WalletFundedScreenStep::WaitingForAssetLock
            {
                *step = WalletFundedScreenStep::ReadyToCreate;
            }
        } else {
            self.error_message = Some(message.to_string());
        }
    }
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::Identity(IdentityResult::ToppedUpIdentity(
            qualified_identity,
            fee_result,
        )) = backend_task_success_result
        {
            self.identity = qualified_identity;
            self.completed_fee_result = Some(fee_result);
            self.funding_address = None;
            self.funding_utxo = None;
            self.funding_amount.clear();
            self.funding_amount_exact = None;
            self.funding_amount_input = None;
            self.copied_to_clipboard = None;
            self.error_message = None;

            let mut step = self.step.write_or_recover();
            *step = WalletFundedScreenStep::Success;
            return;
        }

        let mut step = self.step.write_or_recover();
        let current_step = *step;
        match current_step {
            WalletFundedScreenStep::ChooseFundingMethod => {}
            WalletFundedScreenStep::WaitingOnFunds => {
                if let Some(funding_address) = self.funding_address.as_ref()
                    && let BackendTaskSuccessResult::Core(CoreResult::Item(
                        CoreItem::ReceivedAvailableUTXOTransaction(_, outpoints_with_addresses),
                    )) = &backend_task_success_result
                {
                    for (outpoint, tx_out, address) in outpoints_with_addresses {
                        if funding_address == address {
                            *step = WalletFundedScreenStep::FundsReceived;
                            self.funding_utxo = Some((*outpoint, tx_out.clone(), address.clone()))
                        }
                    }
                }
            }
            WalletFundedScreenStep::FundsReceived => {}
            WalletFundedScreenStep::ReadyToCreate => {}
            WalletFundedScreenStep::WaitingForAssetLock => {
                if let BackendTaskSuccessResult::Core(CoreResult::Item(
                    CoreItem::ReceivedAvailableUTXOTransaction(tx, _),
                )) = &backend_task_success_result
                    && let Some(TransactionPayload::AssetLockPayloadType(asset_lock_payload)) =
                        &tx.special_transaction_payload
                    && asset_lock_payload.credit_outputs.iter().any(|tx_out| {
                        let Ok(address) =
                            Address::from_script(&tx_out.script_pubkey, self.app_context.network)
                        else {
                            return false;
                        };
                        if let Some(wallet) = &self.wallet {
                            let wallet = wallet.read_or_recover();
                            wallet.known_addresses.contains_key(&address)
                        } else {
                            false
                        }
                    })
                {
                    *step = WalletFundedScreenStep::WaitingForPlatformAcceptance;
                }
            }
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {}
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
            let _dark_mode = ui.ctx().style().visuals.dark_mode;

            // Display error message at the top, outside of scroll area
            if let Some(error_message) = self.error_message.clone() {
                let message_color = egui::Color32::from_rgb(255, 100, 100);

                ui.horizontal(|ui| {
                    egui::Frame::new()
                        .fill(message_color.gamma_multiply(0.1))
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, message_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&error_message).color(message_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.error_message = None;
                                }
                            });
                        });
                });
                ui.add_space(10.0);
            }

            ScrollArea::vertical().show(ui, |ui| {
                let step = { *self.step.read_or_recover() };
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

                let mut step_number = 1;
                ui.heading(format!("{}. Choose your funding method.", step_number).as_str());
                step_number += 1;
                ui.add_space(10.0);

                self.render_funding_method(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Extract the funding method from the RwLock to minimize borrow scope
                let funding_method = *self.funding_method.read_or_recover();
                if funding_method == FundingMethod::NoSelection {
                    return;
                }

                if funding_method == FundingMethod::UseWalletBalance
                    || funding_method == FundingMethod::UseUnusedAssetLock
                    || funding_method == FundingMethod::AddressWithQRCode
                    || funding_method == FundingMethod::UsePlatformAddress
                {
                    // Check if there's more than one wallet to show selection UI
                    let wallet_count = self.app_context.wallets.read_or_recover().len();

                    if wallet_count > 1 {
                        ui.horizontal(|ui| {
                            ui.heading(format!(
                                "{}. Choose the wallet to use to top up this identity.",
                                step_number
                            ));
                            ui.add_space(10.0);

                            // Add info icon with hover tooltip and click popup
                            if crate::ui::helpers::info_icon_button(ui, WALLET_SELECTION_TOOLTIP)
                                .clicked()
                            {
                                self.show_pop_up_info = Some(WALLET_SELECTION_TOOLTIP.to_string());
                            }
                        });
                        step_number += 1;

                        ui.add_space(10.0);
                    }

                    self.render_wallet_selection(ui);

                    if self.wallet.is_none() {
                        return;
                    };

                    if let Some(wallet) = &self.wallet {
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
                            return;
                        }
                    }

                    if wallet_count > 1 {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                    }
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
                    FundingMethod::UsePlatformAddress => {
                        inner_action |= self.render_ui_by_platform_address(ui, step_number);
                    }
                }
            });

            inner_action
        });

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.wallet
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
                    let mut popup = InfoPopup::new("Wallet Selection Info", &show_pop_up_info_text);
                    if popup.show(ui).inner {
                        self.show_pop_up_info = None;
                    }
                });
        }

        action
    }
}
