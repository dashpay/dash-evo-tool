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
            .send(TaskResult::Success(Box::new(
                BackendTaskSuccessResult::FetchedNonce,
            )))
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

        match state_transition.broadcast_and_wait(sdk, None).await {
            Ok(returned_contract) => {
                self.replace_contract(data_contract.id(), &returned_contract)?;
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
