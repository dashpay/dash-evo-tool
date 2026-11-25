use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use dash_sdk::dash_platform_queries::transition::validation::ensure_valid_state_transition_structure;
use dash_sdk::dpp::identity::PartialIdentity;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::prelude::IdentityNonce;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::state_transition::data_contract_create_transition::DataContractCreateTransition;
use dash_sdk::dpp::state_transition::data_contract_create_transition::methods::DataContractCreateTransitionMethodsV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::put_contract::ensure_private_document_entries_fit;
use dash_sdk::{
    Error as SdkError, Sdk,
    dpp::data_contract::accessors::v0::DataContractV0Getters,
    platform::{DataContract, Identifier, IdentityPublicKey},
};

use super::{BackendTaskSuccessResult, FeeResult};
use crate::backend_task::error::{ContractSaveWarning, TaskError};
use crate::model::qualified_contract::InsertTokensToo::AllTokensShouldBeAdded;
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};

/// Attempts at saving a contract Platform already confirmed.
const LOCAL_SAVE_ATTEMPTS: u32 = 3;

/// Pause between attempts at saving a confirmed contract.
const LOCAL_SAVE_RETRY_DELAY: Duration = Duration::from_millis(200);

/// What the user registered. The registration is the same; its result and
/// wording differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationKind {
    /// A contract from the contract registration screen.
    Contract,
    /// A token's contract from the token creator.
    Token,
}

impl RegistrationKind {
    fn registered(
        self,
        fee_result: FeeResult,
        save_warning: Option<ContractSaveWarning>,
    ) -> BackendTaskSuccessResult {
        match self {
            Self::Contract => BackendTaskSuccessResult::RegisteredContract {
                fee_result,
                save_warning,
            },
            Self::Token => BackendTaskSuccessResult::RegisteredTokenContract { save_warning },
        }
    }

    fn save_warning(self, contract_id: Identifier, source: TaskError) -> ContractSaveWarning {
        let source = Arc::new(source);
        match self {
            Self::Contract => ContractSaveWarning::Contract {
                contract_id,
                source,
            },
            Self::Token => ContractSaveWarning::Token {
                contract_id,
                source,
            },
        }
    }

    fn unconfirmed(self, contract_id: Identifier, source: SdkError) -> TaskError {
        let source_error = Box::new(source);
        match self {
            Self::Contract => TaskError::ContractRegistrationUnconfirmed {
                contract_id,
                source_error,
            },
            Self::Token => TaskError::TokenRegistrationUnconfirmed {
                contract_id,
                source_error,
            },
        }
    }
}

impl AppContext {
    /// Registers `data_contract` on Platform. The transition is built here, so
    /// the contract ID Platform assigns (from the owner and identity nonce) is
    /// known even when the result cannot be read, and the contract is then
    /// looked up under that ID.
    #[expect(
        clippy::too_many_arguments,
        reason = "the registration's inputs plus the SDK and the frame-loop sender"
    )]
    pub async fn register_data_contract(
        &self,
        data_contract: DataContract,
        alias: String,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        kind: RegistrationKind,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let fee_result =
            FeeResult::estimated_only(self.fee_estimator().estimate_contract_create_base());
        let alias = (!alias.is_empty()).then_some(alias);

        ensure_private_document_entries_fit(&data_contract, None)?;
        let identity_nonce = sdk
            .get_identity_nonce(data_contract.owner_id(), true, None)
            .await?;
        let (transition, contract_id) = contract_create_transition(
            data_contract,
            identity_nonce,
            signing_key,
            &identity,
            sdk.version(),
        )
        .await?;

        match transition
            .broadcast_and_wait::<DataContract>(sdk, None)
            .await
        {
            Ok(contract) => Ok(self
                .finish_confirmed_registration(&contract, alias.as_deref(), fee_result, kind)
                .await),
            Err(error) if may_have_registered(&error) => {
                let found = self
                    .refetch_contract_after_unread_result(sdk, contract_id, &sender)
                    .await?;
                self.settle_unconfirmed_registration(
                    found,
                    contract_id,
                    error,
                    alias.as_deref(),
                    fee_result,
                    kind,
                )
                .await
            }
            Err(error) => Err(TaskError::from(error)),
        }
    }

    /// Settles a registration whose result could not be read, by the lookup
    /// under its Platform-assigned ID: found is confirmed; not found stays
    /// unresolved, never a failure inviting a second registration.
    async fn settle_unconfirmed_registration(
        &self,
        found: Option<DataContract>,
        contract_id: Identifier,
        error: SdkError,
        alias: Option<&str>,
        fee_result: FeeResult,
        kind: RegistrationKind,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match found {
            Some(contract) => Ok(self
                .finish_confirmed_registration(&contract, alias, fee_result, kind)
                .await),
            None => Err(kind.unconfirmed(contract_id, error)),
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
        kind: RegistrationKind,
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
            kind.save_warning(contract.id(), source)
        });
        kind.registered(fee_result, save_warning)
    }
}

