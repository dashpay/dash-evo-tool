use super::TokenSigningKey;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use eframe::egui::{self, Ui};
use egui::RichText;

use super::tokens_screen::IdentityTokenInfo;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::SigningKeyStatus;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::helpers::TransactionType;
use crate::ui::identity::get_selected_wallet;
use crate::ui::identity::keys::add_key_screen::AddKeyScreen;
use crate::ui::identity::keys::key_info_screen::KeyInfoScreen;
use crate::ui::state::PendingRequest;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::tokens::validate_signing_key;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};

/// Internal states for the purchase process.
#[derive(PartialEq)]
pub enum PurchaseTokensStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

/// A UI Screen for purchasing tokens from an existing token contract
pub struct PurchaseTokenScreen {
    pub app_context: Arc<AppContext>,
    token_action_availability: crate::model::token::TokenActionAvailability,

    pub identity_token_info: IdentityTokenInfo,
    selected_key: TokenSigningKey,
    show_advanced_options: bool,

    // Specific to this transition - using AmountInput components following design pattern
    amount_to_purchase_input: Option<AmountInput>,
    amount_to_purchase_value: Option<Amount>,
    fetched_pricing_schedule: Option<TokenPricingSchedule>,
    calculated_price_credits: Option<Credits>,
    pricing_fetch_attempted: bool,

    /// Screen stuff
    confirmation_dialog: Option<ConfirmationDialog>,
    status: PurchaseTokensStatus,

    // Wallet fields
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    // Banner handle for elapsed time display
    /// The purchase in flight; only its own result or error ends the wait.
    purchase_request: PendingRequest,
    refresh_banner: Option<BannerHandle>,
}

