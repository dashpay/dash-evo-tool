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
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::state::state_error::StateError;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{Fetch, Identity};

/// Converts a broadcast SDK error into a typed `TaskError`.
///
/// Matches known consensus error variants from two SDK error paths
/// (`StateTransitionBroadcastError` and `Protocol/ConsensusError`),
/// falling back to `TaskError::Generic` for unrecognised errors.
fn broadcast_error(error: &SdkError) -> TaskError {
    tracing::warn!("AddKeyToIdentity broadcast failed: {:?}", error);

    let consensus_error = match error {
        SdkError::StateTransitionBroadcastError(broadcast_err) => broadcast_err.cause.as_ref(),
        SdkError::Protocol(ProtocolError::ConsensusError(ce)) => Some(ce.as_ref()),
        _ => None,
    };

    if let Some(ce) = consensus_error {
        match ce {
            ConsensusError::StateError(StateError::DuplicatedIdentityPublicKeyStateError(_)) => {
                return TaskError::DuplicateIdentityPublicKey;
            }
            ConsensusError::StateError(StateError::DuplicatedIdentityPublicKeyIdStateError(_)) => {
                return TaskError::DuplicateIdentityPublicKeyId;
            }
            ConsensusError::StateError(
                StateError::IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError(e),
            ) => {
                return TaskError::IdentityPublicKeyContractBoundsConflict {
                    contract_id: e.contract_id().to_string(Encoding::Base58),
                };
            }
            _ => {}
        }
    }

    TaskError::Generic(format!("Broadcasting error: {}", error))
}

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
            .await
            .map_err(|e| format!("Fetch nonce error: {}", e))?;
        let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
            return Err("Master key not found".to_string().into());
        };
        let master_key_id = master_key.identity_public_key.id();
        let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
            .await
            .map_err(|e| format!("Fetch identity error: {}", e))?
            .ok_or_else(|| TaskError::Generic("Identity not found".into()))?;
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
        .map_err(|e| format!("IdentityUpdateTransition error: {}", e))?;

        let result = state_transition
            .broadcast_and_wait(sdk, None)
            .await
            .map_err(|ref e| broadcast_error(e))?;

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
            .map(|_| BackendTaskSuccessResult::AddedKeyToIdentity(fee_result))
            .map_err(|e| TaskError::Generic(format!("Database error: {}", e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::consensus::state::identity::duplicated_identity_public_key_id_state_error::DuplicatedIdentityPublicKeyIdStateError;
    use dash_sdk::dpp::consensus::state::identity::duplicated_identity_public_key_state_error::DuplicatedIdentityPublicKeyStateError;
    use dash_sdk::dpp::consensus::state::identity::identity_public_key_already_exists_for_unique_contract_bounds_error::IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError;
    use dash_sdk::dpp::identity::Purpose;
    use dash_sdk::dpp::identifier::Identifier;

    #[test]
    fn test_duplicate_public_key_error() {
        let consensus =
            ConsensusError::from(DuplicatedIdentityPublicKeyStateError::new(vec![1, 2]));
        let sdk_err = SdkError::from(consensus);
        let err = broadcast_error(&sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKey));
    }

    #[test]
    fn test_duplicate_public_key_id_error() {
        let consensus = ConsensusError::from(DuplicatedIdentityPublicKeyIdStateError::new(vec![3]));
        let sdk_err = SdkError::from(consensus);
        let err = broadcast_error(&sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKeyId));
    }

    #[test]
    fn test_contract_bounds_conflict_error() {
        let contract_id = Identifier::random();
        let identity_id = Identifier::random();
        let consensus = ConsensusError::from(
            IdentityPublicKeyAlreadyExistsForUniqueContractBoundsError::new(
                identity_id,
                contract_id,
                Purpose::AUTHENTICATION,
                2,
                1,
            ),
        );
        let sdk_err = SdkError::from(consensus);
        let err = broadcast_error(&sdk_err);
        let expected_contract_id = contract_id.to_string(Encoding::Base58);
        assert!(
            matches!(err, TaskError::IdentityPublicKeyContractBoundsConflict { ref contract_id } if *contract_id == expected_contract_id)
        );
    }

    #[test]
    fn test_broadcast_error_with_cause_duplicate_key() {
        // Test the StateTransitionBroadcastError path (the actual production path
        // when Platform returns a structured broadcast error with a consensus cause).
        let consensus = ConsensusError::from(DuplicatedIdentityPublicKeyStateError::new(vec![1]));
        let broadcast_err = dash_sdk::error::StateTransitionBroadcastError {
            code: 40206,
            message: "duplicate key".to_string(),
            cause: Some(consensus),
        };
        let sdk_err = SdkError::StateTransitionBroadcastError(broadcast_err);
        let err = broadcast_error(&sdk_err);
        assert!(matches!(err, TaskError::DuplicateIdentityPublicKey));
    }

    #[test]
    fn test_unknown_sdk_error_falls_back() {
        let sdk_err = SdkError::Generic("connection timeout".to_string());
        let err = broadcast_error(&sdk_err);
        assert!(matches!(err, TaskError::Generic(ref s) if s.contains("connection timeout")));
    }
}
