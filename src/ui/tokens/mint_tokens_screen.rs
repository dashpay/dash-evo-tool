use super::tokens_screen::IdentityTokenInfo;
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
use crate::ui::helpers::{TransactionType, add_key_chooser, render_group_action_text};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Ui};
use eframe::egui::{Frame, Margin};
use egui::RichText;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
/// Internal states for the mint process.
#[derive(PartialEq)]
pub enum MintTokensStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

/// A UI Screen for minting tokens from an existing token contract
pub struct MintTokensScreen {
    pub identity_token_info: IdentityTokenInfo,
    selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,
    pub public_note: Option<String>,
    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,
    known_identities: Vec<QualifiedIdentity>,

    pub recipient_identity_id: String,

    pub amount: Option<Amount>,
    pub amount_input: Option<AmountInput>,
    status: MintTokensStatus,

    /// Basic references
    pub app_context: Arc<AppContext>,

    /// Confirmation popup
    confirmation_dialog: Option<ConfirmationDialog>,

    // If needed for password-based wallet unlocking:
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    // Banner handle for elapsed time display
    refresh_banner: Option<BannerHandle>,
}

impl MintTokensScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        let known_identities = app_context
            .load_local_qualified_identities()
            .expect("Identities not loaded");

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

        let set_error_banner = |msg: &str| {
            MessageBanner::set_global(app_context.egui_ctx(), msg, MessageType::Error);
        };

        let group = match identity_token_info
            .token_config
            .manual_minting_rules()
            .authorized_to_make_change_action_takers()
        {
            AuthorizedActionTakers::NoOne => {
                set_error_banner("Minting is not allowed on this token");
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != identity_token_info.identity.identity.id()
                {
                    set_error_banner(
                        "You are not allowed to mint this token. Only the contract owner is.",
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    set_error_banner("You are not allowed to mint this token");
                }
                None
            }
            AuthorizedActionTakers::MainGroup => {
                match identity_token_info.token_config.main_control_group() {
                    None => {
                        set_error_banner(
                            "Invalid contract: No main control group, though one should exist",
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
                                set_error_banner(&format!("Invalid contract: {}", e));
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
                        set_error_banner(&format!("Invalid contract: {}", e));
                        None
                    }
                }
            }
        };

        let mut is_unilateral_group_member = false;
        if group.is_some()
            && let Some((_, group)) = group.clone()
        {
            let your_power = group
                .members()
                .get(&identity_token_info.identity.identity.id());

            if let Some(your_power) = your_power
                && your_power >= &group.required_power()
            {
                is_unilateral_group_member = true;
            }
        };

        // Attempt to get an unlocked wallet reference
        let mut wallet_error = None;
        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key.as_ref(),
            &mut wallet_error,
        );
        if let Some(e) = wallet_error {
            set_error_banner(&e);
        }

        Self {
            identity_token_info,
            selected_key: possible_key,
            show_advanced_options: false,
            public_note: None,
            group,
            is_unilateral_group_member,
            group_action_id: None,
            known_identities,
            recipient_identity_id: "".to_string(),
            amount: None,
            amount_input: None,
            status: MintTokensStatus::NotStarted,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
            refresh_banner: None,
        }
    }

    /// Renders an amount input for the user to specify an amount to mint
    fn render_amount_input(&mut self, ui: &mut Ui) {
        // Lazy initialization with proper token configuration
        let amount_input = self.amount_input.get_or_insert_with(|| {
            // Create appropriate Amount based on token configuration
            let token_amount = Amount::from_token(&self.identity_token_info, 0);
            AmountInput::new(token_amount).with_label("Amount to Mint:")
        });

        // Check if input should be disabled when operation is in progress
        let enabled = match self.status {
            MintTokensStatus::WaitingForResult | MintTokensStatus::Complete => false,
            MintTokensStatus::NotStarted | MintTokensStatus::Error => true,
        };

        let response = ui.add_enabled_ui(enabled, |ui| amount_input.show(ui)).inner;

        response.inner.update(&mut self.amount);
        // errors are handled inside AmountInput
    }

    /// Renders an optional text input for the user to specify a "Recipient Identity"
    fn render_recipient_input(&mut self, ui: &mut Ui) {
        let _response = ui.add(
            IdentitySelector::new(
                "mint_recipient_selector",
                &mut self.recipient_identity_id,
                &self.known_identities,
            )
            .width(300.0)
            .label("Recipient:")
            .exclude(&[self.identity_token_info.identity.identity.id()]),
        );

        // If empty, minted tokens go to the 'issuer' identity (self.identity).
    }

    /// Renders a confirm popup with the final "Are you sure?" step
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let msg = format!(
            "Are you sure you want to mint {} tokens to {}?",
            self.amount.clone().unwrap_or(Amount::new(0, 0)),
            self.recipient_identity_id
        );

        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm Mint", msg)
                .confirm_text(Some("Mint"))
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
            self.status = MintTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Invalid amount",
                MessageType::Error,
            );
            return AppAction::None;
        }

        let parsed_receiver_id = Identifier::from_string_try_encodings(
            &self.recipient_identity_id,
            &[
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex,
            ],
        );

        if parsed_receiver_id.is_err() {
            self.status = MintTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Invalid receiver",
                MessageType::Error,
            );
            return AppAction::None;
        }

        let receiver_id = parsed_receiver_id.unwrap();
        self.status = MintTokensStatus::WaitingForResult;
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Minting tokens...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.refresh_banner = Some(handle);

        let data_contract = Arc::new(self.identity_token_info.data_contract.contract.clone());

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

        AppAction::BackendTask(BackendTask::TokenTask(Box::new(TokenTask::MintTokens {
            sending_identity: self.identity_token_info.identity.clone(),
            data_contract,
            token_position: self.identity_token_info.token_position,
            signing_key: self.selected_key.clone().expect("No key selected"),
            public_note: if self.group_action_id.is_some() {
                None
            } else {
                self.public_note.clone()
            },
            recipient_id: Some(receiver_id),
            amount: self.amount.clone().unwrap_or(Amount::new(0, 0)).value(),
            group_info,
        })))
    }
    /// Renders a simple "Success!" screen after completion
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_group_token_success_screen_with_fee(
            ui,
            "Mint",
            self.group_action_id.is_some(),
            self.is_unilateral_group_member,
            self.group.is_some(),
            &self.app_context,
            None,
        )
    }
}

