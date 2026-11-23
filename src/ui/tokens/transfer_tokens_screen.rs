use super::TokenSigningKey;
use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::{QualifiedIdentity, SigningKeyStatus};
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
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt, ResultBannerExt};
use crate::ui::helpers::TransactionType;
use crate::ui::identity::keys::add_key_screen::AddKeyScreen;
use crate::ui::identity::keys::key_info_screen::KeyInfoScreen;
use crate::ui::state::PendingRequest;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};

use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Ui};
use eframe::egui::{Frame, Margin};
use egui::RichText;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::ui::identity::get_selected_wallet;
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
    identity: Option<QualifiedIdentity>,
    pub identity_token_balance: IdentityTokenBalance,
    known_identities: Vec<QualifiedIdentity>,
    selected_key: TokenSigningKey,
    show_advanced_options: bool,
    public_note: Option<String>,
    receiver_identity_id: String,
    amount: Option<Amount>,
    amount_input: Option<AmountInput>,
    transfer_tokens_status: TransferTokensStatus,
    max_amount: Amount,
    pub app_context: Arc<AppContext>,
    token_action_availability: crate::model::token::TokenActionAvailability,
    confirmation_dialog: Option<ConfirmationDialog>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    // Banner handle for elapsed time display
    refresh_banner: Option<BannerHandle>,
    /// The transfer in flight; only its own result or error ends the wait.
    transfer_request: PendingRequest,
}

