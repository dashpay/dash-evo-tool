use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::amount::{Amount, DASH_DECIMAL_PLACES};
use crate::model::wallet::Wallet;
use crate::ui::components::ComponentResponse;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::contracts_documents::group_actions_screen::GroupActionsScreen;
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Pricing type selection
#[derive(PartialEq, Clone)]
pub enum PricingType {
    SinglePrice,
    TieredPricing,
    RemovePricing,
}

impl From<TokenPricingSchedule> for PricingType {
    fn from(schedule: TokenPricingSchedule) -> Self {
        match schedule {
            TokenPricingSchedule::SinglePrice(_) => PricingType::SinglePrice,
            TokenPricingSchedule::SetPrices(_) => PricingType::TieredPricing,
        }
    }
}

impl From<Option<TokenPricingSchedule>> for PricingType {
    fn from(schedule: Option<TokenPricingSchedule>) -> Self {
        match schedule {
            Some(schedule) => PricingType::from(schedule),
            None => PricingType::RemovePricing,
        }
    }
}

/// Internal states for the mint process.
#[derive(PartialEq)]
pub enum SetTokenPriceStatus {
    NotStarted,
    WaitingForResult(u64), // Use seconds or millis
    ErrorMessage(String),
    Complete,
}

/// A UI Screen for minting tokens from an existing token contract
pub struct SetTokenPriceScreen {
    pub identity_token_info: IdentityTokenInfo,
    selected_key: Option<IdentityPublicKey>,
    pub public_note: Option<String>,
    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,

    pub token_pricing_schedule: String,
    /// Token pricing schedule to use; if None, we will remove the pricing schedule
    pub pricing_type: PricingType,

    // AmountInput components for pricing - following the design pattern
    single_price_amount: Option<Amount>,
    single_price_input: Option<AmountInput>,

    // Tiered pricing with AmountInput components
    pub tiered_prices: Vec<(Option<AmountInput>, Option<AmountInput>)>, // (amount_input, price_input)
    status: SetTokenPriceStatus,
    error_message: Option<String>,

    /// Basic references
    pub app_context: Arc<AppContext>,

    /// Confirmation popup
    show_confirmation_popup: bool,
    confirmation_dialog: Option<ConfirmationDialog>,

