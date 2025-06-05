use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;

use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::fungible_tokens::mint::TokenMintTransitionBuilder;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};

use crate::model::proof_log_item::{ProofLogItem, RequestType};
use tokio::sync::mpsc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn mint_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: u64,
        optional_recipient: Option<Identifier>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let builder = TokenMintTransitionBuilder::new(
            data_contract,
            token_position,
            sending_identity.identity.id(),
            amount,
        );

        let mut builder = if let Some(recipient_id) = optional_recipient {
            builder.issued_to_identity_id(recipient_id)
        } else {
            builder
        };

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
            .map_err(|e| format!("Error signing Mint Tokens state transition: {}", e))?;

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
                        "Error broadcasting Mint Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Mint Tokens transition: {}", e),
            })?;

        // Return success
        Ok(BackendTaskSuccessResult::Message("MintTokens".to_string()))
    }
}
