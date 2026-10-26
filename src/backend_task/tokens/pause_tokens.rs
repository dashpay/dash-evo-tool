use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::platform::tokens::transitions::EmergencyActionResult;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn pause_tokens(
        &self,
        actor_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let actor = self.token_actor(actor_identity).await?;
        let common = self.token_action_common(data_contract.clone(), token_position, &signing_key);

        self.execute_token_op(
            actor.pause(common, public_note, group_info),
            |result| match result {
                EmergencyActionResult::Document(document) => {
                    tracing::info!("PauseTokens: document id={}", document.id());
                }
                EmergencyActionResult::GroupActionWithDocument(power, document) => {
                    tracing::info!(
                        "PauseTokens: group action power={}, has_doc={}",
                        power,
                        document.is_some()
                    );
                }
            },
            BackendTaskSuccessResult::PausedTokens,
        )
        .await
    }
}