    // If needed for password-based wallet unlocking:
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

/// 1 Dash = 100,000,000,000 credits
pub const CREDITS_PER_DASH: Credits = 100_000_000_000;

impl SetTokenPriceScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let possible_key: Option<&IdentityPublicKey> = identity_token_info
            .identity
            .identity
            .get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            );

        let mut error_message = None;

        let group = match identity_token_info
            .token_config
            .distribution_rules()
            .change_direct_purchase_pricing_rules()
            .authorized_to_make_change_action_takers()
        {
            AuthorizedActionTakers::NoOne => {
                error_message =
                    Some("Setting token price is not allowed on this token".to_string());
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != identity_token_info.identity.identity.id()
                {
                    error_message = Some(
                        "You are not allowed to set token price on this token. Only the contract owner is."
                            .to_string(),
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    error_message =
                        Some("You are not allowed to set token price on this token".to_string());
                }
                None
            }
            AuthorizedActionTakers::MainGroup => {
                match identity_token_info.token_config.main_control_group() {
                    None => {
                        error_message = Some(
                            "Invalid contract: No main control group, though one should exist"
                                .to_string(),
                        );
                        None
                    }
                    Some(group_pos) => {
                        match identity_token_info
                            .data_contract
                            .contract
                            .expected_group(group_pos)
                        {
                            Ok(group) => Some((group_pos, group.clone())),
                            Err(e) => {
                                error_message = Some(format!("Invalid contract: {}", e));
                                None
                            }
                        }
                    }
                }
            }
            AuthorizedActionTakers::Group(group_pos) => {
                match identity_token_info
                    .data_contract
                    .contract
                    .expected_group(*group_pos)
                {
                    Ok(group) => Some((*group_pos, group.clone())),
                    Err(e) => {
                        error_message = Some(format!("Invalid contract: {}", e));
                        None
                    }
                }
            }
        };

        let mut is_unilateral_group_member = false;
        if group.is_some() {
            if let Some((_, group)) = group.clone() {
                let your_power = group
                    .members()
                    .get(&identity_token_info.identity.identity.id());

                if let Some(your_power) = your_power {
                    if your_power >= &group.required_power() {
                        is_unilateral_group_member = true;
                    }
                }
            }
        };

        // Attempt to get an unlocked wallet reference
        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key,
            &mut error_message,
        );

        Self {
            identity_token_info: identity_token_info.clone(),
            selected_key: possible_key.cloned(),
            public_note: None,
            group,
            is_unilateral_group_member,
            group_action_id: None,
            token_pricing_schedule: "".to_string(),
            pricing_type: PricingType::RemovePricing,
            single_price_amount: None,
            single_price_input: None,
            tiered_prices: vec![(None, None)],
            status: SetTokenPriceStatus::NotStarted,
            error_message: None,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            confirmation_dialog: None,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    pub fn with_schedule(self, token_pricing_schedule: Option<TokenPricingSchedule>) -> Self {
        let token_decimals = self
            .identity_token_info
            .token_config
            .conventions()
            .decimals();
        let decimal_multiplier = 10u64.pow(token_decimals as u32);

        let (single_price_amount, tiered_prices) = match &token_pricing_schedule {
            Some(TokenPricingSchedule::SinglePrice(price_per_smallest_unit)) => {
                // Convert price per smallest unit back to price per token for display
                let price_per_token = price_per_smallest_unit * decimal_multiplier;
                let amount =
                    Amount::new(price_per_token, DASH_DECIMAL_PLACES).with_unit_name("DASH");
                (Some(amount), vec![(None, None)])
            }
            Some(TokenPricingSchedule::SetPrices(prices)) => {
                let tiered_prices = prices
                    .iter()
                    .map(|(amount, price_per_smallest_unit)| {
                        // Create amount input for token threshold
                        let amount_input = AmountInput::new(Amount::from_token(
                            &self.identity_token_info,
                            *amount,
                        ))
                        .with_hint_text("Token amount threshold");

                        // Convert price per smallest unit back to price per token for display
                        let price_per_token = price_per_smallest_unit * decimal_multiplier;
                        let price = Amount::new(price_per_token, DASH_DECIMAL_PLACES)
                            .with_unit_name("DASH");
                        let price_input = AmountInput::new(price)
                            .with_hint_text("Enter price in Dash")
                            .with_min_amount(Some(1));
                        (Some(amount_input), Some(price_input))
                    })
                    .collect::<Vec<_>>();

                (None, tiered_prices)
            }
            None => (None, vec![(None, None)]),
        };

        Self {
            pricing_type: PricingType::from(token_pricing_schedule),
            single_price_amount,
            tiered_prices,
            ..self
        }
    }

    /// Renders the pricing input UI
    fn render_pricing_input(&mut self, ui: &mut Ui) {
        // Radio buttons for pricing type
        ui.horizontal(|ui| {
            ui.radio_value(
                &mut self.pricing_type,
                PricingType::SinglePrice,
                "Single Price",
            );
            ui.radio_value(
                &mut self.pricing_type,
                PricingType::TieredPricing,
                "Tiered Pricing",
            );
            ui.radio_value(
                &mut self.pricing_type,
                PricingType::RemovePricing,
                "Remove Pricing (Make Token Not For Sale)",
            );
        });

        ui.add_space(10.0);

        match self.pricing_type {
            PricingType::SinglePrice => {
                ui.label("Set a fixed price per token:");

                // Lazy initialization of AmountInput following the design pattern
                let single_price_input = self.single_price_input.get_or_insert_with(|| {
                    let initial_amount = self
                        .single_price_amount
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| Amount::new_dash(0.0));
                    AmountInput::new(initial_amount)
                        .with_label("Price per token:")
                        .with_hint_text("Enter price in Dash")
                        .with_min_amount(Some(1)) // Minimum 1 credit (very small amount)
                });

                let response = single_price_input.show(ui);

                // Update the domain data if there's a valid change
                if response.inner.has_changed() && response.inner.is_valid() {
                    self.single_price_amount = response.inner.changed_value().clone();
                }

                // Show validation preview
                if let Some(amount) = &self.single_price_amount {
                    if amount.value() > 0 {
                        ui.add_space(5.0);
                        let credits = amount.value();
                        ui.colored_label(
                            Color32::DARK_GREEN,
                            format!("Price: {} per token ({} credits)", amount, credits),
                        );
                    }
                }
            }
            PricingType::TieredPricing => {
                ui.label("Add pricing tiers to offer volume discounts");
                ui.add_space(10.0);

                // Tiered pricing table
                let table = TableBuilder::new(ui)
                    .striped(false)
                    .resizable(false)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(120.0).resizable(false))
                    .column(Column::exact(200.0).resizable(false))
                    .column(Column::exact(80.0).resizable(false))
                    .min_scrolled_height(0.0);

                let mut to_remove = None;
                let can_remove = self.tiered_prices.len() > 1;

                table
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.label(
                                RichText::new("Minimum Amount")
                                    .color(Color32::BLACK)
                                    .strong()
                                    .underline(),
                            );
                        });
                        header.col(|ui| {
                            ui.label(
                                RichText::new("Price per Token")
                                    .color(Color32::BLACK)
                                    .strong()
                                    .underline(),
                            );
                        });
                        header.col(|ui| {
                            ui.label(
                                RichText::new("Remove")
                                    .color(Color32::BLACK)
                                    .strong()
                                    .underline(),
                            );
                        });
                    })
                    .body(|mut body| {
                        for i in 0..self.tiered_prices.len() {
                            body.row(30.0, |mut row| {
                                row.col(|ui| {
                                    if i == 0 {
                                        // First tier is hardcoded to 1 token - create AmountInput with value 1
                                        let amount_input =
                                            self.tiered_prices[i].0.get_or_insert_with(|| {
                                                AmountInput::new(Amount::from_token(
                                                    &self.identity_token_info,
                                                    1,
                                                ))
                                                .with_hint_text("Token amount threshold")
                                            });
                                        amount_input.show(ui);
                                        // Make sure it's always 1 - we could disable editing or show as read-only
                                    } else {
                                        // Other tiers use AmountInput for token amounts
                                        let amount_input =
                                            self.tiered_prices[i].0.get_or_insert_with(|| {
                                                AmountInput::new(Amount::from_token(
                                                    &self.identity_token_info,
                                                    0,
                                                ))
                                                .with_hint_text("Token amount threshold")
                                            });
                                        amount_input.show(ui);
                                    }
                                });
                                row.col(|ui| {
                                    // Use AmountInput for price with lazy initialization
                                    let price_input =
                                        self.tiered_prices[i].1.get_or_insert_with(|| {
                                            AmountInput::new(Amount::new_dash(0.0))
                                                .with_hint_text("Enter price in Dash")
                                                .with_min_amount(Some(1)) // Minimum 1 credit
                                        });

                                    let _response = price_input.show(ui);
                                });
                                row.col(|ui| {
                                    if can_remove && i > 0 && ui.small_button("X").clicked() {
                                        to_remove = Some(i);
                                    }
                                });
                            });
                        }
                    });

                if let Some(i) = to_remove {
                    self.tiered_prices.remove(i);
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Add Tier").clicked() {
                        // Add empty tier with lazy initialization
                        self.tiered_prices.push((None, None));
                    }
                });

                // Show preview
                ui.add_space(10.0);
                self.render_tiered_pricing_preview(ui);
            }
            PricingType::RemovePricing => {
                ui.colored_label(Color32::from_rgb(180, 100, 0), "WARNING: This will remove the pricing schedule, making the token unavailable for direct purchase.");
                ui.label("Users will no longer be able to buy this token directly.");
            }
        }
    }

    fn render_tiered_pricing_preview(&self, ui: &mut Ui) {
        let mut valid_tiers = Vec::new();
        let mut has_errors = false;

        for (amount_input, price_input) in &self.tiered_prices {
            let Some(price) = price_input.as_ref().and_then(|input| input.current_value()) else {
                continue; // Skip if no price input is available
            };

            let Some(amount_value) = amount_input
                .as_ref()
                .and_then(|input| input.current_value())
            else {
                has_errors = true;
                continue; // Skip if amount is invalid
            };

            valid_tiers.push((amount_value, price));
        }

        // Only show preview if there are valid tiers or errors
        if !valid_tiers.is_empty() || has_errors {
            ui.group(|ui| {
                // Sort tiers by amount
                if !valid_tiers.is_empty() {
                    valid_tiers.sort_by_key(|(amount, _)| amount.value());
                }

                if has_errors {
                    ui.colored_label(Color32::DARK_RED, "X Some tiers have invalid values");
                }

                if !valid_tiers.is_empty() {
                    ui.colored_label(Color32::DARK_GREEN, "Pricing Structure:");
                    for (amount, price) in &valid_tiers {
                        let credits = price.value();
                        ui.label(format!(
                            "  - {} or more tokens: {} each ({} credits)",
                            amount, price, credits
                        ));
                    }
                }
            });
        }
    }

    /// Validates and creates the pricing schedule from the UI inputs
    fn create_pricing_schedule(&self) -> Result<Option<TokenPricingSchedule>, String> {
        match self.pricing_type {
            PricingType::RemovePricing => Ok(None),
            PricingType::SinglePrice => match &self.single_price_amount {
                Some(amount) if amount.value() > 0 => {
                    // User enters price per whole token, but Platform expects price per smallest unit
                    let credits_price_per_token = amount.value();
                    let token_decimals = self
                        .identity_token_info
                        .token_config
                        .conventions()
                        .decimals();

                    // Convert price per token to price per smallest unit
                    let decimal_divisor = 10u64.pow(token_decimals as u32);
                    let price_per_smallest_unit = credits_price_per_token / decimal_divisor;

                    Ok(Some(TokenPricingSchedule::SinglePrice(
                        price_per_smallest_unit,
                    )))
                }
                Some(_) => Err("Price must be greater than 0".to_string()),
                None => Err("Please enter a price".to_string()),
            },
            PricingType::TieredPricing => {
                let mut map = std::collections::BTreeMap::new();
                let token_decimals = self
                    .identity_token_info
                    .token_config
                    .conventions()
                    .decimals();
                let decimal_divisor = 10u64.pow(token_decimals as u32);

                for (amount_input, price_input) in &self.tiered_prices {
                    let Some(price) = price_input.as_ref().and_then(|input| input.current_value())
                    else {
                        continue;
                    };

                    let Some(amount_value) = amount_input
                        .as_ref()
                        .and_then(|input| input.current_value())
                    else {
                        continue;
                    };

                    let amount = amount_value.value();
                    // Convert price per token to price per smallest unit
                    let price_per_smallest_unit = price.value() / decimal_divisor;
                    map.insert(amount, price_per_smallest_unit);
                }

                if map.is_empty() {
                    return Err("Please add at least one pricing tier".to_string());
                }

                Ok(Some(TokenPricingSchedule::SetPrices(map)))
            }
        }
    }

    /// Validate the current pricing configuration before showing confirmation dialog
    fn validate_pricing_configuration(&self) -> Result<(), String> {
        match self.pricing_type {
            PricingType::RemovePricing => Ok(()),
            PricingType::SinglePrice => match &self.single_price_amount {
                Some(amount) if amount.value() > 0 => Ok(()),
                Some(_) => Err("Price must be greater than 0".to_string()),
                None => Err("Please enter a price".to_string()),
            },
            PricingType::TieredPricing => {
                let mut valid_tiers = 0;

                for (amount_input, price_input) in &self.tiered_prices {
                    let Some(price) = price_input.as_ref().and_then(|input| input.current_value())
                    else {
                        continue;
                    };

                    let Some(amount_value) = amount_input
                        .as_ref()
                        .and_then(|input| input.current_value())
                    else {
                        continue;
                    };

                    if amount_value.value() > 0 && price.value() > 0 {
                        valid_tiers += 1;
                    }
                }

                if valid_tiers == 0 {
                    return Err("Please add at least one valid pricing tier".to_string());
                }

                Ok(())
            }
        }
    }

    /// Generate the confirmation message for the set price dialog
    ///
    /// ## Panics
    ///
    /// Panics if the pricing type is not set correctly or if the single price is not a valid number.
    fn confirmation_message(&self) -> String {
        match &self.pricing_type {
            PricingType::RemovePricing => {
                "WARNING: Are you sure you want to remove the pricing schedule? This will make the token unavailable for direct purchase.".to_string()
            }
            PricingType::SinglePrice => {
                if let Some(amount) = &self.single_price_amount {
                    format!(
                        "Are you sure you want to set a fixed price of {} per token?",
                        amount
                    )
                } else {
                    "Are you sure you want to set the pricing schedule?".to_string()
                }
            }
            PricingType::TieredPricing => {
                let mut message = "Are you sure you want to set the following tiered pricing?".to_string();
                for (amount_input, price_input) in &self.tiered_prices {
                    let Some(price) = price_input.as_ref().and_then(|input| input.current_value()) else {
                        continue; // Skip if no price input is available
                    };

                    let Some(amount_value) = amount_input
                        .as_ref()
                        .and_then(|input| input.current_value())
                    else {
                        continue;
                    };

                    message.push_str(&format!(
                        "\n  - {} or more tokens: {} each",
                        amount_value, price
                    ));
                }
                message
            }
        }
    }

    /// Handle the confirmation action when user clicks OK
    fn confirmation_ok(&mut self) -> AppAction {
        self.show_confirmation_popup = false;
        self.confirmation_dialog = None; // Reset the dialog for next use

        // Validate user input and create pricing schedule
        let token_pricing_schedule_opt = match self.create_pricing_schedule() {
            Ok(schedule) => schedule,
            Err(error) => {
                // This should not happen if validation was done before opening dialog,
                // but we handle it as a safety net
                self.set_error_state(format!("Validation error: {}", error));
                return AppAction::None;
            }
        };

        // Set waiting state
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        self.status = SetTokenPriceStatus::WaitingForResult(now);

        // Prepare group info
        let group_info = if self.group_action_id.is_some() {
            self.group.as_ref().map(|(pos, _)| {
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                    GroupStateTransitionInfo {
                        group_contract_position: *pos,
                        action_id: self.group_action_id.unwrap(),
                        action_is_proposer: false,
                    },
                )
            })
        } else {
            self.group.as_ref().map(|(pos, _)| {
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
            })
        };

        // Create and return the backend task
        AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::SetDirectPurchasePrice {
                identity: self.identity_token_info.identity.clone(),
                data_contract: Arc::new(self.identity_token_info.data_contract.contract.clone()),
                token_position: self.identity_token_info.token_position,
                signing_key: self.selected_key.clone().expect("Expected a key"),
                public_note: if self.group_action_id.is_some() {
                    None
                } else {
                    self.public_note.clone()
                },
                token_pricing_schedule: token_pricing_schedule_opt,
                group_info,
            },
        )))
    }

    /// Handle the cancel action when user clicks Cancel or closes dialog
    fn confirmation_cancel(&mut self) -> AppAction {
        self.show_confirmation_popup = false;
        self.confirmation_dialog = None; // Reset the dialog for next use
        AppAction::None
    }

    /// Set error state with the given message
    fn set_error_state(&mut self, error: String) {
        self.error_message = Some(error.clone());
        self.status = SetTokenPriceStatus::ErrorMessage(error);
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        // Prepare values before borrowing
        let confirmation_message = self.confirmation_message();
        let is_danger_mode = self.pricing_type == PricingType::RemovePricing;

        // Lazy initialization of the confirmation dialog
        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm pricing schedule update", confirmation_message)
                .confirm_text(Some("Confirm"))
                .cancel_text(Some("Cancel"))
                .danger_mode(is_danger_mode)
        });

        let response = confirmation_dialog.show(ui);

        match response.inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => self.confirmation_ok(),
            Some(ConfirmationStatus::Canceled) => self.confirmation_cancel(),
            None => AppAction::None,
        }
    }

    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            if self.group_action_id.is_some() {
                // This is already initiated by the group, we are just signing it
                ui.heading("Group Action to Set Price Signed Successfully.");
            } else if !self.is_unilateral_group_member && self.group.is_some() {
                ui.heading("Group Action to Set Price Initiated.");
            } else {
                ui.heading("Set Price of Token Successfully.");
            }

            ui.add_space(20.0);

            if self.group_action_id.is_some() {
                if ui.button("Back to Group Actions").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenMyTokenBalances,
                    );
                }
            } else {
                if ui.button("Back to Tokens").clicked() {
                    action = AppAction::PopScreenAndRefresh;
                }

                if !self.is_unilateral_group_member && ui.button("Go to Group Actions").clicked() {
                    action = AppAction::PopThenAddScreenToMainScreen(
                        RootScreenType::RootScreenDocumentQuery,
                        Screen::GroupActionsScreen(GroupActionsScreen::new(
                            &self.app_context.clone(),
                        )),
                    );
                }
            }
        });
        action
    }
}

