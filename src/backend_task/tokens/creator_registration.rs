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
use dash_sdk::dpp::dashcore::hashes::sha256;
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
fn protocol(source: impl Into<Box<ProtocolError>>) -> TaskError {
    TaskError::from(dash_sdk::Error::Protocol(*source.into()))
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
        // The transition is built here, not by `PutContract`, so run its preflight.
        dash_sdk::platform::transition::put_contract::ensure_private_document_entries_fit(
            &contract, None,
        )
        .map_err(|error| match error {
            dash_sdk::Error::Protocol(source) => TaskError::InvalidTokenDraft {
                source: Box::new(crate::model::token_creator::DraftError::private_documents(
                    source,
                )),
            },
            other => TaskError::from(other),
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
            .creator_cache_receipt(&contract, &record.alias)
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

    /// Saves the accepted contract and its token only where nothing is cached yet: a later
    /// configuration update may have replaced the creation-time settings.
    fn creator_cache_receipt(&self, accepted: &DataContract, alias: &str) -> Result<(), TaskError> {
        let cached = match self.get_contract_by_id(&accepted.id()) {
            Ok(cached) => cached.map(|cached| cached.contract),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Cached token contract is unreadable; filling missing tokens from the accepted contract"
                );
                None
            }
        };
        let contract = cached.as_ref().unwrap_or(accepted);
        let mut missing = Vec::new();
        for position in contract.tokens().keys().copied() {
            // An existing entry is kept even if this build cannot read it.
            if let Some(token_id) = contract.token_id(position)
                && !self.has_token_entry(&token_id)?
            {
                missing.push(position);
            }
        }
        // The contract entry itself is written only when absent.
        self.insert_contract_if_not_exists(
            contract,
            Some(alias),
            crate::model::qualified_contract::InsertTokensToo::SomeTokensShouldBeAdded(missing),
        )
    }

    /// Saves the receipt of a creation Platform confirmed, from the proven record alone: a
    /// network read could return a later version of the contract and look like a conflict.
    pub(super) fn save_confirmed_token_creation(
        &self,
        proven: RegistrationRecord,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let journal = self.token_registration_journal()?;
        let current = journal.read()?.ok_or(TaskError::TokenCreatorChanged)?;
        if proven.status != RegistrationStatus::Confirmed
            || proven.network != self.network
            || current.attempt_id != proven.attempt_id
            || current.contract_bytes != proven.contract_bytes
        {
            return Err(TaskError::TokenCreatorChanged);
        }
        let accepted = proven.contract().map_err(protocol)?;
        self.creator_accept(&journal, current, accepted)
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

    /// An owner Platform and this device both hold, with one enabled HIGH
    /// authentication key (id 1) changed by `edit` and its private key held.
    fn owner_holding(
        context: &AppContext,
        seed: u8,
        edit: impl FnOnce(&mut IdentityPublicKey),
    ) -> QualifiedIdentity {
        use crate::model::qualified_identity::PrivateKeyTarget;
        use crate::model::qualified_identity::encrypted_key_storage::{KeyStorage, PrivateKeyData};
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let mut owner = crate::context::test_support::user_identity_with_signing_key(
            Identifier::from([seed; 32]),
            context.network,
            false,
        );
        let mut key = owner.identity.get_public_key_by_id(1).unwrap().clone();
        edit(&mut key);
        owner.identity.add_public_key(key.clone());
        owner.private_keys = KeyStorage::from(std::collections::BTreeMap::from([(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 1),
            (key.into(), PrivateKeyData::InVault),
        )]));
        context
            .insert_local_qualified_identity(&owner, &None)
            .unwrap();
        owner
    }

    fn mock_sdk() -> Sdk {
        dash_sdk::SdkBuilder::new_mock()
            .with_version(PlatformVersion::latest())
            .build()
            .unwrap()
    }

    /// `read_only` only stops the key record from changing; Platform lets such
    /// a key sign. A disabled key still cannot.
    #[tokio::test(flavor = "multi_thread")]
    async fn token_creator_signs_with_a_read_only_key_but_never_a_disabled_one() {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let read_only = owner_holding(&context, 41, |key| key.set_read_only(true));
        let disabled = owner_holding(&context, 42, |key| key.set_disabled_at(1));
        let mut sdk = mock_sdk();
        for owner in [&read_only, &disabled] {
            sdk.mock()
                .expect_fetch::<Identity, _>(owner.identity.id(), Some(owner.identity.clone()))
                .await
                .unwrap();
        }
        let key =
            |owner: &QualifiedIdentity| owner.identity.get_public_key_by_id(1).unwrap().clone();
        assert!(
            context
                .creator_current_owner(&sdk, read_only.identity.id(), &key(&read_only))
                .await
                .is_ok()
        );
        assert!(matches!(
            context
                .creator_current_owner(&sdk, disabled.identity.id(), &key(&disabled))
                .await,
            Err(TaskError::TokenCreatorKeyUnavailable)
        ));
    }

    /// The SDK's PutContract preflight refuses private document types whose
    /// entries cannot hold their largest payload; the creator builds the
    /// transition itself, so its review must run the same check and name the
    /// type and the size it needs.
    #[tokio::test(flavor = "multi_thread")]
    async fn token_creator_refuses_private_documents_whose_entries_cannot_hold_them() {
        use crate::model::token_creator::{DraftIssue, Section};
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::query_types::IdentityNonceFetcher;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let owner = owner_holding(&context, 43, |_| {});
        let owner_id = owner.identity.id();
        let prepare = |entry_size: u32| {
            let context = context.clone();
            let owner = owner.clone();
            async move {
                let mut sdk = mock_sdk();
                sdk.mock()
                    .expect_fetch::<Identity, _>(owner_id, Some(owner.identity.clone()))
                    .await
                    .unwrap();
                sdk.mock()
                    .expect_fetch::<IdentityNonceFetcher, _>(
                        owner_id,
                        Some(IdentityNonceFetcher(6)),
                    )
                    .await
                    .unwrap();
                // The SDK's own example: the largest payload needs 376-byte entries.
                let mut draft = TokenDraft::example(false);
                draft.documents = r#"{"request":{"type":"object","private":{"entrySize":ENTRY},"documentsMutable":false,"canBeDeleted":false,"properties":{"reference":{"type":"integer","position":0},"key":{"type":"array","byteArray":true,"minItems":69,"maxItems":69,"position":1}},"required":["reference","key"],"additionalProperties":false}}"#
                    .replace("ENTRY", &entry_size.to_string());
                let key = owner.identity.get_public_key_by_id(1).unwrap().clone();
                context
                    .prepare_token_creation(&sdk, draft, owner_id, key)
                    .await
            }
        };
        let refused = prepare(375).await;
        assert!(
            matches!(&refused, Err(TaskError::InvalidTokenDraft { source })
                if source.section == Section::Documents
                    && matches!(&source.issue, DraftIssue::PrivateEntrySize {
                        document_type,
                        minimum: 376,
                        ..
                    } if document_type == "request")),
            "{refused:?}"
        );
        let accepted = prepare(376).await;
        assert!(
            matches!(
                accepted,
                Ok(BackendTaskSuccessResult::TokenCreationPrepared(_))
            ),
            "{accepted:?}"
        );
    }

    /// Restoring a receipt must not replace token settings a later update
    /// saved, but it still fills a missing token entry.
    #[tokio::test(flavor = "multi_thread")]
    async fn token_registration_restore_keeps_newer_token_settings() {
        use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Setters;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let (mut record, contract) = fixture(&context);
        record.status = RegistrationStatus::Confirmed;
        let journal = context.token_registration_journal().unwrap();
        journal.claim(&record).unwrap();
        context
            .insert_contract_if_not_exists(
                &contract,
                Some("Fixture"),
                crate::model::qualified_contract::InsertTokensToo::AllTokensShouldBeAdded,
            )
            .unwrap();
        let token_id = contract.token_id(0).unwrap();
        let mut updated = contract.expected_token_configuration(0).unwrap().clone();
        updated.set_max_supply(Some(123));
        context
            .insert_token(&token_id, "Fixture", updated.clone(), &contract.id(), 0)
            .unwrap();

        context
            .creator_accept(&journal, record.clone(), contract.clone())
            .unwrap();
        assert_eq!(
            context.get_token_config_for_id(&token_id).unwrap(),
            Some(updated),
            "the later update's settings stay"
        );

        // The cached contract is newer than the accepted one.
        use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Setters;
        let mut newer = contract.clone();
        let mut newer_config = contract.expected_token_configuration(0).unwrap().clone();
        newer_config.set_max_supply(Some(456));
        newer.add_token(0, newer_config.clone());
        context.replace_contract(contract.id(), &newer).unwrap();
        context.remove_token(&token_id).unwrap();
        context
            .creator_accept(&journal, record, contract.clone())
            .unwrap();
        assert_eq!(
            context.get_token_config_for_id(&token_id).unwrap(),
            Some(newer_config),
            "a missing entry is filled from the cached contract"
        );
    }

    /// A cache entry this build cannot read never blocks the restore: it is
    /// kept as it is, and missing entries are still filled.
    #[tokio::test(flavor = "multi_thread")]
    async fn token_registration_restore_fills_missing_entries_past_unreadable_ones() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let (mut record, contract) = fixture(&context);
        record.status = RegistrationStatus::Confirmed;
        let journal = context.token_registration_journal().unwrap();
        journal.claim(&record).unwrap();
        let token_id = contract.token_id(0).unwrap();

        // An unreadable payload, then an unreadable stored record.
        for envelope in [false, true] {
            context.remove_contract(&contract.id()).unwrap();
            context.put_unreadable_token_entry_for_test(&token_id, &contract.id(), envelope);
            let Ok(BackendTaskSuccessResult::TokenCreationOutcome(outcome)) =
                context.creator_accept(&journal, record.clone(), contract.clone())
            else {
                panic!("the restore is accepted ({envelope})");
            };
            assert!(outcome.local_cache_saved, "{envelope}");
            assert!(
                context.get_token_config_for_id(&token_id).is_err(),
                "the unreadable entry is left as it was ({envelope})"
            );

            context.remove_token(&token_id).unwrap();
            context.put_unreadable_contract_entry_for_test(&contract.id(), envelope);
            let Ok(BackendTaskSuccessResult::TokenCreationOutcome(outcome)) =
                context.creator_accept(&journal, record.clone(), contract.clone())
            else {
                panic!("the restore is accepted ({envelope})");
            };
            assert!(outcome.local_cache_saved, "{envelope}");
            assert_eq!(
                context.get_token_config_for_id(&token_id).unwrap(),
                Some(contract.expected_token_configuration(0).unwrap().clone()),
                "the missing token is filled from the accepted contract ({envelope})"
            );
        }
    }

    /// After Platform confirmed a creation whose receipt was not saved, the
    /// retry saves the proven record without reading the network, so a later
    /// configuration update can never turn it into a conflict.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_confirmed_receipt_is_saved_from_the_proven_record() {
        use dash_sdk::dpp::dashcore::Network;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let (mut record, _) = fixture(&context);
        let journal = context.token_registration_journal().unwrap();
        journal.claim(&record).unwrap();
        record.status = RegistrationStatus::Submitted;
        journal.update(&record).unwrap();
        drop(journal);
        let status = || {
            let journal = context.token_registration_journal().unwrap();
            journal.read().unwrap().unwrap().status
        };
        let mut proven = record.clone();
        proven.status = RegistrationStatus::Confirmed;

        let other_network = if context.network == Network::Testnet {
            Network::Devnet
        } else {
            Network::Testnet
        };
        type Change = fn(&mut RegistrationRecord, Network);
        let foreign: [(&str, Change); 4] = [
            ("another attempt", |r, _| r.attempt_id = [99; 32]),
            ("other contract bytes", |r, _| r.contract_bytes.push(0)),
            ("another network", |r, network| r.network = network),
            ("not confirmed", |r, _| {
                r.status = RegistrationStatus::Submitted
            }),
        ];
        for (case, change) in foreign {
            let mut other = proven.clone();
            change(&mut other, other_network);
            assert!(
                matches!(
                    context.save_confirmed_token_creation(other),
                    Err(TaskError::TokenCreatorChanged)
                ),
                "{case}"
            );
            assert_eq!(status(), RegistrationStatus::Submitted, "{case}");
        }

        let Ok(BackendTaskSuccessResult::TokenCreationOutcome(outcome)) =
            context.save_confirmed_token_creation(proven.clone())
        else {
            panic!("the proven receipt is saved");
        };
        assert!(outcome.journal_saved && outcome.local_cache_saved);
        assert_eq!(status(), RegistrationStatus::Confirmed);

        // A journal an earlier network check left at Conflict is repaired.
        {
            let journal = context.token_registration_journal().unwrap();
            let mut conflict = journal.read().unwrap().unwrap();
            conflict.status = RegistrationStatus::Conflict;
            journal.update(&conflict).unwrap();
        }
        assert!(context.save_confirmed_token_creation(proven).is_ok());
        assert_eq!(status(), RegistrationStatus::Confirmed);
    }
}
