use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;

use super::tokens_screen::IdentityTokenInfo;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::components::{Component, ComponentResponse};
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::DashColors;
use crate::ui::{BackendTaskSuccessResult, MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::IdentityPublicKey;

/// Internal states for the purchase process.
#[derive(PartialEq)]
pub enum PurchaseTokensStatus {
    NotStarted,
    WaitingForResult(u64), // Use seconds or millis
    ErrorMessage(String),
    Complete,
}

/// A UI Screen for purchasing tokens from an existing token contract
pub struct PurchaseTokenScreen {
    pub app_context: Arc<AppContext>,

    pub identity_token_info: IdentityTokenInfo,
    selected_key: Option<IdentityPublicKey>,

    // Specific to this transition - using AmountInput components following design pattern
    amount_to_purchase_input: Option<AmountInput>,
    amount_to_purchase_value: Option<Amount>,
    fetched_pricing_schedule: Option<TokenPricingSchedule>,
    calculated_price_credits: Option<Credits>,
    pricing_fetch_attempted: bool,

    /// Screen stuff
    show_confirmation_popup: bool,
    status: PurchaseTokensStatus,
    error_message: Option<String>,

    // Wallet fields
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl PurchaseTokenScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let possible_key = identity_token_info
            .identity
            .identity
            .get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            )
            .cloned();

        let mut error_message = None;

        // Attempt to get an unlocked wallet reference
        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key.as_ref(),
            &mut error_message,
        );

        Self {
            identity_token_info,
            selected_key: possible_key,
            amount_to_purchase_input: None,
            amount_to_purchase_value: None,
            fetched_pricing_schedule: None,
            calculated_price_credits: None,
            pricing_fetch_attempted: false,
            status: PurchaseTokensStatus::NotStarted,
            error_message: None,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
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
                    self.error_message = Some("Failed to get token ID from contract".to_string());
                    self.status = PurchaseTokensStatus::ErrorMessage(
                        "Failed to get token ID from contract".to_string(),
                    );
                }
            }
        });

        // Show fetched pricing schedule
        if let Some(pricing_schedule) = &self.fetched_pricing_schedule {
            ui.add_space(5.0);
            ui.label("Current pricing:");
            let token_decimals = self
                .identity_token_info
                .token_config
                .conventions()
                .decimals();
            let decimal_multiplier = 10u64.pow(token_decimals as u32);

            match pricing_schedule {
                TokenPricingSchedule::SinglePrice(price_per_smallest_unit) => {
                    // Convert price per smallest unit to price per token for display
                    let price_per_token = price_per_smallest_unit * decimal_multiplier;
                    let price =
                        Amount::new(price_per_token, DASH_DECIMAL_PLACES).with_unit_name("DASH");
                    ui.label(format!("  Fixed price: {} per token", price));
                }
                TokenPricingSchedule::SetPrices(tiers) => {
                    ui.label("  Tiered pricing:");
                    for (amount_value, price_per_smallest_unit) in tiers {
                        let amount = Amount::from_token(&self.identity_token_info, *amount_value);
                        // Convert price per smallest unit to price per token for display
                        let price_per_token = price_per_smallest_unit * decimal_multiplier;
                        let price = Amount::new(price_per_token, DASH_DECIMAL_PLACES)
                            .with_unit_name("DASH");
                        ui.label(format!("    {} tokens: {} each", amount, price));
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
            let price_per_token = match pricing_schedule {
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
            // So we just multiply them directly
            let total_price = amount.saturating_mul(price_per_token);
            self.calculated_price_credits = Some(total_price);
        } else {
            self.calculated_price_credits = None;
        }
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Purchase")
            .collapsible(false)
            .open(&mut is_open)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(245, 245, 245))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(200, 200, 200),
                    ))
                    .shadow(egui::epaint::Shadow::default())
                    .inner_margin(egui::Margin::same(20))
                    .corner_radius(egui::CornerRadius::same(8)),
            )
            .show(ui.ctx(), |ui| {
                // Validate user input
                let amount_value = self.amount_to_purchase_value.as_ref();
                let Some(amount) = amount_value else {
                    self.error_message = Some("Please enter a valid amount.".into());
                    self.status = PurchaseTokensStatus::ErrorMessage("Invalid amount".into());
                    self.show_confirmation_popup = false;
                    return;
                };

                let Some(total_price_credits) = self.calculated_price_credits else {
                    self.error_message = Some(
                        "Cannot calculate total price. Please fetch token pricing first.".into(),
                    );
                    self.status = PurchaseTokensStatus::ErrorMessage("No pricing fetched".into());
                    self.show_confirmation_popup = false;
                    return;
                };

                let total_price_dash =
                    Amount::new(total_price_credits, DASH_DECIMAL_PLACES).with_unit_name("DASH");

                ui.label(format!(
                    "Are you sure you want to purchase {} token(s) for {} ({} Credits)?",
                    amount, total_price_dash, total_price_credits
                ));

                ui.add_space(10.0);

                // Confirm button
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = PurchaseTokensStatus::WaitingForResult(now);

                    // Dispatch the actual backend purchase action
                    action = AppAction::BackendTasks(
                        vec![
                            BackendTask::TokenTask(Box::new(TokenTask::PurchaseTokens {
                                identity: self.identity_token_info.identity.clone(),
                                data_contract: Arc::new(
                                    self.identity_token_info.data_contract.contract.clone(),
                                ),
                                token_position: self.identity_token_info.token_position,
                                signing_key: self.selected_key.clone().expect("Expected a key"),
                                amount: amount.value(),
                                total_agreed_price: total_price_credits,
                            })),
                            BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                        ],
                        BackendTasksExecutionMode::Sequential,
                    );
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.show_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_confirmation_popup = false;
        }
        action
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Purchase Successful!");

            ui.add_space(20.0);

            if ui.button("Back to Tokens").clicked() {
                // Pop this screen and refresh
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }
}

