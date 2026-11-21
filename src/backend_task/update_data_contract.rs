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
        transition::{
            broadcast::BroadcastStateTransition,
            put_contract::ensure_private_document_entries_fit_on_update,
        },
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
        let signing_time =
            super::contract::prepare_contract_signing_key(sdk, &identity, &signing_key).await?;
        // Estimate fee for contract update
        let estimated_fee = self.fee_estimator().estimate_contract_update();

        // Platform registers a private document type whose entries are too
        // small for its documents, and the entry size can never change after.
        // Contract creation is checked inside the SDK; this transition is
        // built here, so the same check runs here, before anything is signed
        // or changed. It reads the stored contract only when the update
        // declares a private type, and leaves unchanged types alone.
        ensure_private_document_entries_fit_on_update(sdk, data_contract)
            .await
            .map_err(TaskError::from)?;

        let contract_id = data_contract.id();

        // Fetch the identity contract nonce
        let identity_contract_nonce = sdk
            .get_identity_contract_nonce(identity.identity.id(), data_contract.id(), true, None)
            .await
            .map_err(TaskError::from)?;

        data_contract.increment_version();

        // Update UI
        sender
            .send(TaskResult::interim_success(
                self.network,
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

        super::contract::validate_contract_signing_key(
            &identity,
            &signing_key,
            sdk.protocol_version_info().verified_version,
            signing_time,
        )?;
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

        // Unlocking can outlive an expiry or activation; confirm a fresh block before sending.
        super::contract::prepare_contract_signing_key(sdk, &identity, &signing_key).await?;

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
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use dash_sdk::dpp::data_contract::conversion::value::v0::DataContractValueConversionMethodsV0;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{KeyType, Purpose};
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    use dash_sdk::dpp::platform_value::{BinaryData, platform_value};
    use dash_sdk::dpp::version::PlatformVersion;

    /// A contract with the private type `request` of 375-byte entries. Its
    /// required fields need 376 bytes; with `label`, an optional string of at
    /// most 20 bytes, its largest document needs 398.
    fn private_contract(label: bool) -> DataContract {
        let id = Identifier::from([7; 32]).to_string(Encoding::Base58);
        let owner_id = Identifier::from([8; 32]).to_string(Encoding::Base58);
        let mut properties = platform_value!({
            "reference": { "type": "integer", "position": 0 },
            "key": {
                "type": "array", "byteArray": true,
                "minItems": 69, "maxItems": 69, "position": 1
            }
        });
        if label {
            properties
                .set_value(
                    "label",
                    platform_value!({ "type": "string", "maxBytes": 20, "position": 2 }),
                )
                .expect("a map");
        }
        DataContract::from_value(
            platform_value!({
                "$formatVersion": "1",
                "id": id,
                "ownerId": owner_id,
                "version": 1,
                "documentSchemas": {
                    "request": {
                        "type": "object",
                        "private": { "entrySize": 375 },
                        "documentsMutable": false,
                        "canBeDeleted": false,
                        "properties": properties,
                        "required": ["reference", "key"],
                        "additionalProperties": false
                    }
                }
            }),
            true,
            PlatformVersion::latest(),
        )
        .expect("the contract registers")
    }

    /// An identity holding its unlimited signing key `1`, so the signing key
    /// check passes without a network read.
    fn owner_holding_unlimited_key(context: &AppContext) -> (QualifiedIdentity, IdentityPublicKey) {
        let signing_key = IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id: 1,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::CRITICAL,
            contract_bounds: None,
            key_type: KeyType::ECDSA_SECP256K1,
            read_only: false,
            data: BinaryData::new(vec![2; 33]),
            disabled_at: None,
        });
        let mut owner = dash_sdk::dpp::identity::Identity::create_basic_identity(
            Identifier::from([8; 32]),
            PlatformVersion::latest(),
        )
        .expect("basic identity");
        owner.add_public_key(signing_key.clone());
        let identity = QualifiedIdentity {
            identity: owner,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: context.network(),
        };
        (identity, signing_key)
    }

    /// A contract registered with a private document type whose entries are
    /// too small for its documents is refused before the nonce is read or
    /// anything is signed: Platform would register it, and its entry size can
    /// never change afterwards.
    #[tokio::test]
    async fn a_contract_whose_private_type_cannot_hold_its_documents_is_not_registered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        // The mock holds no answers: reading the nonce would fail it.
        let sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .expect("a mock SDK");
        let (identity, signing_key) = owner_holding_unlimited_key(&context);
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());

        let result = context
            .register_data_contract(
                private_contract(true),
                String::new(),
                identity,
                signing_key,
                &sdk,
                sender,
            )
            .await;

        assert!(
            matches!(
                &result,
                Err(TaskError::PrivateDocumentEntrySizeTooSmall {
                    entry_size: 375,
                    minimum_entry_size: 398,
                    ..
                })
            ),
            "{result:?}"
        );
    }

    /// An update that changes a private document type so that its entries can
    /// no longer hold its largest document is refused before the nonce is
    /// read or anything is signed, and the caller's contract is left as it
    /// was. Platform would accept the update, and the entry size can never
    /// change afterwards.
    #[tokio::test]
    async fn an_update_whose_private_type_cannot_hold_its_documents_is_refused_before_signing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        let stored = private_contract(false);
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .expect("a mock SDK");
        // The only answer the mock holds: reading the nonce would fail it.
        sdk.mock()
            .expect_fetch(stored.id(), Some(stored.clone()))
            .await
            .expect("stored contract expectation");

        let (identity, signing_key) = owner_holding_unlimited_key(&context);
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());

        let mut update = private_contract(true);
        let version_before = update.version();
        let result = context
            .update_data_contract(&mut update, identity, signing_key, &sdk, sender)
            .await;

        assert!(
            matches!(
                &result,
                Err(TaskError::PrivateDocumentEntrySizeTooSmall {
                    entry_size: 375,
                    minimum_entry_size: 398,
                    ..
                })
            ),
            "{result:?}"
        );
        assert_eq!(update.version(), version_before);
        // `request` is already registered, so raising its entry size is not
        // open to the user: the update would fail Platform's own validation.
        // The message must say what is.
        let message = result.expect_err("refused").to_string();
        assert!(
            message.contains(
                "For a type that is already registered, the entry size cannot change: make the added fields smaller, remove them, or define a new type."
            ),
            "{message}"
        );
    }

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
