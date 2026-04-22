use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::info::v0::IdentityTokenInfoV0Accessors;
use dash_sdk::platform::tokens::builders::unfreeze::TokenUnfreezeTransitionBuilder;
use dash_sdk::platform::tokens::transitions::UnfreezeResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn unfreeze_tokens(
        &self,
        actor_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        unfreeze_identity: Identifier,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut builder = TokenUnfreezeTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            actor_identity.identity.id(),
            unfreeze_identity,
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        if let Some(group_info) = group_info {
            builder = builder.with_using_group_info(group_info);
        }

        if let Some(options) = self.state_transition_options() {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_unfreeze_identity(builder, &signing_key, actor_identity)
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

        // Log the proof-verified unfreeze result
        match result {
            UnfreezeResult::IdentityInfo(identity_id, info) => {
                tracing::info!(
                    "UnfreezeTokens: identity {} frozen={}",
                    identity_id,
                    info.frozen()
                );
            }
            UnfreezeResult::HistoricalDocument(document) => {
                tracing::info!("UnfreezeTokens: historical document id={}", document.id());
            }
            UnfreezeResult::GroupActionWithDocument(power, doc) => {
                tracing::info!(
                    "UnfreezeTokens: group action power={}, has_doc={}",
                    power,
                    doc.is_some()
                );
            }
            UnfreezeResult::GroupActionWithIdentityInfo(power, info) => {
                tracing::info!(
                    "UnfreezeTokens: group action power={}, frozen={}",
                    power,
                    info.frozen()
                );
            }
        }

        // Return success with fee result
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::UnfrozeTokens(fee_result))
    }
}