impl ScreenLike for SetTokenPriceScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Successfully set token pricing schedule")
                    || message == "SetDirectPurchasePrice"
                {
                    self.status = SetTokenPriceStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = SetTokenPriceStatus::ErrorMessage(message.to_string());
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
        let mut action;

        // Build a top panel
        if self.group_action_id.is_some() {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Contracts", AppAction::GoToMainScreen),
                    ("Group Actions", AppAction::PopScreen),
                    ("SetPrice", AppAction::None),
                ],
                vec![],
            );
        } else {
            action = add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Tokens", AppAction::GoToMainScreen),
                    (&self.identity_token_info.token_alias, AppAction::PopScreen),
                    ("SetPrice", AppAction::None),
                ],
                vec![],
            );
        }

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        island_central_panel(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // If we are in the "Complete" status, just show success screen
                if self.status == SetTokenPriceStatus::Complete {
                    action |= self.show_success_screen(ui);
                    return;
                }

            ui.heading("Set Token Pricing Schedule");
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
                ui.heading("1. Select the key to sign the SetPrice transaction");
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

                // 2) Pricing schedule
                ui.heading("2. Pricing Configuration");
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group SetPrice so you are not allowed to choose the pricing schedule.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!("Schedule: {}", self.token_pricing_schedule));
                } else {
                    self.render_pricing_input(ui);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("3. Public note (optional)");
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group SetPrice so you are not allowed to put a note.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!(
                        "Note: {}",
                        self.public_note.clone().unwrap_or("None".to_string())
                    ));
                } else {
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
                            self.public_note = if !txt.is_empty() { Some(txt) } else { None };
                        }
                    });
                }

                let set_price_text = if let Some((_, group)) = self.group.as_ref() {
                    let your_power = group
                        .members()
                        .get(&self.identity_token_info.identity.identity.id());
                    if your_power.is_none() {
                        self.error_message =
                            Some("Only group members can set price on this token".to_string());
                    }
                    ui.heading("This is a group action, it is not immediate.");
                    ui.label(format!(
                        "Members are : \n{}",
                        group
                            .members()
                            .iter()
                            .map(|(member, power)| {
                                if member == &self.identity_token_info.identity.identity.id() {
                                    format!("{} (You) with power {}", member, power)
                                } else {
                                    format!("{} with power {}", member, power)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", \n")
                    ));
                    ui.add_space(10.0);
                    if let Some(your_power) = your_power {
                        if *your_power >= group.required_power() {
                            ui.label(format!("Even though this is a group action, you are able to unilaterally approve it because your power ({}) in the group exceeds the required amount : {}", *your_power,  group.required_power()));
                            "Set Price"
                        } else {
                            ui.label(format!("You will need at least {} voting power for this action to go through. Contact other group members to let them know to authorize this action after you have initiated it.", group.required_power()));
                            "Initiate Group SetPrice"
                        }
                    } else {
                        "Test SetPrice (It should fail)"
                    }
                } else {
                    "Set Price"
                };

                // Set price button
                let validation_result = self.validate_pricing_configuration();
                let button_active = validation_result.is_ok() && !matches!(self.status, SetTokenPriceStatus::WaitingForResult(_));

                let button_color = if validation_result.is_ok() {
                    Color32::from_rgb(0, 128, 255)
                } else {
                    Color32::from_rgb(100, 100, 100)
                };

                let button = egui::Button::new(RichText::new(set_price_text).color(Color32::WHITE))
                    .fill(button_color)
                    .corner_radius(3.0);

                let button_response = ui.add_enabled(button_active, button);

                if let Err(hover_message) = validation_result {
                                    button_response.on_disabled_hover_text(hover_message);
                } else if button_response.clicked() {
                    self.show_confirmation_popup = true;
                }

                // If the user pressed "Set Price," show a popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    SetTokenPriceStatus::NotStarted => {
                        // no-op
                    }
                    SetTokenPriceStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Setting price... elapsed: {} seconds", elapsed));
                    }
                    SetTokenPriceStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
                    }
                    SetTokenPriceStatus::Complete => {
                        // handled above
                    }
                }
            }
            }); // end of ScrollArea
        });

        action
    }
}

impl ScreenWithWalletUnlock for SetTokenPriceScreen {
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
