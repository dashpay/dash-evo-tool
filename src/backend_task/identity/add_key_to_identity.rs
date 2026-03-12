use super::BackendTaskSuccessResult;
use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_sdk::Error as SdkError;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{Fetch, Identity};

impl AppContext {
    pub(super) async fn add_key_to_identity(
        &self,
        sdk: &Sdk,
        mut qualified_identity: QualifiedIdentity,
        mut public_key_to_add: QualifiedIdentityPublicKey,
        private_key: [u8; 32],
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let new_identity_nonce = sdk
            .get_identity_nonce(qualified_identity.identity.id(), true, None)
            .await?;
        let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
            return Err("Master key not found".to_string().into());
        };
        let master_key_id = master_key.identity_public_key.id();
        let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
            .await?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        qualified_identity.identity = identity;
        qualified_identity.identity.bump_revision();
        public_key_to_add
            .identity_public_key
            .set_id(qualified_identity.identity.get_public_key_max_id() + 1);
        qualified_identity.private_keys.insert_non_encrypted(
            (
                PrivateKeyOnMainIdentity,
                public_key_to_add.identity_public_key.id(),
            ),
            (public_key_to_add.clone(), private_key),
        );
        // Track balance before operation for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_update();

        let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
            &qualified_identity.identity,
            &master_key_id,
            vec![public_key_to_add.identity_public_key.clone()],
            vec![],
            new_identity_nonce,
            UserFeeIncrease::default(),
            &qualified_identity,
            sdk.version(),
            None,
        )
        .map_err(|e| TaskError::IdentityUpdateTransitionError {
            source_error: Box::new(SdkError::Protocol(e)),
        })?;

        let result = state_transition.broadcast_and_wait(sdk, None).await?;

        // Log and handle the proof result
        tracing::info!("AddKeyToIdentity proof result: {}", result);

        let new_balance = match result {
            StateTransitionProofResult::VerifiedPartialIdentity(identity) => {
                // Update the identity with proof-verified public keys
                let balance = identity.balance;
                for public_key in identity.loaded_public_keys.into_values() {
                    qualified_identity.identity.add_public_key(public_key);
                }
                balance
            }
            other => {
                tracing::warn!(
                    "Unexpected proof result type for add key to identity: {}",
                    other
                );
                // Still add the key we tried to add, since the broadcast succeeded
                qualified_identity
                    .identity
                    .add_public_key(public_key_to_add.identity_public_key.clone());
                None
            }
        };

        // Calculate and log actual fee paid
        let actual_fee = if let Some(balance_after) = new_balance {
            let fee = balance_before.saturating_sub(balance_after);
            tracing::info!(
                "AddKeyToIdentity complete: estimated fee {} credits, actual fee {} credits",
                estimated_fee,
                fee
            );
            if fee != estimated_fee {
                tracing::warn!(
                    "Fee mismatch: estimated {} vs actual {} (diff: {})",
                    estimated_fee,
                    fee,
                    fee as i64 - estimated_fee as i64
                );
            }
            qualified_identity.identity.set_balance(balance_after);
            fee
        } else {
            // If we couldn't determine the balance, use the estimate
            estimated_fee
        };

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        self.update_local_qualified_identity(&qualified_identity)
            .map_err(|e| TaskError::IdentitySaveError { source: e })?;
        Ok(BackendTaskSuccessResult::AddedKeyToIdentity(fee_result))
    }
}
