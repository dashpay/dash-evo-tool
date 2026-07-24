use super::{BackendTaskSuccessResult, FeeResult};
use crate::{
    app::TaskResult, backend_task::error::TaskError, context::AppContext,
    model::qualified_identity::QualifiedIdentity,
};
use dash_sdk::{
    Error, Sdk,
    dpp::{
        data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters},
        identity::{SecurityLevel, accessors::IdentityGettersV0},
        state_transition::{
            StateTransition, StateTransitionSigningOptions,
            data_contract_update_transition::DataContractUpdateTransition,
        },
        version::TryIntoPlatformVersioned,
    },
    platform::{
        DataContract, Identifier, IdentityPublicKey,
        transition::broadcast::BroadcastStateTransition,
    },
};

fn validate_updated_contract_id(
    expected: Identifier,
    returned: Identifier,
) -> Result<(), TaskError> {
    if returned != expected {
        return Err(TaskError::UpdatedContractIdMismatch { expected, returned });
    }
    Ok(())
}

impl AppContext {
    pub async fn update_data_contract(
        &self,
        data_contract: &mut DataContract,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        // Estimate fee for contract update
        let estimated_fee = self.fee_estimator().estimate_contract_update();

        // Increment the version of the data contract
        data_contract.increment_version();
        let contract_id = data_contract.id();

        // Fetch the identity contract nonce
        let identity_contract_nonce = sdk
            .get_identity_contract_nonce(identity.identity.id(), data_contract.id(), true, None)
            .await
            .map_err(TaskError::from)?;

        // Update UI
        sender
            .send(TaskResult::unattributed_success(
                BackendTaskSuccessResult::FetchedNonce,
            ))
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        let contract_update_transition: DataContractUpdateTransition =
            (data_contract.clone(), identity_contract_nonce)
                .try_into_platform_versioned(sdk.version())
                .map_err(|e: dash_sdk::dpp::ProtocolError| {
                    TaskError::from(dash_sdk::Error::Protocol(e))
                })?;

        let mut state_transition = StateTransition::DataContractUpdate(contract_update_transition);

        state_transition
            .sign_external_with_options(
                &signing_key,
                &identity,
                None::<
                    fn(Identifier, String) -> Result<SecurityLevel, dash_sdk::dpp::ProtocolError>,
                >,
                StateTransitionSigningOptions {
                    allow_signing_with_any_security_level: false,
                    allow_signing_with_any_purpose: false,
                },
            )
            .await
            .map_err(|e| TaskError::from(dash_sdk::Error::Protocol(e)))?;

        match state_transition
            .broadcast_and_wait_for_affected_state::<DataContract>(sdk, None)
            .await
        {
            Ok(returned_contract) => {
                validate_updated_contract_id(contract_id, returned_contract.id())?;
                self.replace_contract(contract_id, &returned_contract)?;
                Ok(BackendTaskSuccessResult::UpdatedContract(
                    FeeResult::estimated_only(estimated_fee),
                ))
            }
            Err(e @ Error::DriveProofError(..)) => {
                self.recover_contract_after_proof_error(
                    sdk,
                    contract_id,
                    e,
                    &sender,
                    |ctx, contract| {
                        ctx.replace_contract(contract.id(), contract).ok();
                    },
                )
                .await
            }
            Err(e) => Err(crate::backend_task::error::TaskError::from(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returned_contract_id_mismatch_is_rejected_before_caching() {
        let expected = Identifier::from([1; 32]);
        let returned = Identifier::from([2; 32]);

        let error = validate_updated_contract_id(expected, returned)
            .expect_err("a mismatched returned contract must be rejected");

        assert!(matches!(
            &error,
            TaskError::UpdatedContractIdMismatch {
                expected: error_expected,
                returned: error_returned,
            } if *error_expected == expected && *error_returned == returned
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "The platform returned contract {returned} instead of {expected}. Please try the update again."
            )
        );
    }

    #[test]
    fn matching_returned_contract_id_is_accepted() {
        let contract_id = Identifier::from([3; 32]);

        assert!(validate_updated_contract_id(contract_id, contract_id).is_ok());
    }
}
