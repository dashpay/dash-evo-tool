//! Shared scaffold for single-shot token action screens.
//!
//! Pause, resume, freeze, unfreeze, destroy-frozen-funds, burn and mint all share
//! the same shell: authorization resolution, wallet unlock, an advanced key chooser,
//! a per-action form section, a public note, a fee estimate, a confirmation dialog
//! and a success screen. [`TokenActionScreen`] implements that shell once; each
//! action supplies only its differences through the [`TokenAction`] trait.

use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::{BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::user_role::UserRole;
use crate::model::wallet::Wallet;
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::helpers::{
    TransactionType, add_key_chooser, check_token_authorization, render_group_action_text,
};
use crate::ui::identities::get_selected_wallet;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::tokens::validate_signing_key;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::group::{GroupStateTransitionInfo, GroupStateTransitionInfoStatus};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Ui};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Sequential heading numbers ("1.", "2.", …) shared between the scaffold and forms.
pub struct StepCounter(usize);

impl StepCounter {
    fn new() -> Self {
        StepCounter(0)
    }

    /// Advances to and returns the next step index (1-based).
    pub fn advance(&mut self) -> usize {
        self.0 += 1;
        self.0
    }
}

/// Lifecycle of a single-shot token action.
#[derive(PartialEq)]
pub enum TokenActionStatus {
    NotStarted,
    WaitingForResult,
    Error,
    Complete,
}

/// Context handed to a [`TokenAction`] while rendering or dispatching.
pub struct TokenActionCtx<'a> {
    pub info: &'a IdentityTokenInfo,
    pub app_context: &'a Arc<AppContext>,
    /// True when co-signing an existing group action (form inputs are read-only).
    pub is_group_signing: bool,
}

impl TokenActionCtx<'_> {
    /// Shared `Arc` over the token's data contract for backend dispatch.
    pub fn data_contract(&self) -> Arc<dash_sdk::platform::DataContract> {
        Arc::new(self.info.data_contract.contract.clone())
    }
}

