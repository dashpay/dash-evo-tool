//! Shared scaffold for single-shot token action screens.
//!
//! Pause, resume, freeze, unfreeze, destroy-frozen-funds, burn and mint all share
//! the same shell: authorization resolution, wallet unlock, an advanced key chooser,
//! a per-action form section, a public note, a fee estimate, a confirmation dialog
//! and a success screen. [`TokenActionScreen`] implements that shell once; each
//! action supplies only its differences through the [`TokenAction`] trait.

use super::TokenSigningKey;
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::SigningKeyStatus;
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
use crate::ui::helpers::{TransactionType, check_token_authorization, render_group_action_text};
use crate::ui::identity::get_selected_wallet;
use crate::ui::identity::keys::add_key_screen::AddKeyScreen;
use crate::ui::identity::keys::key_info_screen::KeyInfoScreen;
use crate::ui::state::PendingRequest;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::tokens::validate_signing_key;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
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

/// Shown above a group proposal while the co-signer's wallet is locked.
const LOCKED_PROPOSAL_NOTE: &str = "Your wallet is locked. You can review this proposal now and unlock the wallet when you sign it.";

/// Default tooltip on a disabled primary button: the form is incomplete.
pub const FILL_IN_THE_FORM: &str = "Fill in the form to continue.";

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

    /// Whether the form holds everything [`Self::build_action`] needs. The primary
    /// button stays disabled until it does.
    fn inputs_ready(&self) -> bool {
        true
    }

    /// Tooltip on the disabled primary button: what the user must do before
    /// the action can run.
    fn disabled_reason(&self) -> &'static str {
        FILL_IN_THE_FORM
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

    selected_key: TokenSigningKey,
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
    token_action_availability: crate::model::token::TokenActionAvailability,
    /// The action in flight; only its own result or error ends the wait.
    request: PendingRequest,
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
            .token_action_key(identity_token_info.data_contract.contract.id())
            .cloned();

        let takers = A::authorized_takers(&identity_token_info.token_config);
        let auth = check_token_authorization(&takers, &identity_token_info, A::VERB);
        if let Some(msg) = auth.error_message {
            super::set_error_banner(app_context, &msg);
        }

        let selected_wallet = possible_key
            .as_ref()
            .map_or(Ok(None), |key| {
                get_selected_wallet(&identity_token_info.identity, None, Some(key))
            })
            .unwrap_or_else(|e| {
                super::set_error_banner(app_context, &e);
                None
            });

        let action = A::new(&identity_token_info, app_context);
        let token_action_availability = super::load_token_action_availability(
            app_context,
            &identity_token_info.identity.identity.id(),
        );

        Self {
            common: TokenActionCommon {
                identity_token_info,
                group_action_id: None,
                public_note: None,
                selected_key: TokenSigningKey::automatic(possible_key),
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
                token_action_availability,
                request: PendingRequest::default(),
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
    fn render_no_keys(&self, ui: &mut Ui, key_status: SigningKeyStatus) -> AppAction {
        let mut action = AppAction::None;
        let identity = &self.common.identity_token_info.identity;
        let dark_mode = ui.style().visuals.dark_mode;

        ui.colored_label(
            DashColors::error_color(dark_mode),
            super::token_action_key_message(key_status, identity),
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
                let note_label = ui
                    .label("Public note (optional):")
                    .info_tooltip("A note about the transaction that can be seen by the public.");
                ui.add_space(10.0);
                let mut txt = self.common.public_note.clone().unwrap_or_default();
                if ui
                    .text_edit_singleline(&mut txt)
                    .labelled_by(note_label.id)
                    .changed()
                {
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
            .estimate_token_transition();
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
        if self.common.request.is_pending() {
            return AppAction::None;
        }
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
                self.common
                    .request
                    .dispatch_action_on(dispatch, self.common.app_context.network)
            }
            None => {
                self.common.status = TokenActionStatus::Error;
                AppAction::None
            }
        }
    }
}

impl<A: TokenAction> ScreenLike for TokenActionScreen<A> {
    fn cached_data_changed(&mut self, update: crate::backend_task::CacheUpdate) {
        use crate::backend_task::CacheDomain;
        if update.network == self.common.app_context.network
            && matches!(update.domain, CacheDomain::Identities | CacheDomain::All)
        {
            self.refresh();
        }
    }
    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.common.request.settle(context);
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // After the action's own outcome, a later error (its balance refresh) changes nothing.
        if !self.common.request.is_pending()
            && self.common.status != TokenActionStatus::Complete
            && matches!(message_type, MessageType::Error | MessageType::Warning)
        {
            self.common.refresh_banner.take_and_clear();
            self.common.status = TokenActionStatus::Error;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let Some(fee_result) = A::success_fee(&backend_task_success_result) {
            self.common.request.clear();
            self.common.refresh_banner.take_and_clear();
            self.common.completed_fee_result = Some(fee_result);
            self.common.status = TokenActionStatus::Complete;
        }
    }

    fn refresh(&mut self) {
        self.common.token_action_availability = super::load_token_action_availability(
            &self.common.app_context,
            &self.common.identity_token_info.identity.identity.id(),
        );
        if let Ok(all) = self.common.app_context.load_local_qualified_identities()
            && let Some(updated) = all.into_iter().find(|id| {
                id.identity.id() == self.common.identity_token_info.identity.identity.id()
            })
        {
            self.common.identity_token_info.identity = updated;
        }
        let contract_id = self.common.identity_token_info.data_contract.contract.id();
        if self.common.selected_key.refresh_with_wallet(
            &self.common.app_context,
            &self.common.identity_token_info.identity,
            TransactionType::TokenAction,
            contract_id,
            &mut self.common.selected_wallet,
            &mut self.common.wallet_open_attempted,
        ) {
            self.common.confirmation_dialog = None;
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

            // An identity whose wallet is not loaded yet cannot send token
            // transactions: explain that under the heading instead of the form.
            super::recheck_token_action_availability(
                &self.common.app_context,
                &self.common.identity_token_info.identity.identity.id(),
                &mut self.common.token_action_availability,
            );
            if self
                .common
                .token_action_availability
                .unavailable_reason()
                .is_some()
            {
                ui.heading(A::PAGE_HEADING);
                ui.add_space(10.0);
                super::show_token_actions_unavailable(ui, self.common.token_action_availability);
                return AppAction::None;
            }

            let followed = self.common.selected_key.follow_automatic_with_wallet(
                &self.common.app_context,
                &self.common.identity_token_info.identity,
                TransactionType::TokenAction,
                self.common.identity_token_info.data_contract.contract.id(),
                &mut self.common.selected_wallet,
                &mut self.common.wallet_open_attempted,
            );
            let key_status = followed.status;
            if followed.changed {
                // A pending confirmation was for the previous key.
                self.common.confirmation_dialog = None;
            }
            if key_status == SigningKeyStatus::ManualChoiceOnly {
                // Only a contract-group key can sign it: the user picks it below.
                self.common.show_advanced_options = true;
            }
            let signing_available = matches!(
                key_status,
                SigningKeyStatus::Usable | SigningKeyStatus::ManualChoiceOnly
            );
            // A group proposal stays visible, read-only, to a co-signer
            // without a key that can sign it.
            if !signing_available && self.common.group_action_id.is_none() {
                ui.heading(A::PAGE_HEADING);
                ui.add_space(10.0);
                return self.render_no_keys(ui, key_status);
            }

            let mut inner_action = AppAction::None;

            // A locked wallet hides a new action's form; a co-signer reads
            // the proposal first and unlocks when signing it.
            let mut wallet_locked = false;
            if signing_available && let Some(wallet) = &self.common.selected_wallet {
                if !self.common.wallet_open_attempted {
                    if let Err(e) = try_open_wallet_no_password(&self.common.app_context, wallet) {
                        MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                            .disable_auto_dismiss();
                    }
                    self.common.wallet_open_attempted = true;
                }
                wallet_locked = wallet_needs_unlock(wallet);
                if wallet_locked && self.common.group_action_id.is_none() {
                    ui.heading(A::PAGE_HEADING);
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

            // The checkbox shares the heading's row. A bare right-to-left layout
            // here would take all remaining height and centre the checkbox in it,
            // pushing the whole form to the bottom of the panel.
            ui.horizontal(|ui| {
                ui.heading(A::PAGE_HEADING);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.common.show_advanced_options, "Advanced Options");
                });
            });
            ui.add_space(10.0);
            if !signing_available {
                inner_action |= self.render_no_keys(ui, key_status);
            }
            if wallet_locked {
                ui.colored_label(Color32::from_rgb(200, 150, 50), LOCKED_PROPOSAL_NOTE);
                ui.add_space(10.0);
            }

            if signing_available && self.common.show_advanced_options {
                let n = step.advance();
                ui.heading(format!(
                    "{}. Select the key to sign the {} transition",
                    n,
                    A::VERB
                ));
                ui.add_space(10.0);
                if key_status == SigningKeyStatus::ManualChoiceOnly {
                    ui.label(super::CONTRACT_GROUP_KEY_NOTE);
                    ui.add_space(5.0);
                }
                inner_action |= self.common.selected_key.show_chooser(
                    ui,
                    &self.common.app_context,
                    &self.common.identity_token_info.identity,
                    TransactionType::TokenAction,
                    self.common.identity_token_info.data_contract.contract.id(),
                    &mut self.common.selected_wallet,
                    &mut self.common.wallet_open_attempted,
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
                let steps_before = step.0;
                self.action.render_form(ui, &action_ctx, &mut step);
                // Pause and resume have no form; a separator there would only
                // leave a blank band under the heading.
                if step.0 > steps_before {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                }
            }

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

            if signing_available
                && (self
                    .common
                    .app_context
                    .user_role()
                    .at_least(UserRole::Power)
                    || !button_text.contains("Test"))
            {
                ui.add_space(10.0);
                let in_flight = self.common.request.is_pending();
                let inputs_ready = self.action.inputs_ready();
                let ready =
                    inputs_ready && self.common.selected_key.as_ref().is_some() && !in_flight;
                if ComponentStyles::add_primary_button_enabled(ui, ready, button_text)
                    .disabled_tooltip(if in_flight {
                        "Wait for the current action to finish."
                    } else if !inputs_ready {
                        self.action.disabled_reason()
                    } else {
                        super::CHOOSE_SIGNING_KEY
                    })
                    .clicked()
                {
                    if wallet_locked {
                        self.common.wallet_unlock_popup.open();
                    } else {
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
            }

            if signing_available && !wallet_locked && self.common.confirmation_dialog.is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::amount::Amount;
    use crate::model::qualified_contract::QualifiedContract;
    use crate::ui::tokens::mint_tokens_screen::MintAction;
    use crate::ui::tokens::mint_tokens_screen::MintTokensScreen;
    use crate::ui::tokens::mint_tokens_screen::tests::{signing_key, token_info};
    use crate::ui::tokens::tests::{
        identity_with_expired_key, store_identity_with_usable_wallet_key,
    };
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::platform::Identifier;

    /// Regression: the action's key was picked only when the form opened, so
    /// once the identity's key became usable a refresh opened the form with
    /// no key to sign. A refresh picks it then, with its wallet.
    #[test]
    fn a_refresh_picks_the_token_action_key_that_became_usable() {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = IdentityTokenInfo {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity: identity_with_expired_key(),
            data_contract: QualifiedContract {
                contract: context.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            token_position: 0,
        };
        let mut screen = MintTokensScreen::new(info, &context);
        assert!(screen.common.selected_key.as_ref().is_none());
        let seed_hash = store_identity_with_usable_wallet_key(&context);

        screen.refresh();
        assert_eq!(
            screen.common.selected_key.as_ref().map(|key| key.id()),
            Some(1)
        );
        let wallet = screen
            .common
            .selected_wallet
            .as_ref()
            .expect("the key's wallet");
        assert_eq!(wallet.read().expect("wallet").seed_hash(), seed_hash);
    }

    #[test]
    fn token_refresh_replaces_an_automatic_key_with_a_better_key_and_its_wallet() {
        use crate::context::test_support::{auth_key, identity_holding, test_app_context};
        use crate::ui::tokens::tests::store_identity_with_wallet_key;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
            IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
        };

        let budgeted = auth_key(1, SecurityLevel::CRITICAL).with_limits(Some(1_000_000), None);
        let mut disabled = budgeted.clone();
        disabled.set_disabled_at(1);
        let expired = auth_key(1, SecurityLevel::CRITICAL).with_limits(None, Some(1));

        for refreshed_first_key in [budgeted.clone(), disabled, expired] {
            let dir = tempfile::tempdir().expect("tempdir");
            let context = test_app_context(dir.path());
            let mut identity = identity_holding(std::slice::from_ref(&budgeted));
            let old_wallet = crate::model::wallet::test_support::open_wallet_on(
                crate::test_support::keys::labelled_seed("token_refresh_wallet", 0),
            );
            let old_wallet_hash = old_wallet.seed_hash();
            crate::context::test_support::hold_as_wallet_reference(
                &mut identity,
                1,
                old_wallet_hash,
            );
            identity
                .associated_wallets
                .insert(old_wallet_hash, Arc::new(RwLock::new(old_wallet)));
            let info = IdentityTokenInfo {
                token_id: Identifier::from([4; 32]),
                token_alias: "Test token".into(),
                identity,
                data_contract: QualifiedContract {
                    contract: context.dpns_contract().as_ref().clone(),
                    alias: None,
                },
                token_config: TokenConfigurationV0::default_most_restrictive().into(),
                token_position: 0,
            };
            let mut screen = MintTokensScreen::new(info, &context);
            assert_eq!(
                screen.common.selected_key.as_ref().map(|key| key.id()),
                Some(1)
            );
            screen.common.wallet_open_attempted = true;
            let seed_hash = store_identity_with_wallet_key(
                &context,
                identity_holding(&[refreshed_first_key, auth_key(2, SecurityLevel::CRITICAL)]),
                2,
            );

            screen.refresh();
            assert_eq!(
                screen.common.selected_key.as_ref().map(|key| key.id()),
                Some(2)
            );
            let wallet = screen
                .common
                .selected_wallet
                .as_ref()
                .expect("the new key's wallet");
            assert_eq!(wallet.read().expect("wallet").seed_hash(), seed_hash);
            assert!(!screen.common.wallet_open_attempted);
        }
    }

    /// A confirmation opened for one key is closed when a refresh picks
    /// another, so Confirm never signs with a key the dialog was not opened for.
    #[test]
    fn token_refresh_closes_the_confirmation_when_the_key_changes() {
        use crate::context::test_support::{auth_key, identity_holding, test_app_context};
        use crate::ui::tokens::tests::store_identity_with_wallet_key;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

        let dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(dir.path());
        let budgeted = auth_key(1, SecurityLevel::CRITICAL).with_limits(Some(1_000_000), None);
        let info = IdentityTokenInfo {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity: identity_holding(std::slice::from_ref(&budgeted)),
            data_contract: QualifiedContract {
                contract: context.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            token_position: 0,
        };
        let mut screen = MintTokensScreen::new(info, &context);
        screen.common.confirmation_dialog = Some(ConfirmationDialog::new("Mint", "Mint 5?"));
        store_identity_with_wallet_key(
            &context,
            identity_holding(&[budgeted, auth_key(2, SecurityLevel::CRITICAL)]),
            2,
        );

        screen.refresh();
        assert_eq!(
            screen.common.selected_key.as_ref().map(|key| key.id()),
            Some(2)
        );
        assert!(screen.common.confirmation_dialog.is_none());
    }

    /// Regression: picking a key by hand left the signing wallet as it was,
    /// so with only a contract-group key held by a wallet the form showed no
    /// unlock prompt and Confirm failed at signing.
    #[test]
    fn picking_a_key_by_hand_follows_its_wallet() {
        use crate::context::test_support::{
            auth_key, bound_to, identity_holding, test_app_context,
        };
        use crate::ui::helpers::format_key_label;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        use egui_kittest::kittest::Queryable;

        let dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(dir.path());
        let key = bound_to(
            auth_key(1, SecurityLevel::CRITICAL),
            ContractBounds::ContractGroup {
                id: Identifier::new([6; 32]),
            },
        );
        let mut identity = identity_holding(std::slice::from_ref(&key));
        let wallet = crate::model::wallet::test_support::open_wallet_on(
            crate::test_support::keys::labelled_seed("manual_pick_wallet", 0),
        );
        let seed_hash = wallet.seed_hash();
        crate::context::test_support::hold_as_wallet_reference(&mut identity, 1, seed_hash);
        identity
            .associated_wallets
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));
        let info = IdentityTokenInfo {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity,
            data_contract: QualifiedContract {
                contract: context.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: TokenConfigurationV0::default_most_restrictive().into(),
            token_position: 0,
        };
        let screen = MintTokensScreen::new(info, &context);
        assert!(screen.common.selected_key.as_ref().is_none());
        assert!(screen.common.selected_wallet.is_none());

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut MintTokensScreen| {
                    screen.ui(ui);
                },
                screen,
            );
        harness.run();
        harness.get_by_value("Select Key…").click();
        harness.run();
        harness
            .get_by_label(&format!("{} (contract group)", format_key_label(&key)))
            .click();
        harness.run();

        let screen = harness.state();
        assert_eq!(screen.common.selected_key.as_ref().map(|k| k.id()), Some(1));
        let wallet = screen
            .common
            .selected_wallet
            .as_ref()
            .expect("the chosen key's wallet");
        assert_eq!(wallet.read().expect("wallet").seed_hash(), seed_hash);
    }

    #[test]
    fn token_refresh_keeps_an_explicit_key_only_while_eligible() {
        use crate::context::test_support::{
            auth_key, bound_to, identity_holding, test_app_context,
        };
        use crate::ui::helpers::format_key_label;
        use crate::ui::tokens::tests::store_identity_with_wallet_key;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
            IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
        };
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        use egui_kittest::kittest::Queryable;

        let key = auth_key(1, SecurityLevel::CRITICAL);
        let group_key = bound_to(
            key.clone(),
            ContractBounds::ContractGroup {
                id: Identifier::new([6; 32]),
            },
        );
        let unlimited_key = auth_key(2, SecurityLevel::CRITICAL);
        for (key, warning, other_key) in [
            (
                key.clone().with_limits(Some(1_000_000), None),
                "spending limit",
                None,
            ),
            (group_key, "contract group", None),
            (
                key.with_limits(None, Some(1)),
                "expired",
                Some(unlimited_key.clone()),
            ),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let context = test_app_context(dir.path());
            let mut local_keys = vec![key.clone()];
            local_keys.extend(other_key);
            let info = IdentityTokenInfo {
                token_id: Identifier::from([4; 32]),
                token_alias: "Test token".into(),
                identity: identity_holding(&local_keys),
                data_contract: QualifiedContract {
                    contract: context.dpns_contract().as_ref().clone(),
                    alias: None,
                },
                token_config: TokenConfigurationV0::default_most_restrictive().into(),
                token_position: 0,
            };
            let mut screen = MintTokensScreen::new(info, &context);
            screen.common.show_advanced_options = true;
            let label = format_key_label(&key);
            let combo = screen
                .common
                .selected_key
                .as_ref()
                .map(format_key_label)
                .unwrap_or_else(|| "Select Key…".into());
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1280.0, 900.0))
                .build_ui_state(
                    |ui, screen: &mut MintTokensScreen| {
                        screen.ui(ui);
                    },
                    screen,
                );
            harness.run();
            harness.get_by_value(&combo).click();
            harness.run();
            harness
                .get_by_label(&format!("{label} ({warning})"))
                .click();
            harness.run();

            assert_eq!(
                harness
                    .state()
                    .common
                    .selected_key
                    .as_ref()
                    .map(|key| key.id()),
                Some(1),
                "{warning}: chosen explicitly"
            );

            store_identity_with_wallet_key(
                &context,
                identity_holding(&[key.clone(), unlimited_key.clone()]),
                2,
            );
            harness.state_mut().refresh();
            assert_eq!(
                harness
                    .state()
                    .common
                    .selected_key
                    .as_ref()
                    .map(|key| key.id()),
                Some(1),
                "{warning}: kept after refresh"
            );

            let mut disabled = key;
            disabled.set_disabled_at(1);
            store_identity_with_wallet_key(
                &context,
                identity_holding(&[disabled, unlimited_key.clone()]),
                2,
            );
            harness.state_mut().refresh();
            assert_eq!(
                harness
                    .state()
                    .common
                    .selected_key
                    .as_ref()
                    .map(|key| key.id()),
                Some(2)
            );
        }
    }

    /// The lower of two expiry-limited keys, chosen automatically with its
    /// wallet, expires while the confirmation is open: it is never submitted.
    /// The form moves to the other key and its wallet without a refresh, and
    /// asks for confirmation again.
    #[test]
    fn an_automatic_key_expiring_under_the_confirmation_is_not_submitted() {
        use crate::backend_task::{BackendTask, tokens::TokenTask};
        use crate::context::test_support::{auth_key, identity_holding, test_app_context};
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Setters;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use egui_kittest::kittest::Queryable;
        use std::cell::RefCell;

        let dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(dir.path());
        let expired = auth_key(1, SecurityLevel::CRITICAL).with_limits(None, Some(1));
        let live = auth_key(2, SecurityLevel::CRITICAL).with_limits(None, Some(u64::MAX));
        let mut identity = identity_holding(&[expired.clone(), live]);
        let wallet = crate::model::wallet::test_support::open_wallet_on(
            crate::test_support::keys::labelled_seed("token_expiry_wallet", 0),
        );
        let wallet_hash = wallet.seed_hash();
        crate::context::test_support::hold_as_wallet_reference(&mut identity, 1, wallet_hash);
        let wallet = Arc::new(RwLock::new(wallet));
        identity
            .associated_wallets
            .insert(wallet_hash, wallet.clone());
        let mut config = TokenConfigurationV0::default_most_restrictive();
        config
            .distribution_rules
            .set_new_tokens_destination_identity(Some(Identifier::from([9; 32])));
        let info = IdentityTokenInfo {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity,
            data_contract: QualifiedContract {
                contract: context.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: config.into(),
            token_position: 0,
        };
        let screen = MintTokensScreen::new(info, &context);
        let submitted = RefCell::new(Vec::new());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut MintTokensScreen| {
                    if let AppAction::BackendTasksWithContexts(tasks, _) = screen.ui(ui) {
                        for (task, _) in tasks {
                            if let BackendTask::TokenTask(task) = task
                                && let TokenTask::MintTokens { signing_key, .. } = *task
                            {
                                submitted.borrow_mut().push(signing_key.id());
                            }
                        }
                    }
                },
                screen,
            );
        harness.run();
        let amount_field = "Amount to Mint:";
        let text_input = egui::accesskit::Role::TextInput;
        harness
            .get_by_role_and_label(text_input, amount_field)
            .focus();
        harness.run();
        harness
            .get_by_role_and_label(text_input, amount_field)
            .type_text("5");
        harness.run();
        // Key 1 and its wallet were the automatic choice when the user asked
        // to mint; the key has expired since.
        let common = &mut harness.state_mut().common;
        common.selected_key = super::super::TokenSigningKey::automatic(Some(expired));
        common.selected_wallet = Some(wallet);
        common.wallet_open_attempted = true;
        common.confirmation_dialog = Some(
            ConfirmationDialog::new("Confirm Mint", "Mint the tokens?")
                .confirm_text(Some("Confirm pending mint")),
        );
        harness.run_steps(4);
        if let Some(confirm) = harness.query_by_label("Confirm pending mint") {
            confirm.click();
            harness.run_steps(4);
        }

        assert_eq!(*submitted.borrow(), Vec::<u32>::new());
        let common = &harness.state().common;
        assert_eq!(common.selected_key.as_ref().map(|key| key.id()), Some(2));
        assert!(common.selected_wallet.is_none(), "key 2 needs no wallet");
    }

    /// A co-signer whose key cannot sign for this contract still sees the
    /// group mint it was asked to sign, read-only, but cannot confirm it.
    #[test]
    fn a_group_mint_stays_visible_when_no_key_can_sign_it() {
        use crate::context::test_support::{auth_key, bound_to, identity_holding, test_app_context};
        use crate::model::amount::Amount;
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Setters;
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        use egui_kittest::kittest::Queryable;

        let dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(dir.path());
        let elsewhere = bound_to(
            auth_key(1, SecurityLevel::CRITICAL),
            ContractBounds::SingleContract {
                id: Identifier::from([9; 32]),
            },
        );
        let mut config = TokenConfigurationV0::default_most_restrictive();
        config
            .distribution_rules
            .set_minting_allow_choosing_destination(true);
        let info = IdentityTokenInfo {
            token_id: Identifier::from([4; 32]),
            token_alias: "Test token".into(),
            identity: identity_holding(&[elsewhere]),
            data_contract: QualifiedContract {
                contract: context.dpns_contract().as_ref().clone(),
                alias: None,
            },
            token_config: config.into(),
            token_position: 0,
        };
        let amount = Amount::from_token(&info, 5);
        let recipient = Identifier::from([5; 32]).to_string(Encoding::Base58);
        let mut screen = MintTokensScreen::new(info, &context);
        screen.common.group_action_id = Some(Identifier::from([7; 32]));
        screen.action.amount = Some(amount.clone());
        screen.action.recipient_identity_id = recipient.clone();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut MintTokensScreen| {
                    screen.ui(ui);
                },
                screen,
            );
        harness.run();

        harness.get_by_label(&format!("Amount: {amount}"));
        harness.get_by_label(&format!("Recipient: {recipient}"));
        assert!(harness.query_by_label("Sign Mint").is_none());
    }

    /// Gives `info` an identity whose only token key comes from a locked
    /// password-protected wallet.
    fn hold_key_in_a_locked_wallet(info: &mut IdentityTokenInfo) {
        use crate::context::test_support::{auth_key, identity_holding};
        let mut identity = identity_holding(&[auth_key(1, SecurityLevel::CRITICAL)]);
        let mut wallet = crate::model::wallet::test_support::open_wallet_on(
            crate::test_support::keys::labelled_seed("locked_token_wallet", 0),
        );
        let seed_hash = wallet.seed_hash();
        wallet.uses_password = true;
        wallet.wallet_seed.close();
        crate::context::test_support::hold_as_wallet_reference(&mut identity, 1, seed_hash);
        identity
            .associated_wallets
            .insert(seed_hash, Arc::new(RwLock::new(wallet)));
        info.identity = identity;
    }

    /// Renders `screen` for a co-signer with a locked wallet, checks every
    /// `proposal_labels` is shown, then presses `sign_button`: it asks to
    /// unlock the wallet and sends nothing.
    fn assert_locked_co_signer_reads_then_unlocks<A: TokenAction>(
        mut screen: TokenActionScreen<A>,
        proposal_labels: &[String],
        sign_button: &str,
    ) {
        use egui_kittest::kittest::Queryable;
        use std::cell::Cell;

        assert!(
            screen
                .common
                .selected_wallet
                .as_ref()
                .is_some_and(wallet_needs_unlock),
            "the signing key's wallet is locked"
        );
        screen.common.group_action_id = Some(Identifier::from([7; 32]));
        let sent = Cell::new(false);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut TokenActionScreen<A>| {
                    if matches!(screen.ui(ui), AppAction::BackendTasksWithContexts(..)) {
                        sent.set(true);
                    }
                },
                screen,
            );
        harness.run();

        for label in proposal_labels {
            assert!(harness.query_by_label(label).is_some(), "shows {label}");
        }
        assert!(
            harness
                .query_by_label(super::LOCKED_PROPOSAL_NOTE)
                .is_some()
        );
        assert!(!harness.state().common.wallet_unlock_popup.is_open());

        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, sign_button)
            .click();
        harness.run();

        let common = &harness.state().common;
        assert!(
            common.wallet_unlock_popup.is_open(),
            "signing asks to unlock"
        );
        assert!(common.confirmation_dialog.is_none());
        assert!(!sent.get(), "nothing is sent while the wallet is locked");
    }

    /// A co-signer with a locked wallet reads a group mint's amount and
    /// recipient; unlocking is asked for only when they sign.
    #[test]
    fn a_locked_co_signer_reads_a_group_mint_before_unlocking() {
        use dash_sdk::dpp::platform_value::string_encoding::Encoding;
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut info = token_info(&context, None, true);
        hold_key_in_a_locked_wallet(&mut info);
        let amount = Amount::from_token(&info, 5);
        let recipient = Identifier::from([5; 32]).to_string(Encoding::Base58);
        let mut screen = MintTokensScreen::new(info, &context);
        screen.action.amount = Some(amount.clone());
        screen.action.recipient_identity_id = recipient.clone();

        assert_locked_co_signer_reads_then_unlocks(
            screen,
            &[
                format!("Amount: {amount}"),
                format!("Recipient: {recipient}"),
            ],
            "Sign Mint",
        );
    }

    /// A co-signer with a locked wallet reads a group burn's amount;
    /// unlocking is asked for only when they sign.
    #[test]
    fn a_locked_co_signer_reads_a_group_burn_before_unlocking() {
        use crate::ui::tokens::burn_tokens_screen::BurnTokensScreen;
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut info = token_info(&context, None, false);
        hold_key_in_a_locked_wallet(&mut info);
        let amount = Amount::from_token(&info, 5);
        let mut screen = BurnTokensScreen::new(info, &context);
        screen.action.amount = Some(amount.clone());

        assert_locked_co_signer_reads_then_unlocks(
            screen,
            &[format!("Amount: {amount}")],
            "Sign Burn",
        );
    }

    /// A confirmation opened before the co-signer's wallet locked is neither
    /// shown nor sendable while the wallet stays locked.
    #[test]
    fn a_confirmation_left_open_when_the_wallet_locks_sends_nothing() {
        use egui_kittest::kittest::Queryable;
        use std::cell::Cell;
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut info = token_info(&context, None, false);
        hold_key_in_a_locked_wallet(&mut info);
        let mut screen =
            crate::ui::tokens::burn_tokens_screen::BurnTokensScreen::new(info, &context);
        screen.action.amount = Some(Amount::from_token(&screen.common.identity_token_info, 5));
        screen.common.group_action_id = Some(Identifier::from([7; 32]));
        screen.common.confirmation_dialog = Some(
            ConfirmationDialog::new("Confirm Burn", "Burn the tokens?")
                .confirm_text(Some("Confirm pending burn")),
        );
        let sent = Cell::new(false);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut TokenActionScreen<_>| {
                    if matches!(screen.ui(ui), AppAction::BackendTasksWithContexts(..)) {
                        sent.set(true);
                    }
                },
                screen,
            );
        harness.run_steps(4);
        let shown = harness.query_by_label("Confirm pending burn");
        let was_shown = shown.is_some();
        if let Some(confirm) = shown {
            confirm.click();
            harness.run_steps(4);
        }

        assert!(!was_shown, "the confirmation is hidden while locked");
        assert!(!sent.get(), "nothing is sent while the wallet is locked");
    }

    /// A new action, not a proposal to co-sign, still asks to unlock a
    /// locked wallet before showing its form.
    #[test]
    fn a_new_token_action_asks_to_unlock_before_its_form() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut info = token_info(&context, None, true);
        hold_key_in_a_locked_wallet(&mut info);
        let screen = MintTokensScreen::new(info, &context);

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut MintTokensScreen| {
                    screen.ui(ui);
                },
                screen,
            );
        harness.run();

        assert!(
            harness
                .query_by_label("Wallet is locked. Please unlock to continue.")
                .is_some()
        );
        assert!(harness.query_by_label("Unlock Wallet").is_some());
        assert!(
            harness
                .query_by_role_and_label(egui::accesskit::Role::TextInput, "Amount to Mint:")
                .is_none()
        );
        assert!(
            harness
                .query_by_label(super::LOCKED_PROPOSAL_NOTE)
                .is_none()
        );
    }

    /// A token action cannot be undone, so a second confirmation while the
    /// first is in flight must not send it again.
    #[test]
    fn a_token_action_in_flight_is_never_sent_twice() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, Some(Identifier::from([5; 32])), false);
        let mut screen = TokenActionScreen::<MintAction>::new(info.clone(), &context);
        screen.common.selected_key = TokenSigningKey::automatic(Some(signing_key()));
        screen.action.amount = Some(Amount::from_token(&info, 5));

        use crate::ui::tokens::mint_tokens_screen::tests::{
            report_error, sent_request, unrelated_request,
        };
        let request = sent_request(&screen.submit()).expect("the action is sent");
        assert!(
            sent_request(&screen.submit()).is_none(),
            "a second confirmation while waiting must not send it again"
        );
        report_error(&mut screen, &unrelated_request(context.network));
        assert!(
            sent_request(&screen.submit()).is_none(),
            "another task's error does not end the wait"
        );
        report_error(&mut screen, &request.expect("the action's own context"));
        assert!(
            sent_request(&screen.submit()).is_some(),
            "a failed action can be sent again"
        );
    }

    /// The action and its balance refresh are sent together; the refresh
    /// failing after the action succeeded must not bring the form back.
    #[test]
    fn a_completed_action_stays_complete_when_its_balance_refresh_fails() {
        use crate::ui::tokens::mint_tokens_screen::tests::{report_error, unrelated_request};
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, Some(Identifier::from([5; 32])), false);
        let mut screen = TokenActionScreen::<MintAction>::new(info.clone(), &context);
        screen.common.selected_key = TokenSigningKey::automatic(Some(signing_key()));
        screen.action.amount = Some(Amount::from_token(&info, 5));
        screen.submit();
        screen.display_task_result(BackendTaskSuccessResult::MintedTokens(
            crate::backend_task::FeeResult {
                estimated_fee: 1,
                actual_fee: Some(1),
            },
        ));
        report_error(&mut screen, &unrelated_request(context.network));
        assert!(screen.common.status == TokenActionStatus::Complete);
    }

    /// A burn is sent with its balance refresh. Only the burn's own error
    /// ends the wait, and the refresh failing after the burn went through
    /// keeps the screen complete.
    #[test]
    fn only_the_burn_itself_settles_a_burn_and_its_refresh() {
        use crate::ui::tokens::burn_tokens_screen::BurnAction;
        use crate::ui::tokens::mint_tokens_screen::tests::report_error;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let info = token_info(&context, None, false);
        let mut screen = TokenActionScreen::<BurnAction>::new(info.clone(), &context);
        screen.common.selected_key = TokenSigningKey::automatic(Some(signing_key()));
        screen.action.amount = Some(Amount::from_token(&info, 5));
        let burn = |action: AppAction| {
            let AppAction::BackendTasksWithContexts(tasks, _) = action else {
                panic!("the burn and its refresh are sent together");
            };
            let [(_, burn), (_, refresh)] = tasks.as_slice() else {
                panic!("expected the burn and its refresh, got {tasks:?}");
            };
            (burn.clone(), refresh.clone())
        };
        let (burn_request, refresh_request) = burn(screen.submit());

        report_error(&mut screen, &refresh_request);
        assert!(
            matches!(screen.submit(), AppAction::None),
            "the refresh's error does not end the burn's wait"
        );
        report_error(&mut screen, &burn_request);
        let (_, refresh_request) = burn(screen.submit());

        screen.display_task_result(BackendTaskSuccessResult::BurnedTokens(
            crate::backend_task::FeeResult {
                estimated_fee: 1,
                actual_fee: Some(1),
            },
        ));
        report_error(&mut screen, &refresh_request);
        assert!(screen.common.status == TokenActionStatus::Complete);
    }

    /// Regression: with no key selected (here only a contract-group key, for
    /// the user to choose) the action button stayed enabled, and confirming
    /// only then reported that no key was selected.
    #[test]
    fn a_token_action_without_a_signing_key_asks_for_one() {
        use crate::context::test_support::{auth_key, bound_to, identity_holding};
        use crate::ui::tokens::pause_tokens_screen::PauseTokensScreen;
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        use egui_kittest::kittest::{NodeT, Queryable};

        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut info = token_info(&context, None, false);
        info.identity = identity_holding(&[bound_to(
            auth_key(1, SecurityLevel::CRITICAL),
            ContractBounds::ContractGroup {
                id: Identifier::from([0x6A; 32]),
            },
        )]);
        let screen = PauseTokensScreen::new(info, &context);
        assert!(screen.common.selected_key.as_ref().is_none());

        let mut harness = crate::test_support::harness_builder_with_app_fonts()
            .with_size(egui::vec2(1280.0, 900.0))
            .build_ui_state(
                |ui, screen: &mut PauseTokensScreen| {
                    screen.ui(ui);
                },
                screen,
            );
        harness.run();
        let button = harness
            .query_all_by_role_and_label(egui::accesskit::Role::Button, "Pause")
            .last()
            .expect("the Pause button");
        assert!(
            !button
                .accesskit_node()
                .data()
                .supports_action(egui::accesskit::Action::Click),
            "the action cannot be confirmed without a key"
        );
        button.hover();
        harness.run();
        assert!(
            harness
                .query_by_label(super::super::CHOOSE_SIGNING_KEY)
                .is_some()
        );
    }
}