/// Builds and signs the create transition for `data_contract`, as the SDK's
/// `PutContract` does, and returns it with the contract ID Platform assigns:
/// derived from the owner and `identity_nonce`, replacing the contract's own.
async fn contract_create_transition<S: Signer<IdentityPublicKey>>(
    data_contract: DataContract,
    identity_nonce: IdentityNonce,
    signing_key: IdentityPublicKey,
    signer: &S,
    platform_version: &PlatformVersion,
) -> Result<(StateTransition, Identifier), TaskError> {
    let owner_id = data_contract.owner_id();
    let key_id = signing_key.id();
    let partial_identity = PartialIdentity {
        id: owner_id,
        loaded_public_keys: BTreeMap::from([(key_id, signing_key)]),
        balance: None,
        revision: None,
        not_found_public_keys: Default::default(),
    };
    let transition = DataContractCreateTransition::new_from_data_contract(
        data_contract,
        identity_nonce,
        &partial_identity,
        key_id,
        signer,
        platform_version,
        None,
    )
    .await
    .map_err(|e| TaskError::from(SdkError::Protocol(e)))?;
    ensure_valid_state_transition_structure(&transition, platform_version)
        .map_err(|e| TaskError::from(SdkError::from(e)))?;
    let contract_id = DataContract::generate_data_contract_id_v0(owner_id, identity_nonce);
    Ok((transition, contract_id))
}

