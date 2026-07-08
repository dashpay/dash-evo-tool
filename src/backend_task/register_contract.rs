use dash_sdk::{
    Error, Sdk,
    dpp::data_contract::accessors::v0::DataContractV0Getters,
    platform::{DataContract, IdentityPublicKey, transition::put_contract::PutContract},
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
        let estimated_fee = self.fee_estimator().estimate_contract_create_base();
        let contract_id = data_contract.id();

        match data_contract
            .put_to_platform_and_wait_for_response(sdk, signing_key.clone(), &identity, None)
            .await
        {
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
