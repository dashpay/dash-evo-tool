use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::methods::v0::TokenPerpetualDistributionV0Accessors;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::{MessageType, Screen, ScreenLike};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use super::tokens_screen::IdentityTokenBalance;

/// States for the claim flow
#[derive(PartialEq)]
pub enum ClaimTokensStatus {
    NotStarted,
    WaitingForResult(u64),
    ErrorMessage(String),
    Complete,
}

pub struct ClaimTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_balance: IdentityTokenBalance,
    selected_key: Option<dash_sdk::platform::IdentityPublicKey>,
    pub public_note: Option<String>,
    token_contract: QualifiedContract,
    token_configuration: TokenConfiguration,
    distribution_type: Option<TokenDistributionType>,
    status: ClaimTokensStatus,
    error_message: Option<String>,
    pub app_context: Arc<AppContext>,
    show_confirmation_popup: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl ClaimTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        token_contract: QualifiedContract,
        token_configuration: TokenConfiguration,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let identity = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|id| id.identity.id() == identity_token_balance.identity_id)
            .expect("No local qualified identity found for this token’s identity.");

        let identity_clone = identity.identity.clone();
        let possible_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
                SecurityLevel::CRITICAL,
            ]),
            KeyType::all_key_types().into(),
            false,
        );

        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, possible_key.clone(), &mut error_message);

        let distribution_type = match (
            token_configuration
                .distribution_rules()
                .perpetual_distribution()
                .is_some(),
            token_configuration
                .distribution_rules()
                .pre_programmed_distribution()
                .is_some(),
        ) {
            (true, true) => None,
            (true, false) => Some(TokenDistributionType::Perpetual),
            (false, true) => Some(TokenDistributionType::PreProgrammed),
            (false, false) => None,
        };

        Self {
            identity,
            identity_token_balance,
            selected_key: possible_key.cloned(),
            public_note: None,
            token_contract,
            token_configuration,
            distribution_type,
            status: ClaimTokensStatus::NotStarted,
            error_message: None,
            app_context: app_context.clone(),
            show_confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    fn render_key_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Select Key:");
            egui::ComboBox::from_id_salt("claim_key_selector")
                .selected_text(match &self.selected_key {
                    Some(key) => format!("Key ID: {}", key.id()),
                    None => "Select a key".to_string(),
                })
                .show_ui(ui, |ui| {
                    if self.app_context.developer_mode {
                        // Show all loaded public keys
                        for key in self.identity.identity.public_keys().values() {
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    } else {
                        // Show only "available" auth keys
                        for key_wrapper in self.identity.available_authentication_keys() {
                            if key_wrapper.identity_public_key.security_level()
                                == SecurityLevel::MASTER
                            {
                                // Master keys can't sign claims
                                continue;
                            }
                            let key = &key_wrapper.identity_public_key;
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    }
                });
        });
    }

    fn render_token_distribution_type_selector(&mut self, ui: &mut Ui) {
        let show_perpetual = if let Some(perpetual_distribution) = self
            .token_configuration
            .distribution_rules()
            .perpetual_distribution()
        {
            match perpetual_distribution.distribution_recipient() {
                TokenDistributionRecipient::ContractOwner => {
                    self.token_contract.contract.owner_id() == self.identity.identity.id()
                }
                TokenDistributionRecipient::Identity(id) => self.identity.identity.id() == id,
                TokenDistributionRecipient::EvonodesByParticipation => true,
            }
        } else {
            false
        };
        let show_pre_programmed = self
            .token_configuration
            .distribution_rules()
            .pre_programmed_distribution()
            .is_some();
        ui.horizontal(|ui| {
            ui.label("Select Distribution Type:");
            egui::ComboBox::from_id_salt("claim_distribution_type_selector")
                .selected_text(match &self.distribution_type {
                    Some(TokenDistributionType::Perpetual) => "Perpetual".to_string(),
                    Some(TokenDistributionType::PreProgrammed) => "PreProgrammed".to_string(),
                    None => "Select a type".to_string(),
                })
                .show_ui(ui, |ui| {
                    if !show_perpetual && !show_pre_programmed {
                        ui.label("No distributions to potentially claim for this token");
                    }
                    if show_perpetual {
                        ui.selectable_value(
                            &mut self.distribution_type,
                            Some(TokenDistributionType::Perpetual),
                            "Perpetual",
                        );
                    }
                    if show_pre_programmed {
                        ui.selectable_value(
                            &mut self.distribution_type,
                            Some(TokenDistributionType::PreProgrammed),
                            "PreProgrammed",
                        );
                    }
                });
        });
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;
        let distribution_type = self
            .distribution_type
            .clone()
            .unwrap_or(TokenDistributionType::Perpetual);
        egui::Window::new("Confirm Claim")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label("Are you sure you want to claim tokens for this contract?");
                ui.add_space(10.0);

                // Confirm
                if ui.button("Confirm").clicked() {
                    self.show_confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.status = ClaimTokensStatus::WaitingForResult(now);

                    action = AppAction::BackendTasks(
                        vec![
                            BackendTask::TokenTask(TokenTask::ClaimTokens {
                                data_contract: self.token_contract.contract.clone(),
                                token_position: self.identity_token_balance.token_position,
                                actor_identity: self.identity.clone(),
                                distribution_type,
                                signing_key: self.selected_key.clone().expect("No key selected"),
                                public_note: self.public_note.clone(),
                            }),
                            BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
                        ],
                        BackendTasksExecutionMode::Sequential,
                    );
                }

                if ui.button("Cancel").clicked() {
                    self.show_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_confirmation_popup = false;
        }
        action
    }

    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Claimed Successfully!");

            ui.add_space(20.0);

            if ui.button("Back to Tokens").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });
        action
    }
}

