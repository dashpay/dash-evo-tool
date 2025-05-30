//! Transfer tokens from one identity to another

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::transfer::TokenTransferTransitionBuilder;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
use tokio::sync::mpsc;

use crate::app::TaskResult;
use crate::model::proof_log_item::{ProofLogItem, RequestType};

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
            .map_err(|e| {
                format!(
                    "Error signing token transfer state transition: {}",
                    e.to_string()
                )
            })?;

        let _ = state_transition
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
                        "Error broadcasting Transfer Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Transfer Tokens transition: {}", e),
            })?;

        Ok(BackendTaskSuccessResult::Message(
            "TransferTokens".to_string(),
        ))
    }
}
