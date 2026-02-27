use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner};
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};

use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Ui};
use eframe::egui::{Frame, Margin};
use egui::{Color32, RichText};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::ui::identities::get_selected_wallet;
use crate::ui::tokens::validate_signing_key;

use super::tokens_screen::IdentityTokenBalance;

#[derive(PartialEq)]
pub enum TransferTokensStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

pub struct TransferTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_balance: IdentityTokenBalance,
    known_identities: Vec<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,
    pub public_note: Option<String>,
    pub receiver_identity_id: String,
    pub amount: Option<Amount>,
    pub amount_input: Option<AmountInput>,
    transfer_tokens_status: TransferTokensStatus,
    max_amount: Amount,
    pub app_context: Arc<AppContext>,
    confirmation_dialog: Option<ConfirmationDialog>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    // Banner handle for elapsed time display
    refresh_banner: Option<BannerHandle>,
}

impl TransferTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let known_identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_else(|e| {
                MessageBanner::set_global(
                    app_context.egui_ctx(),
                    format!("Failed to load identities: {e}"),
                    MessageType::Error,
                );
                vec![]
            });

        let identity = known_identities
            .iter()
            .find(|identity| identity.identity.id() == identity_token_balance.identity_id)
            .expect("Identity must exist in local store after successful navigation")
            .clone();
        let max_amount = Amount::from(&identity_token_balance);
        let identity_clone = identity.identity.clone();
        let selected_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL]),
            KeyType::all_key_types().into(),
            false,
        );
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key).unwrap_or_else(|e| {
                MessageBanner::set_global(app_context.egui_ctx(), &e, MessageType::Error);
                None
            });

        let amount = Some(Amount::from(&identity_token_balance).with_value(0));

        Self {
            identity,
            identity_token_balance,
            known_identities,
            selected_key: selected_key.cloned(),
            show_advanced_options: false,
            public_note: None,
            receiver_identity_id: String::new(),
            amount,
            amount_input: None,
            transfer_tokens_status: TransferTokensStatus::NotStarted,
            max_amount,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
            refresh_banner: None,
        }
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.label(format!("Available balance: {}", self.max_amount));
        ui.add_space(5.0);

        // Lazy initialization with proper decimal places
        let amount_input = match self.amount_input.as_mut() {
            Some(input) => input,
            _ => {
                self.amount_input = Some(
                    AmountInput::new(
                        self.amount
                            .as_ref()
                            .unwrap_or(&Amount::from(&self.identity_token_balance)),
                    )
                    .with_label("Amount:")
                    .with_max_button(true),
                );

                self.amount_input
                    .as_mut()
                    .expect("AmountInput should be initialized above")
            }
        };

        // Check if input should be disabled when operation is in progress
        let enabled = match self.transfer_tokens_status {
            TransferTokensStatus::WaitingForResult | TransferTokensStatus::Complete => false,
            TransferTokensStatus::NotStarted | TransferTokensStatus::Error => {
                amount_input.set_max_amount(Some(self.max_amount.value()));
                true
            }
        };

        let response = ui.add_enabled_ui(enabled, |ui| amount_input.show(ui)).inner;

        response.inner.update(&mut self.amount);
        // errors are handled inside AmountInput
    }

    fn render_to_identity_input(&mut self, ui: &mut Ui) {
        let _response = ui.add(
            IdentitySelector::new(
                "transfer_recipient_selector",
                &mut self.receiver_identity_id,
                &self.known_identities,
            )
            .width(300.0)
            .label("Recipient:")
            .exclude(&[self.identity.identity.id()]),
        );
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let msg = format!(
            "Are you sure you want to transfer {} tokens to {}?",
            self.amount.clone().unwrap_or(Amount::new(0, 0)),
            self.receiver_identity_id
        );

        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm Transfer", msg)
                .confirm_text(Some("Transfer"))
                .cancel_text(Some("Cancel"))
        });

        let response = confirmation_dialog.show(ui);
        match response.inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.confirmation_dialog = None;
                self.confirmation_ok()
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    fn confirmation_ok(&mut self) -> AppAction {
        if self.amount.is_none() || self.amount == Some(Amount::new(0, 0)) {
            self.transfer_tokens_status = TransferTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Invalid amount",
                MessageType::Error,
            );
            return AppAction::None;
        }

        let Ok(receiver_id) = Identifier::from_string_try_encodings(
            &self.receiver_identity_id,
            &[
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex,
            ],
        ) else {
            self.transfer_tokens_status = TransferTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Invalid receiver",
                MessageType::Error,
            );
            return AppAction::None;
        };

        // Validate signing key before transitioning to waiting state
        let Some(signing_key) = validate_signing_key(&self.app_context, &self.selected_key) else {
            return AppAction::None;
        };

        let data_contract = match self
            .app_context
            .get_unqualified_contract_by_id(&self.identity_token_balance.data_contract_id)
        {
            Ok(Some(contract)) => Arc::new(contract),
            _ => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Data contract not found",
                    MessageType::Error,
                );
                return AppAction::None;
            }
        };

        self.transfer_tokens_status = TransferTokensStatus::WaitingForResult;
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Transferring tokens...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.refresh_banner = Some(handle);

        AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::TransferTokens {
                sending_identity: self.identity.clone(),
                recipient_id: receiver_id,
                amount: self.amount.clone().unwrap_or(Amount::new(0, 0)).value(),
                data_contract,
                token_position: self.identity_token_balance.token_position,
                signing_key,
                public_note: self.public_note.clone(),
            },
        )))
    }
    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Transfer Successful!".to_string(),
            vec![("Back to Tokens".to_string(), AppAction::PopScreenAndRefresh)],
            None,
        )
    }
}