/// The per-action differences between the otherwise-identical token action screens.
pub trait TokenAction: Sized + 'static {
    /// Verb used in breadcrumbs, headings and buttons (e.g. `"Pause"`).
    const VERB: &'static str;
    /// Central-panel page heading (e.g. `"Mint Tokens"`).
    const PAGE_HEADING: &'static str;
    /// Info-banner text shown while the operation runs (e.g. `"Minting tokens..."`).
    const PROGRESS: &'static str;
    /// Confirmation dialog title (e.g. `"Confirm Mint"`).
    const CONFIRM_TITLE: &'static str;
    /// Whether the confirmation dialog is styled as destructive.
    const DANGER: bool = false;

    /// Builds the action-specific state.
    fn new(info: &IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self;

    /// The change-control rules that gate this action.
    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers;

    /// Confirmation dialog body; may reflect the action's current inputs.
    fn confirm_message(&self, ctx: &TokenActionCtx) -> String;

    /// Renders the per-action form section between the key chooser and the note.
    fn render_form(&mut self, ui: &mut Ui, ctx: &TokenActionCtx, step: &mut StepCounter) {
        let _ = (ui, ctx, step);
    }

    /// Validates inputs and builds the dispatch action. Returns `None` after showing
    /// an error banner when the inputs are invalid.
    fn build_action(
        &mut self,
        ctx: &TokenActionCtx,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Option<AppAction>;

    /// Extracts the fee from this action's success result variant.
    fn success_fee(result: &BackendTaskSuccessResult) -> Option<FeeResult>;
}

/// State shared by every token action screen, independent of the concrete action.
pub struct TokenActionCommon {
    pub identity_token_info: IdentityTokenInfo,
    pub group_action_id: Option<Identifier>,
    pub public_note: Option<String>,

    selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,
    group: Option<(GroupContractPosition, Group)>,
    is_unilateral_group_member: bool,
    status: TokenActionStatus,
    confirmation_dialog: Option<ConfirmationDialog>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    completed_fee_result: Option<FeeResult>,
    refresh_banner: Option<BannerHandle>,
    app_context: Arc<AppContext>,
}

impl TokenActionCommon {
    fn group_info(&self) -> Option<GroupStateTransitionInfoStatus> {
        if let Some(action_id) = self.group_action_id {
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
        }
    }
}

/// A single-shot token action screen parameterised by its [`TokenAction`].
pub struct TokenActionScreen<A: TokenAction> {
    pub common: TokenActionCommon,
    pub action: A,
}

impl<A: TokenAction> TokenActionScreen<A> {
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

        let takers = A::authorized_takers(&identity_token_info.token_config);
        let auth = check_token_authorization(&takers, &identity_token_info, A::VERB);
        if let Some(msg) = auth.error_message {
            super::set_error_banner(app_context, &msg);
        }

        let selected_wallet =
            get_selected_wallet(&identity_token_info.identity, None, possible_key.as_ref())
                .unwrap_or_else(|e| {
                    super::set_error_banner(app_context, &e);
                    None
                });

        let action = A::new(&identity_token_info, app_context);

        Self {
            common: TokenActionCommon {
                identity_token_info,
                group_action_id: None,
                public_note: None,
                selected_key: possible_key,
                show_advanced_options: false,
                group: auth.group,
                is_unilateral_group_member: auth.is_unilateral_group_member,
                status: TokenActionStatus::NotStarted,
                confirmation_dialog: None,
                selected_wallet,
                wallet_unlock_popup: WalletUnlockPopup::new(),
                wallet_open_attempted: false,
                completed_fee_result: None,
                refresh_banner: None,
                app_context: app_context.clone(),
            },
            action,
        }
    }

    /// Rebinds the shared app context (used on network switch).
    pub fn set_app_context(&mut self, app_context: Arc<AppContext>) {
        self.common.app_context = app_context;
    }

    fn ctx(&self) -> TokenActionCtx<'_> {
        TokenActionCtx {
            info: &self.common.identity_token_info,
            app_context: &self.common.app_context,
            is_group_signing: self.common.group_action_id.is_some(),
        }
    }

    fn show_success_screen(&self, ui: &mut Ui) -> AppAction {
        crate::ui::helpers::show_group_token_success_screen_with_fee(
            ui,
            A::VERB,
            self.common.group_action_id.is_some(),
            self.common.is_unilateral_group_member,
            self.common.group.is_some(),
            &self.common.app_context,
            None,
        )
    }

    /// Renders the missing-key banner with "Check Keys" / "Add key" shortcuts.
    fn render_no_keys(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let identity = &self.common.identity_token_info.identity;
        let dark_mode = ui.style().visuals.dark_mode;

        ui.colored_label(
            DashColors::error_color(dark_mode),
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
                    &self.common.app_context,
                )));
            }
            ui.add_space(5.0);
        }

        if ui.button("Add key").clicked() {
            action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                identity.clone(),
                &self.common.app_context,
            )));
        }

        action
    }

    /// Renders the public-note step (editable, or read-only while group-signing).
    fn render_public_note(&mut self, ui: &mut Ui, step: usize) {
        ui.heading(format!("{}. Public note (optional)", step));
        ui.add_space(5.0);
        if self.common.group_action_id.is_some() {
            ui.label(format!(
                "You are signing an existing group {} so you are not allowed to put a note.",
                A::VERB
            ));
            ui.add_space(5.0);
            ui.label(format!(
                "Note: {}",
                self.common
                    .public_note
                    .clone()
                    .unwrap_or("None".to_string())
            ));
        } else {
            ui.horizontal(|ui| {
                ui.label("Public note (optional):")
                    .info_tooltip("A note about the transaction that can be seen by the public.");
                ui.add_space(10.0);
                let mut txt = self.common.public_note.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut txt).changed() {
                    self.common.public_note = if !txt.is_empty() { Some(txt) } else { None };
                }
            });
        }
    }

    fn render_fee_estimate(&self, ui: &mut Ui) {
        let estimated_fee = self
            .common
            .app_context
            .fee_estimator()
            .estimate_document_batch(1);
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
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let Some(dialog) = self.common.confirmation_dialog.as_mut() else {
            return AppAction::None;
        };
        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                self.common.confirmation_dialog = None;
                self.submit()
            }
            Some(ConfirmationStatus::Canceled) => {
                self.common.confirmation_dialog = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }

    fn submit(&mut self) -> AppAction {
        let Some(signing_key) =
            validate_signing_key(&self.common.app_context, self.common.selected_key.as_ref())
        else {
            return AppAction::None;
        };

        let group_info = self.common.group_info();
        let public_note = if self.common.group_action_id.is_some() {
            None
        } else {
            self.common.public_note.clone()
        };

        let ctx = TokenActionCtx {
            info: &self.common.identity_token_info,
            app_context: &self.common.app_context,
            is_group_signing: self.common.group_action_id.is_some(),
        };

        match self
            .action
            .build_action(&ctx, signing_key, public_note, group_info)
        {
            Some(dispatch) => {
                self.common.status = TokenActionStatus::WaitingForResult;
                let handle = MessageBanner::set_global(
                    self.common.app_context.egui_ctx(),
                    A::PROGRESS,
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.common.refresh_banner = Some(handle);
                dispatch
            }
            None => {
                self.common.status = TokenActionStatus::Error;
                AppAction::None
            }
        }
    }
}

