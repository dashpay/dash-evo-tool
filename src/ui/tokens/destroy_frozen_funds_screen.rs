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

/// Destroys the frozen funds of a target identity for a contract.
pub type DestroyFrozenFundsScreen = TokenActionScreen<DestroyFrozenFundsAction>;

pub struct DestroyFrozenFundsAction {
    /// Identity whose frozen funds are to be destroyed (Base58 or hex).
    pub frozen_identity_id: String,
    frozen_identities: Vec<QualifiedIdentity>,
}

impl TokenAction for DestroyFrozenFundsAction {
    const VERB: &'static str = "Destroy";
    const PAGE_HEADING: &'static str = "Destroy Frozen Funds";
    const PROGRESS: &'static str = "Destroying frozen funds...";
    const CONFIRM_TITLE: &'static str = "Confirm Destroy Frozen Funds";
    const DANGER: bool = true;

    fn new(_info: &IdentityTokenInfo, app_context: &Arc<AppContext>) -> Self {
        DestroyFrozenFundsAction {
            frozen_identity_id: String::new(),
            frozen_identities: super::load_identities_with_banner(app_context),
        }
    }

    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers {
        *config
            .destroy_frozen_funds_rules()
            .authorized_to_make_change_action_takers()
    }

    fn confirm_message(&self, _ctx: &TokenActionCtx) -> String {
        format!(
            "Are you sure you want to destroy the frozen funds of identity {}?",
            self.frozen_identity_id
        )
    }

    fn render_form(&mut self, ui: &mut Ui, ctx: &TokenActionCtx, step: &mut StepCounter) {
        let n = step.advance();
        ui.heading(format!(
            "{}. Enter the identity ID whose frozen funds to destroy",
            n
        ));
        ui.add_space(5.0);
        if ctx.is_group_signing {
            ui.label(
                "You are signing an existing group Destroy Frozen Funds so you are not allowed to choose the identity.",
            );
            ui.add_space(5.0);
            ui.label(format!("Identity: {}", self.frozen_identity_id));
        } else {
            ui.add(
                IdentitySelector::new(
                    "destroy_frozen_identity_selector",
                    &mut self.frozen_identity_id,
                    &self.frozen_identities,
                )
                .label("Frozen Identity ID:")
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
        let Ok(frozen_identity) = Identifier::from_string_try_encodings(
            &self.frozen_identity_id,
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
            TokenTask::DestroyFrozenFunds {
                actor_identity: ctx.info.identity.clone(),
                data_contract: ctx.data_contract(),
                token_position: ctx.info.token_position,
                signing_key,
                public_note,
                frozen_identity,
                group_info,
            },
        ))))
    }

    fn success_fee(result: &BackendTaskSuccessResult) -> Option<FeeResult> {
        match result {
            BackendTaskSuccessResult::DestroyedFrozenFunds(fee) => Some(fee.clone()),
            _ => None,
        }
    }
}