impl PurchaseTokenScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let possible_key = identity_token_info
            .identity
            .token_action_key(identity_token_info.data_contract.contract.id())
            .cloned();

        // Attempt to get an unlocked wallet reference
        let selected_wallet = possible_key
            .as_ref()
            .map_or(Ok(None), |key| {
                get_selected_wallet(&identity_token_info.identity, None, Some(key))
            })
            .unwrap_or_else(|e| {
                MessageBanner::set_global(app_context.egui_ctx(), &e, MessageType::Error);
                None
            });

        let token_action_availability = super::load_token_action_availability(
            app_context,
            &identity_token_info.identity.identity.id(),
        );
        Self {
            identity_token_info,
            selected_key: TokenSigningKey::automatic(possible_key),
            show_advanced_options: false,
            amount_to_purchase_input: None,
            amount_to_purchase_value: None,
            fetched_pricing_schedule: None,
            calculated_price_credits: None,
            pricing_fetch_attempted: false,
            status: PurchaseTokensStatus::NotStarted,
            token_action_availability,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            completed_fee_result: None,
            purchase_request: PendingRequest::default(),
            refresh_banner: None,
        }
    }

    /// Renders AmountInput components for the user to specify an amount to purchase
    fn render_amount_input(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.horizontal(|ui| {
            // Use AmountInput for token amount with lazy initialization
            let amount_input = self.amount_to_purchase_input.get_or_insert_with(|| {
                AmountInput::new(
                    Amount::new(
                        0,
                        self.identity_token_info
                            .token_config
                            .conventions()
                            .decimals(),
                    )
                    .with_unit_name(&self.identity_token_info.token_alias),
                )
                .with_label("Amount to Purchase:")
                .with_hint_text("Enter token amount to purchase")
                .with_min_amount(Some(1))
            });

            let response = amount_input.show(ui);
            response.inner.update(&mut self.amount_to_purchase_value);

            // When amount changes, update domain data and recalculate the price
            if response.inner.has_changed() {
                self.recalculate_price();
                self.confirmation_dialog = None;
            }

            // Fetch pricing button
            if ui.button("Fetch Token Price").clicked() {
                let token_id_opt = self
                    .identity_token_info
                    .data_contract
                    .contract
                    .token_id(self.identity_token_info.token_position);

                if let Some(token_id) = token_id_opt {
                    action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                        TokenTask::QueryTokenPricing(token_id),
                    )));
                } else {
                    self.status = PurchaseTokensStatus::Error;
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Failed to get token ID from contract",
                        MessageType::Error,
                    );
                }
            }
        });

        // Show fetched pricing schedule
        if let Some(pricing_schedule) = &self.fetched_pricing_schedule {
            ui.add_space(5.0);
            ui.label("Current pricing:");
            let dark_mode = ui.style().visuals.dark_mode;

            match pricing_schedule {
                TokenPricingSchedule::SinglePrice(price_per_unit) => {
                    // Convert price per smallest unit to price per whole token for display, guarding for the minimal
                    // representable value (using Amount ref display which pads decimals properly)
                    if *price_per_unit == 0 {
                        ui.colored_label(
                            DashColors::error_color(dark_mode),
                            "  Fixed price: FREE (pricing schedule stores 0 credits per unit)",
                        );
                    } else {
                        let price_per_token = (*price_per_unit as u128)
                            .saturating_mul(self.token_decimal_multiplier() as u128)
                            .min(u64::MAX as u128)
                            as u64;
                        let price = Amount::new(price_per_token, DASH_DECIMAL_PLACES)
                            .with_unit_name("DASH");
                        ui.label(format!("  Fixed price: {} per token", price));
                    }
                }
                TokenPricingSchedule::SetPrices(tiers) => {
                    ui.label("  Tiered pricing:");
                    for (amount_value, price_per_unit) in tiers {
                        let amount = Amount::from_token(&self.identity_token_info, *amount_value);
                        // Convert price per smallest unit to price per token for display
                        if *price_per_unit == 0 {
                            ui.colored_label(
                                DashColors::error_color(dark_mode),
                                format!("    {} tokens: FREE (tier stores 0 credits)", amount),
                            );
                        } else {
                            let price_per_token = (*price_per_unit as u128)
                                .saturating_mul(self.token_decimal_multiplier() as u128)
                                .min(u64::MAX as u128)
                                as u64;
                            let price = Amount::new(price_per_token, DASH_DECIMAL_PLACES)
                                .with_unit_name("DASH");
                            ui.label(format!("    {} tokens: {} each", amount, price));
                        }
                    }
                }
            }
        }

        action
    }

    /// Recalculates the total price based on amount and pricing schedule
    fn recalculate_price(&mut self) {
        if let (Some(pricing_schedule), Some(amount_value)) = (
            &self.fetched_pricing_schedule,
            &self.amount_to_purchase_value,
        ) {
            let amount = amount_value.value();
            let price_per_unit = match pricing_schedule {
                TokenPricingSchedule::SinglePrice(price) => *price,
                TokenPricingSchedule::SetPrices(tiers) => {
                    // Find the appropriate tier for this amount
                    let mut applicable_price = 0u64;
                    for (tier_amount, tier_price) in tiers {
                        if amount >= *tier_amount {
                            applicable_price = *tier_price;
                        }
                    }
                    applicable_price
                }
            };

            // The price from Platform is per smallest unit, and amount is in smallest units
            // So we multiply them directly using wider arithmetic to avoid overflow
            let total_price = (amount as u128)
                .saturating_mul(price_per_unit as u128)
                .min(u64::MAX as u128) as u64;
            self.calculated_price_credits = Some(total_price);
        } else {
            self.calculated_price_credits = None;
        }
    }

    fn token_decimal_multiplier(&self) -> u64 {
        10u64.pow(
            self.identity_token_info
                .token_config
                .conventions()
                .decimals() as u32,
        )
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let Some(amount) = self.amount_to_purchase_value.as_ref() else {
            self.status = PurchaseTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please enter a valid amount.",
                MessageType::Error,
            );
            self.confirmation_dialog = None;
            return AppAction::None;
        };

        let Some(total_price_credits) = self.calculated_price_credits else {
            self.status = PurchaseTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Cannot calculate total price. Please fetch token pricing first.",
                MessageType::Error,
            );
            self.confirmation_dialog = None;
            return AppAction::None;
        };

        let Some(dialog) = self.confirmation_dialog.as_mut() else {
            return AppAction::None;
        };

        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                if self.purchase_request.is_pending() {
                    self.confirmation_dialog = None;
                    return AppAction::None;
                }
                let Some(signing_key) =
                    validate_signing_key(&self.app_context, self.selected_key.as_ref())
                else {
                    return AppAction::None;
                };

                self.confirmation_dialog = None;
                self.status = PurchaseTokensStatus::WaitingForResult;
                let handle = MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Purchasing tokens...",
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.refresh_banner = Some(handle);

                let purchase = BackendTask::TokenTask(Box::new(TokenTask::PurchaseTokens {
                    identity: self.identity_token_info.identity.clone(),
                    data_contract: Arc::new(
                        self.identity_token_info.data_contract.contract.clone(),
                    ),
                    token_position: self.identity_token_info.token_position,
                    signing_key,
                    amount: amount.value(),
                    total_agreed_price: total_price_credits,
                }));
                let refresh = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
                self.purchase_request.dispatch_action_on(
                    AppAction::BackendTasks(
                        vec![purchase, refresh],
                        BackendTasksExecutionMode::Sequential,
                    ),
                    self.app_context.network,
                )
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Purchase Successful!".to_string(),
            vec![("Back to Tokens".to_string(), AppAction::PopScreenAndRefresh)],
            None,
        )
    }
}

