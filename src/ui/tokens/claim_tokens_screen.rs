use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::model::datetime::local_date_time_from_unix_millis;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::token::once_per_identity_amount;
use crate::model::user_role::UserRole;
use crate::ui::components::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::helpers::{TransactionType, add_key_chooser};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

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
use eframe::egui::{self, Color32, Frame, Margin, Ui};
use egui::RichText;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{MessageType, Screen, ScreenLike};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{wallet_needs_unlock, try_open_wallet_no_password, WalletUnlockPopup, WalletUnlockResult};
use crate::ui::identity::get_selected_wallet;
use crate::ui::identity::keys::add_key_screen::AddKeyScreen;
use crate::ui::identity::keys::key_info_screen::KeyInfoScreen;
use super::tokens_screen::{IdentityTokenBasicInfo, IdentityTokenIdentifier};

/// States for the claim flow
#[derive(PartialEq)]
pub enum ClaimTokensStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

pub struct ClaimTokensScreen {
    pub identity: Option<QualifiedIdentity>,
    pub identity_token_basic_info: IdentityTokenBasicInfo,
    selected_key: Option<dash_sdk::platform::IdentityPublicKey>,
    show_advanced_options: bool,
    pub public_note: Option<String>,
    token_contract: QualifiedContract,
    token_configuration: TokenConfiguration,
    distribution_type: Option<TokenDistributionType>,
    status: ClaimTokensStatus,
    refresh_banner: Option<BannerHandle>,
    pub app_context: Arc<AppContext>,
    token_action_availability: crate::model::token::TokenActionAvailability,
    /// The open claim confirmation and the claim it was opened for.
    confirmation_dialog: Option<PendingClaim>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_open_attempted: bool,
    wallet_unlock_popup: WalletUnlockPopup,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    /// When the selected identity took its once-per-identity claim of this
    /// token, as far as this device knows (a local hint; Platform has no query
    /// for it).
    once_per_identity_claimed_at: Option<u64>,
    /// The network refused this identity's once-per-identity claim while this
    /// screen was open. Unproven node output, so it is never saved; it only
    /// keeps a retry from being charged again on this screen.
    once_per_identity_refused: bool,
}

/// An open claim confirmation and the claim it names: the claim is built when
/// the dialog opens, and that exact claim is sent on confirm.
struct PendingClaim {
    dialog: ConfirmationDialog,
    action: Box<AppAction>,
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
            .find(|id| id.identity.id() == identity_token_basic_info.identity_id);

        let (selected_key, selected_wallet) = if let Some(ref id) = identity {
            let identity_inner = &id.identity;
            let key = identity_inner
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                )
                .or_else(|| {
                    identity_inner.get_first_public_key_matching(
                        Purpose::TRANSFER,
                        HashSet::from([SecurityLevel::CRITICAL]),
                        KeyType::all_key_types().into(),
                        false,
                    )
                })
                .cloned();