impl ScreenLike for TransferTokensScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if let MessageType::Error = message_type {
            if let Some(h) = self.refresh_banner.take() {
                h.clear();
            }
            self.transfer_tokens_status = TransferTokensStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::TransferredTokens(fee_result) = backend_task_success_result
        {
            if let Some(h) = self.refresh_banner.take() {
                h.clear();
            }
            self.completed_fee_result = Some(fee_result);
            self.transfer_tokens_status = TransferTokensStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        // Refresh the identity because there might be new keys
        if let Some(refreshed) = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_else(|e| {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Failed to load local identities: {e}"),
                    MessageType::Error,
                );
                vec![]
            })
            .into_iter()
            .find(|identity| identity.identity.id() == self.identity.identity.id())
        {
            self.identity = refreshed;
        }
        match self
            .app_context
            .db
            .get_identity_token_balances(&self.app_context)
        {
            Ok(token_balances) => {
                self.max_amount = token_balances
                    .values()
                    .find(|balance| balance.identity_id == self.identity.identity.id())
                    .map(Amount::from)
                    .unwrap_or_default();
            }
            Err(e) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Failed to load token balances: {e}"),
                    MessageType::Error,
                );
            }
        }
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_balance.token_alias,
                    AppAction::PopScreen,
                ),
                ("Transfer", AppAction::None),
            ],
            vec![],
        );

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        let central_panel_action = island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // Show the success screen if the transfer was successful
            if self.transfer_tokens_status == TransferTokensStatus::Complete {
                return self.show_success(ui);
            }

            ui.heading(format!(
                "Transfer {}",
                self.identity_token_balance.token_alias
            ));
            ui.add_space(10.0);

            let has_keys = if self.app_context.is_developer_mode() {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self
                    .identity
                    .available_authentication_keys_with_critical_security_level()
                    .is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    format!(
                        "You do not have any authentication keys with CRITICAL security level loaded for this {} identity.",
                        self.identity.identity_type
                    ),
                );
                ui.add_space(10.0);

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Keys").clicked() {
                        return AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    return AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                if let Some(wallet) = &self.selected_wallet {
                    if let Err(e) = try_open_wallet_no_password(wallet) {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
                    }
                    if wallet_needs_unlock(wallet) {
                        ui.add_space(10.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to continue.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                        return AppAction::None;
                    }
                }

                // Header with Advanced Options checkbox
                ui.horizontal(|ui| {
                    ui.heading("Transfer Tokens");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Key selection (only in advanced mode)
                if self.show_advanced_options {
                    ui.heading("1. Select the key to sign the transaction with");
                    ui.add_space(10.0);
                    add_key_chooser(
                        ui,
                        &self.app_context,
                        &self.identity,
                        &mut self.selected_key,
                        TransactionType::TokenTransfer,
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                // Input the amount to transfer
                let step_num = if self.show_advanced_options { "2" } else { "1" };
                ui.heading(format!("{}. Input the amount to transfer", step_num));
                ui.add_space(5.0);

                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the ID of the identity to transfer to
                let step_num = if self.show_advanced_options { "3" } else { "2" };
                ui.heading(format!("{}. ID of the identity to transfer to", step_num));
                ui.add_space(5.0);
                self.render_to_identity_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                let step_num = if self.show_advanced_options { "4" } else { "3" };
                ui.heading(format!("{}. Public note (optional)", step_num));
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Public note (optional):");
                    ui.add_space(10.0);
                    let mut txt = self.public_note.clone().unwrap_or_default();
                    if ui
                        .text_edit_singleline(&mut txt)
                        .on_hover_text(
                            "A note about the transaction that can be seen by the public.",
                        )
                        .changed()
                    {
                        self.public_note = Some(txt);
                    }
                });
                ui.add_space(10.0);

                // Fee estimation display
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_document_batch(1); // Token transfers are document batch transitions

                Frame::new()
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated fee:")
                                    .color(DashColors::text_secondary(dark_mode))
                                    .size(14.0),
                            );
                            ui.label(
                                RichText::new(format_credits_as_dash(estimated_fee))
                                    .color(DashColors::text_primary(dark_mode))
                                    .size(14.0),
                            );
                        });
                    });

                ui.add_space(10.0);

                // Transfer button

                let has_enough_balance = self.identity.identity.balance() > estimated_fee;
                let ready = self.amount.is_some()
                    && !self.receiver_identity_id.is_empty()
                    && self.selected_key.is_some()
                    && has_enough_balance;
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button = egui::Button::new(RichText::new("Transfer").color(Color32::WHITE))
                    .fill(DashColors::ACTION_BUTTON_BLUE)
                    .frame(true)
                    .corner_radius(3.0);
                let hover_text = if !has_enough_balance {
                    format!(
                        "Insufficient identity balance for fee (need at least {})",
                        format_credits_as_dash(estimated_fee)
                    )
                } else {
                    "Please ensure all fields are filled correctly".to_string()
                };

                if ui
                    .add_enabled(ready, button)
                    .on_disabled_hover_text(&hover_text)
                    .clicked()
                {
                    // Use the amount value directly since it's already parsed
                    if self.amount.as_ref().is_some_and(|v| v > &self.max_amount) {
                        self.transfer_tokens_status = TransferTokensStatus::Error;
                        MessageBanner::set_global(
                            ui.ctx(),
                            "Amount exceeds available balance",
                            MessageType::Error,
                        );
                    } else if self.amount.as_ref().is_none_or(|a| a.value() == 0) {
                        self.transfer_tokens_status = TransferTokensStatus::Error;
                        MessageBanner::set_global(
                            ui.ctx(),
                            "Amount must be greater than zero",
                            MessageType::Error,
                        );
                    } else {
                        let msg = format!(
                            "Are you sure you want to transfer {} tokens to {}?",
                            self.amount.clone().unwrap_or(Amount::new(0, 0)),
                            self.receiver_identity_id
                        );
                        self.confirmation_dialog = Some(
                            ConfirmationDialog::new("Confirm Transfer", msg)
                                .confirm_text(Some("Transfer"))
                                .cancel_text(Some("Cancel")),
                        );
                    }
                }

                if self.confirmation_dialog.is_some() {
                    return self.show_confirmation_popup(ui);
                }

                // Handle transfer status messages
                ui.add_space(5.0);
                match &self.transfer_tokens_status {
                    TransferTokensStatus::NotStarted => {
                        // Do nothing
                    }
                    TransferTokensStatus::WaitingForResult => {
                        // Elapsed display is handled by the global MessageBanner
                    }
                    TransferTokensStatus::Error => {
                        // Error display is handled by the global MessageBanner
                    }
                    TransferTokensStatus::Complete => {
                        // Handled above
                    }
                }
            }

            AppAction::None
        });
        action |= central_panel_action;

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        action
    }
}