impl PurchaseTokenScreen {
    /// A price result only sets the status of a screen with no purchase in
    /// flight or completed; otherwise it would reopen the purchase form.
    fn price_may_change_status(&self) -> bool {
        !self.purchase_request.is_pending() && self.status != PurchaseTokensStatus::Complete
    }
}

impl ScreenLike for PurchaseTokenScreen {
    fn cached_data_changed(&mut self, update: crate::backend_task::CacheUpdate) {
        use crate::backend_task::CacheDomain;
        if update.network == self.app_context.network
            && matches!(update.domain, CacheDomain::Identities | CacheDomain::All)
        {
            self.refresh();
        }
    }
    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::TokenPricing {
                token_id: _,
                prices,
            } => {
                self.pricing_fetch_attempted = true;
                if let Some(schedule) = prices {
                    self.fetched_pricing_schedule = Some(schedule);
                    self.recalculate_price();
                    if self.price_may_change_status() {
                        self.status = PurchaseTokensStatus::NotStarted;
                    }
                } else {
                    // No pricing schedule found - token is not for sale
                    if self.price_may_change_status() {
                        self.status = PurchaseTokensStatus::Error;
                    }
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "This token is not available for direct purchase. No pricing has been set.",
                        MessageType::Error,
                    );
                }
            }
            BackendTaskSuccessResult::PurchasedTokens(fee_result) => {
                self.purchase_request.clear();
                self.refresh_banner.take_and_clear();
                self.completed_fee_result = Some(fee_result);
                self.status = PurchaseTokensStatus::Complete;
            }
            _ => {}
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.purchase_request.settle(context);
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        // After the purchase's own outcome, a later error (its balance refresh) changes nothing.
        if !self.purchase_request.is_pending()
            && self.status != PurchaseTokensStatus::Complete
            && matches!(message_type, MessageType::Error | MessageType::Warning)
        {
            self.refresh_banner.take_and_clear();
            self.status = PurchaseTokensStatus::Error;
        }
    }

    fn refresh(&mut self) {
        self.token_action_availability = super::load_token_action_availability(
            &self.app_context,
            &self.identity_token_info.identity.identity.id(),
        );
        // If you need to reload local identity data or re-check keys:
        if let Ok(all_identities) = self.app_context.load_local_qualified_identities()
            && let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity_token_info.identity.identity.id())
        {
            self.identity_token_info.identity = updated_identity;
        }
        let contract_id = self.identity_token_info.data_contract.contract.id();
        self.selected_key.refresh(
            &self.app_context,
            &self.identity_token_info.identity,
            TransactionType::TokenAction,
            contract_id,
        );
        crate::ui::identity::sync_signing_wallet(
            Some(&self.identity_token_info.identity),
            self.selected_key.as_ref(),
            &mut self.selected_wallet,
            &mut self.wallet_open_attempted,
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        // Build a top panel
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (&self.identity_token_info.token_alias, AppAction::PopScreen),
                ("Purchase", AppAction::None),
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

        island_central_panel(ui, |ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            // If we are in the "Complete" status, just show success screen
            if self.status == PurchaseTokensStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            // The page heading shares its row with the Advanced Options toggle.
            ui.horizontal(|ui| {
                ui.heading("Purchase Tokens");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                });
            });
            ui.add_space(10.0);

            super::recheck_token_action_availability(
                &self.app_context,
                &self.identity_token_info.identity.identity.id(),
                &mut self.token_action_availability,
            );
            if super::show_token_actions_unavailable(ui, self.token_action_availability) {
                return;
            }

            // Signing eligibility affects submission, not reads or previews.
            let key_status = super::token_action_key_status(
                &self.app_context,
                &self.identity_token_info.identity,
                self.identity_token_info.data_contract.contract.id(),
            );
            if self.selected_key.follow_automatic_with_wallet(
                &self.identity_token_info.identity,
                TransactionType::TokenAction,
                self.identity_token_info.data_contract.contract.id(),
                &mut self.selected_wallet,
                &mut self.wallet_open_attempted,
            ) {
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
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    super::token_action_key_message(key_status, &self.identity_token_info.identity),
                );
                ui.add_space(10.0);

                // Show "Add key" or "Check keys" option
                let first_key = self
                    .identity_token_info
                    .identity
                    .identity
                    .get_first_public_key_matching(
                        Purpose::AUTHENTICATION,
                        HashSet::from([SecurityLevel::CRITICAL]),
                        KeyType::all_key_types().into(),
                        false,
                    );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity_token_info.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity_token_info.identity.clone(),
                        &self.app_context,
                    )));
                }
            }
            {
                // Possibly handle locked wallet scenario (similar to TransferTokens)
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
                    ui.heading("1. Select the key to sign the Purchase transaction");
                    ui.add_space(10.0);
                    if key_status == SigningKeyStatus::ManualChoiceOnly {
                        ui.label(super::CONTRACT_GROUP_KEY_NOTE);
                        ui.add_space(5.0);
                    }
                    action |= self.selected_key.show_chooser(
                        ui,
                        &self.app_context,
                        &self.identity_token_info.identity,
                        TransactionType::TokenAction,
                        self.identity_token_info.data_contract.contract.id(),
                    );
                    ui.add_space(10.0);
                    ui.separator();
                }
                ui.add_space(10.0);

                crate::ui::identity::sync_signing_wallet(
                    Some(&self.identity_token_info.identity),
                    self.selected_key.as_ref(),
                    &mut self.selected_wallet,
                    &mut self.wallet_open_attempted,
                );
                // Amount to purchase
                let step_num = if self.show_advanced_options { 2 } else { 1 };
                ui.heading(format!("{}. Amount to purchase and price", step_num));
                ui.add_space(5.0);
                action |= self.render_amount_input(ui);

                ui.add_space(10.0);

                // Display calculated price and total agreed price input
                if let Some(calculated_price_credits) = self.calculated_price_credits {
                    ui.group(|ui| {
                        ui.heading("Calculated total price:");
                        let dash_amount = Amount::new(calculated_price_credits, DASH_DECIMAL_PLACES)
                            .with_unit_name("DASH");
                        ui.label(format!("{} DASH ({} credits)",dash_amount, calculated_price_credits));
                        ui.label("Note: This is the calculated price based on the current pricing schedule.");

                        ui.add_space(10.0);

                    });
                } else if self.fetched_pricing_schedule.is_some() {
                    ui.colored_label(
                        DashColors::error_color(dark_mode),
                        "Please enter a valid amount to see the price.",
                    );
                } else {
                    ui.label("Click 'Fetch Token Price' to retrieve current pricing.");
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display estimated fee before action button
                let estimated_fee = self.app_context.fee_estimator().estimate_token_transition();
                let dark_mode = ui.style().visuals.dark_mode;
                egui::Frame::new()
                    .fill(DashColors::surface(dark_mode))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated Fee:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format_credits_as_dash(estimated_fee))
                                    .color(DashColors::text_primary(dark_mode))
                                    .strong(),
                            );
                        });
                    });
                ui.add_space(10.0);

                // Purchase button (disabled if no valid amounts are available)
                let in_flight = self.purchase_request.is_pending();
                let can_purchase = !in_flight
                    && matches!(
                        key_status,
                        SigningKeyStatus::Usable | SigningKeyStatus::ManualChoiceOnly
                    )
                    && self.selected_key.as_ref().is_some()
                    && self
                        .selected_wallet
                        .as_ref()
                        .is_none_or(|wallet| !wallet_needs_unlock(wallet))
                    && self.fetched_pricing_schedule.is_some()
                    && self.calculated_price_credits.unwrap_or_default() > 0
                    && self
                        .amount_to_purchase_value
                        .as_ref()
                        .map(|v| v.value())
                        .unwrap_or_default()
                        > 0;
                let purchase_text = "Purchase".to_string();

                if can_purchase {
                    if ComponentStyles::add_primary_button(ui, purchase_text).clicked()
                        && self.confirmation_dialog.is_none()
                    {
                        if let (Some(amount), Some(total_price_credits)) = (
                            self.amount_to_purchase_value.as_ref(),
                            self.calculated_price_credits,
                        ) {
                            let total_price_dash =
                                Amount::new(total_price_credits, DASH_DECIMAL_PLACES)
                                    .with_unit_name("DASH");

                            self.confirmation_dialog = Some(ConfirmationDialog::new(
                                "Confirm Purchase".to_string(),
                                format!(
                                    "Are you sure you want to purchase {} for {} ({} Credits)?",
                                    amount, total_price_dash, total_price_credits
                                ),
                            ));
                        } else {
                            self.status = PurchaseTokensStatus::Error;
                            MessageBanner::set_global(
                                ui.ctx(),
                                "Cannot calculate total price. Please fetch token pricing first.",
                                MessageType::Error,
                            );
                        }
                    }
                } else {
                    ComponentStyles::add_primary_button_enabled(ui, false, purchase_text)
                        .disabled_tooltip(if in_flight {
                            "Wait for the current purchase to finish."
                        } else if self.pricing_fetch_attempted
                            && self.fetched_pricing_schedule.is_none()
                        {
                            "This token is not available for purchase"
                        } else {
                            "Fetch token price and enter amount first"
                        });
                }

                // Show confirmation dialog if it exists
                if can_purchase && self.confirmation_dialog.is_some() {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    PurchaseTokensStatus::NotStarted => {
                        // no-op
                    }
                    PurchaseTokensStatus::WaitingForResult => {
                        // Elapsed display is handled by the global MessageBanner
                    }
                    PurchaseTokensStatus::Error => {
                        // Error display is handled by the global MessageBanner
                    }
                    PurchaseTokensStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

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
    use crate::model::user_role::UserRole;
    use crate::ui::tokens::mint_tokens_screen::tests::{signing_key, token_info};
    use egui_kittest::kittest::Queryable;
    use std::{cell::RefCell, rc::Rc};

    /// A purchase is charged, so while one is in flight the Purchase button
    /// must not send another.
    #[test]
    fn a_purchase_in_flight_is_never_sent_twice() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        context.set_user_role(UserRole::Developer);
        let mut info = token_info(&context, None, false);
        info.identity.identity.add_public_key(signing_key());
        let mut screen = PurchaseTokenScreen::new(info.clone(), &context);
        screen.selected_key = TokenSigningKey {
            selected: Some(signing_key()),
            manually_selected: true,
        };
        screen.display_task_result(BackendTaskSuccessResult::TokenPricing {
            token_id: info.token_id,
            prices: Some(TokenPricingSchedule::SinglePrice(10)),
        });
        let purchases = Rc::new(RefCell::new(0));
        let seen = purchases.clone();
        let mut harness = crate::test_support::harness_builder_with_app_fonts()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                move |ui, screen: &mut PurchaseTokenScreen| {
                    if let AppAction::BackendTasksWithContexts(tasks, _) = screen.ui(ui) {
                        *seen.borrow_mut() += tasks
                            .iter()
                            .filter(|(task, _)| {
                                matches!(task, BackendTask::TokenTask(task)
                                    if matches!(**task, TokenTask::PurchaseTokens { .. }))
                            })
                            .count();
                    }
                },
                screen,
            );
        harness.run_steps(3);
        harness.get_by_label("Amount to Purchase:").focus();
        harness.run_steps(2);
        harness.get_by_label("Amount to Purchase:").type_text("5");
        harness.run_steps(3);
        for _ in 0..2 {
            harness
                .query_all_by_role_and_label(egui::accesskit::Role::Button, "Purchase")
                .last()
                .expect("the Purchase button")
                .click();
            harness.run_steps(3);
            if let Some(confirm) = harness.query_by_label("Confirm") {
                confirm.click();
                harness.run_steps(3);
            }
        }
        assert_eq!(*purchases.borrow(), 1, "one purchase while it is in flight");
    }

    /// Clicks Purchase, then Confirm.
    fn purchase(harness: &mut egui_kittest::Harness<'_, PurchaseTokenScreen>) {
        harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Purchase")
            .last()
            .expect("the Purchase button")
            .click();
        harness.run_steps(3);
        if let Some(confirm) = harness.query_by_label("Confirm") {
            confirm.click();
            harness.run_steps(3);
        }
    }

    /// Only the purchase's own outcome ends the wait: a price fetch or another
    /// task's error must not open Purchase again, while the purchase's own
    /// error does.
    #[test]
    fn only_the_purchase_itself_releases_the_purchase_button() {
        use crate::backend_task::BackendTaskContext;
        use crate::backend_task::error::TaskError;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        context.set_user_role(UserRole::Developer);
        let mut info = token_info(&context, None, false);
        info.identity.identity.add_public_key(signing_key());
        let mut screen = PurchaseTokenScreen::new(info.clone(), &context);
        screen.selected_key = TokenSigningKey {
            selected: Some(signing_key()),
            manually_selected: true,
        };
        let price = BackendTaskSuccessResult::TokenPricing {
            token_id: info.token_id,
            prices: Some(TokenPricingSchedule::SinglePrice(10)),
        };
        screen.display_task_result(price.clone());
        let sent = Rc::new(RefCell::new(Vec::new()));
        let seen = sent.clone();
        let is_purchase = |task: &BackendTask| {
            matches!(task, BackendTask::TokenTask(task)
                if matches!(**task, TokenTask::PurchaseTokens { .. }))
        };
        let mut harness = crate::test_support::harness_builder_with_app_fonts()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                move |ui, screen: &mut PurchaseTokenScreen| match screen.ui(ui) {
                    AppAction::BackendTasks(tasks, _) => seen
                        .borrow_mut()
                        .extend(tasks.iter().filter(|t| is_purchase(t)).map(|_| None)),
                    AppAction::BackendTasksWithContexts(tasks, _) => seen.borrow_mut().extend(
                        tasks
                            .iter()
                            .filter(|(t, _)| is_purchase(t))
                            .map(|(_, request)| Some(request.clone())),
                    ),
                    _ => {}
                },
                screen,
            );
        harness.run_steps(3);
        harness.get_by_label("Amount to Purchase:").focus();
        harness.run_steps(2);
        harness.get_by_label("Amount to Purchase:").type_text("5");
        harness.run_steps(3);
        purchase(&mut harness);
        assert_eq!(sent.borrow().len(), 1);

        harness.state_mut().display_task_result(price);
        let other = BackendTaskContext::for_dispatch_on(&BackendTask::None, context.network);
        harness
            .state_mut()
            .display_backend_task_error(&other, &TaskError::WalletStateInconsistent);
        harness
            .state_mut()
            .display_message("Fetching the price failed", MessageType::Error);
        harness.run_steps(2);
        purchase(&mut harness);
        assert_eq!(sent.borrow().len(), 1, "still one purchase in flight");

        let request = sent.borrow()[0].clone().expect("the purchase's context");
        harness
            .state_mut()
            .display_backend_task_error(&request, &TaskError::WalletStateInconsistent);
        harness
            .state_mut()
            .display_message("The purchase failed", MessageType::Error);
        harness.run_steps(2);
        purchase(&mut harness);
        assert_eq!(
            sent.borrow().len(),
            2,
            "a failed purchase can be tried again"
        );
    }

    use crate::model::amount::DASH_DECIMAL_PLACES;

    #[test]
    fn test_token_pricing_storage_and_calculation() {
        // Test how prices should be stored and calculated

        // Case 1: Token with 8 decimals (like the user's case)
        let token_decimals_8 = 8u8;
        let user_price_per_token_dash = 0.001; // User wants 0.001 DASH per token
        let user_price_per_token_credits =
            (user_price_per_token_dash * 10f64.powi(DASH_DECIMAL_PLACES as i32)) as u64;

        println!("Test 1 - Token with 8 decimals, price 0.001 DASH per token:");
        println!(
            "  User enters: {} DASH per token",
            user_price_per_token_dash
        );
        println!(
            "  In credits: {} credits per token",
            user_price_per_token_credits
        );

        // Platform expects price per smallest unit, not per token
        let decimal_divisor_8 = 10u64.pow(token_decimals_8 as u32);
        let platform_price_per_smallest_unit = user_price_per_token_credits / decimal_divisor_8;

        println!(
            "  Platform stores: {} credits per smallest unit",
            platform_price_per_smallest_unit
        );

        // When buying 1 token (100,000,000 smallest units)
        let tokens_to_buy = 1u64;
        let amount_smallest_units = tokens_to_buy * 10u64.pow(token_decimals_8 as u32);
        let total_price = amount_smallest_units * platform_price_per_smallest_unit;

        println!(
            "  Buying {} token ({} smallest units)",
            tokens_to_buy, amount_smallest_units
        );
        println!(
            "  Total: {} credits (should be {} credits for 0.001 DASH)",
            total_price, user_price_per_token_credits
        );

        assert_eq!(
            total_price, user_price_per_token_credits,
            "Total should match expected price"
        );

        // Case 2: Token with 2 decimals
        let token_decimals_2 = 2u8;
        let user_price_2 = 0.1; // 0.1 DASH per token
        let user_price_credits_2 = (user_price_2 * 10f64.powi(DASH_DECIMAL_PLACES as i32)) as u64;

        let divisor_2 = 10u64.pow(token_decimals_2 as u32);
        let platform_price_2 = user_price_credits_2 / divisor_2;

        // Buy 5 tokens
        let amount_2 = 5 * 10u64.pow(token_decimals_2 as u32); // 500 smallest units
        let total_2 = amount_2 * platform_price_2;

        println!("\nTest 2 - Token with 2 decimals, 5 tokens at 0.1 DASH each:");
        println!(
            "  Platform price: {} credits per smallest unit",
            platform_price_2
        );
        println!("  Total for 5 tokens: {} credits", total_2);

        assert_eq!(
            total_2,
            5 * user_price_credits_2,
            "Should be 0.5 DASH total"
        );

        // Case 3: Token with 0 decimals
        let _token_decimals_0 = 0u8;
        let user_price_0 = 0.05; // 0.05 DASH per token
        let user_price_credits_0 = (user_price_0 * 10f64.powi(DASH_DECIMAL_PLACES as i32)) as u64;

        // With 0 decimals, price per token = price per smallest unit
        let platform_price_0 = user_price_credits_0; // No division needed

        let amount_0 = 10; // 10 tokens = 10 smallest units (no decimals)
        let total_0 = amount_0 * platform_price_0;

        println!("\nTest 3 - Token with 0 decimals, 10 tokens at 0.05 DASH each:");
        println!(
            "  Platform price: {} credits per smallest unit",
            platform_price_0
        );
        println!("  Total for 10 tokens: {} credits", total_0);

        assert_eq!(
            total_0,
            10 * user_price_credits_0,
            "Should be 0.5 DASH total"
        );
    }
    #[test]
    fn interop_token_price_fetch_without_a_signing_key() {
        use super::*;
        use crate::model::qualified_contract::QualifiedContract;
        use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
        use egui_kittest::kittest::{NodeT, Queryable};
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let info = IdentityTokenInfo {
            token_id: dash_sdk::platform::Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity: crate::ui::tokens::tests::identity_with_expired_key(),
            data_contract: QualifiedContract {
                contract: ctx.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            token_position: 0,
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 1000.0))
            .build_ui_state(
                |ui, screen: &mut PurchaseTokenScreen| {
                    screen.ui(ui);
                },
                PurchaseTokenScreen::new(info, &ctx),
            );
        harness.run();
        assert!(
            !harness
                .get_by_label("Fetch Token Price")
                .accesskit_node()
                .is_disabled()
        );
        harness
            .state_mut()
            .display_task_result(BackendTaskSuccessResult::TokenPricing {
                token_id: dash_sdk::platform::Identifier::from([4; 32]),
                prices: Some(TokenPricingSchedule::SinglePrice(42)),
            });
        harness.run();
        assert!(harness.query_by_label("Current pricing:").is_some());
        assert!(harness.query_by_label_contains("Fixed price:").is_some());
        assert!(harness.query_all_by_label("Purchase").any(|button| {
            !button
                .accesskit_node()
                .data()
                .supports_action(egui::accesskit::Action::Click)
        }));
    }

    /// A failed balance refresh after the purchase went through must not
    /// bring the purchase form back.
    #[test]
    fn a_completed_purchase_stays_complete_when_its_balance_refresh_fails() {
        use crate::ui::tokens::mint_tokens_screen::tests::{report_error, unrelated_request};
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, None, false);
        let mut screen = PurchaseTokenScreen::new(info, &context);
        screen.display_task_result(BackendTaskSuccessResult::PurchasedTokens(
            crate::backend_task::FeeResult {
                estimated_fee: 1,
                actual_fee: Some(1),
            },
        ));
        report_error(&mut screen, &unrelated_request(context.network));
        assert!(screen.status == PurchaseTokensStatus::Complete);
    }

    /// A price that arrives after the purchase went through must not bring
    /// the purchase form back.
    #[test]
    fn a_late_price_keeps_a_completed_purchase_complete() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, None, false);
        let mut screen = PurchaseTokenScreen::new(info.clone(), &context);
        screen.display_task_result(BackendTaskSuccessResult::PurchasedTokens(
            crate::backend_task::FeeResult {
                estimated_fee: 1,
                actual_fee: Some(1),
            },
        ));
        for prices in [Some(TokenPricingSchedule::SinglePrice(10)), None] {
            screen.display_task_result(BackendTaskSuccessResult::TokenPricing {
                token_id: info.token_id,
                prices,
            });
            assert!(screen.status == PurchaseTokensStatus::Complete);
        }
    }
}
