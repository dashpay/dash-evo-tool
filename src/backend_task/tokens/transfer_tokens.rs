//! Transfer tokens from one identity to another

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::transfer::TokenTransferTransitionBuilder;
use dash_sdk::platform::transition::put_settings::PutSettings;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::Sdk;
use tokio::sync::mpsc;

use crate::app::TaskResult;
impl AppContext {
    pub async fn transfer_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        recipient_id: Identifier,
        amount: u64,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let builder = TokenTransferTransitionBuilder::new(
            data_contract,
            token_position,
            sending_identity.identity.id(),
            recipient_id,
            amount,
        );

        let state_transition = builder
            .sign(sdk, &signing_key, sending_identity, self.platform_version)
            .await
            .map_err(|e| format!("Error signing state transition: {:?}", e))?;

        let _ = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting state transition: {:?}", e.to_string()))?;

        Ok(BackendTaskSuccessResult::Message(
            "TransferTokens".to_string(),
        ))
    }
}
