// file: backend_task/tokens/mint_tokens.rs

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::mint::TokenMintTransitionBuilder;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::Sdk;

use tokio::sync::mpsc;

impl AppContext {
    pub async fn mint_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        amount: u64,
        optional_recipient: Option<Identifier>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let builder = TokenMintTransitionBuilder::new(
            data_contract,
            token_position,
            sending_identity.identity.id(),
            amount,
        );

        let builder = if let Some(recipient_id) = optional_recipient {
            builder.issued_to_identity_id(recipient_id)
        } else {
            builder
        };

        // Optionally chain `with_public_note(...)`, `with_settings(...)`, etc.

        let state_transition = builder
            .sign(sdk, &signing_key, sending_identity, self.platform_version)
            .await
            .map_err(|e| format!("Error signing Mint Tokens state transition: {:?}", e))?;

        // broadcast and wait
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting Mint Tokens transition: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message("MintTokens".to_string()))
    }
}
