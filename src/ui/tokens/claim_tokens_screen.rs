use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::components::Component;
use crate::ui::components::MessageBanner;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::helpers::{TransactionType, add_key_chooser};
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
use eframe::egui::{self, Color32, Context, Frame, Margin, Ui};
use egui::RichText;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{wallet_needs_unlock, try_open_wallet_no_password, WalletUnlockPopup, WalletUnlockResult};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use super::tokens_screen::IdentityTokenBasicInfo;

/// States for the claim flow
#[derive(PartialEq)]
pub enum ClaimTokensStatus {
    NotStarted,
    WaitingForResult(u64),
    Error,
    Complete,
}

pub struct ClaimTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_basic_info: IdentityTokenBasicInfo,
    selected_key: Option<dash_sdk::platform::IdentityPublicKey>,
    show_advanced_options: bool,
    pub public_note: Option<String>,
    token_contract: QualifiedContract,
    token_configuration: TokenConfiguration,
    distribution_type: Option<TokenDistributionType>,
    status: ClaimTokensStatus,
    pub app_context: Arc<AppContext>,
    confirmation_dialog: Option<ConfirmationDialog>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
}

impl ClaimTokensScreen {
    pub fn new(
        identity_token_basic_info: IdentityTokenBasicInfo,
        token_contract: QualifiedContract,
        token_configuration: TokenConfiguration,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let identity = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .find(|id| id.identity.id() == identity_token_basic_info.identity_id)
            .expect("No local qualified identity found for this token’s identity.");

        let identity_clone = identity.identity.clone();
        let mut possible_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL]),
            KeyType::all_key_types().into(),
            false,
        );

        if possible_key.is_none() {
            possible_key = identity_clone.get_first_public_key_matching(
                Purpose::TRANSFER,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            );
        }

        let mut wallet_error = None;
        let selected_wallet = get_selected_wallet(&identity, None, possible_key, &mut wallet_error);
        if let Some(e) = wallet_error {
            MessageBanner::set_global(app_context.egui_ctx(), &e, MessageType::Error);
        }

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
            identity_token_basic_info,
            selected_key: possible_key.cloned(),
            show_advanced_options: false,
            public_note: None,
            token_contract,
            token_configuration,
            distribution_type,
            status: ClaimTokensStatus::NotStarted,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
        }
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
        let distribution_type = self
            .distribution_type
            .unwrap_or(TokenDistributionType::Perpetual);

        let dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new(
                "Confirm Claim".to_string(),
                "Are you sure you want to claim tokens for this contract?".to_string(),
            )
        });

        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.confirmation_dialog = None;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.status = ClaimTokensStatus::WaitingForResult(now);

                AppAction::BackendTasks(
                    vec![
                        BackendTask::TokenTask(Box::new(TokenTask::ClaimTokens {
                            data_contract: Arc::new(self.token_contract.contract.clone()),
                            token_position: self.identity_token_basic_info.token_position,
                            actor_identity: self.identity.clone(),
                            distribution_type,
                            signing_key: self.selected_key.clone().expect("No key selected"),
                            public_note: self.public_note.clone(),
                        })),
                        BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                    ],
                    BackendTasksExecutionMode::Sequential,
                )
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Claimed Successfully!".to_string(),
            vec![("Back to Tokens".to_string(), AppAction::PopScreenAndRefresh)],
            None,
        )
    }
}

impl ScreenLike for ClaimTokensScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if let MessageType::Error = message_type {
            self.status = ClaimTokensStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::ClaimedTokens(fee_result) = backend_task_success_result {
            self.completed_fee_result = Some(fee_result);
            self.status = ClaimTokensStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        if let Ok(all) = self.app_context.load_local_qualified_identities()
            && let Some(updated) = all
                .into_iter()
                .find(|id| id.identity.id() == self.identity.identity.id())
        {
            self.identity = updated;
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_basic_info.token_alias,
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

        island_central_panel(ctx, |ui| {
            if self.status == ClaimTokensStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("Claim Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
            let has_keys = if self.app_context.is_developer_mode() {
                !self.identity.identity.public_keys().is_empty()
            } else {
                match self.identity.identity_type {
                    IdentityType::User => !self
                        .identity
                        .available_authentication_keys_with_critical_security_level()
                        .is_empty(),
                    IdentityType::Masternode | IdentityType::Evonode => {
                        !self.identity.available_transfer_keys().is_empty()
                    }
                }
            };

            if !has_keys {
                ui.colored_label(
                    Color32::RED,
                    format!(
                        "No authentication keys with CRITICAL security level found for this {} identity.",
                        self.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                let first_key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
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
                        return;
                    }
                }

                // Header with Advanced Options checkbox
                ui.horizontal(|ui| {
                    ui.heading("Claim Tokens");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Key selection (only in advanced mode)
                if self.show_advanced_options {
                    ui.heading("1. Select the key to sign the Claim transition");
                    ui.add_space(10.0);
                    add_key_chooser(
                        ui,
                        &self.app_context,
                        &self.identity,
                        &mut self.selected_key,
                        TransactionType::TokenClaim,
                    );
                    ui.add_space(10.0);
                }

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
                    ui.heading("Understanding Claim Limitations");
                    ui.add_space(5.0);
                    let extra_info = if let Some(perpetual_distribution) = self
                        .token_configuration
                        .distribution_rules()
                        .perpetual_distribution()
                    {
                        let function_string = match perpetual_distribution
                            .distribution_type()
                            .function()
                        {
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
                                "a variable amount based on an inverted logarithmic function"
                                    .to_string()
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

                // Fee estimation display
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_document_batch(1); // Token operations are document batch transitions

                let dark_mode = ui.ctx().style().visuals.dark_mode;
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

                let button = egui::Button::new(RichText::new("Claim").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 0))
                    .corner_radius(3.0);

                if ui.add(button).clicked() {
                    if self.distribution_type.is_none() {
                        self.status = ClaimTokensStatus::Error;
                        MessageBanner::set_global(
                            ui.ctx(),
                            "Please select a distribution type.",
                            MessageType::Error,
                        );
                        return;
                    } else if self.confirmation_dialog.is_none() {
                        self.confirmation_dialog = Some(ConfirmationDialog::new(
                            "Confirm Claim".to_string(),
                            "Are you sure you want to claim tokens for this contract?".to_string(),
                        ));
                    }
                }

                // Show confirmation dialog if it exists
                if self.confirmation_dialog.is_some() {
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
                    ClaimTokensStatus::Error => {
                        // Error display is handled by the global MessageBanner
                    }
                    ClaimTokensStatus::Complete => {}
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