impl ScreenLike for MintTokensScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if let MessageType::Error = message_type {
            if let Some(h) = self.refresh_banner.take() {
                h.clear();
            }
            self.status = MintTokensStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::MintedTokens(fee_result) = backend_task_success_result {
            if let Some(h) = self.refresh_banner.take() {
                h.clear();
            }
            self.completed_fee_result = Some(fee_result);
            self.status = MintTokensStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        // If you need to reload local identity data or re-check keys:
        if let Ok(all_identities) = self.app_context.load_local_user_identities()
            && let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity_token_info.identity.identity.id())
        {
            self.identity_token_info.identity = updated_identity;
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
                    ("Mint", AppAction::None),
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
                    ("Mint", AppAction::None),
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

        let central_panel_action = island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // If we are in the "Complete" status, just show success screen
            if self.status == MintTokensStatus::Complete {
                return self.show_success_screen(ui);
            }

            ui.heading("Mint Tokens");
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
                    ui.heading("Mint Tokens");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Key selection (only in advanced mode)
                if self.show_advanced_options {
                    ui.heading("1. Select the key to sign the Mint transaction");
                    ui.add_space(10.0);
                    add_key_chooser(
                        ui,
                        &self.app_context,
                        &self.identity_token_info.identity,
                        &mut self.selected_key,
                        TransactionType::TokenAction,
                    );
                    ui.add_space(10.0);
                    ui.separator();
                }
                ui.add_space(10.0);

                // Amount to mint
                let step_num = if self.show_advanced_options { 2 } else { 1 };
                ui.heading(format!("{}. Amount to mint", step_num));
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group Mint so you are not allowed to choose the amount.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!(
                        "Amount: {}",
                        self.amount
                            .as_ref()
                            .map(|a| a.to_string())
                            .unwrap_or_default()
                    ));
                } else {
                    self.render_amount_input(ui);
                }

                if self
                    .identity_token_info
                    .token_config
                    .distribution_rules()
                    .minting_allow_choosing_destination()
                    || self.app_context.is_developer_mode()
                {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    if self
                        .identity_token_info
                        .token_config
                        .distribution_rules()
                        .new_tokens_destination_identity()
                        .is_some()
                    {
                        let step_num = if self.show_advanced_options { 3 } else { 2 };
                        ui.heading(format!("{}. Recipient identity (optional)", step_num));
                    } else {
                        let step_num = if self.show_advanced_options { 3 } else { 2 };
                        ui.heading(format!("{}. Recipient identity (required)", step_num));
                    }
                    ui.add_space(5.0);
                    self.render_recipient_input(ui);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                let step_num = if self.show_advanced_options { 4 } else { 3 };
                ui.heading(format!("{}. Public note (optional)", step_num));
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group Mint so you are not allowed to put a note.",
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

                // Fee estimation display
                let fee_estimator = self.app_context.fee_estimator();
                let estimated_fee = fee_estimator.estimate_document_batch(1); // Token operations are document batch transitions

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

                let button_text = render_group_action_text(
                    ui,
                    &self.group,
                    &self.identity_token_info,
                    "Mint",
                    &self.group_action_id,
                );

                // Display estimated fee before action button
                let estimated_fee = fee_estimator.estimate_token_transition();
                ui.add_space(10.0);
                let dark_mode = ui.ctx().style().visuals.dark_mode;
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

                // Mint button
                if self.app_context.is_developer_mode() || !button_text.contains("Test") {
                    ui.add_space(10.0);
                    let button =
                        egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
                            .fill(DashColors::ACTION_BUTTON_BLUE)
                            .corner_radius(3.0);

                    if ui.add(button).clicked() {
                        let msg = format!(
                            "Are you sure you want to mint {} tokens to {}?",
                            self.amount.clone().unwrap_or(Amount::new(0, 0)),
                            self.recipient_identity_id
                        );
                        self.confirmation_dialog = Some(
                            ConfirmationDialog::new("Confirm Mint", msg)
                                .confirm_text(Some("Mint"))
                                .cancel_text(Some("Cancel")),
                        );
                    }
                }

                // If the user pressed "Mint," show a popup
                if self.confirmation_dialog.is_some() {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    MintTokensStatus::NotStarted => {}
                    MintTokensStatus::WaitingForResult => {
                        // Elapsed display is handled by the global MessageBanner
                    }
                    MintTokensStatus::Error => {
                        // Error display is handled by the global MessageBanner
                    }
                    MintTokensStatus::Complete => {
                        // handled above
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
