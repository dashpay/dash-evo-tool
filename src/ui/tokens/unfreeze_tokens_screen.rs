use super::token_action_screen::{StepCounter, TokenAction, TokenActionCtx, TokenActionScreen};
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::components::identity_selector::IdentitySelector;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::Ui;
use std::sync::Arc;

/// Unfreezes a previously frozen identity's tokens for a contract.
pub type UnfreezeTokensScreen = TokenActionScreen<UnfreezeAction>;

pub struct UnfreezeAction {
    /// Identity to unfreeze (Base58 or hex).
    pub unfreeze_identity_id: String,
    frozen_identities: Vec<QualifiedIdentity>,
}

impl TokenAction for UnfreezeAction {
    const VERB: &'static str = "Unfreeze";
    const PAGE_HEADING: &'static str = "Unfreeze Identity’s Tokens";
    const PROGRESS: &'static str = "Unfreezing tokens...";
    const CONFIRM_TITLE: &'static str = "Confirm Unfreeze";

    fn new(_info: &IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        UnfreezeAction {
            unfreeze_identity_id: String::new(),
            frozen_identities: super::load_identities_with_banner(app_context),
        }
    }

    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers {
        *config
            .unfreeze_rules()
            .authorized_to_make_change_action_takers()
    }

    fn confirm_message(&self, _ctx: &TokenActionCtx) -> String {
        format!(
            "Are you sure you want to unfreeze identity {}?",
            self.unfreeze_identity_id
        )
    }

    fn render_form(&mut self, ui: &mut Ui, ctx: &TokenActionCtx, step: &mut StepCounter) {
        let n = step.advance();
        ui.heading(format!("{}. Enter the identity ID to unfreeze", n));
        ui.add_space(5.0);
        if ctx.is_group_signing {
            ui.label(
                "You are signing an existing group Unfreeze so you are not allowed to choose the identity.",
            );
            ui.add_space(5.0);
            ui.label(format!("Identity: {}", self.unfreeze_identity_id));
        } else {
            ui.add(
                IdentitySelector::new(
                    "unfreeze_identity_selector",
                    &mut self.unfreeze_identity_id,
                    &self.frozen_identities,
                )
                .label("Unfreeze Identity ID:")
                .width(300.0),
            );
        }
    }

    fn build_action(
        &mut self,
        ctx: &TokenActionCtx,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Option<AppAction> {
        let Ok(unfreeze_identity) = Identifier::from_string_try_encodings(
            &self.unfreeze_identity_id,
            &[Encoding::Base58, Encoding::Hex],
        ) else {
            MessageBanner::set_global(
                ctx.app_context.egui_ctx(),
                "Please enter a valid identity ID.",
                MessageType::Error,
            );
            return None;
        };

        Some(AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::UnfreezeTokens {
                actor_identity: ctx.info.identity.clone(),
                data_contract: ctx.data_contract(),
                token_position: ctx.info.token_position,
                signing_key,
                public_note,
                unfreeze_identity,
                group_info,
            },
        ))))
    }

    fn success_fee(result: &BackendTaskSuccessResult) -> Option<FeeResult> {
        match result {
            BackendTaskSuccessResult::UnfrozeTokens(fee) => Some(fee.clone()),
            _ => None,
        }
    }
}
