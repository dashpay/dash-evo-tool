use std::sync::Arc;
use std::time::Duration;

use dash_sdk::{
    Error, Sdk,
    dpp::data_contract::accessors::v0::DataContractV0Getters,
    platform::{DataContract, IdentityPublicKey, transition::put_contract::PutContract},
};

use super::{BackendTaskSuccessResult, FeeResult};
use crate::backend_task::error::{ContractSaveWarning, TaskError};
use crate::model::qualified_contract::InsertTokensToo::AllTokensShouldBeAdded;
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};

/// Attempts at saving a contract Platform already confirmed.
const LOCAL_SAVE_ATTEMPTS: u32 = 3;

/// Pause between attempts at saving a confirmed contract.
const LOCAL_SAVE_RETRY_DELAY: Duration = Duration::from_millis(200);

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
                Ok(self
                    .finish_confirmed_registration(
                        &returned_contract,
                        optional_alias.as_deref(),
                        FeeResult::estimated_only(estimated_fee),
                    )
                    .await)
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

    /// Saves a contract Platform confirmed and reports the registration as
    /// done. The fee is already paid, so a save that keeps failing becomes a
    /// warning on the success, never an error inviting a second registration.
    async fn finish_confirmed_registration(
        &self,
        contract: &DataContract,
        alias: Option<&str>,
        fee_result: FeeResult,
    ) -> BackendTaskSuccessResult {
        let saved = retry_local_save(|| {
            self.insert_contract_if_not_exists(contract, alias, AllTokensShouldBeAdded)
        })
        .await;
        let save_warning = saved.err().map(|source| {
            tracing::warn!(
                ?source,
                contract_id = %contract.id(),
                "Contract registered on Platform could not be saved locally"
            );
            ContractSaveWarning::Contract {
                contract_id: contract.id(),
                source: Arc::new(source),
            }
        });
        BackendTaskSuccessResult::RegisteredContract {
            fee_result,
            save_warning,
        }
    }
}

/// Runs a local write up to [`LOCAL_SAVE_ATTEMPTS`] times, returning the last
/// error if none succeeds.
async fn retry_local_save(
    mut save: impl FnMut() -> Result<(), TaskError>,
) -> Result<(), TaskError> {
    let mut attempt = 1;
    loop {
        match save() {
            Ok(()) => return Ok(()),
            Err(error) if attempt >= LOCAL_SAVE_ATTEMPTS => return Err(error),
            Err(error) => {
                tracing::debug!(
                    ?error,
                    attempt,
                    "Saving a registered contract failed; retrying"
                );
                tokio::time::sleep(LOCAL_SAVE_RETRY_DELAY).await;
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{test_app_context, wired_test_app_context};

    /// Platform confirmed the registration, but the contract cannot be saved
    /// here: the task still succeeds, and the warning names the contract.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_confirmed_registration_succeeds_even_when_it_cannot_be_saved() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No wallet backend is wired, so every local save fails.
        let ctx = test_app_context(dir.path());
        let contract = ctx.dpns_contract().as_ref().clone();

        let result = ctx
            .finish_confirmed_registration(&contract, Some("alias"), FeeResult::estimated_only(7))
            .await;

        let BackendTaskSuccessResult::RegisteredContract {
            fee_result,
            save_warning: Some(ContractSaveWarning::Contract { contract_id, .. }),
        } = result
        else {
            panic!("expected a registered contract with a save warning, got {result:?}");
        };
        assert_eq!(contract_id, contract.id());
        assert_eq!(fee_result.estimated_fee, 7);
    }

    /// A confirmed registration is saved here, with its alias, and reports
    /// no warning.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_confirmed_registration_is_saved_locally() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let contract = ctx.dpns_contract().as_ref().clone();

        let result = ctx
            .finish_confirmed_registration(&contract, Some("alias"), FeeResult::estimated_only(7))
            .await;

        assert!(
            matches!(
                result,
                BackendTaskSuccessResult::RegisteredContract {
                    save_warning: None,
                    ..
                }
            ),
            "{result:?}"
        );
        let saved = ctx
            .get_contract_by_id(&contract.id())
            .expect("read saved contract")
            .expect("contract saved");
        assert_eq!(saved.alias.as_deref(), Some("alias"));
    }

    /// A save that fails once is retried and succeeds.
    #[tokio::test]
    async fn a_failed_local_save_is_retried() {
        let mut calls = 0;
        let saved = retry_local_save(|| {
            calls += 1;
            if calls == 1 {
                Err(TaskError::WalletBackendNotYetWired)
            } else {
                Ok(())
            }
        })
        .await;

        assert!(saved.is_ok(), "{saved:?}");
        assert_eq!(calls, 2);
    }

    /// A save that keeps failing stops after a bounded number of attempts.
    #[tokio::test]
    async fn a_local_save_that_keeps_failing_gives_up() {
        let mut calls = 0;
        let saved = retry_local_save(|| {
            calls += 1;
            Err(TaskError::WalletBackendNotYetWired)
        })
        .await;

        assert!(matches!(saved, Err(TaskError::WalletBackendNotYetWired)));
        assert_eq!(calls, LOCAL_SAVE_ATTEMPTS);
    }
}
