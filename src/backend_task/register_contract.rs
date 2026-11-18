use dash_sdk::{
    Error, Sdk,
    dpp::data_contract::accessors::v0::DataContractV0Getters,
    platform::{
        DataContract, IdentityPublicKey,
        transition::{broadcast::BroadcastStateTransition, waitable::Waitable},
    },
};

use super::{BackendTaskSuccessResult, FeeResult};
use crate::backend_task::error::TaskError;
use crate::model::qualified_contract::InsertTokensToo::AllTokensShouldBeAdded;
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};

impl AppContext {
    pub async fn register_data_contract(
        &self,
        data_contract: DataContract,
        alias: String,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let signing_time =
            super::contract::prepare_contract_signing_key(sdk, &identity, &signing_key).await?;
        let estimated_fee = self.fee_estimator().estimate_contract_create_base();
        let contract_id = data_contract.id();

        let transition =
            prepare_contract_transition(sdk, data_contract, &identity, &signing_key, signing_time)
                .await?;
        let result = async {
            transition.broadcast(sdk, None).await?;
            DataContract::wait_for_response(sdk, transition, None).await
        }
        .await;
        match result {
            Ok(returned_contract) => {
                let optional_alias = match alias.is_empty() {
                    true => None,
                    false => Some(alias),
                };
                self.insert_contract_if_not_exists(
                    &returned_contract,
                    optional_alias.as_deref(),
                    AllTokensShouldBeAdded,
                )?;
                Ok(BackendTaskSuccessResult::RegisteredContract(
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
                        let optional_alias = ctx
                            .get_contract_by_id(&contract.id())
                            .ok()
                            .flatten()
                            .and_then(|c| c.alias);
                        ctx.insert_contract_if_not_exists(
                            contract,
                            optional_alias.as_deref(),
                            AllTokensShouldBeAdded,
                        )
                        .ok();
                    },
                )
                .await
            }
            Err(e) => Err(TaskError::from(e)),
        }
    }
}

/// Use DPP's builder so a nonce fetch or an unlock cannot bypass a newly
/// confirmed signing rule between preflight and broadcast.
async fn prepare_contract_transition(
    sdk: &Sdk,
    contract: DataContract,
    identity: &QualifiedIdentity,
    key: &IdentityPublicKey,
    signing_time: u64,
) -> Result<dash_sdk::dpp::state_transition::StateTransition, TaskError> {
    use dash_sdk::dpp::identity::{
        PartialIdentity, identity_public_key::accessors::v0::IdentityPublicKeyGettersV0,
    };
    use dash_sdk::dpp::state_transition::data_contract_create_transition::{
        DataContractCreateTransition, methods::DataContractCreateTransitionMethodsV0,
    };
    let nonce = sdk
        .get_identity_nonce(contract.owner_id(), true, None)
        .await?;
    super::contract::validate_contract_signing_key(
        identity,
        key,
        sdk.protocol_version_info().verified_version,
        signing_time,
    )?;
    let partial = PartialIdentity {
        id: contract.owner_id(),
        loaded_public_keys: std::collections::BTreeMap::from([(key.id(), key.clone())]),
        balance: None,
        revision: None,
        not_found_public_keys: Default::default(),
    };
    let transition = DataContractCreateTransition::new_from_data_contract(
        contract,
        nonce,
        &partial,
        key.id(),
        identity,
        sdk.version(),
        None,
    )
    .await
    .map_err(Error::from)?;
    dash_sdk::dash_platform_queries::transition::validation::ensure_valid_state_transition_structure(
        &transition, sdk.version(),
    ).map_err(Error::from)?;
    // Unlocking can outlive an expiry or activation; confirm a fresh block before sending.
    super::contract::prepare_contract_signing_key(sdk, identity, key).await?;
    Ok(transition)
}
