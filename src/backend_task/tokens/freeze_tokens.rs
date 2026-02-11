use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::info::v0::IdentityTokenInfoV0Accessors;
use dash_sdk::platform::tokens::builders::freeze::TokenFreezeTransitionBuilder;
use dash_sdk::platform::tokens::transitions::FreezeResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
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
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut builder = TokenFreezeTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            actor_identity.identity.id(),
            freeze_identity,
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
            .token_freeze(builder, &signing_key, actor_identity)
            .await
            .map_err(|e| match e {
                Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                    self.db
                        .insert_proof_log_item(ProofLogItem {
                            request_type: RequestType::BroadcastStateTransition,
                            request_bytes: vec![],
                            verification_path_query_bytes: vec![],
                            height: block_info.height,
                            time_ms: block_info.time_ms,
                            proof_bytes,
                            error: Some(proof_error.to_string()),
                        })
                        .ok();
                    format!(
                        "Error broadcasting Freeze Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Freeze Tokens transition: {}", e),
            })?;

        // Log the proof-verified freeze result
        match result {
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
        }

        // Return success with fee result
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::FrozeTokens(fee_result))
    }
}
