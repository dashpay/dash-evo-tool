//! Transfer tokens from one identity to another

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::transfer::TokenTransferTransitionBuilder;
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
        public_note: Option<String>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut builder = TokenTransferTransitionBuilder::new(
            data_contract,
            token_position,
            sending_identity.identity.id(),
            recipient_id,
            amount,
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
                sending_identity,
                self.platform_version,
                options,
            )
            .await
            .map_err(|e| {
                format!(
                    "Error signing token transfer state transition: {}",
                    e.to_string()
                )
            })?;

        let _ = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| {
                format!(
                    "Error broadcasting token transfer state transition: {}",
                    e.to_string()
                )
            })?;

        Ok(BackendTaskSuccessResult::Message(
            "TransferTokens".to_string(),
        ))
    }
}
