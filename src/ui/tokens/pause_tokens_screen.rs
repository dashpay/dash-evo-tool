use super::token_action_screen::{TokenAction, TokenActionCtx, TokenActionScreen};
use super::tokens_screen::IdentityTokenInfo;
use crate::app::AppAction;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::platform::IdentityPublicKey;
use std::sync::Arc;

/// Pauses all token transfers for a contract (emergency action).
pub type PauseTokensScreen = TokenActionScreen<PauseAction>;

pub struct PauseAction;

impl TokenAction for PauseAction {
    const VERB: &'static str = "Pause";
    const PAGE_HEADING: &'static str = "Pause Token Contract";
    const PROGRESS: &'static str = "Pausing tokens...";
    const CONFIRM_TITLE: &'static str = "Confirm Pause";

    fn new(_info: &IdentityTokenInfo, _app_context: &Arc<AppContext>) -> Self {
        PauseAction
    }

    fn authorized_takers(config: &TokenConfiguration) -> AuthorizedActionTakers {
        *config
            .emergency_action_rules()
            .authorized_to_make_change_action_takers()
    }

    fn confirm_message(&self, _ctx: &TokenActionCtx) -> String {
        "Are you sure you want to pause token transfers for this contract?".to_string()
    }

    fn build_action(
        &mut self,
        ctx: &TokenActionCtx,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
    ) -> Option<AppAction> {
        Some(AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::PauseTokens {
                actor_identity: ctx.info.identity.clone(),
                data_contract: ctx.data_contract(),
                token_position: ctx.info.token_position,
                signing_key,
                public_note,
                group_info,
            },
        ))))
    }

    fn success_fee(result: &BackendTaskSuccessResult) -> Option<FeeResult> {
        match result {
            BackendTaskSuccessResult::PausedTokens(fee) => Some(fee.clone()),
            _ => None,
        }
    }
}
