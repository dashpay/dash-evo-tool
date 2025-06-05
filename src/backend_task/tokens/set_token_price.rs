use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::set_price::TokenChangeDirectPurchasePriceTransitionBuilder;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use dash_sdk::{Error, Sdk};

use crate::model::proof_log_item::{ProofLogItem, RequestType};
use tokio::sync::mpsc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn set_direct_purchase_price(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        token_pricing_schedule: Option<TokenPricingSchedule>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut builder = TokenChangeDirectPurchasePriceTransitionBuilder::new(
            data_contract,
            token_position,
            sending_identity.identity.id(),
            token_pricing_schedule,
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        if let Some(group_info) = group_info {
            builder = builder.with_using_group_info(group_info);
        }

        let options = self.state_transition_options();

        let state_transition = builder
            .sign(
                sdk,
                &signing_key,
                sending_identity,
                self.platform_version(),
                options,
            )
            .await
            .map_err(|e| format!("Error signing SetPrice state transition: {}", e))?;

        // broadcast and wait
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
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
                        "Error broadcasting SetPrice Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting SetPrice Tokens transition: {}", e),
            })?;

        // Return success
        Ok(BackendTaskSuccessResult::Message(
            "SetDirectPurchasePrice".to_string(),
        ))
    }
}
