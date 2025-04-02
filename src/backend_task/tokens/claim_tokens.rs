use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;

use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::claim::TokenClaimTransitionBuilder;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use dash_sdk::Sdk;

impl AppContext {
    pub async fn claim_tokens(
        &self,
        data_contract: &DataContract,
        token_position: u16,
        actor_identity: &QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Build
        let builder = TokenClaimTransitionBuilder::new(
            data_contract,
            token_position,
            actor_identity.identity.id(),
            distribution_type,
        );

        // Sign
        let state_transition = builder
            .sign(sdk, &signing_key, actor_identity, self.platform_version)
            .await
            .map_err(|e| format!("Error signing ClaimTokens transition: {}", e.to_string()))?;

        // Broadcast
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting ClaimTokens transition: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message("ClaimTokens".to_string()))
    }
}