impl TransferTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let known_identities = app_context
            .load_local_qualified_identities()
            .or_show_error(app_context.egui_ctx())
            .unwrap_or_default();

        let identity = known_identities
            .iter()
            .find(|identity| identity.identity.id() == identity_token_balance.identity_id)
            .cloned()
            .or_else(|| {
                MessageBanner::set_global(
                    app_context.egui_ctx(),
                    "Identity not found in local store — cannot open Transfer screen",
                    MessageType::Error,
                );
                None
            });

        let max_amount = Amount::from(&identity_token_balance);
        let selected_key: Option<IdentityPublicKey> = identity.as_ref().and_then(|id| {
            id.single_token_transition_key(identity_token_balance.data_contract_id)
                .cloned()
        });
        let selected_wallet = identity.as_ref().and_then(|id| {
            selected_key
                .as_ref()
                .map_or(Ok(None), |key| get_selected_wallet(id, None, Some(key)))
                .or_show_error(app_context.egui_ctx())
                .unwrap_or(None)
        });

        let amount = Some(Amount::from(&identity_token_balance).with_value(0));

        let token_action_availability =
            super::load_token_action_availability(app_context, &identity_token_balance.identity_id);
        Self {
            identity,
            identity_token_balance,
            known_identities,
            selected_key: TokenSigningKey::automatic(selected_key),
            show_advanced_options: false,
            public_note: None,
            receiver_identity_id: String::new(),
            amount,
            amount_input: None,
            transfer_tokens_status: TransferTokensStatus::NotStarted,
            max_amount,
            token_action_availability,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            completed_fee_result: None,
            refresh_banner: None,
            transfer_request: PendingRequest::default(),
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
        let exclude: Vec<_> = self
            .identity
            .as_ref()
            .map(|id| id.identity.id())
            .into_iter()
            .collect();
        let _response = ui.add(
            IdentitySelector::new(
                "transfer_recipient_selector",
                &mut self.receiver_identity_id,
                &self.known_identities,
            )
            .with_context(&self.app_context)
            .width(300.0)
            .label("Recipient:")
            .exclude(&exclude),
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
        if self.transfer_request.is_pending() {
            return AppAction::None;
        }
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
        let Some(signing_key) = validate_signing_key(&self.app_context, self.selected_key.as_ref())
        else {
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

        let Some(identity) = self.identity.clone() else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "No identity loaded — cannot transfer tokens",
                MessageType::Error,
            );
            return AppAction::None;
        };

        self.transfer_tokens_status = TransferTokensStatus::WaitingForResult;
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Transferring tokens...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.refresh_banner = Some(handle);

        let task = BackendTask::TokenTask(Box::new(TokenTask::TransferTokens {
            sending_identity: identity,
            recipient_id: receiver_id,
            amount: self.amount.clone().unwrap_or(Amount::new(0, 0)).value(),
            data_contract,
            token_position: self.identity_token_balance.token_position,
            signing_key,
            public_note: self.public_note.clone(),
        }));
        self.transfer_request
            .dispatch_on(task, self.app_context.network)
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
    fn cached_data_changed(&mut self, update: crate::backend_task::CacheUpdate) {
        use crate::backend_task::CacheDomain;
        if update.network == self.app_context.network
            && matches!(
                update.domain,
                CacheDomain::Tokens | CacheDomain::Identities | CacheDomain::All
            )
        {
            self.refresh();
        }
    }
    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.transfer_request.settle(context);
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        // After the transfer's own outcome, a later error changes nothing.
        if !self.transfer_request.is_pending()
            && self.transfer_tokens_status != TransferTokensStatus::Complete
            && matches!(message_type, MessageType::Error | MessageType::Warning)
        {
            self.refresh_banner.take_and_clear();
            self.transfer_tokens_status = TransferTokensStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::TransferredTokens(fee_result) = backend_task_success_result
        {
            self.transfer_request.clear();
            self.refresh_banner.take_and_clear();
            self.completed_fee_result = Some(fee_result);
            self.transfer_tokens_status = TransferTokensStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        self.token_action_availability = super::load_token_action_availability(
            &self.app_context,
            &self.identity_token_balance.identity_id,
        );
        // Refresh the identity because there might be new keys
        if let Some(current_id) = self.identity.as_ref().map(|id| id.identity.id()) {
            let all_ids = self
                .app_context
                .load_local_qualified_identities()
                .unwrap_or_else(|e| {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Failed to load local identities",
                        MessageType::Error,
                    )
                    .with_details(e);
                    vec![]
                });
            if let Some(refreshed) = all_ids
                .into_iter()
                .find(|identity| identity.identity.id() == current_id)
            {
                self.identity = Some(refreshed);
            }
        }
        if let Some(identity) = &self.identity
            && self.selected_key.refresh_with_wallet(
                &self.app_context,
                identity,
                TransactionType::TokenTransfer,
                self.identity_token_balance.data_contract_id,
                &mut self.selected_wallet,
                &mut self.wallet_open_attempted,
            )
        {
            self.confirmation_dialog = None;
        }
        if let Some(current_id) = self.identity.as_ref().map(|id| id.identity.id()) {
            match self.app_context.identity_token_balances() {
                Ok(token_balances) => {
                    self.max_amount = token_balances
                        .values()
                        .find(|balance| {
                            balance.identity_id == current_id
                                && balance.data_contract_id
                                    == self.identity_token_balance.data_contract_id
                                && balance.token_position
                                    == self.identity_token_balance.token_position
                        })
                        .map(Amount::from)
                        .unwrap_or_default();
                }
                Err(e) => {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Failed to load token balances",
                        MessageType::Error,
                    )
                    .with_details(e);
                }
            }
        }
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
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
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ui, &self.app_context);

        let central_panel_action = island_central_panel(ui, |ui| {
            let dark_mode = ui.style().visuals.dark_mode;

            // Show the success screen if the transfer was successful
            if self.transfer_tokens_status == TransferTokensStatus::Complete {
                return self.show_success(ui);
            }

            // Guard: require a loaded identity before rendering interactive content.
            // Use as_ref() to avoid a per-frame clone of the full QualifiedIdentity.
            let Some(identity) = self.identity.as_ref() else {
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    "No identity loaded. Please load an identity and reopen this screen.",
                );
                return AppAction::None;
            };

            // The page heading shares its row with the Advanced Options toggle.
            ui.horizontal(|ui| {
                ui.heading(format!(
                    "Transfer {}",
                    self.identity_token_balance.token_alias
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                });
            });
            ui.add_space(10.0);

            super::recheck_token_action_availability(
                &self.app_context,
                &self.identity_token_balance.identity_id,
                &mut self.token_action_availability,
            );
            if super::show_token_actions_unavailable(ui, self.token_action_availability) {
                return AppAction::None;
            }

            let followed = self.selected_key.follow_automatic(
                &self.app_context,
                identity,
                TransactionType::TokenTransfer,
                self.identity_token_balance.data_contract_id,
            );
            let key_status = followed.status;
            if followed.changed {
                // A pending confirmation was for the previous key.
                self.confirmation_dialog = None;
            }
            if key_status == SigningKeyStatus::ManualChoiceOnly {
                // Only a contract-group key can sign it: the user picks it below.
                self.show_advanced_options = true;
            }
            if !matches!(
                key_status,
                SigningKeyStatus::Usable | SigningKeyStatus::ManualChoiceOnly
            ) {
                let identity_type = identity.identity_type;
                // Extract key data before releasing the borrow (needed for click handlers).
                let key = [Purpose::AUTHENTICATION, Purpose::TRANSFER]
                    .into_iter()
                    .find_map(|purpose| {
                        identity.identity.get_first_public_key_matching(
                            purpose,
                            HashSet::from([SecurityLevel::CRITICAL]),
                            KeyType::all_key_types().into(),
                            false,
                        )
                    })
                    .cloned();
                let message = if key_status == SigningKeyStatus::HeldButUnusable {
                    "No key on this device can sign this token transfer. The identity's keys are disabled, expired, reserved for another app or token, or not available on this device. Add a new key, or load the wallet that holds one, then try again.".to_string()
                } else {
                    format!(
                        "You do not have a CRITICAL authentication or transfer key loaded for this {identity_type} identity."
                    )
                };
                ui.colored_label(DashColors::error_color(dark_mode), message);
                ui.add_space(10.0);

                if let Some(key) = key {
                    if ui.button("Check Keys").clicked() {
                        // Clone only on button click, not every frame.
                        let identity = self.identity.clone().expect("checked above");
                        return AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            identity,
                            key,
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    // Clone only on button click, not every frame.
                    let identity = self.identity.clone().expect("checked above");
                    return AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        identity,
                        &self.app_context,
                    )));
                }
            }
            {
                if let Some(wallet) = &self.selected_wallet {
                    if !self.wallet_open_attempted {
                        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                            MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                                .disable_auto_dismiss();
                        }
                        self.wallet_open_attempted = true;
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
                    }
                }

                // Key selection (only in advanced mode)
                if self.show_advanced_options {
                    ui.heading("1. Select the key to sign the transaction with");
                    ui.add_space(10.0);
                    if key_status == SigningKeyStatus::ManualChoiceOnly {
                        ui.label(super::CONTRACT_GROUP_KEY_NOTE);
                        ui.add_space(5.0);
                    }
                    action |= self.selected_key.show_chooser(
                        ui,
                        &self.app_context,
                        identity,
                        TransactionType::TokenTransfer,
                        self.identity_token_balance.data_contract_id,
                        &mut self.selected_wallet,
                        &mut self.wallet_open_attempted,
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                crate::ui::identity::sync_signing_wallet(
                    Some(identity),
                    self.selected_key.as_ref(),
                    &mut self.selected_wallet,
                    &mut self.wallet_open_attempted,
                );
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
                    let note_label = ui.label("Public note (optional):").info_tooltip(
                        "A note about the transaction that can be seen by the public.",
                    );
                    ui.add_space(10.0);
                    let mut txt = self.public_note.clone().unwrap_or_default();
                    if ui
                        .text_edit_singleline(&mut txt)
                        .labelled_by(note_label.id)
                        .changed()
                    {
                        self.public_note = Some(txt);
                    }
                });
                ui.add_space(10.0);

                // Fee estimation display
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_token_transition();

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

                let identity = self.identity.as_ref().expect("checked above");
                let has_enough_balance = identity.identity.balance() > estimated_fee;
                let in_flight = self.transfer_request.is_pending();
                let ready = matches!(
                    key_status,
                    SigningKeyStatus::Usable | SigningKeyStatus::ManualChoiceOnly
                ) && self.amount.is_some()
                    && !self.receiver_identity_id.is_empty()
                    && self.selected_key.as_ref().is_some()
                    && self
                        .selected_wallet
                        .as_ref()
                        .is_none_or(|wallet| !wallet_needs_unlock(wallet))
                    && has_enough_balance
                    && !in_flight;
                let hover_text = if in_flight {
                    "Wait for the current transfer to finish.".to_string()
                } else if !has_enough_balance {
                    format!(
                        "Insufficient identity balance for fee (need at least {})",
                        format_credits_as_dash(estimated_fee)
                    )
                } else if self.selected_key.as_ref().is_none() {
                    super::CHOOSE_SIGNING_KEY.to_string()
                } else {
                    "Please ensure all fields are filled correctly".to_string()
                };

                if ComponentStyles::add_primary_button_enabled(ui, ready, "Transfer")
                    .disabled_tooltip(&hover_text)
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

                if ready && self.confirmation_dialog.is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_contract::InsertTokensToo;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interop_transfer_cannot_confirm_after_the_signing_key_expires() {
        use crate::ui::tokens::tests::identity_with_expired_key;
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        use egui_kittest::{
            Harness,
            kittest::{NodeT, Queryable},
        };
        use std::cell::Cell;

        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let mut identity = identity_with_expired_key();
        identity.identity.set_balance(1_000_000_000_000);
        ctx.insert_local_qualified_identity(&identity, &None)
            .unwrap();
        let balance = IdentityTokenBalance {
            token_id: Identifier::from([4; 32]),
            token_alias: "Example".into(),
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            identity_id: identity.identity.id(),
            balance: 100,
            estimated_unclaimed_rewards: None,
            data_contract_id: ctx.dpns_contract().id(),
            token_position: 0,
        };
        let mut screen = TransferTokensScreen::new(balance, &ctx);
        screen.amount = Some(Amount::new(1, 0));
        screen.receiver_identity_id = Identifier::from([7; 32]).to_string(Encoding::Base58);
        screen.selected_key.selected = identity.identity.public_keys().get(&1).cloned();
        assert!(screen.selected_key.as_ref().is_some());
        screen.confirmation_dialog = Some(
            ConfirmationDialog::new("Confirm Transfer", "Send the token?")
                .confirm_text(Some("Confirm pending transfer")),
        );
        let dispatched = Cell::new(false);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, screen: &mut TransferTokensScreen| {
                    if let AppAction::BackendTaskWithContext {
                        task: BackendTask::TokenTask(task),
                        ..
                    } = screen.ui(ui)
                        && matches!(*task, TokenTask::TransferTokens { .. })
                    {
                        dispatched.set(true);
                    }
                },
                screen,
            );
        harness.run_steps(4);
        assert!(harness.query_by_label("Confirm pending transfer").is_none());
        assert!(harness.query_all_by_label("Transfer").any(|button| {
            !button
                .accesskit_node()
                .data()
                .supports_action(egui::accesskit::Action::Click)
        }));
        assert!(!dispatched.get());
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// An automatically chosen key that expires while the confirmation is
    /// open is never submitted, though another key still signs: the form
    /// switches to that key and asks for confirmation again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_automatic_key_expiring_under_the_confirmation_is_not_submitted() {
        use crate::context::test_support::{auth_key, identity_holding};
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        use egui_kittest::{Harness, kittest::Queryable};
        use std::cell::RefCell;

        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let expired = auth_key(1, SecurityLevel::CRITICAL).with_limits(None, Some(1));
        let live = auth_key(2, SecurityLevel::CRITICAL).with_limits(None, Some(u64::MAX));
        // The form submits only for a contract this device stores.
        ctx.insert_contract_if_not_exists(
            &ctx.dpns_contract(),
            None,
            crate::model::qualified_contract::InsertTokensToo::NoTokensShouldBeAdded,
        )
        .unwrap();
        let mut identity = identity_holding(&[expired.clone(), live]);
        identity.identity.set_balance(1_000_000_000_000);
        ctx.insert_local_qualified_identity(&identity, &None)
            .unwrap();
        let balance = IdentityTokenBalance {
            token_id: Identifier::from([4; 32]),
            token_alias: "Example".into(),
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            identity_id: identity.identity.id(),
            balance: 100,
            estimated_unclaimed_rewards: None,
            data_contract_id: ctx.dpns_contract().id(),
            token_position: 0,
        };
        let mut screen = TransferTokensScreen::new(balance, &ctx);
        screen.amount = Some(Amount::new(1, 0));
        screen.receiver_identity_id = Identifier::from([7; 32]).to_string(Encoding::Base58);
        // Key 1 was the automatic choice before it expired.
        screen.selected_key = TokenSigningKey::automatic(Some(expired));
        screen.confirmation_dialog = Some(
            ConfirmationDialog::new("Confirm Transfer", "Send the token?")
                .confirm_text(Some("Confirm pending transfer")),
        );
        let submitted = RefCell::new(Vec::new());
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, screen: &mut TransferTokensScreen| {
                    if let AppAction::BackendTaskWithContext {
                        task: BackendTask::TokenTask(task),
                        ..
                    } = screen.ui(ui)
                        && let TokenTask::TransferTokens { signing_key, .. } = *task
                    {
                        submitted.borrow_mut().push(signing_key.id());
                    }
                },
                screen,
            );
        harness.run_steps(4);
        if let Some(confirm) = harness.query_by_label("Confirm pending transfer") {
            confirm.click();
            harness.run_steps(4);
        }
        assert_eq!(*submitted.borrow(), Vec::<u32>::new());
        assert_eq!(
            harness.state().selected_key.as_ref().map(|key| key.id()),
            Some(2)
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// A transfer is irreversible, so a second confirmation while the first
    /// is in flight must not send it again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_transfer_in_flight_is_never_sent_twice() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let contract = context.dpns_contract();
        context
            .insert_contract_if_not_exists(&contract, None, InsertTokensToo::NoTokensShouldBeAdded)
            .unwrap();
        let identity = QualifiedIdentity::from_bytes(
            &crate::database::test_helpers::basic_legacy_identity_blob(
                [3; 32],
                None,
                context.network,
            ),
        )
        .unwrap();
        let balance = IdentityTokenBalance {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            identity_id: identity.identity.id(),
            balance: 100,
            estimated_unclaimed_rewards: None,
            data_contract_id: contract.id(),
            token_position: 0,
        };
        let mut screen = TransferTokensScreen::new(balance, &context);
        screen.identity = Some(identity);
        screen.selected_key = TokenSigningKey::automatic(Some(
            IdentityPublicKeyV0 {
                id: 0,
                key_type: KeyType::ECDSA_HASH160,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::CRITICAL,
                read_only: false,
                data: BinaryData::new(vec![1; 20]),
                disabled_at: None,
                contract_bounds: None,
            }
            .into(),
        ));
        screen.amount = Some(Amount::new(5, 0));
        screen.receiver_identity_id = Identifier::from([5; 32]).to_string(Encoding::Base58);

        use crate::ui::tokens::mint_tokens_screen::tests::{
            report_error, sent_request, unrelated_request,
        };
        let request = sent_request(&screen.confirmation_ok()).expect("the transfer is sent");
        assert!(
            sent_request(&screen.confirmation_ok()).is_none(),
            "a second confirmation while waiting must not send it again"
        );
        report_error(&mut screen, &unrelated_request(context.network));
        assert!(
            sent_request(&screen.confirmation_ok()).is_none(),
            "another task's error does not end the wait"
        );
        report_error(&mut screen, &request.expect("the transfer's own context"));
        assert!(
            sent_request(&screen.confirmation_ok()).is_some(),
            "a failed transfer can be sent again"
        );
    }

    /// An error that arrives after the transfer went through, such as a
    /// background task's, must not replace the success screen.
    #[test]
    fn a_completed_transfer_stays_complete_when_another_task_fails() {
        use crate::ui::tokens::mint_tokens_screen::tests::{report_error, unrelated_request};
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let balance = IdentityTokenBalance {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            identity_id: Identifier::from([3; 32]),
            balance: 100,
            estimated_unclaimed_rewards: None,
            data_contract_id: Identifier::from([6; 32]),
            token_position: 0,
        };
        let mut screen = TransferTokensScreen::new(balance, &context);
        screen.display_task_result(BackendTaskSuccessResult::TransferredTokens(FeeResult {
            estimated_fee: 1,
            actual_fee: Some(1),
        }));
        report_error(&mut screen, &unrelated_request(context.network));
        assert!(screen.transfer_tokens_status == TransferTokensStatus::Complete);
    }

    /// Regression: with no key selected (here only a contract-group key, for
    /// the user to choose) the disabled Transfer button asked to fill in the
    /// fields, though they were filled in.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_transfer_without_a_signing_key_asks_for_one() {
        use crate::context::test_support::{auth_key, bound_to, identity_holding};
        use dash_sdk::dpp::identity::accessors::IdentitySettersV0;
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        use egui_kittest::{Harness, kittest::Queryable};

        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let mut identity = identity_holding(&[bound_to(
            auth_key(1, SecurityLevel::CRITICAL),
            ContractBounds::ContractGroup {
                id: Identifier::from([0x6A; 32]),
            },
        )]);
        identity.identity.set_balance(1_000_000_000_000);
        ctx.insert_local_qualified_identity(&identity, &None)
            .unwrap();
        let balance = IdentityTokenBalance {
            token_id: Identifier::from([4; 32]),
            token_alias: "Example".into(),
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            identity_id: identity.identity.id(),
            balance: 100,
            estimated_unclaimed_rewards: None,
            data_contract_id: ctx.dpns_contract().id(),
            token_position: 0,
        };
        let mut screen = TransferTokensScreen::new(balance, &ctx);
        assert!(screen.selected_key.as_ref().is_none());
        screen.amount = Some(Amount::new(1, 0));
        screen.receiver_identity_id = Identifier::from([7; 32]).to_string(Encoding::Base58);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, screen: &mut TransferTokensScreen| {
                    screen.ui(ui);
                },
                screen,
            );
        harness.run_steps(4);
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Transfer")
            .last()
            .expect("the Transfer button")
            .hover();
        harness.run();
        assert!(
            harness
                .query_by_label(super::super::CHOOSE_SIGNING_KEY)
                .is_some()
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }
}
