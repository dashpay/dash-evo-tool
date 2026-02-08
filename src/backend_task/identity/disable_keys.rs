use super::{BackendTaskSuccessResult, IdentityResult};
use crate::backend_task::FeeResult;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::qualified_identity::QualifiedIdentity;
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
    pub(super) async fn disable_identity_keys(
        &self,
        sdk: &Sdk,
        mut qualified_identity: QualifiedIdentity,
        key_ids_to_disable: Vec<KeyID>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let new_identity_nonce = sdk
            .get_identity_nonce(qualified_identity.identity.id(), true, None)
            .await
            .map_err(|e| format!("Fetch nonce error: {}", e))?;
        let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
            return Err(
                "Master key not found. Cannot disable keys without a master key.".to_string(),
            );
        };
        let master_key_id = master_key.identity_public_key.id();
        let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
            .await
            .map_err(|e| format!("Fetch identity error: {}", e))?
            .ok_or("Identity not found on Platform".to_string())?;
        qualified_identity.identity = identity;
        qualified_identity.identity.bump_revision();

        // Track balance before operation for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_update();

        let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
            &qualified_identity.identity,
            &master_key_id,
            vec![],                     // no keys to add
            key_ids_to_disable.clone(), // keys to disable
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

        tracing::info!("DisableKeys proof result: {}", result);

        let new_balance = match result {
            StateTransitionProofResult::VerifiedPartialIdentity(partial_identity) => {
                // Update keys from proof: the proof returns keys with disabled_at timestamps set
                for public_key in partial_identity.loaded_public_keys.into_values() {
                    if key_ids_to_disable.contains(&public_key.id()) {
                        // Update the key in our local identity with the disabled version
                        if let Some(existing_key) = qualified_identity
                            .identity
                            .public_keys_mut()
                            .get_mut(&public_key.id())
                        {
                            existing_key.set_disabled_at(public_key.disabled_at().unwrap_or(1));
                        }
                    }
                }
                partial_identity.balance
            }
            other => {
                tracing::warn!("Unexpected proof result type for disable keys: {}", other);
                // Manually mark keys as disabled since broadcast succeeded
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                for key_id in &key_ids_to_disable {
                    if let Some(existing_key) = qualified_identity
                        .identity
                        .public_keys_mut()
                        .get_mut(key_id)
                    {
                        existing_key.set_disabled_at(timestamp);
                    }
                }
                None
            }
        };

        // Calculate and log actual fee paid
        let actual_fee = if let Some(balance_after) = new_balance {
            let fee = balance_before.saturating_sub(balance_after);
            tracing::info!(
                "DisableKeys complete: estimated fee {} credits, actual fee {} credits",
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
                BackendTaskSuccessResult::Identity(IdentityResult::DisabledKeys(
                    qualified_identity,
                    fee_result,
                ))
            })
            .map_err(|e| format!("Database error: {}", e))
    }
}
