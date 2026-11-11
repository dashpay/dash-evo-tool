//! Preparation, immutable signing, and read-only recovery for Token Creator.

use super::*;
use crate::context::token_registration::TokenRegistrationJournal;
use crate::model::token_creator::{
    TokenDraft,
    contract::canonical_contract,
    registration::{
        PreparedRegistration, RegistrationOutcome, RegistrationRecord, RegistrationStatus,
    },
};
use dash_sdk::dpp::dashcore::hashes::{Hash, sha256};
use dash_sdk::dpp::{
    data_contract::{
        accessors::{v0::DataContractV0Getters, v1::DataContractV1Getters},
        serialized_version::DataContractInSerializationFormat,
    },
    identity::{
        Identity, PartialIdentity, Purpose, SecurityLevel,
        identity_public_key::accessors::v0::IdentityPublicKeyGettersV0, signer::Signer,
    },
    serialization::{PlatformSerializable, PlatformSerializableWithPlatformVersion},
    state_transition::{
        StateTransition,
        data_contract_create_transition::{
            DataContractCreateTransition, accessors::DataContractCreateTransitionAccessorsV0,
            methods::DataContractCreateTransitionMethodsV0,
        },
    },
    version::TryFromPlatformVersioned,
};
use dash_sdk::platform::transition::{
    broadcast::BroadcastStateTransition, put_settings::PutSettings,
};

fn settings() -> Option<PutSettings> {
    let mut settings = PutSettings {
        identity_nonce_stale_time_s: Some(0),
        wait_timeout: Some(std::time::Duration::from_secs(60)),
        ..Default::default()
    };
    settings.request_settings.retries = Some(0);
    Some(settings)
}
fn protocol(source: ProtocolError) -> TaskError {
    TaskError::from(dash_sdk::Error::Protocol(source))
}

async fn reserve_reviewed_nonce(
    sdk: &Sdk,
    owner: Identifier,
    reviewed: u64,
) -> Result<u64, TaskError> {
    sdk.refresh_identity_nonce(&owner).await;
    let current = sdk.get_identity_nonce(owner, false, settings()).await?;
    if current.checked_add(1) != Some(reviewed) {
        return Err(TaskError::TokenCreatorChanged);
    }
    // This is the only reservation. Another identity operation may have reserved in between.
    let reserved = sdk.get_identity_nonce(owner, true, settings()).await?;
    if reserved != reviewed {
        return Err(TaskError::TokenCreatorChanged);
    }
    Ok(reserved)
}

impl AppContext {
    async fn creator_current_owner(
        &self,
        sdk: &Sdk,
        owner: Identifier,
        key: &IdentityPublicKey,
    ) -> Result<QualifiedIdentity, TaskError> {
        let fresh = await_network_request_with_timeout(
            NETWORK_REQUEST_TIMEOUT,
            Identity::fetch(sdk, owner),
            |source| TaskError::TokenCreatorReadTimeout { source },
        )
        .await??
        .ok_or(TaskError::TokenCreatorOwnerNotFound)?;
        let current = fresh
            .get_public_key_by_id(key.id())
            .ok_or(TaskError::TokenCreatorKeyUnavailable)?;
        if current != key
            || key.disabled_at().is_some()
            || key.read_only()
            || key.contract_bounds().is_some()
            || key.purpose() != Purpose::AUTHENTICATION
            || !matches!(
                key.security_level(),
                SecurityLevel::CRITICAL | SecurityLevel::HIGH
            )
        {
            return Err(TaskError::TokenCreatorKeyUnavailable);
        }
        let mut local = self
            .get_local_qualified_identity(&owner)?
            .ok_or(TaskError::TokenCreatorOwnerNotFound)?;
        if local.network != self.network || !local.can_sign_with(key) {
            return Err(TaskError::TokenCreatorKeyUnavailable);
        }
        local.identity = fresh;
        Ok(local)
    }