impl ScreenLike for PurchaseTokenScreen {
    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::TokenPricing {
            token_id: _,
            prices,
        } = result
        {
            self.pricing_fetch_attempted = true;
            if let Some(schedule) = prices {
                self.fetched_pricing_schedule = Some(schedule);
                self.recalculate_price();
                self.status = PurchaseTokensStatus::NotStarted;
            } else {
                // No pricing schedule found - token is not for sale
                self.status = PurchaseTokensStatus::ErrorMessage(
                    "This token is not available for direct purchase. No pricing has been set."
                        .to_string(),
                );
                self.error_message = Some(
                    "This token is not available for direct purchase. No pricing has been set."
                        .to_string(),
                );
            }
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Successfully purchaseed tokens") || message == "PurchaseTokens"
                {
                    self.status = PurchaseTokensStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = PurchaseTokensStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {
        // If you need to reload local identity data or re-check keys:
        if let Ok(all_identities) = self.app_context.load_local_user_identities() {
            if let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity_token_info.identity.identity.id())
            {
                self.identity_token_info.identity = updated_identity;
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        // Build a top panel
        let mut action = add_top_panel(
            ctx,
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
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            // If we are in the "Complete" status, just show success screen
            if self.status == PurchaseTokensStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Purchase Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.is_developer_mode() {
                !self
                    .identity_token_info
                    .identity
                    .identity
                    .public_keys()
                    .is_empty()
            } else {
                !self
                    .identity_token_info
                    .identity
                    .available_authentication_keys_with_critical_security_level()
                    .is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    format!(
                        "No authentication keys with CRITICAL security level found for this {} identity.",
                        self.identity_token_info.identity.identity_type,
                    ),
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
            } else {
                // Possibly handle locked wallet scenario (similar to TransferTokens)
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        // Must unlock before we can proceed
                        return;
                    }
                }

                // 1) Key selection
                ui.heading("1. Select the key to sign the Purchase transaction");
                ui.add_space(10.0);

                let mut selected_identity = Some(self.identity_token_info.identity.clone());
                add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    std::iter::once(&self.identity_token_info.identity),
                    &mut selected_identity,
                    &mut self.selected_key,
                    TransactionType::TokenAction,
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // 2) Amount to purchase
                ui.heading("2. Amount to purchase and price");
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

                // Purchase button (disabled if no valid amounts are available)
                let can_purchase = self.fetched_pricing_schedule.is_some()
                    && self.calculated_price_credits.unwrap_or_default() > 0
                    && self
                        .amount_to_purchase_value
                        .as_ref()
                        .map(|v| v.value())
                        .unwrap_or_default()
                        > 0;
                let purchase_text = "Purchase".to_string();

                if can_purchase {
                    let button =
                        egui::Button::new(RichText::new(purchase_text).color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .corner_radius(3.0);

                    if ui.add(button).clicked() {
                        self.show_confirmation_popup = true;
                    }
                } else {
                    let button = egui::Button::new(
                        RichText::new(purchase_text).color(DashColors::muted_color(dark_mode)),
                    )
                    .fill(Color32::from_rgb(50, 50, 50))
                    .corner_radius(3.0);

                    ui.add_enabled(false, button).on_hover_text(
                        if self.pricing_fetch_attempted && self.fetched_pricing_schedule.is_none() {
                            "This token is not available for purchase"
                        } else {
                            "Fetch token price and enter amount first"
                        },
                    );
                }

                // If the user pressed "Purchase," show a popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    PurchaseTokensStatus::NotStarted => {
                        // no-op
                    }
                    PurchaseTokensStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Purchasing... elapsed: {} seconds", elapsed));
                    }
                    PurchaseTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(
                            DashColors::error_color(dark_mode),
                            format!("Error: {}", msg),
                        );
                    }
                    PurchaseTokensStatus::Complete => {
                        // handled above
                    }
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for PurchaseTokenScreen {
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

#[cfg(test)]
mod tests {
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
}
