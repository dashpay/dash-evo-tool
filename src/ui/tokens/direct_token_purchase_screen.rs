use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;

use super::tokens_screen::IdentityTokenInfo;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{add_identity_key_chooser, TransactionType};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
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

    // Specific to this transition
    amount_to_purchase: String,
    total_agreed_price: String,
    fetched_pricing_schedule: Option<TokenPricingSchedule>,
    calculated_price: Option<Credits>,
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
            amount_to_purchase: "".to_string(),
            total_agreed_price: "".to_string(),
            fetched_pricing_schedule: None,
            calculated_price: None,
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

    /// Renders a text input for the user to specify an amount to purchase
    fn render_amount_input(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.horizontal(|ui| {
            ui.label("Amount to Purchase:");
            let response = ui.text_edit_singleline(&mut self.amount_to_purchase);

            // When amount changes, recalculate the price if we have pricing schedule
            if response.changed() {
                self.recalculate_price();
            }

            // Fetch pricing button
            if ui.button("Fetch Token Price").clicked() {
                // We need to fetch the current pricing schedule from the contract
                self.status = PurchaseTokensStatus::WaitingForResult(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs(),
                );
                action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                    TokenTask::FetchTokenPricingSchedule {
                        data_contract: Arc::new(
                            self.identity_token_info.data_contract.contract.clone(),
                        ),
                        token_position: self.identity_token_info.token_position,
                    },
                )));
            }
        });

        // Show fetched pricing schedule
        if let Some(pricing_schedule) = &self.fetched_pricing_schedule {
            ui.add_space(5.0);
            ui.label("Current pricing:");
            match pricing_schedule {
                TokenPricingSchedule::SinglePrice(price) => {
                    ui.label(format!("  Fixed price: {} credits per token", price));
                }
                TokenPricingSchedule::SetPrices(tiers) => {
                    ui.label("  Tiered pricing:");
                    for (amount, price) in tiers {
                        ui.label(format!("    {} tokens: {} credits each", amount, price));
                    }
                }
            }
        }

        action
    }

    /// Recalculates the total price based on amount and pricing schedule
    fn recalculate_price(&mut self) {
        if let (Some(pricing_schedule), Ok(amount)) = (
            &self.fetched_pricing_schedule,
            self.amount_to_purchase.parse::<u64>(),
        ) {
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

            let total_price = amount.saturating_mul(price_per_token);
            self.calculated_price = Some(total_price);
            self.total_agreed_price = total_price.to_string();
        } else {
            self.calculated_price = None;
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
                let amount_ok = self.amount_to_purchase.parse::<u64>().ok();
                if amount_ok.is_none() {
                    self.error_message = Some("Please enter a valid amount.".into());
                    self.status = PurchaseTokensStatus::ErrorMessage("Invalid amount".into());
                    self.show_confirmation_popup = false;
                    return;
                }

                let total_agreed_price_ok: Option<Credits> =
                    self.total_agreed_price.parse::<u64>().ok();
                if total_agreed_price_ok.is_none() {
                    self.error_message = Some("Please enter a valid total agreed price.".into());
                    self.status =
                        PurchaseTokensStatus::ErrorMessage("Invalid total agreed price".into());
                    self.show_confirmation_popup = false;
                    return;
                }

                ui.label(format!(
                    "Are you sure you want to purchase {} token(s) for {} Credits?",
                    self.amount_to_purchase, self.total_agreed_price
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
                                amount: amount_ok.expect("Expected a valid amount"),
                                total_agreed_price: total_agreed_price_ok
                                    .expect("Expected a valid total agreed price"),
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
        if let BackendTaskSuccessResult::TokenPricingSchedule(pricing_schedule) = result {
            self.pricing_fetch_attempted = true;
            if let Some(schedule) = pricing_schedule {
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
            // If we are in the "Complete" status, just show success screen
            if self.status == PurchaseTokensStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Purchase Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode.load(Ordering::Relaxed) {
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
                    Color32::DARK_RED,
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

                // Display calculated price
                if let Some(calculated_price) = self.calculated_price {
                    ui.group(|ui| {
                        ui.heading("Calculated total price:");
                        ui.label(format!("{} credits", calculated_price));
                        ui.label("Note: This is the calculated price based on the current pricing schedule.");
                    });
                } else if self.fetched_pricing_schedule.is_some() {
                    ui.colored_label(
                        Color32::DARK_RED,
                        "Please enter a valid amount to see the price.",
                    );
                } else {
                    ui.label("Click 'Fetch Token Price' to retrieve current pricing.");
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Purchase button (disabled if no pricing is available)
                let can_purchase =
                    self.fetched_pricing_schedule.is_some() && self.calculated_price.is_some();
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
                    let button =
                        egui::Button::new(RichText::new(purchase_text).color(Color32::GRAY))
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
                        ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
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
