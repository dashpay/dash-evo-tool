use crate::context::AppContext;
use crate::model::qualified_identity::EncryptedPrivateKeyTarget::PrivateKeyOnMainIdentity;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{Fetch, Identity, IdentityPublicKey};
use dash_sdk::Sdk;

use super::BackendTaskSuccessResult;

impl AppContext {
    pub(super) async fn add_key_to_identity(
        &self,
        sdk: &Sdk,
        mut qualified_identity: QualifiedIdentity,
        mut public_key_to_add: IdentityPublicKey,
        private_key: [u8; 32],
    ) -> Result<BackendTaskSuccessResult, String> {
        let new_identity_nonce = sdk
            .get_identity_nonce(qualified_identity.identity.id(), true, None)
            .await
            .map_err(|e| format!("Fetch nonce error: {}", e))?;
        let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
            return Err("Master key not found".to_string());
        };
        let master_key_id = master_key.id();
        let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
            .await
            .map_err(|e| format!("Fetch nonce error: {}", e))?
            .unwrap();
        qualified_identity.identity = identity;
        qualified_identity.identity.bump_revision();
        public_key_to_add.set_id(qualified_identity.identity.get_public_key_max_id() + 1);
        qualified_identity.encrypted_private_keys.insert(
            (PrivateKeyOnMainIdentity, public_key_to_add.id()),
            (public_key_to_add.clone(), private_key.clone()),
        );
        let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
            &qualified_identity.identity,
            &master_key_id,
            vec![public_key_to_add.clone()],
            vec![],
            new_identity_nonce,
            UserFeeIncrease::default(),
            &qualified_identity,
            sdk.version(),
            None,
        )
        .map_err(|e| format!("IdentityUpdateTransition error: {}", e))?;

        let result = state_transition
            .broadcast_and_wait(sdk, None)
            .await
            .map_err(|e| format!("Broadcasting error: {}", e))?;

        if let StateTransitionProofResult::VerifiedPartialIdentity(identity) = result {
            for public_key in identity.loaded_public_keys.into_values() {
                qualified_identity.identity.add_public_key(public_key);
            }
        }

        self.insert_local_qualified_identity(&qualified_identity)
            .map(|_| {
                BackendTaskSuccessResult::Message("Successfully added key to identity".to_string())
            })
            .map_err(|e| format!("Database error: {}", e))
    }
}
