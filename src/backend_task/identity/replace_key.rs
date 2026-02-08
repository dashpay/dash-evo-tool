use super::{BackendTaskSuccessResult, IdentityResult};
use crate::backend_task::FeeResult;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::KeyID;
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
    /// Replace a key on an identity by adding a new key and disabling the old one
    /// in a single IdentityUpdateTransition. This is primarily used for replacing
    /// the master key, but works for any key replacement.
    pub(super) async fn replace_identity_key(
        &self,
        sdk: &Sdk,
        mut qualified_identity: QualifiedIdentity,
        old_key_id: KeyID,
        mut new_qualified_key: QualifiedIdentityPublicKey,
        new_private_key: [u8; 32],
    ) -> Result<BackendTaskSuccessResult, String> {
        // Get fresh nonce from Platform
        let new_identity_nonce = sdk
            .get_identity_nonce(qualified_identity.identity.id(), true, None)
            .await
            .map_err(|e| format!("Fetch nonce error: {}", e))?;

        // We need the current master key to sign this transition
        let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
            return Err(
                "Master key not found. Cannot replace key without a valid master key.".to_string(),
            );
        };
        let master_key_id = master_key.identity_public_key.id();

        // Fetch fresh identity from Platform
        let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
            .await
            .map_err(|e| format!("Fetch identity error: {}", e))?
            .ok_or("Identity not found on Platform".to_string())?;
        qualified_identity.identity = identity;
        qualified_identity.identity.bump_revision();

        // Assign the next available key ID to the new key
        new_qualified_key
            .identity_public_key
            .set_id(qualified_identity.identity.get_public_key_max_id() + 1);

        // Store the new private key in the identity's key storage
        qualified_identity.private_keys.insert_non_encrypted(
            (
                PrivateKeyOnMainIdentity,
                new_qualified_key.identity_public_key.id(),
            ),
            (new_qualified_key.clone(), new_private_key),
        );

        // Track balance for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_update();

        // Create a single IdentityUpdateTransition that both adds the new key
        // and disables the old key atomically
        let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
            &qualified_identity.identity,
            &master_key_id,
            vec![new_qualified_key.identity_public_key.clone()], // add new key
            vec![old_key_id],                                    // disable old key
            new_identity_nonce,
            UserFeeIncrease::default(),
            &qualified_identity,
            sdk.version(),
            None,
        )
        .map_err(|e| format!("IdentityUpdateTransition error: {}", e))?;

        // Broadcast and wait for proof
        let result = state_transition
            .broadcast_and_wait(sdk, None)
            .await
            .map_err(|e| format!("Broadcasting error: {}", e))?;

        tracing::info!("ReplaceKey proof result: {}", result);

        let new_balance = match result {
            StateTransitionProofResult::VerifiedPartialIdentity(partial_identity) => {
                // Update keys from proof
                for public_key in partial_identity.loaded_public_keys.into_values() {
                    if public_key.id() == old_key_id {
                        // Update the old key with its disabled_at timestamp
                        if let Some(existing_key) = qualified_identity
                            .identity
                            .public_keys_mut()
                            .get_mut(&public_key.id())
                        {
                            existing_key.set_disabled_at(public_key.disabled_at().unwrap_or(1));
                        }
                    } else {
                        // Add the new key from proof
                        qualified_identity.identity.add_public_key(public_key);
                    }
                }
                partial_identity.balance
            }
            other => {
                tracing::warn!("Unexpected proof result type for replace key: {}", other);
                // Manually update since broadcast succeeded
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                if let Some(existing_key) = qualified_identity
                    .identity
                    .public_keys_mut()
                    .get_mut(&old_key_id)
                {
                    existing_key.set_disabled_at(timestamp);
                }
                qualified_identity
                    .identity
                    .add_public_key(new_qualified_key.identity_public_key.clone());
                None
            }
        };

        // Calculate actual fee
        let actual_fee = if let Some(balance_after) = new_balance {
            let fee = balance_before.saturating_sub(balance_after);
            tracing::info!(
                "ReplaceKey complete: estimated fee {} credits, actual fee {} credits",
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
            estimated_fee
        };

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        self.update_local_qualified_identity(&qualified_identity)
            .map(|_| {
                BackendTaskSuccessResult::Identity(IdentityResult::ReplacedKey(
                    qualified_identity,
                    fee_result,
                ))
            })
            .map_err(|e| format!("Database error: {}", e))
    }
}
