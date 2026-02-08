use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::FeeResult;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::Screen;
use crate::ui::components::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::DashColors;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Color32, Frame, Margin, Ui};
use egui::RichText;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Shared status enum for all token operation screens.
#[derive(PartialEq)]
pub enum OperationStatus {
    NotStarted,
    WaitingForResult(u64),
    ErrorMessage(String),
    Complete,
}

/// Shared base struct containing fields and logic common to all token operation screens.
pub struct TokenOperationBase {
    pub identity: QualifiedIdentity,
    pub identity_token_info: IdentityTokenInfo,
    pub selected_key: Option<dash_sdk::platform::IdentityPublicKey>,
    pub show_advanced_options: bool,
    pub group: Option<(GroupContractPosition, Group)>,
    pub is_unilateral_group_member: bool,
    pub group_action_id: Option<Identifier>,
    pub public_note: Option<String>,
    pub status: OperationStatus,
    pub error_message: Option<String>,
    pub app_context: Arc<AppContext>,
    pub confirmation_dialog: Option<ConfirmationDialog>,
    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    pub wallet_unlock_popup: WalletUnlockPopup,
    pub completed_fee_result: Option<FeeResult>,
}

impl TokenOperationBase {
    /// Create a new base with shared initialization logic.
    ///
    /// `rules_accessor` extracts the relevant `ChangeControlRules` from the token config
    /// (e.g., `|c| c.emergency_action_rules()` for pause/resume).
    pub fn new<F>(
        identity_token_info: &IdentityTokenInfo,
        app_context: &Arc<AppContext>,
        rules_accessor: F,
    ) -> Self
    where
        F: FnOnce(&dyn TokenConfigurationV0Getters) -> &ChangeControlRules,
    {
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

        let rules = rules_accessor(&identity_token_info.token_config);

        let group = match rules.authorized_to_make_change_action_takers() {
            AuthorizedActionTakers::NoOne => {
                error_message = Some("This action is not allowed on this token".to_string());
                None
            }
            AuthorizedActionTakers::ContractOwner => {
                if identity_token_info.data_contract.contract.owner_id()
                    != identity_token_info.identity.identity.id()
                {
                    error_message = Some(
                        "You are not allowed to perform this action. Only the contract owner is."
                            .to_string(),
                    );
                }
                None
            }
            AuthorizedActionTakers::Identity(identifier) => {
                if identifier != &identity_token_info.identity.identity.id() {
                    error_message = Some("You are not allowed to perform this action".to_string());
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

        let selected_wallet = get_selected_wallet(
            &identity_token_info.identity,
            None,
            possible_key.as_ref(),
            &mut error_message,
        );

        Self {
            identity: identity_token_info.identity.clone(),
            identity_token_info: identity_token_info.clone(),
            selected_key: possible_key,
            show_advanced_options: false,
            group,
            is_unilateral_group_member,
            group_action_id: None,
            public_note: None,
            status: OperationStatus::NotStarted,
            error_message,
            app_context: app_context.clone(),
            confirmation_dialog: None,
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            completed_fee_result: None,
        }
    }

    /// Render key validation check. Returns `true` if the user has valid keys and
    /// rendering should continue; `false` if no keys found (renders error + buttons).
    pub fn render_key_check(&self, ui: &mut Ui, action: &mut AppAction) -> bool {
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

            let first_key = self.identity.identity.get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                HashSet::from([SecurityLevel::CRITICAL]),
                KeyType::all_key_types().into(),
                false,
            );

            if let Some(key) = first_key {
                if ui.button("Check Keys").clicked() {
                    *action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                        self.identity.clone(),
                        key.clone(),
                        None,
                        &self.app_context,
                    )));
                }
                ui.add_space(5.0);
            }

