use dash_sdk::{
    Error, Sdk,
    dpp::data_contract::accessors::v0::DataContractV0Getters,
    platform::{
        DataContract, IdentityPublicKey,
        transition::{
            broadcast::BroadcastStateTransition, put_contract::ensure_private_document_entries_fit,
        },
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
        // A failed wait after a possible submission is reported unconfirmed,
        // so a paid registration is never offered as a plain retry.
        match transition
            .broadcast_and_wait::<DataContract>(sdk, None)
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
    // Platform registers a private document type whose entries are too small
    // for its documents, and the entry size can never change after.
    ensure_private_document_entries_fit(&contract, None)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::{auth_key, identity_holding, test_app_context};
    use dash_sdk::dapi_client::transport::TransportError;
    use dash_sdk::dapi_client::{
        DapiClientError, DumpData, ExecutionError, ExecutionResponse, mock::MockResult,
    };
    use dash_sdk::dapi_grpc::platform::v0::{
        BroadcastStateTransitionRequest, WaitForStateTransitionResultRequest,
    };
    use dash_sdk::dapi_grpc::tonic::Status;
    use dash_sdk::dpp::data_contract::conversion::value::v0::DataContractValueConversionMethodsV0;
    use dash_sdk::dpp::identity::SecurityLevel;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
    use dash_sdk::dpp::platform_value::platform_value;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    use dash_sdk::platform::transition::broadcast_request::BroadcastRequestForStateTransition;
    use dash_sdk::query_types::IdentityNonceFetcher;

    /// An offline SDK that answers the identity nonce of `owner`.
    async fn sdk_with_nonce(owner: Identifier) -> Sdk {
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .expect("a mock SDK");
        sdk.mock()
            .expect_fetch::<IdentityNonceFetcher, _>(owner, Some(IdentityNonceFetcher(1)))
            .await
            .expect("nonce expectation");
        sdk
    }

    fn save<R>(dir: &std::path::Path, request: &R, response: &MockResult<R>)
    where
        R: dash_sdk::dapi_client::transport::TransportRequest,
    {
        let dump = DumpData::new(request, response);
        dump.save(&dir.join(dump.filename().expect("dump name")))
            .expect("dump saved");
    }

    /// A registration whose broadcast may have reached Platform, and whose
    /// result cannot be read, may already be paid for: the user is told to
    /// check before trying again, never just to retry.
    #[tokio::test]
    async fn a_registration_that_may_have_been_accepted_is_reported_unconfirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let context = test_app_context(dir.path());
        let mut key = auth_key(1, SecurityLevel::CRITICAL);
        let secret = crate::test_support::keys::labelled_secret(
            "context::test_support::identity_holding",
            1,
        );
        key.set_data(
            dash_sdk::dpp::identity::KeyType::ECDSA_SECP256K1
                .public_key_data_from_private_key_data(&secret, context.network())
                .expect("generated key")
                .into(),
        );
        let identity = identity_holding(std::slice::from_ref(&key));
        let owner = Identifier::from([8; 32]);
        let contract = DataContract::from_value(
            platform_value!({
                "$formatVersion": "1",
                "id": Identifier::from([7; 32]).to_string(Encoding::Base58),
                "ownerId": owner.to_string(Encoding::Base58),
                "version": 1,
                "documentSchemas": { "note": {
                    "type": "object",
                    "properties": { "text": { "type": "string", "maxLength": 20, "position": 0 } },
                    "additionalProperties": false
                } }
            }),
            true,
            PlatformVersion::latest(),
        )
        .expect("a contract");

        // The exact transition registration signs, from a separate SDK so
        // both read the same nonce.
        let transition = prepare_contract_transition(
            &sdk_with_nonce(owner).await,
            contract.clone(),
            &identity,
            &key,
            0,
        )
        .await
        .expect("a signed transition");
        let dumps = tempfile::tempdir().expect("tempdir");
        let broadcast: BroadcastStateTransitionRequest = transition
            .broadcast_request_for_state_transition()
            .expect("broadcast request");
        save(
            dumps.path(),
            &broadcast,
            &Err(ExecutionError {
                possibly_accepted: false,
                inner: DapiClientError::Transport(TransportError::Grpc(Status::already_exists(
                    "tx already exists in cache",
                ))),
                address: None,
                retries: 0,
            }),
        );
        // No proof comes back: the outcome stays unknown.
        let wait: WaitForStateTransitionResultRequest = transition
            .wait_for_state_transition_result_request()
            .expect("wait request");
        save(
            dumps.path(),
            &wait,
            &Ok(ExecutionResponse {
                inner: Default::default(),
                retries: 0,
                address: "http://127.0.0.1".parse().expect("address"),
            }),
        );
        let mut sdk = sdk_with_nonce(owner).await;
        sdk.mock()
            .load_expectations_sync(dumps.path())
            .expect("submission expectations");
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());

        let result = context
            .register_data_contract(contract, String::new(), identity, key, &sdk, sender)
            .await;

        assert!(
            matches!(&result, Err(TaskError::PlatformResultUnconfirmed { .. })),
            "{result:?}"
        );
    }
}