    pub(super) async fn prepare_token_creation(
        &self,
        sdk: &Sdk,
        draft: TokenDraft,
        owner: Identifier,
        key: IdentityPublicKey,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let journal = self.token_registration_journal()?;
        if journal
            .read()?
            .is_some_and(|record| record.blocks_registration())
        {
            return Err(TaskError::TokenCreatorBusy);
        }
        let identity = self.creator_current_owner(sdk, owner, &key).await?;
        sdk.refresh_identity_nonce(&owner).await;
        let nonce = sdk
            .get_identity_nonce(owner, false, settings())
            .await?
            .checked_add(1)
            .ok_or(TaskError::TokenCreatorChanged)?;
        let version = sdk.version();
        let params = draft
            .parse(
                owner,
                self.network,
                version,
                chrono::Utc::now().timestamp_millis().max(0) as u64,
            )
            .map_err(|source| TaskError::InvalidTokenDraft {
                source: Box::new(source),
            })?;
        let alias = params
            .token_names
            .first()
            .map(|entry| entry.0.clone())
            .unwrap_or_default();
        let contract = canonical_contract(owner, nonce, params, version).map_err(|source| {
            TaskError::InvalidTokenDraft {
                source: Box::new(source),
            }
        })?;
        let registration_cost =
            crate::model::fee_estimation::token_creator_registration_charge(&contract, version)
                .map_err(protocol)?;
        Ok(BackendTaskSuccessResult::TokenCreationPrepared(Box::new(
            PreparedRegistration {
                contract,
                nonce,
                key,
                network: self.network,
                protocol_version: version.protocol_version,
                alias,
                registration_cost,
                owner_balance: identity.identity.balance(),
            },
        )))
    }

    pub(super) async fn submit_token_creation(
        &self,
        sdk: &Sdk,
        prepared: PreparedRegistration,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let journal = self.token_registration_journal()?;
        if journal
            .read()?
            .is_some_and(|record| record.blocks_registration())
        {
            return Err(TaskError::TokenCreatorBusy);
        }
        if prepared.network != self.network
            || prepared.protocol_version != sdk.version().protocol_version
        {
            return Err(TaskError::TokenCreatorChanged);
        }
        crate::model::token_creator::contract::validate_submission_time(
            &prepared.contract,
            chrono::Utc::now().timestamp_millis().max(0) as u64,
        )
        .map_err(|source| TaskError::InvalidTokenDraft {
            source: Box::new(source),
        })?;
        let owner = prepared.contract.owner_id();
        let identity = self
            .creator_current_owner(sdk, owner, &prepared.key)
            .await?;
        let required = crate::model::fee_estimation::token_creator_registration_charge(
            &prepared.contract,
            sdk.version(),
        )
        .map_err(protocol)?;
        if required != prepared.registration_cost {
            return Err(TaskError::TokenCreatorChanged);
        }
        if identity.identity.balance() < required {
            return Err(TaskError::TokenCreatorInsufficientCredits {
                available: identity.identity.balance(),
                required,
            });
        }
        let nonce = reserve_reviewed_nonce(sdk, owner, prepared.nonce).await?;
        let version = sdk.version();
        if version.protocol_version != prepared.protocol_version {
            return Err(TaskError::TokenCreatorChanged);
        }
        let expected = DataContractInSerializationFormat::try_from_platform_versioned(
            &prepared.contract,
            version,
        )
        .map_err(protocol)?;
        let partial = PartialIdentity {
            id: owner,
            loaded_public_keys: BTreeMap::from([(prepared.key.id(), prepared.key.clone())]),
            balance: Some(identity.identity.balance()),
            revision: None,
            not_found_public_keys: Default::default(),
        };
        // Default PutContract would choose another nonce and ID. Sign exactly this approved contract.
        let transition = DataContractCreateTransition::new_from_data_contract(
            prepared.contract.clone(),
            nonce,
            &partial,
            prepared.key.id(),
            &identity,
            version,
            None,
        )
        .await
        .map_err(protocol)?;
        let StateTransition::DataContractCreate(creation) = &transition else {
            return Err(TaskError::TokenCreatorChanged);
        };
        if creation.data_contract() != &expected {
            return Err(TaskError::TokenCreatorChanged);
        }
        let contract_bytes = prepared
            .contract
            .serialize_to_bytes_with_platform_version(version)
            .map_err(protocol)?;
        let mut binding = contract_bytes.clone();
        binding.extend_from_slice(&prepared.key.id().to_be_bytes());
        binding.extend_from_slice(crate::wallet_backend::network_prefix(self.network).as_bytes());
        let record = RegistrationRecord {
            attempt_id: sha256::Hash::hash(&binding).to_byte_array(),
            network: self.network,
            owner,
            contract_id: prepared.contract.id(),
            token_id: prepared
                .contract
                .token_id(0)
                .ok_or(TaskError::TokenCreatorChanged)?,
            nonce,
            key_id: prepared.key.id(),
            protocol_version: version.protocol_version,
            contract_bytes,
            alias: prepared.alias,
            status: RegistrationStatus::Submitted,
        };
        // Persist before the first possible send. A crash after this point never enables paid retry.
        crate::model::token_creator::contract::validate_submission_time(
            &prepared.contract,
            chrono::Utc::now().timestamp_millis().max(0) as u64,
        )
        .map_err(|source| TaskError::InvalidTokenDraft {
            source: Box::new(source),
        })?;
        if sdk.version().protocol_version != prepared.protocol_version {
            return Err(TaskError::TokenCreatorChanged);
        }
        // The SDK serializes before dispatch. Do this deterministic check before the durable claim.
        let encoded =
            transition
                .serialize_to_bytes()
                .map_err(|source| TaskError::InvalidTokenDraft {
                    source: Box::new(crate::model::token_creator::DraftError::protocol(
                        crate::model::token_creator::Section::Review,
                        "Contract size",
                        source,
                    )),
                })?;
        crate::model::token_creator::contract::validate_creation_size(encoded.len(), version)
            .map_err(|source| TaskError::InvalidTokenDraft {
                source: Box::new(source),
            })?;
        journal.claim(&record)?;
        if let Err(error) = transition.broadcast(sdk, settings()).await {
            return self.creator_broadcast_error(&journal, record, error);
        }
        match transition
            .wait_for_response::<DataContract>(sdk, settings())
            .await
        {
            Ok(contract) => self.creator_accept(&journal, record, contract),
            Err(error) => self.creator_unknown(&journal, record, error),
        }
    }

