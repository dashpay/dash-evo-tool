use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::component_trait::Component;
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
use crate::ui::tokens::validate_signing_key;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Frame, Margin, Ui};
use egui::RichText;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Internal states for the freeze operation
#[derive(PartialEq)]
pub enum FreezeTokensStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

/// A UI Screen that allows freezing an identity’s tokens for a particular contract
pub struct FreezeTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_info: IdentityTokenInfo,
    selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,
    pub public_note: Option<String>,

    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,
    known_identities: Vec<QualifiedIdentity>,

    /// The identity we want to freeze
    pub freeze_identity_id: String,

    status: FreezeTokensStatus,

    // Basic references
    pub app_context: Arc<AppContext>,

    // Confirmation dialog
    confirmation_dialog: Option<ConfirmationDialog>,

    // If password-based wallet unlocking is needed
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    // Banner handle for elapsed time display
    refresh_banner: Option<BannerHandle>,
}

impl FreezeTokensScreen {
    pub fn new(identity_token_info: IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
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
            .freeze_rules()
            .authorized_to_make_change_action_takers()
        {
            AuthorizedActionTakers::NoOne => {
                set_error_banner("Freezing is not allowed on this token");
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != identity_token_info.identity.identity.id()
                {
                    set_error_banner(
                        "You are not allowed to freeze this token. Only the contract owner is.",
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    set_error_banner("You are not allowed to freeze this token");
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
        let selected_wallet =
            get_selected_wallet(&identity_token_info.identity, None, possible_key.as_ref())
                .unwrap_or_else(|e| {
                    set_error_banner(&e);
                    None
                });

        Self {
            identity: identity_token_info.identity.clone(),
            identity_token_info,
            selected_key: possible_key,
            show_advanced_options: false,
            group,
            is_unilateral_group_member,
            group_action_id: None,
            public_note: None,
            freeze_identity_id: String::new(),
            status: FreezeTokensStatus::NotStarted,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            known_identities,
            completed_fee_result: None,
            refresh_banner: None,
        }
    }

    /// Renders text input for the identity to freeze
    fn render_freeze_identity_input(&mut self, ui: &mut Ui) {
        let _response = ui.add(
            IdentitySelector::new(
                "freeze_identity_selector",
                &mut self.freeze_identity_id,
                &self.known_identities,
            )
            .label("Freeze Identity ID:")
            .width(300.0),
        );
    }

    /// Confirmation popup
    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let msg = format!(
            "Are you sure you want to freeze identity {}?",
            self.freeze_identity_id
        );

        let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new("Confirm Freeze", msg)
                .confirm_text(Some("Confirm"))
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

    /// Handle confirmation OK action
    fn confirmation_ok(&mut self) -> AppAction {
        // Validate user input
        let Ok(freeze_id) = Identifier::from_string_try_encodings(
            &self.freeze_identity_id,
            &[
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58,
                dash_sdk::dpp::platform_value::string_encoding::Encoding::Hex,
            ],
        ) else {
            self.status = FreezeTokensStatus::Error;
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Please enter a valid identity ID.",
                MessageType::Error,
            );
            return AppAction::None;
        };

        // Validate signing key before transitioning to waiting state
        let Some(signing_key) = validate_signing_key(&self.app_context, &self.selected_key) else {
            return AppAction::None;
        };

        self.status = FreezeTokensStatus::WaitingForResult;
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Freezing tokens...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.refresh_banner = Some(handle);

        // Grab the data contract for this token from the app context
        let data_contract = Arc::new(self.identity_token_info.data_contract.contract.clone());

        let group_info = if let Some(action_id) = self.group_action_id {
            self.group.as_ref().map(|(pos, _)| {
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoOtherSigner(
                    GroupStateTransitionInfo {
                        group_contract_position: *pos,
                        action_id,
                        action_is_proposer: false,
                    },
                )
            })
        } else {
            self.group.as_ref().map(|(pos, _)| {
                GroupStateTransitionInfoStatus::GroupStateTransitionInfoProposer(*pos)
            })
        };

        // Dispatch to backend
        AppAction::BackendTask(BackendTask::TokenTask(Box::new(TokenTask::FreezeTokens {
            actor_identity: self.identity.clone(),
            data_contract,
            token_position: self.identity_token_info.token_position,
            signing_key,
            public_note: if self.group_action_id.is_some() {
                None
            } else {
                self.public_note.clone()
            },
            freeze_identity: freeze_id,
            group_info,
        })))
    }

    /// Success screen
    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_group_token_success_screen_with_fee(
            ui,
            "Freeze",
            self.group_action_id.is_some(),
            self.is_unilateral_group_member,
            self.group.is_some(),
            &self.app_context,
            None,
        )
    }
}

impl ScreenLike for FreezeTokensScreen {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if let MessageType::Error = message_type {
            if let Some(h) = self.refresh_banner.take() {
                h.clear();
            }
            self.status = FreezeTokensStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::FrozeTokens(fee_result) = backend_task_success_result {
            if let Some(h) = self.refresh_banner.take() {
                h.clear();
            }
            self.completed_fee_result = Some(fee_result);
            self.status = FreezeTokensStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        // Reload identity if needed
        if let Ok(all_identities) = self.app_context.load_local_user_identities()
            && let Some(updated_identity) = all_identities
                .into_iter()
                .find(|id| id.identity.id() == self.identity.identity.id())
        {
            self.identity = updated_identity;
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
                    ("Freeze", AppAction::None),
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
                    ("Freeze", AppAction::None),
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
            if self.status == FreezeTokensStatus::Complete {
                return self.show_success_screen(ui);
            }

            ui.heading("Freeze Identity’s Tokens");
            ui.add_space(10.0);

            // Check if user has any auth keys
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
                    Color32::RED,
                    format!(
                        "No authentication keys with CRITICAL security level found for this {} identity.",
                        self.identity.identity_type,
                    ),
                );
                ui.add_space(10.0);

                // Show "Add key" or "Check keys" option
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
                        return AppAction::None;
                    }
                }

                // Header with Advanced Options checkbox
                ui.horizontal(|ui| {
                    ui.heading("Freeze Tokens");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Key selection (only in advanced mode)
                if self.show_advanced_options {
                    ui.heading("1. Select the key to sign the Freeze transition");
                    ui.add_space(10.0);
                    add_key_chooser(
                        ui,
                        &self.app_context,
                        &self.identity,
                        &mut self.selected_key,
                        TransactionType::TokenAction,
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                }

                // Identity to freeze
                let step_num = if self.show_advanced_options { 2 } else { 1 };
                ui.heading(format!("{}. Enter the identity ID to freeze", step_num));
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group Freeze so you are not allowed to choose the identity.",
                    );
                    ui.add_space(5.0);
                    ui.label(format!("Identity: {}", self.freeze_identity_id));
                } else {
                    self.render_freeze_identity_input(ui);
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                let step_num = if self.show_advanced_options { 3 } else { 2 };
                ui.heading(format!("{}. Public note (optional)", step_num));
                ui.add_space(5.0);
                if self.group_action_id.is_some() {
                    ui.label(
                        "You are signing an existing group Freeze so you are not allowed to put a note.",
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

                let button_text = render_group_action_text(
                    ui,
                    &self.group,
                    &self.identity_token_info,
                    "Freeze",
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

                // Freeze button
                if self.app_context.is_developer_mode() || !button_text.contains("Test") {
                    ui.add_space(10.0);
                    let button =
                        egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
                            .fill(DashColors::ACTION_BUTTON_BLUE)
                            .corner_radius(3.0);

                    if ui.add(button).clicked() {
                        // Initialize confirmation dialog when button is clicked
                        self.confirmation_dialog = None; // Reset for fresh dialog
                    }
                }

                // Show confirmation dialog if it exists
                if self.confirmation_dialog.is_some() {
                    action |= self.show_confirmation_popup(ui);
                }

                // Show in-progress or error messages
                ui.add_space(10.0);
                match &self.status {
                    FreezeTokensStatus::NotStarted => {
                        // no-op
                    }
                    FreezeTokensStatus::WaitingForResult => {
                        // Elapsed display is handled by the global MessageBanner
                    }
                    FreezeTokensStatus::Error => {
                        // Error display is handled by the global MessageBanner
                    }
                    FreezeTokensStatus::Complete => {
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