impl<A: TokenAction> ScreenLike for TokenActionScreen<A> {
    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.common.refresh_banner.take_and_clear();
            self.common.status = TokenActionStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let Some(fee_result) = A::success_fee(&backend_task_success_result) {
            self.common.refresh_banner.take_and_clear();
            self.common.completed_fee_result = Some(fee_result);
            self.common.status = TokenActionStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        if let Ok(all) = self.common.app_context.load_local_user_identities()
            && let Some(updated) = all.into_iter().find(|id| {
                id.identity.id() == self.common.identity_token_info.identity.identity.id()
            })
        {
            self.common.identity_token_info.identity = updated;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;

        let breadcrumbs = if self.common.group_action_id.is_some() {
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Group Actions", AppAction::PopScreen),
                (A::VERB, AppAction::None),
            ]
        } else {
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    self.common.identity_token_info.token_alias.as_str(),
                    AppAction::PopScreen,
                ),
                (A::VERB, AppAction::None),
            ]
        };
        let mut action = add_top_panel(ui, &self.common.app_context, breadcrumbs, vec![]);

        action |= add_left_panel(
            ui,
            &self.common.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );
        action |= add_tokens_subscreen_chooser_panel(ui, &self.common.app_context);

        let central_panel_action = island_central_panel(ui, |ui| {
            if self.common.status == TokenActionStatus::Complete {
                return self.show_success_screen(ui);
            }

            ui.heading(A::PAGE_HEADING);
            ui.add_space(10.0);

            let identity = &self.common.identity_token_info.identity;
            let has_keys = if self
                .common
                .app_context
                .user_role()
                .at_least(UserRole::Developer)
            {
                !identity.identity.public_keys().is_empty()
            } else {
                !identity
                    .available_authentication_keys_with_critical_security_level()
                    .is_empty()
            };

            if !has_keys {
                return self.render_no_keys(ui);
            }

            let mut inner_action = AppAction::None;

            // Handle a locked wallet before showing the form.
            if let Some(wallet) = &self.common.selected_wallet {
                if !self.common.wallet_open_attempted {
                    if let Err(e) = try_open_wallet_no_password(&self.common.app_context, wallet) {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                            .disable_auto_dismiss();
                    }
                    self.common.wallet_open_attempted = true;
                }
                if wallet_needs_unlock(wallet) {
                    ui.add_space(10.0);
                    ui.colored_label(
                        Color32::from_rgb(200, 150, 50),
                        "Wallet is locked. Please unlock to continue.",
                    );
                    ui.add_space(8.0);
                    if ui.button("Unlock Wallet").clicked() {
                        self.common.wallet_unlock_popup.open();
                    }
                    return AppAction::None;
                }
            }

            let mut step = StepCounter::new();

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.common.show_advanced_options, "Advanced Options");
            });
            ui.add_space(10.0);

            if self.common.show_advanced_options {
                let n = step.advance();
                ui.heading(format!(
                    "{}. Select the key to sign the {} transition",
                    n,
                    A::VERB
                ));
                ui.add_space(10.0);
                add_key_chooser(
                    ui,
                    &self.common.app_context,
                    &self.common.identity_token_info.identity,
                    &mut self.common.selected_key,
                    TransactionType::TokenAction,
                );
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
            }

            {
                let action_ctx = TokenActionCtx {
                    info: &self.common.identity_token_info,
                    app_context: &self.common.app_context,
                    is_group_signing: self.common.group_action_id.is_some(),
                };
                self.action.render_form(ui, &action_ctx, &mut step);
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            let note_step = step.advance();
            self.render_public_note(ui, note_step);

            ui.add_space(10.0);
            self.render_fee_estimate(ui);

            let button_text = render_group_action_text(
                ui,
                &self.common.group,
                &self.common.identity_token_info,
                A::VERB,
                &self.common.group_action_id,
            );

            if self
                .common
                .app_context
                .user_role()
                .at_least(UserRole::Power)
                || !button_text.contains("Test")
            {
                ui.add_space(10.0);
                if ComponentStyles::add_primary_button(ui, button_text).clicked() {
                    let message = self.action.confirm_message(&self.ctx());
                    let mut dialog = ConfirmationDialog::new(A::CONFIRM_TITLE, message)
                        .confirm_text(Some(A::VERB))
                        .cancel_text(Some("Cancel"));
                    if A::DANGER {
                        dialog = dialog.danger_mode(true);
                    }
                    self.common.confirmation_dialog = Some(dialog);
                }
            }

            if self.common.confirmation_dialog.is_some() {
                inner_action |= self.show_confirmation_popup(ui);
            }

            ui.add_space(10.0);
            inner_action
        });

        action |= central_panel_action;

        if self.common.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.common.selected_wallet
        {
            let _ = self
                .common
                .wallet_unlock_popup
                .show(ctx, wallet, &self.common.app_context);
        }

        action
    }
}
