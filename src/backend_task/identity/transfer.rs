use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::platform::transition::transfer::TransferToIdentity;
use dash_sdk::platform::Identifier;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn transfer_to_identity(
        &self,
        mut qualified_identity: QualifiedIdentity,
        to_identifier: Identifier,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let remaining_balance = qualified_identity
            .identity
            .clone()
            .transfer_credits(
                &self.sdk,
                to_identifier,
                credits,
                id.and_then(|key_id| qualified_identity.identity.get_public_key_by_id(key_id)),
                qualified_identity.clone(),
                None,
            )
            .await
            .map_err(|e| format!("Transfer error: {}", e))?;
        qualified_identity.identity.set_balance(remaining_balance);
        self.update_local_qualified_identity(&qualified_identity)
            .map(|_| {
                BackendTaskSuccessResult::Message("Successfully transferred credits".to_string())
            })
            .map_err(|e| e.to_string())
    }
}