            let selected_wallet = get_selected_wallet(id, None, key.as_ref()).unwrap_or(None);
            (key, selected_wallet)
        } else {
            (None, None)
        };

        // Pre-select the distribution when the token has exactly one.
        let rules = token_configuration.distribution_rules();
        let offered: Vec<TokenDistributionType> = [
            (
                rules.perpetual_distribution().is_some(),
                TokenDistributionType::Perpetual,
            ),
            (
                rules.pre_programmed_distribution().is_some(),
                TokenDistributionType::PreProgrammed,
            ),
            (
                once_per_identity_amount(&token_configuration).is_some(),
                TokenDistributionType::OncePerIdentity,
            ),
        ]
        .into_iter()
        .filter_map(|(present, kind)| present.then_some(kind))
        .collect();
        let distribution_type = match offered.as_slice() {
            [only] => Some(*only),
            _ => None,
        };
        let once_per_identity_claimed_at =
            load_once_per_identity_claim(app_context, &identity_token_basic_info);

        let token_action_availability = super::load_token_action_availability(
            app_context,
            &identity_token_basic_info.identity_id,
        );
        Self {
            identity,
            identity_token_basic_info,
            selected_key,
            show_advanced_options: false,
            public_note: None,
            token_contract,
            token_configuration,
            distribution_type,
            status: ClaimTokensStatus::NotStarted,
            refresh_banner: None,
            token_action_availability,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_open_attempted: false,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
            once_per_identity_claimed_at,
            once_per_identity_refused: false,
        }
    }

    fn render_token_distribution_type_selector(&mut self, ui: &mut Ui) {
        let show_perpetual = if let Some(perpetual_distribution) = self
            .token_configuration
            .distribution_rules()
            .perpetual_distribution()
            && let Some(identity) = &self.identity
        {
            match perpetual_distribution.distribution_recipient() {
                TokenDistributionRecipient::ContractOwner => {
                    self.token_contract.contract.owner_id() == identity.identity.id()
                }
                TokenDistributionRecipient::Identity(id) => identity.identity.id() == id,
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
        let show_once_per_identity = once_per_identity_amount(&self.token_configuration).is_some();
        ui.horizontal(|ui| {
            ui.label("Select Distribution Type:");
            egui::ComboBox::from_id_salt("claim_distribution_type_selector")
                .selected_text(match &self.distribution_type {
                    Some(TokenDistributionType::Perpetual) => "Perpetual".to_string(),
                    Some(TokenDistributionType::PreProgrammed) => "PreProgrammed".to_string(),
                    Some(TokenDistributionType::OncePerIdentity) => "Once per identity".to_string(),
                    None => "Select a type".to_string(),
                })
                .show_ui(ui, |ui| {
                    if !show_perpetual && !show_pre_programmed && !show_once_per_identity {
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
                    if show_once_per_identity {
                        ui.selectable_value(
                            &mut self.distribution_type,
                            Some(TokenDistributionType::OncePerIdentity),
                            "Once per identity",
                        );
                    }
                });
        });
    }

    /// What a once-per-identity claim pays, and whether this identity already
    /// took it.
    fn render_once_per_identity_info(&mut self, ui: &mut Ui) -> AppAction {
        let Some(amount) = once_per_identity_amount(&self.token_configuration) else {
            return AppAction::None;
        };
        let mut action = AppAction::None;
        ui.heading("Once-per-identity distribution");
        ui.add_space(5.0);
        ui.label(format!(
            "Every identity can claim {amount} base tokens from this distribution, once."
        ));
        // Advisory only: the hint may be wrong (e.g. recorded on a claim the
        // network later dropped), so it warns but never blocks a claim.
        if let Some(claimed_at_ms) = self.once_per_identity_claimed_at {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(
                RichText::new(format!(
                    "This identity may have already claimed its share on {date}. You can still claim; the network refuses a second claim.",
                    date = local_date_time_from_unix_millis(claimed_at_ms)
                ))
                .color(DashColors::warning_color(dark_mode)),
            );
            if ui.button("Dismiss").clicked() {
                action = self.dismiss_claim_note();
            }
        }
        if self.once_per_identity_refused {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(
                RichText::new(
                    "The network says this identity has already claimed its share, so Claim is turned off. Reopen this screen to try again.",
                )
                .color(DashColors::warning_color(dark_mode)),
            );
        }
        ui.add_space(10.0);
        action
    }

    /// The pair this screen's once-per-identity note is about.
    fn claim_note_pair(&self) -> IdentityTokenIdentifier {
        IdentityTokenIdentifier {
            identity_id: self.identity_token_basic_info.identity_id,
            token_id: self.identity_token_basic_info.token_id,
        }
    }

    /// Forget the once-per-identity note. A backend task saves the dismissal;
    /// the note goes away when it reports success.
    fn dismiss_claim_note(&mut self) -> AppAction {
        AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::ClearOncePerIdentityClaimHint(self.claim_note_pair()),
        )))
    }

    /// Ask the user to confirm the claim of the selected distribution. The
    /// claim is built now and the dialog blocks the form behind it, so what
    /// the user confirms is what is sent.
    fn open_claim_confirmation(&mut self) {
        let Some(distribution_type) = self.distribution_type else {
            return;
        };
        let Some(identity) = self.identity.clone() else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Identity not available",
                MessageType::Error,
            );
            self.status = ClaimTokensStatus::Error;
            return;
        };
        let Some(signing_key) = self.selected_key.clone() else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "No signing key selected",
                MessageType::Error,
            );
            self.status = ClaimTokensStatus::Error;
            return;
        };
        let action = AppAction::BackendTasks(
            vec![
                BackendTask::TokenTask(Box::new(TokenTask::ClaimTokens {
                    data_contract: Arc::new(self.token_contract.contract.clone()),
                    token_position: self.identity_token_basic_info.token_position,
                    actor_identity: identity,
                    distribution_type,
                    signing_key,
                    public_note: self.public_note.clone(),
                })),
                BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
            ],
            BackendTasksExecutionMode::Sequential,
        );
        self.confirmation_dialog = Some(PendingClaim {
            dialog: ConfirmationDialog::new(
                "Confirm Claim".to_string(),
                claim_confirmation_text(
                    distribution_type,
                    &self.identity_token_basic_info.token_alias,
                    once_per_identity_amount(&self.token_configuration),
                ),
            )
            .blocks_input(true),
            action: Box::new(action),
        });
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let Some(pending) = self.confirmation_dialog.as_mut() else {
            return AppAction::None;
        };
        match pending.dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => self.confirm_claim(),
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    /// The user confirmed: send the claim the confirmation was opened for.
    fn confirm_claim(&mut self) -> AppAction {
        let Some(pending) = self.confirmation_dialog.take() else {
            return AppAction::None;
        };
        self.status = ClaimTokensStatus::WaitingForResult;
        self.refresh_banner.take_and_clear();
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Claiming tokens...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.refresh_banner = Some(handle);
        *pending.action
    }

    /// Whether Claim may be pressed for the selected distribution.
    fn claim_allowed(&self) -> bool {
        !(self.once_per_identity_refused
            && self.distribution_type == Some(TokenDistributionType::OncePerIdentity))
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
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.refresh_banner.take_and_clear();
            self.status = ClaimTokensStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        self.refresh_banner.take_and_clear();
        match backend_task_success_result {
            BackendTaskSuccessResult::ClaimedTokens(fee_result) => {
                self.completed_fee_result = Some(fee_result);
                self.status = ClaimTokensStatus::Complete;
                // A successful once-per-identity claim records its hint.
                self.once_per_identity_claimed_at = load_once_per_identity_claim(
                    &self.app_context,
                    &self.identity_token_basic_info,
                );
            }
            BackendTaskSuccessResult::OncePerIdentityClaimHintCleared(pair)
                if pair == self.claim_note_pair() =>
            {
                self.once_per_identity_claimed_at = None;
            }
            _ => {}
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if let TaskError::TokenOncePerIdentityAlreadyClaimed {
            token_id,
            identity_id,
            ..
        } = error
            && *token_id == self.identity_token_basic_info.token_id
            && *identity_id == self.identity_token_basic_info.identity_id
        {
            self.once_per_identity_refused = true;
        }
        false
    }

    fn refresh(&mut self) {
        self.token_action_availability = super::load_token_action_availability(
            &self.app_context,
            &self.identity_token_basic_info.identity_id,
        );
        if let Some(current) = &self.identity
            && let Ok(all) = self.app_context.load_local_qualified_identities()
            && let Some(updated) = all
                .into_iter()
                .find(|id| id.identity.id() == current.identity.id())
        {
            self.identity = Some(updated);
        }
        self.once_per_identity_claimed_at =
            load_once_per_identity_claim(&self.app_context, &self.identity_token_basic_info);
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
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
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ui, &self.app_context);

        island_central_panel(ui, |ui| {
            if self.status == ClaimTokensStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            let Some(identity) = self.identity.as_ref() else {
                ui.colored_label(
                    egui::Color32::RED,
                    "Identity not found in local store. Please refresh or re-open this screen.",
                );
                return;
            };

            ui.heading("Claim Tokens");
            ui.add_space(10.0);

            if super::show_token_actions_unavailable(ui, self.token_action_availability) {
                return;
            }

            // Check if user has any auth keys
            let has_keys = if self.app_context.user_role().at_least(UserRole::Developer) {
                !identity.identity.public_keys().is_empty()
            } else {
                match identity.identity_type {
                    IdentityType::User => !identity
                        .available_authentication_keys_with_critical_security_level()
                        .is_empty(),
                    IdentityType::Masternode | IdentityType::Evonode => {
                        !identity.available_transfer_keys().is_empty()
                    }
                }
            };

            if !has_keys {
                ui.colored_label(
                    Color32::RED,
                    format!(
                        "No authentication keys with CRITICAL security level found for this {} identity.",
                        identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                let first_key = identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = first_key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                // Possibly handle locked wallet scenario
                if let Some(wallet) = &self.selected_wallet {
                    if !self.wallet_open_attempted {
                        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                            let handle = MessageBanner::set_global(
                                self.app_context.egui_ctx(),
                                "Unable to open wallet. Please unlock it and try again.",
                                MessageType::Error,
                            );
                            handle.with_details(e);
                            handle.disable_auto_dismiss();
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
                        identity,
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
                    ui.label("Public note (optional):").info_tooltip(
                        "A note about the transaction that can be seen by the public.",
                    );
                    ui.add_space(10.0);
                    let mut txt = self.public_note.clone().unwrap_or_default();
                    if ui.text_edit_singleline(&mut txt).changed() {
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

                if self.distribution_type == Some(TokenDistributionType::OncePerIdentity) {
                    action |= self.render_once_per_identity_info(ui);
                }

                // Fee estimation display
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_document_batch(1); // Token operations are document batch transitions

                let dark_mode = ui.style().visuals.dark_mode;
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

                if ComponentStyles::add_primary_button_enabled(ui, self.claim_allowed(), "Claim")
                    .clicked()
                {
                    if self.distribution_type.is_none() {
                        MessageBanner::set_global(
                            ctx,
                            "Please select a distribution type.",
                            MessageType::Error,
                        );
                        self.status = ClaimTokensStatus::Error;
                        return;
                    } else if self.confirmation_dialog.is_none() {
                        self.open_claim_confirmation();
                    }
                }

                // Show confirmation dialog if it exists
                if self.confirmation_dialog.is_some() {
                    action |= self.show_confirmation_popup(ui);
                }

                // Status display is handled by the global MessageBanner
                // (progress with elapsed timer, errors, etc.)
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

/// The claim confirmation text: which distribution of `token` is claimed and,
/// for the once-per-identity distribution, how much.
fn claim_confirmation_text(
    distribution_type: TokenDistributionType,
    token: &str,
    once_per_identity_amount: Option<dash_sdk::dpp::balances::credits::TokenAmount>,
) -> String {
    match (distribution_type, once_per_identity_amount) {
        (TokenDistributionType::Perpetual, _) => format!(
            "You are about to claim the {token} rewards this identity has earned from the perpetual distribution."
        ),
        (TokenDistributionType::PreProgrammed, _) => format!(
            "You are about to claim the {token} this identity is due from the pre-programmed distribution."
        ),
        (TokenDistributionType::OncePerIdentity, Some(amount)) => format!(
            "You are about to claim this identity's one-time share of {amount} base {token} tokens from the once-per-identity distribution. Each identity can claim it only once."
        ),
        (TokenDistributionType::OncePerIdentity, None) => format!(
            "You are about to claim this identity's one-time share of {token} from the once-per-identity distribution. Each identity can claim it only once."
        ),
    }
}

/// The once-per-identity claim hint for the screen's identity and token. A
/// read failure only hides the hint (Platform still refuses a second claim),
/// so it is logged and treated as unknown.
fn load_once_per_identity_claim(
    app_context: &AppContext,
    info: &IdentityTokenBasicInfo,
) -> Option<u64> {
    app_context
        .once_per_identity_claimed_at(&info.token_id, &info.identity_id)
        .unwrap_or_else(|error| {
            tracing::warn!(?error, "Could not read the once-per-identity claim hint");
            None
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::IdentityStatus;
    use crate::ui::tokens::tokens_screen::IdentityTokenBasicInfo;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};

    /// A token offering only a once-per-identity distribution of `amount`.
    fn once_per_identity_token(amount: u64) -> TokenConfiguration {
        let mut config = TokenConfigurationV0::default_most_restrictive();
        let TokenDistributionRules::V0(rules) = config.distribution_rules.clone() else {
            panic!("the default rules are version 0");
        };
        config.distribution_rules =
            crate::model::token::distribution_rules_with_once_per_identity(rules, Some(amount));
        config.into()
    }

    fn screen_for(config: TokenConfiguration) -> (ClaimTokensScreen, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let contract = QualifiedContract {
            contract: load_system_data_contract(
                SystemDataContract::DPNS,
                PlatformVersion::latest(),
            )
            .expect("a contract"),
            alias: None,
        };
        let info = IdentityTokenBasicInfo {
            token_id: Identifier::new([0x70; 32]),
            token_alias: "Gold".to_string(),
            identity_id: Identifier::new([0x71; 32]),
            contract_id: contract.contract.id(),
            token_position: 0,
        };
        let mut screen = ClaimTokensScreen::new(info, contract, config, &context);
        screen.identity = Some(QualifiedIdentity {
            identity: Identity::create_basic_identity(
                Identifier::new([0x71; 32]),
                PlatformVersion::latest(),
            )
            .expect("basic identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        });
        screen.selected_key = Some(IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id: 1,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::CRITICAL,
            contract_bounds: None,
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: vec![2; 33].into(),
            disabled_at: None,
        }));
        (screen, dir)
    }

    /// Regression: the confirmation asked only whether to "claim tokens for
    /// this contract", so the user could not see which distribution, or for a
    /// one-time claim how much, they were about to spend their claim on.
    #[test]
    fn the_claim_confirmation_names_the_distribution_and_the_amount() {
        let text =
            claim_confirmation_text(TokenDistributionType::OncePerIdentity, "Gold", Some(25));
        assert!(text.contains("Gold"), "{text}");
        assert!(text.contains("once-per-identity"), "{text}");
        assert!(text.contains("25"), "{text}");
        for (kind, name) in [
            (TokenDistributionType::Perpetual, "perpetual"),
            (TokenDistributionType::PreProgrammed, "pre-programmed"),
        ] {
            let text = claim_confirmation_text(kind, "Gold", None);
            assert!(text.contains(name), "{text}");
            assert!(text.contains("Gold"), "{text}");
        }
    }

    /// Regression: Confirm read the distribution at confirm time, so one
    /// changed while the dialog was open was claimed instead of the one shown.
    #[test]
    fn confirming_claims_the_distribution_the_confirmation_named() {
        let (mut screen, _dir) = screen_for(once_per_identity_token(25));
        assert_eq!(
            screen.distribution_type,
            Some(TokenDistributionType::OncePerIdentity)
        );
        screen.open_claim_confirmation();
        screen.distribution_type = Some(TokenDistributionType::Perpetual);

        let AppAction::BackendTasks(tasks, _) = screen.confirm_claim() else {
            panic!("the confirmed claim is sent");
        };
        let Some(BackendTask::TokenTask(task)) = tasks.first() else {
            panic!("the claim comes first");
        };
        let TokenTask::ClaimTokens {
            distribution_type, ..
        } = task.as_ref()
        else {
            panic!("a claim was confirmed");
        };
        assert_eq!(*distribution_type, TokenDistributionType::OncePerIdentity);
    }

    /// Regression: a refused second claim left Claim enabled with no note, so
    /// every retry was charged again. The refusal now disables Claim for this
    /// screen, but it is unproven node output, so nothing is saved.
    #[test]
    fn a_refused_second_claim_disables_claim_without_saving_anything() {
        let (mut screen, _dir) = screen_for(once_per_identity_token(25));
        let refusal = |identity_id| TaskError::TokenOncePerIdentityAlreadyClaimed {
            token_id: screen_token_id(),
            identity_id,
            claimed_at_ms: 1_758_140_722_000,
            source_error: Box::new(dash_sdk::Error::Generic("claimed".to_string())),
        };
        assert!(screen.claim_allowed());

        assert!(!screen.display_task_error(&refusal(Identifier::new([0x99; 32]))));
        assert!(
            screen.claim_allowed(),
            "a refusal for another identity changes nothing"
        );

        assert!(
            !screen.display_task_error(&refusal(Identifier::new([0x71; 32]))),
            "the refusal banner still shows"
        );
        assert!(!screen.claim_allowed());
        assert_eq!(
            screen.once_per_identity_claimed_at, None,
            "the refusal does not become the saved hint"
        );
        assert_eq!(
            load_once_per_identity_claim(&screen.app_context, &screen.identity_token_basic_info),
            None
        );

        screen.distribution_type = Some(TokenDistributionType::PreProgrammed);
        assert!(
            screen.claim_allowed(),
            "other distributions of the token stay claimable"
        );
    }

    fn screen_token_id() -> Identifier {
        Identifier::new([0x70; 32])
    }

    /// Dismissing the claim note is saved by a backend task, not from the
    /// click; the note goes away once that task reports success for this
    /// identity and token.
    #[test]
    fn dismissing_the_claim_note_is_saved_by_a_backend_task() {
        let (mut screen, _dir) = screen_for(once_per_identity_token(25));
        screen.once_per_identity_claimed_at = Some(1_758_140_722_000);
        let pair = IdentityTokenIdentifier {
            identity_id: screen.identity_token_basic_info.identity_id,
            token_id: screen.identity_token_basic_info.token_id,
        };

        let AppAction::BackendTask(BackendTask::TokenTask(task)) = screen.dismiss_claim_note()
        else {
            panic!("the dismissal is a backend task");
        };
        assert!(matches!(
            task.as_ref(),
            TokenTask::ClearOncePerIdentityClaimHint(sent) if *sent == pair
        ));
        assert!(
            screen.once_per_identity_claimed_at.is_some(),
            "the note stays until the dismissal is saved"
        );

        let other = IdentityTokenIdentifier {
            identity_id: Identifier::new([0x99; 32]),
            ..pair
        };
        screen.display_task_result(BackendTaskSuccessResult::OncePerIdentityClaimHintCleared(
            other,
        ));
        assert!(screen.once_per_identity_claimed_at.is_some());

        screen.display_task_result(BackendTaskSuccessResult::OncePerIdentityClaimHintCleared(
            pair,
        ));
        assert_eq!(screen.once_per_identity_claimed_at, None);
    }
}