/// Whether a failed registration may still have been executed: its result
/// could not be read or proved, so only a lookup can tell.
fn may_have_registered(error: &SdkError) -> bool {
    matches!(error, SdkError::DriveProofError(..)) || error.is_state_transition_result_unconfirmed()
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
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::state_transition::data_contract_create_transition::accessors::DataContractCreateTransitionAccessorsV0;

    /// Signs anything with a fixed signature: the tests check what is signed,
    /// not the signature.
    #[derive(Debug)]
    struct FixedSigner;

    #[async_trait::async_trait]
    impl Signer<IdentityPublicKey> for FixedSigner {
        async fn sign(
            &self,
            _key: &IdentityPublicKey,
            _data: &[u8],
        ) -> Result<BinaryData, dash_sdk::dpp::ProtocolError> {
            Ok(BinaryData::new(vec![0x1f; 65]))
        }

        async fn sign_create_witness(
            &self,
            _key: &IdentityPublicKey,
            _data: &[u8],
        ) -> Result<dash_sdk::dpp::address_funds::AddressWitness, dash_sdk::dpp::ProtocolError>
        {
            Err(dash_sdk::dpp::ProtocolError::CorruptedCodeExecution(
                "a contract create transition has no address witness".to_string(),
            ))
        }

        fn can_sign_with(&self, _key: &IdentityPublicKey) -> bool {
            true
        }
    }

    fn signing_key() -> IdentityPublicKey {
        let identity = crate::context::test_support::user_identity_with_signing_key(
            Identifier::from([0x21; 32]),
            dash_sdk::dpp::dashcore::Network::Testnet,
            false,
        );
        identity.identity.public_keys()[&1].clone()
    }

    fn proof_error() -> SdkError {
        SdkError::DriveProofError(
            dash_sdk::drive::error::proof::ProofError::CorruptedProof("bad".to_string()),
            Vec::new(),
            dash_sdk::dpp::block::block_info::BlockInfo::default(),
        )
    }

    /// The transition carries the ID Platform assigns, derived from the owner
    /// and the identity nonce, not the contract's placeholder ID; and the
    /// returned ID is that one.
    #[tokio::test]
    async fn the_registered_id_is_derived_from_the_owner_and_nonce() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let contract = ctx.dpns_contract().as_ref().clone();
        let (placeholder, owner) = (contract.id(), contract.owner_id());

        let (transition, contract_id) = contract_create_transition(
            contract.clone(),
            5,
            signing_key(),
            &FixedSigner,
            ctx.platform_version(),
        )
        .await
        .expect("transition");

        assert_eq!(
            contract_id,
            DataContract::generate_data_contract_id_v0(owner, 5)
        );
        assert_ne!(contract_id, placeholder);
        let StateTransition::DataContractCreate(create) = &transition else {
            panic!("expected a contract create transition, got {transition:?}");
        };
        assert_eq!(create.data_contract().id(), contract_id);
        assert_eq!(create.identity_nonce(), 5);

        let (_, next_id) = contract_create_transition(
            contract,
            6,
            signing_key(),
            &FixedSigner,
            ctx.platform_version(),
        )
        .await
        .expect("transition");
        assert_ne!(next_id, contract_id);
    }

    /// A proof error, or a result that could not be read (also when retries
    /// ran out), may hide an executed registration; a refusal before
    /// submission does not.
    #[test]
    fn only_an_unread_result_may_hide_a_registration() {
        assert!(may_have_registered(&proof_error()));
        assert!(may_have_registered(
            &SdkError::StateTransitionResultUnconfirmed(Box::new(proof_error()))
        ));
        assert!(may_have_registered(&SdkError::NoAvailableAddressesToRetry(
            Box::new(SdkError::StateTransitionResultUnconfirmed(Box::new(
                SdkError::Generic("wait failed".to_string())
            )))
        )));
        assert!(!may_have_registered(&SdkError::Generic(
            "refused".to_string()
        )));
    }

    /// A registration found under its assigned ID after an unread result is
    /// confirmed, saved, and reported as done, for both kinds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_registration_found_after_an_unread_result_is_confirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let contract = ctx.dpns_contract().as_ref().clone();

        let result = ctx
            .settle_unconfirmed_registration(
                Some(contract.clone()),
                contract.id(),
                SdkError::StateTransitionResultUnconfirmed(Box::new(proof_error())),
                Some("alias"),
                FeeResult::estimated_only(7),
                RegistrationKind::Contract,
            )
            .await;
        assert!(
            matches!(
                result,
                Ok(BackendTaskSuccessResult::RegisteredContract {
                    save_warning: None,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(
            ctx.get_contract_by_id(&contract.id())
                .expect("read saved contract")
                .is_some()
        );

        let result = ctx
            .settle_unconfirmed_registration(
                Some(contract.clone()),
                contract.id(),
                proof_error(),
                None,
                FeeResult::estimated_only(7),
                RegistrationKind::Token,
            )
            .await;
        assert!(
            matches!(
                result,
                Ok(BackendTaskSuccessResult::RegisteredTokenContract { save_warning: None })
            ),
            "{result:?}"
        );
    }

    /// A registration not found after an unread result stays unresolved: the
    /// error names the assigned ID and asks the user to check, not to retry.
    #[tokio::test]
    async fn a_registration_not_found_after_an_unread_result_stays_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let contract_id = Identifier::from([0x42; 32]);

        for kind in [RegistrationKind::Contract, RegistrationKind::Token] {
            let error = ctx
                .settle_unconfirmed_registration(
                    None,
                    contract_id,
                    SdkError::StateTransitionResultUnconfirmed(Box::new(proof_error())),
                    None,
                    FeeResult::estimated_only(7),
                    kind,
                )
                .await
                .expect_err("an unresolved registration is not a success");
            let named_id = match (&error, kind) {
                (
                    TaskError::ContractRegistrationUnconfirmed { contract_id, .. },
                    RegistrationKind::Contract,
                )
                | (
                    TaskError::TokenRegistrationUnconfirmed { contract_id, .. },
                    RegistrationKind::Token,
                ) => *contract_id,
                _ => panic!("{kind:?}: expected an unconfirmed registration, got {error:?}"),
            };
            assert_eq!(named_id, contract_id);
            let message = error.to_string();
            assert!(message.contains(&contract_id.to_string()), "{message}");
            assert!(!message.contains("try again"), "{message}");
        }
    }

    /// Platform confirmed the registration, but the contract cannot be saved
    /// here: the task still succeeds, with a warning naming the contract in
    /// the words of what was registered.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_confirmed_registration_succeeds_even_when_it_cannot_be_saved() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No wallet backend is wired, so every local save fails.
        let ctx = test_app_context(dir.path());
        let contract = ctx.dpns_contract().as_ref().clone();

        let result = ctx
            .finish_confirmed_registration(
                &contract,
                Some("alias"),
                FeeResult::estimated_only(7),
                RegistrationKind::Contract,
            )
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

        let result = ctx
            .finish_confirmed_registration(
                &contract,
                None,
                FeeResult::estimated_only(7),
                RegistrationKind::Token,
            )
            .await;
        let BackendTaskSuccessResult::RegisteredTokenContract {
            save_warning: Some(warning @ ContractSaveWarning::Token { .. }),
        } = result
        else {
            panic!("expected a registered token with a save warning, got {result:?}");
        };
        assert!(warning.to_string().contains("Import Token"), "{warning}");
    }

    /// A confirmed registration is saved here, with its alias, and reports
    /// no warning.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_confirmed_registration_is_saved_locally() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let contract = ctx.dpns_contract().as_ref().clone();

        let result = ctx
            .finish_confirmed_registration(
                &contract,
                Some("alias"),
                FeeResult::estimated_only(7),
                RegistrationKind::Contract,
            )
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
