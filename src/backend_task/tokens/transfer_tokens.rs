//! Transfer tokens from one identity to another
use std::collections::HashSet;

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::transition::fungible_tokens::transfer::StateTransitionBuilder;
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

        let public_key = match sending_identity.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::MEDIUM,
                SecurityLevel::HIGH,
                SecurityLevel::CRITICAL,
            ]),
            KeyType::all_key_types().iter().cloned().collect(),
            false,
        ) {
            Some(public_key) => public_key,
            None => return Err("No public key found for transfer".to_string()),
        };

        builder
            .broadcast_and_wait_for_result(sdk, public_key, sending_identity, self.platform_version)
            .await
            .map_err(|e| format!("Error transferring tokens: {:?}", e))?;

        Ok(BackendTaskSuccessResult::Message(
            "TransferTokens".to_string(),
        ))
    }
}