    fn creator_broadcast_error(
        &self,
        journal: &TokenRegistrationJournal,
        mut record: RegistrationRecord,
        source: dash_sdk::Error,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // With retries=0 the bare variant occurs before transport initialization. The
        // NoAvailableAddressesToRetry wrapper can follow a send and is deliberately excluded.
        if matches!(
            source,
            dash_sdk::Error::DapiClientError(
                dash_sdk::dapi_client::DapiClientError::NoAvailableAddresses
            )
        ) {
            record.status = RegistrationStatus::NotSubmitted;
            journal.update(&record)?;
            return Err(TaskError::TokenCreatorNotSent {
                source: Box::new(source),
            });
        }
        self.creator_unknown(journal, record, source)
    }

    fn creator_unknown(
        &self,
        journal: &TokenRegistrationJournal,
        mut record: RegistrationRecord,
        source: dash_sdk::Error,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        record.status = RegistrationStatus::Unknown;
        if let Err(error) = journal.update(&record) {
            tracing::warn!(
                ?error,
                "Could not update unresolved token registration; submitted marker remains"
            );
        }
        Err(TaskError::TokenCreatorUnconfirmed {
            contract_id: record.contract_id,
            source: Box::new(source),
        })
    }

    fn creator_accept(
        &self,
        journal: &TokenRegistrationJournal,
        mut record: RegistrationRecord,
        contract: DataContract,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if !record.matches_contract(&contract).map_err(protocol)? {
            record.status = RegistrationStatus::Conflict;
            journal.update(&record)?;
            return Err(TaskError::TokenCreatorConflict {
                contract_id: record.contract_id,
            });
        }
        // Receipt identifiers come from the accepted contract, never the editable draft.
        record.contract_id = contract.id();
        record.token_id = contract.token_id(0).ok_or(TaskError::TokenCreatorChanged)?;
        record.status = RegistrationStatus::Confirmed;
        let journal_saved = journal
            .update(&record)
            .map_err(|error| {
                tracing::warn!(?error, "Token accepted but receipt persistence failed")
            })
            .is_ok();
        let local_cache_saved = self
            .insert_contract_if_not_exists(
                &contract,
                Some(&record.alias),
                crate::model::qualified_contract::InsertTokensToo::AllTokensShouldBeAdded,
            )
            .map_err(|error| {
                tracing::warn!(?error, "Token accepted but local cache persistence failed")
            })
            .is_ok();
        Ok(BackendTaskSuccessResult::TokenCreationOutcome(Box::new(
            RegistrationOutcome {
                record,
                local_cache_saved,
                journal_saved,
            },
        )))
    }

