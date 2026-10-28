use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::tokens::info::v0::IdentityTokenInfoV0Accessors;
use dash_sdk::platform::tokens::transitions::FreezeResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn freeze_tokens(
        &self,
        actor_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        freeze_identity: Identifier,
        group_info: Option<GroupStateTransitionInfoStatus>,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let actor = self.token_actor(actor_identity).await?;
        let common = self.token_action_common(data_contract.clone(), token_position, &signing_key);

        self.execute_token_op(
            actor.freeze(common, freeze_identity, public_note, group_info),
            |result| match result {
                FreezeResult::IdentityInfo(identity_id, info) => {
                    tracing::info!(
                        "FreezeTokens: identity {} frozen={}",
                        identity_id,
                        info.frozen()
                    );
                }
                FreezeResult::HistoricalDocument(document) => {
                    tracing::info!("FreezeTokens: historical document id={}", document.id());
                }
                FreezeResult::GroupActionWithDocument(power, doc) => {
                    tracing::info!(
                        "FreezeTokens: group action power={}, has_doc={}",
                        power,
                        doc.is_some()
                    );
                }
                FreezeResult::GroupActionWithIdentityInfo(power, info) => {
                    tracing::info!(
                        "FreezeTokens: group action power={}, frozen={}",
                        power,
                        info.frozen()
                    );
                }
            },
            BackendTaskSuccessResult::FrozeTokens,
        )
        .await
    }
}
