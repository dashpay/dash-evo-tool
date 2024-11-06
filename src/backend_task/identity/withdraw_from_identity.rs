use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::platform::transition::withdraw_from_identity::WithdrawFromIdentity;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn withdraw_from_identity(
        &self,
        mut qualified_identity: QualifiedIdentity,
        to_address: Option<Address>,
        credits: Credits,
        id: Option<KeyID>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let remaining_balance = qualified_identity
            .identity
            .clone()
            .withdraw(
                &self.sdk,
                to_address,
                credits,
                Some(1),
                id.and_then(|key_id| qualified_identity.identity.get_public_key_by_id(key_id)),
                qualified_identity.clone(),
                None,
            )
            .await
            .map_err(|e| format!("Withdrawal error: {}", e))?;
        qualified_identity.identity.set_balance(remaining_balance);
        self.insert_local_qualified_identity(&qualified_identity)
            .map(|_| {
                BackendTaskSuccessResult::Message("Successfully withdrew from identity".to_string())
            })
            .map_err(|e| format!("Database error: {}", e))
    }
}