impl ScreenLike for ClaimTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Claimed") || message == "ClaimTokens" {
                    self.status = ClaimTokensStatus::Complete;
                }
            }
            MessageType::Error => {
                self.status = ClaimTokensStatus::ErrorMessage(message.to_string());
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // no-op
            }
        }
    }

    fn refresh(&mut self) {
        if let Ok(all) = self.app_context.load_local_qualified_identities() {
            if let Some(updated) = all
                .into_iter()
                .find(|id| id.identity.id() == self.identity.identity.id())
            {
                self.identity = updated;
            }
        }
    }

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
                ("Claim", AppAction::None),
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

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.status == ClaimTokensStatus::Complete {
                action = self.show_success_screen(ui);
                return;
            }

            ui.heading("Claim Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_authentication_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    Color32::RED,
                    format!(
                        "No authentication keys found for this {} identity.",
                        self.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                let first_key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([
                        SecurityLevel::HIGH,
                        SecurityLevel::MEDIUM,
                        SecurityLevel::CRITICAL,
                    ]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Possibly handle locked wallet scenario
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return;
                    }
                }

                ui.heading("1. Select the key to sign the Claim transition");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    self.render_key_selection(ui);
                    ui.add_space(5.0);
                    let identity_id_string =
                        self.identity.identity.id().to_string(Encoding::Base58);
                    let identity_display = self
                        .identity
                        .alias
                        .as_deref()
                        .unwrap_or_else(|| &identity_id_string);
                    ui.label(format!("Identity: {}", identity_display));
                });
                ui.add_space(10.0);

                self.render_token_distribution_type_selector(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("2. Public note (optional)");
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

                if self.distribution_type == Some(TokenDistributionType::Perpetual) {
                    ui.heading("!Understanding Claim Limitations!");
                    ui.add_space(5.0);
                    let extra_info = if let Some(perpetual_distribution) = self.token_configuration.distribution_rules().perpetual_distribution() {
                        let function_string = match perpetual_distribution.distribution_type().function() {
                            DistributionFunction::FixedAmount { amount } => {
                                format!("a fixed amount of {} base tokens", amount)
                            }
                            DistributionFunction::Random { min, max } => {
                                format!("a random amount between {} and {} base tokens", min, max)
                            }
                            DistributionFunction::StepDecreasingAmount {
                                step_count,
                                decrease_per_interval_numerator,
                                decrease_per_interval_denominator,
                                distribution_start_amount,
                                min_value,
                                ..
                            } => {
                                format!(
                                    "a decreasing amount starting at {} and stepping every {} interval{} by {}/{}{}",
                                    distribution_start_amount,
                                    step_count,
                                    if *step_count == 1 { "" } else { "s" },
                                    decrease_per_interval_numerator,
                                    decrease_per_interval_denominator,
                                    min_value
                                        .map(|v| format!(", with a minimum of {}", v))
                                        .unwrap_or_default()
                                )
                            }
                            DistributionFunction::Stepwise(_) => {
                                "a variable amount based on a stepwise function".to_string()
                            }
                            DistributionFunction::Linear { .. } => {
                                "a variable amount based on a linear function".to_string()
                            }
                            DistributionFunction::Polynomial { .. } => {
                                "a variable amount based on a polynomial function".to_string()
                            }
                            DistributionFunction::Exponential { .. } => {
                                "a variable amount based on an exponential function".to_string()
                            }
                            DistributionFunction::Logarithmic { .. } => {
                                "a variable amount based on a logarithmic function".to_string()
                            }
                            DistributionFunction::InvertedLogarithmic { .. } => {
                                "a variable amount based on an inverted logarithmic function".to_string()
                            }
                        };

                        match perpetual_distribution.distribution_type() {
                            RewardDistributionType::BlockBasedDistribution { interval, .. } => {
                                let block_str = if *interval == 1 { "block" } else { "blocks" };
                                format!(
                                    "This token is using a block based distribution where every {} {} it will distribute {}.",
                                    interval, block_str, function_string
                                )
                            }
                            RewardDistributionType::TimeBasedDistribution { interval, .. } => {
                                let duration = Duration::from_millis(*interval);
                                let interval_str = humantime::format_duration(duration).to_string();
                                format!(
                                    "This token is using a time based distribution where every {} it will distribute {}.",
                                    interval_str, function_string
                                )
                            }
                            RewardDistributionType::EpochBasedDistribution { interval, .. } => {
                                let epoch_str = if *interval == 1 { "epoch" } else { "epochs" };
                                format!(
                                    "This token is using an epoch based distribution where every {} {} it will distribute {}.",
                                    interval, epoch_str, function_string
                                )
                            }
                        }
                    } else {
                        String::new()
                    };
                    ui.label(format!("A perpetual distribution can only claim 128 cycles at a time, except for fixed amount distributions where you can claim 32,767 cycles.\n\n\
                    If your token would pay out every hour 1 Token, then you could only claim 128 hours worth of tokens in one claim, you can issue multiple claims back to back until you have nothing left to claim.\n\n\
                    {}", extra_info));
                    ui.add_space(10.0);
                }

                let button = egui::Button::new(RichText::new("Claim").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 0))
                    .corner_radius(3.0);

                if ui.add(button).clicked() {
                    if self.distribution_type.is_none() {
                        self.status = ClaimTokensStatus::ErrorMessage(
                            "Please select a distribution type.".to_string(),
                        );
                        return;
                    } else {
                        self.show_confirmation_popup = true;
                    }
                }

                // If user pressed "Claim," show popup
                if self.show_confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                ui.add_space(10.0);
                match &self.status {
                    ClaimTokensStatus::NotStarted => {}
                    ClaimTokensStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        let elapsed = now - start_time;
                        ui.label(format!("Claiming... elapsed: {}s", elapsed));
                    }
                    ClaimTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(Color32::RED, format!("Error: {}", msg));
                    }
                    ClaimTokensStatus::Complete => {}
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for ClaimTokensScreen {
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