    pub(super) async fn check_token_creation(
        &self,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let journal = self.token_registration_journal()?;
        let mut record = journal.read()?.ok_or(TaskError::TokenCreatorChanged)?;
        if record.network != self.network {
            return Err(TaskError::TokenCreatorChanged);
        }
        if record.status == RegistrationStatus::Confirmed {
            // Re-saving a proven historical receipt must not depend on the contract's
            // later state (it may have been updated) or downgrade an accepted payment.
            let accepted = record.contract().map_err(protocol)?;
            return self.creator_accept(&journal, record, accepted);
        }
        if record.status == RegistrationStatus::NotSubmitted {
            return Err(TaskError::TokenCreatorChanged);
        }
        match await_network_request_with_timeout(
            NETWORK_REQUEST_TIMEOUT,
            DataContract::fetch(sdk, record.contract_id),
            |source| TaskError::TokenCreatorReadTimeout { source },
        )
        .await??
        {
            Some(contract) => self.creator_accept(&journal, record, contract),
            None => {
                record.status = RegistrationStatus::Unknown;
                journal.update(&record)?;
                Ok(BackendTaskSuccessResult::TokenCreationOutcome(Box::new(
                    RegistrationOutcome {
                        record,
                        local_cache_saved: false,
                        journal_saved: true,
                    },
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
    fn fixture(context: &AppContext) -> (RegistrationRecord, DataContract) {
        let version = PlatformVersion::latest();
        let owner = Identifier::from([19; 32]);
        let params = TokenDraft::example(false)
            .parse(owner, context.network, version, 0)
            .unwrap();
        let contract = canonical_contract(owner, 7, params, version).unwrap();
        let record = RegistrationRecord {
            attempt_id: [23; 32],
            network: context.network,
            owner,
            contract_id: contract.id(),
            token_id: contract.token_id(0).unwrap(),
            nonce: 7,
            key_id: 1,
            protocol_version: version.protocol_version,
            contract_bytes: contract
                .serialize_to_bytes_with_platform_version(version)
                .unwrap(),
            alias: "Fixture".into(),
            status: RegistrationStatus::Unknown,
        };
        (record, contract)
    }
    use dash_sdk::dpp::version::PlatformVersion;

    #[test]
    fn token_registration_transport_or_retry_exhaustion_remains_unknown() {
        for source in [
            dash_sdk::Error::Generic("fixture transport timeout".into()),
            dash_sdk::Error::NoAvailableAddressesToRetry(Box::new(
                dash_sdk::Error::DapiClientError(
                    dash_sdk::dapi_client::DapiClientError::NoAvailableAddresses,
                ),
            )),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let context = crate::context::test_support::test_app_context(dir.path());
            let (record, _) = fixture(&context);
            let journal = context.token_registration_journal().unwrap();
            journal.claim(&record).unwrap();
            assert!(
                context
                    .creator_broadcast_error(&journal, record, source)
                    .is_err()
            );
            assert_eq!(
                journal.read().unwrap().unwrap().status,
                RegistrationStatus::Unknown
            );
        }
    }

    #[tokio::test]
    async fn token_registration_confirmed_receipt_is_not_downgraded_by_a_later_absent_read() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let (mut record, _) = fixture(&context);
        record.status = RegistrationStatus::Confirmed;
        context
            .token_registration_journal()
            .unwrap()
            .claim(&record)
            .unwrap();
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .unwrap();
        sdk.mock()
            .expect_fetch::<DataContract, _>(record.contract_id, None)
            .await
            .unwrap();
        let result = context.check_token_creation(&sdk).await.unwrap();
        let BackendTaskSuccessResult::TokenCreationOutcome(outcome) = result else {
            panic!("receipt")
        };
        assert_eq!(outcome.record.status, RegistrationStatus::Confirmed);
        assert!(
            !context
                .token_registration_record()
                .unwrap()
                .unwrap()
                .blocks_registration()
        );
    }

    #[test]
    fn checkpoint_p4_definitely_unsent_failure_allows_corrected_review() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let (mut record, _) = fixture(&context);
        record.status = RegistrationStatus::Submitted;
        let journal = context.token_registration_journal().unwrap();
        journal.claim(&record).unwrap();
        let error = dash_sdk::Error::DapiClientError(
            dash_sdk::dapi_client::DapiClientError::NoAvailableAddresses,
        );
        assert!(
            context
                .creator_broadcast_error(&journal, record, error)
                .is_err()
        );
        assert!(!journal.read().unwrap().unwrap().blocks_registration());
    }

    #[tokio::test]
    async fn token_registration_status_query_never_releases_an_absent_unknown_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let (record, _) = fixture(&context);
        context
            .token_registration_journal()
            .unwrap()
            .claim(&record)
            .unwrap();
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .unwrap();
        sdk.mock()
            .expect_fetch::<DataContract, _>(record.contract_id, None)
            .await
            .unwrap();
        let result = context.check_token_creation(&sdk).await.unwrap();
        let BackendTaskSuccessResult::TokenCreationOutcome(outcome) = result else {
            panic!("expected unresolved outcome")
        };
        assert_eq!(outcome.record.status, RegistrationStatus::Unknown);
        assert!(
            context
                .token_registration_record()
                .unwrap()
                .unwrap()
                .blocks_registration()
        );
    }
    #[tokio::test]
    async fn token_registration_status_query_does_not_confirm_a_different_contract() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let (record, mut different) = fixture(&context);
        different.keywords_mut().unwrap().push("different".into());
        context
            .token_registration_journal()
            .unwrap()
            .claim(&record)
            .unwrap();
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .unwrap();
        sdk.mock()
            .expect_fetch(record.contract_id, Some(different))
            .await
            .unwrap();
        assert!(matches!(
            context.check_token_creation(&sdk).await,
            Err(TaskError::TokenCreatorConflict { .. })
        ));
        assert_eq!(
            context.token_registration_record().unwrap().unwrap().status,
            RegistrationStatus::Conflict
        );
    }
    #[tokio::test]
    async fn token_registration_remote_success_survives_unavailable_local_token_cache() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let (record, accepted) = fixture(&context);
        context
            .token_registration_journal()
            .unwrap()
            .claim(&record)
            .unwrap();
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .unwrap();
        sdk.mock()
            .expect_fetch(record.contract_id, Some(accepted.clone()))
            .await
            .unwrap();
        let result = context.check_token_creation(&sdk).await.unwrap();
        let BackendTaskSuccessResult::TokenCreationOutcome(outcome) = result else {
            panic!("expected confirmed receipt")
        };
        assert_eq!(outcome.record.status, RegistrationStatus::Confirmed);
        assert_eq!(outcome.record.contract_id, accepted.id());
        assert_eq!(outcome.record.token_id, accepted.token_id(0).unwrap());
        assert!(
            !outcome.local_cache_saved,
            "fixture deliberately has no ready wallet backend"
        );
        assert!(
            !context
                .token_registration_record()
                .unwrap()
                .unwrap()
                .blocks_registration()
        );
    }
    #[tokio::test]
    async fn token_registration_expired_review_schedule_returns_to_distribution_before_signing() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let owner = Identifier::from([19; 32]);
        let version = PlatformVersion::latest();
        let mut draft = TokenDraft::example(false);
        draft.scheduled_enabled = true;
        draft
            .scheduled
            .push(crate::model::token_creator::ScheduleDraft {
                id: 0,
                timestamp: Some(1000),
                recipient: crate::model::token_creator::Recipient::Owner,
                amount: crate::model::token_creator::amount::TokenAmountInput::tokens("1"),
            });
        let contract = canonical_contract(
            owner,
            7,
            draft.parse(owner, context.network, version, 0).unwrap(),
            version,
        )
        .unwrap();
        let prepared = PreparedRegistration {
            contract,
            nonce: 7,
            key: IdentityPublicKey::default_versioned(version).unwrap(),
            network: context.network,
            protocol_version: version.protocol_version,
            alias: "Fixture".into(),
            registration_cost: 0,
            owner_balance: 0,
        };
        let sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(version)
            .build()
            .unwrap();
        let result = context.submit_token_creation(&sdk, prepared).await;
        assert!(
            matches!(result,Err(TaskError::InvalidTokenDraft {source}) if source.section==crate::model::token_creator::Section::Distribution)
        );
        assert!(context.token_registration_record().unwrap().is_none());
    }

    #[tokio::test]
    async fn token_registration_reserves_once_and_requires_new_review_after_nonce_drift() {
        use dash_sdk::query_types::IdentityNonceFetcher;
        let owner = Identifier::from([29; 32]);
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .unwrap();
        sdk.mock()
            .expect_fetch::<IdentityNonceFetcher, _>(owner, Some(IdentityNonceFetcher(10)))
            .await
            .unwrap();
        assert!(matches!(
            reserve_reviewed_nonce(&sdk, owner, 9).await,
            Err(TaskError::TokenCreatorChanged)
        ));
        assert_eq!(
            sdk.get_identity_nonce(owner, false, settings())
                .await
                .unwrap(),
            10
        );
        assert_eq!(reserve_reviewed_nonce(&sdk, owner, 11).await.unwrap(), 11);
        assert!(matches!(
            reserve_reviewed_nonce(&sdk, owner, 11).await,
            Err(TaskError::TokenCreatorChanged)
        ));
        assert_eq!(
            sdk.get_identity_nonce(owner, false, settings())
                .await
                .unwrap(),
            11
        );
    }
}
