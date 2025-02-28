use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::unfreeze::TokenUnfreezeTransitionBuilder;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::Sdk;
use tokio::sync::mpsc;

impl AppContext {
    pub async fn unfreeze_tokens(
        &self,
        actor_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        unfreeze_identity: Identifier,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let builder = TokenUnfreezeTransitionBuilder::new(
            data_contract,
            token_position,
            actor_identity.identity.id(),
            unfreeze_identity,
        );

        // Optionally chain .with_public_note(...).with_settings(...), etc.

        let state_transition = builder
            .sign(sdk, &signing_key, actor_identity, self.platform_version)
            .await
            .map_err(|e| format!("Error signing Unfreeze Tokens transition: {:?}", e))?;

        // Broadcast
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting Unfreeze Tokens transition: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message(
            "UnfreezeTokens".to_string(),
        ))
    }
}
