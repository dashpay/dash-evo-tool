use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::platform::Identifier;
use dash_sdk::platform::transition::transfer::TransferToIdentity;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn transfer_to_identity(
        &self,
        mut qualified_identity: QualifiedIdentity,
        to_identifier: Identifier,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk_guard = {
            let guard = self.sdk.read().unwrap();
            guard.clone()
        };

        let (sender_balance, receiver_balance) = qualified_identity
            .identity
            .clone()
            .transfer_credits(
                &sdk_guard,
                to_identifier,
                credits,
                id.and_then(|key_id| qualified_identity.identity.get_public_key_by_id(key_id)),
                qualified_identity.clone(),
                None,
            )
            .await
            .map_err(|e| format!("Transfer error: {}", e))?;
        qualified_identity.identity.set_balance(sender_balance);

        // If the receiver is a local qualified identity, update its balance too
        if let Some(receiver) = self
            .load_local_qualified_identities()
            .map_err(|e| format!("Transfer error: {}", e))?
            .iter_mut()
            .find(|qi| qi.identity.id() == to_identifier)
        {
            receiver.identity.set_balance(receiver_balance);
            self.update_local_qualified_identity(receiver)
                .map_err(|e| format!("Transfer error: {}", e))?;
        }

        self.update_local_qualified_identity(&qualified_identity)
            .map(|_| {
                BackendTaskSuccessResult::Message("Successfully transferred credits".to_string())
            })
            .map_err(|e| e.to_string())
    }
}
