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
        _sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let platform_wallet = self.platform_wallet_for_identity(actor_identity)?;
        let token_wallet = platform_wallet.tokens();

        let result = token_wallet
            .freeze_with_signer(
                data_contract.clone(),
                token_position,
                actor_identity.identity.id(),
                freeze_identity,
                &signing_key,
                actor_identity,
                public_note,
                group_info,
                self.state_transition_options(),
            )
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

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