            if ui.button("Add key").clicked() {
                *action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                    self.identity.clone(),
                    &self.app_context,
                )));
            }
            return false;
        }
        true
    }

    /// Render the wallet-locked overlay. Returns `true` if the wallet is locked
    /// (caller should return early); `false` if unlocked or no wallet.
    pub fn render_wallet_lock_check(&mut self, ui: &mut Ui) -> bool {
        if let Some(wallet) = &self.selected_wallet {
            if let Err(e) = try_open_wallet_no_password(wallet) {
                self.error_message = Some(e);
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
                return true;
            }
        }
        false
    }

    /// Render the advanced options header with checkbox.
    pub fn render_advanced_header(&mut self, ui: &mut Ui, operation_name: &str) {
        ui.horizontal(|ui| {
            ui.heading(format!("{} Tokens", operation_name));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
            });
        });
        ui.add_space(10.0);
    }

    /// Render key selection in advanced mode.
    pub fn render_key_selection(&mut self, ui: &mut Ui, operation_name: &str) {
        if self.show_advanced_options {
            ui.heading(format!(
                "1. Select the key to sign the {} transition",
                operation_name
            ));
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
        }
        ui.add_space(10.0);
    }

    /// Render public note input.
    pub fn render_public_note(&mut self, ui: &mut Ui, operation_name: &str) {
        let step_num = if self.show_advanced_options { 2 } else { 1 };
        ui.heading(format!("{}. Public note (optional)", step_num));
        ui.add_space(5.0);
        if self.group_action_id.is_some() {
            ui.label(format!(
                "You are signing an existing group {} so you are not allowed to put a note.",
                operation_name
            ));
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
                    .on_hover_text("A note about the transaction that can be seen by the public.")
                    .changed()
                {
                    self.public_note = if !txt.is_empty() { Some(txt) } else { None };
                }
            });
        }
    }

    /// Render the fee estimation display.
    pub fn render_fee_estimation(&self, ui: &mut Ui) {
        let fee_estimator = self.app_context.fee_estimator();
        let estimated_fee = fee_estimator.estimate_document_batch(1);

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
    }

    /// Render the operation status (waiting, error, etc.).
    pub fn render_status(&mut self, ui: &mut Ui, waiting_message: &str) {
        render_operation_status(ui, &mut self.status, waiting_message);
    }

    /// Show the wallet unlock popup. Call this after the central panel.
    pub fn show_wallet_unlock_popup(&mut self, ctx: &egui::Context) {
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
    }

    /// Handle the confirmation dialog flow. Returns the action to dispatch if confirmed.
    /// `build_task` is called when the user confirms, and should return the `AppAction`
    /// to dispatch (typically a `BackendTask`).
    pub fn show_confirmation_popup<F>(&mut self, ui: &mut Ui, build_task: F) -> AppAction
    where
        F: FnOnce(&mut Self) -> AppAction,
    {
        let dialog = match self.confirmation_dialog.as_mut() {
            Some(d) => d,
            None => return AppAction::None,
        };

        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.confirmation_dialog = None;

                if self.selected_key.is_none() {
                    self.error_message = Some("No signing key selected".into());
                    self.status = OperationStatus::ErrorMessage("No key selected".into());
                    return AppAction::None;
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.status = OperationStatus::WaitingForResult(now);

                build_task(self)
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    /// Standard ScreenLike::refresh() implementation.
    pub fn refresh(&mut self) {
        if let Ok(all) = self.app_context.load_local_user_identities()
            && let Some(updated) = all
                .into_iter()
                .find(|id| id.identity.id() == self.identity.identity.id())
        {
            self.identity = updated;
        }
    }
}

/// Standalone function to render an `OperationStatus` with consistent styling.
/// Usable by any screen without requiring `TokenOperationBase`.
pub fn render_operation_status(ui: &mut Ui, status: &mut OperationStatus, waiting_message: &str) {
    ui.add_space(10.0);
    match status {
        OperationStatus::NotStarted => {}
        OperationStatus::WaitingForResult(start_time) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let elapsed = now - *start_time;
            ui.label(format!("{}... elapsed: {}s", waiting_message, elapsed));
        }
        OperationStatus::ErrorMessage(msg) => {
            let error_color = Color32::from_rgb(255, 100, 100);
            let msg = msg.clone();
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Error: {}", msg)).color(error_color));
                        ui.add_space(10.0);
                        if ui.small_button("Dismiss").clicked() {
                            *status = OperationStatus::NotStarted;
                        }
                    });
                });
        }
        OperationStatus::Complete => {}
    }
}
