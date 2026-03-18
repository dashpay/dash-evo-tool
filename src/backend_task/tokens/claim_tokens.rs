use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::claim::TokenClaimTransitionBuilder;
use dash_sdk::platform::tokens::transitions::ClaimResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_tokens(
        &self,
        data_contract: Arc<DataContract>,
        token_position: u16,
        actor_identity: &QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Build
        let mut builder = TokenClaimTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            actor_identity.identity.id(),
            distribution_type,
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        let maybe_options = self.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_claim(builder, &signing_key, actor_identity)
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

        // Using the result, update the balance of the claimer identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard claim result - extract claimer and amount from document
                ClaimResult::Document(document) => {
                    if let (Some(claimer_value), Some(amount_value)) =
                        (document.get("claimerId"), document.get("amount"))
                        && let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                            (claimer_value, amount_value)
                        && let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &claimer_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from claim document: {}",
                            e
                        );
                    }
                }

                // Group action with document - assume completed if document exists
                ClaimResult::GroupActionWithDocument(_, document) => {
                    if let (Some(claimer_value), Some(amount_value)) =
                        (document.get("claimerId"), document.get("amount"))
                        && let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                            (claimer_value, amount_value)
                        && let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &claimer_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from group action document: {}",
                            e
                        );
                    }
                }
            }
        }

        // Return success with fee result
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::ClaimedTokens(fee_result))
    }
}
