use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::burn::TokenBurnTransitionBuilder;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use dash_sdk::Sdk;
use tokio::sync::mpsc;

impl AppContext {
    pub async fn burn_tokens(
        &self,
        owner_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        amount: u64,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let builder = TokenBurnTransitionBuilder::new(
            data_contract,
            token_position,
            owner_identity.identity.id(),
            amount,
        );

        // Optionally chain `with_public_note(...)`, `with_settings(...)`, etc.

        let state_transition = builder
            .sign(sdk, &signing_key, owner_identity, self.platform_version)
            .await
            .map_err(|e| format!("Error signing Burn Tokens transition: {:?}", e))?;

        // Broadcast and wait
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting Burn Tokens transition: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message("BurnTokens".to_string()))
    }
}
