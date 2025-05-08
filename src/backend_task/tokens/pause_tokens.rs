use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::emergency_action::TokenEmergencyActionTransitionBuilder;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use dash_sdk::Sdk;
use tokio::sync::mpsc;

impl AppContext {
    pub async fn pause_tokens(
        &self,
        actor_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Use .pause(...) constructor
        let mut builder = TokenEmergencyActionTransitionBuilder::pause(
            data_contract,
            token_position,
            actor_identity.identity.id(),
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        let options = if self.developer_mode {
            Some(StateTransitionCreationOptions {
                signing_options: StateTransitionSigningOptions {
                    allow_signing_with_any_security_level: true,
                    allow_signing_with_any_purpose: true,
                },
                batch_feature_version: None,
                method_feature_version: None,
                base_feature_version: None,
            })
        } else {
            None
        };

        let state_transition = builder
            .sign(
                sdk,
                &signing_key,
                actor_identity,
                self.platform_version,
                options,
            )
            .await
            .map_err(|e| format!("Error signing Pause Tokens transition: {}", e.to_string()))?;

        // Broadcast
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting Pause Tokens transition: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message("PauseTokens".to_string()))
    }
}
