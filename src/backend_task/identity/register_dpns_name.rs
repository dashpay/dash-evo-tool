use std::collections::BTreeMap;

use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::{
    context::AppContext,
    model::{
        dpns::{DpnsNameValidationResult, classify_dpns_registration_outcome, validate_dpns_name},
        qualified_identity::DPNSNameInfo,
    },
};
use bip39::rand::{Rng, SeedableRng, rngs::StdRng};
use dash_sdk::{
    Error as SdkError, Sdk,
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters, document_type::accessors::DocumentTypeV0Getters,
        },
        document::DocumentV0,
        identity::accessors::IdentityGettersV0,
        platform_value::Bytes32,
        util::{hash::hash_double, strings::convert_to_homograph_safe_chars},
    },
    platform::Fetch,
    platform::{Document, transition::put_document::PutDocument},
};

use super::{BackendTaskSuccessResult, RegisterDpnsNameInput};
use crate::model::dpns_usernames::{
    UsernameAvailability, dpns_signing_requirement, key_can_sign_documents,
};

fn rebrand_dpns_domain_conflict(error: TaskError) -> TaskError {
    match error {
        TaskError::PlatformEntryConflict { source_error } => {
            TaskError::DpnsUsernameAlreadyTaken { source_error }
        }
        other => other,
    }
}

impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        sdk: &Sdk,
        input: RegisterDpnsNameInput,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let validation = validate_dpns_name(&input.name_input);
        if validation != DpnsNameValidationResult::Valid {
            return Err(TaskError::InvalidDpnsName { validation });
        }

        // Authoritative re-check before any fee is spent: the name may have been
        // taken, locked, or closed to new requests since the user chose it.
        let availability = self
            .username_availability(
                sdk,
                input.qualified_identity.identity.id(),
                &input.name_input,
            )
            .await?;
        if !availability.allows_registration() {
            return Err(TaskError::UsernameNoLongerAvailable { availability });
        }
        let joined_until = match availability {
            UsernameAvailability::Joinable { join_end, .. } => Some(join_end),
            _ => None,
        };

        let mut rng = StdRng::from_entropy();
        let dpns_contract = self.dpns_contract.clone();

        let qualified_identity = input.qualified_identity;

        let entropy = Bytes32::random_with_rng(&mut rng);
        let preorder_document_type = dpns_contract
            .document_type_for_name("preorder")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let domain_document_type = dpns_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;

        let preorder_id = Document::generate_document_id_v0(
            &dpns_contract.id(),
            &qualified_identity.identity.id(),
            preorder_document_type.name().as_str(),
            entropy.as_slice(),
        );
        let domain_id = Document::generate_document_id_v0(
            &dpns_contract.id(),
            &qualified_identity.identity.id(),
            domain_document_type.name().as_str(),
            entropy.as_slice(),
        );

        let salt: [u8; 32] = rng.r#gen();
        let mut salted_domain_buffer: Vec<u8> = vec![];
        salted_domain_buffer.extend(salt);
        salted_domain_buffer
            .extend((convert_to_homograph_safe_chars(&input.name_input) + ".dash").as_bytes());
        let salted_domain_hash = hash_double(salted_domain_buffer);

        let preorder_document = Document::V0(DocumentV0 {
            id: preorder_id,
            owner_id: qualified_identity.identity.id(),
            creator_id: None,
            properties: BTreeMap::from([(
                "saltedDomainHash".to_string(),
                salted_domain_hash.into(),
            )]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        });
        let domain_document = Document::V0(DocumentV0 {
            id: domain_id,
            owner_id: qualified_identity.identity.id(),
            creator_id: None,
            properties: BTreeMap::from([
                ("parentDomainName".to_string(), "dash".into()),
                ("normalizedParentDomainName".to_string(), "dash".into()),
                ("label".to_string(), input.name_input.clone().into()),
                (
                    "normalizedLabel".to_string(),
                    convert_to_homograph_safe_chars(&input.name_input).into(),
                ),
                ("preorderSalt".to_string(), salt.into()),
                (
                    "records".to_string(),
                    BTreeMap::from([(
                        "identity".to_string(),
                        Into::<dash_sdk::dpp::platform_value::Value>::into(
                            qualified_identity.identity.id(),
                        ),
                    )])
                    .into(),
                ),
                (
                    "subdomainRules".to_string(),
                    BTreeMap::from([(
                        "allowSubdomains".to_string(),
                        Into::<dash_sdk::dpp::platform_value::Value>::into(false),
                    )])
                    .into(),
                ),
            ]),
            revision: None,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        });
        let outcome = classify_dpns_registration_outcome(
            &domain_document_type,
            &domain_document,
            sdk.version(),
        )
        .map_err(|error| SdkError::Protocol(*error))?;

        let public_key = match input.signing_key_id {
            Some(key_id) => qualified_identity
                .identity
                .get_public_key_by_id(key_id)
                .filter(|key| {
                    key_can_sign_documents(key, dpns_signing_requirement(&self.dpns_contract))
                }),
            None => qualified_identity.document_signing_key(&preorder_document_type),
        }
        .ok_or(TaskError::NoDocumentSigningKey)?;

        let fee_estimator = self.fee_estimator();
        let estimated_fee = fee_estimator.estimate_document_batch(2);

        let balance_before = qualified_identity.identity.balance();

        let _ = preorder_document
            .put_to_platform_and_wait_for_response(
                sdk,
                preorder_document_type.to_owned_document_type(),
                Some(entropy.0),
                public_key.clone(),
                None,
                &qualified_identity,
                None,
            )
            // Not rebranded: preorder's only unique index, `saltedDomainHash`, is unrelated to
            // usernames, so conflicts keep the generic `PlatformEntryConflict` message.
            .await?;

        let _ = domain_document
            .put_to_platform_and_wait_for_response(
                sdk,
                domain_document_type.to_owned_document_type(),
                Some(entropy.0),
                public_key.clone(),
                None,
                &qualified_identity,
                None,
            )
            .await
            .map_err(|error| rebrand_dpns_domain_conflict(TaskError::from(error)))?;

        Ok(self
            .finish_username_registration(
                sdk,
                qualified_identity,
                &input.name_input,
                outcome,
                joined_until,
                balance_before,
                estimated_fee,
            )
            .await)
    }

    /// Wrap up a registration whose documents were already broadcast and paid for.
    ///
    /// Infallible on purpose: the user has paid, so a failed re-read or local
    /// save is logged and the registration still reports success, never inviting
    /// a second payment.
    #[expect(
        clippy::too_many_arguments,
        reason = "the registration's already-computed facts, passed once"
    )]
    async fn finish_username_registration(
        &self,
        sdk: &Sdk,
        mut identity: crate::model::qualified_identity::QualifiedIdentity,
        name: &str,
        outcome: crate::model::dpns::DpnsRegistrationOutcome,
        joined_until: Option<u64>,
        balance_before: u64,
        estimated_fee: u64,
    ) -> BackendTaskSuccessResult {
        // The name request is broadcast and paid for: from here on nothing may
        // report failure, or the user would be invited to pay again.
        let identity_id = identity.identity.id();
        if outcome == crate::model::dpns::DpnsRegistrationOutcome::PendingCommunityVote
            && let Err(error) =
                self.record_submitted_username_request(sdk, &identity_id, name, joined_until)
        {
            tracing::warn!(
                ?error,
                "Submitted username request could not be stored locally"
            );
        }

        match self.fetch_owned_dpns_names(sdk, identity_id).await {
            Ok(names) => identity.dpns_names = names,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Registered names could not be re-read after registration"
                );
                if outcome == crate::model::dpns::DpnsRegistrationOutcome::Registered
                    && !identity.dpns_names.iter().any(|known| known.name == name)
                {
                    identity.dpns_names.push(DPNSNameInfo {
                        name: name.to_owned(),
                        acquired_at: crate::utils::time::now_ms(),
                    });
                }
            }
        }

        // Name an unnamed node after its main username, never a pending request.
        let main_username = self.main_username(&identity);
        identity.initialize_node_alias(None, main_username.as_deref());

        let fee_result = match dash_sdk::platform::Identity::fetch_by_identifier(sdk, identity_id)
            .await
        {
            Ok(Some(refreshed_identity)) => {
                let actual_fee = balance_before.saturating_sub(refreshed_identity.balance());
                identity.identity = refreshed_identity;
                if actual_fee != estimated_fee {
                    tracing::warn!(
                        estimated_fee,
                        actual_fee,
                        "Username registration fee differs from the estimate"
                    );
                }
                FeeResult::new(estimated_fee, actual_fee)
            }
            Ok(None) | Err(_) => {
                tracing::warn!("Identity balance could not be re-read after username registration");
                FeeResult::estimated_only(estimated_fee)
            }
        };

        if let Err(error) = self.update_local_qualified_identity(&identity) {
            tracing::warn!(
                ?error,
                "Identity could not be saved locally after username registration"
            );
        }

        BackendTaskSuccessResult::RegisteredDpnsName {
            outcome,
            fee_result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpns_registration_needs_a_high_or_critical_key() {
        use dash_sdk::dpp::data_contracts::SystemDataContract;
        use dash_sdk::dpp::identity::SecurityLevel;
        use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
        let contract = load_system_data_contract(
            SystemDataContract::DPNS,
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("DPNS");
        assert_eq!(dpns_signing_requirement(&contract), SecurityLevel::HIGH);
    }
    use dash_sdk::dpp::consensus::ConsensusError::StateError as ConsensusStateError;
    use dash_sdk::dpp::consensus::state::state_error::StateError;

    fn duplicate_unique_index_conflict(properties: Vec<&str>) -> TaskError {
        let source_error = Box::new(crate::test_support::duplicate_unique_index_broadcast_error(
            properties,
        ));

        TaskError::PlatformEntryConflict { source_error }
    }

    #[test]
    fn rebrand_dpns_domain_conflict_maps_platform_entry_conflict() {
        let error =
            duplicate_unique_index_conflict(vec!["normalizedParentDomainName", "normalizedLabel"]);

        let rebranded = rebrand_dpns_domain_conflict(error);

        assert_eq!(
            rebranded.to_string(),
            "This username is already taken. Please choose a different username and try again."
        );
        let TaskError::DpnsUsernameAlreadyTaken { source_error } = rebranded else {
            panic!("expected DpnsUsernameAlreadyTaken");
        };
        let dash_sdk::Error::StateTransitionBroadcastError(broadcast_error) = source_error.as_ref()
        else {
            panic!("expected StateTransitionBroadcastError source");
        };
        let Some(ConsensusStateError(StateError::DuplicateUniqueIndexError(error))) =
            broadcast_error.cause.as_ref()
        else {
            panic!("expected DuplicateUniqueIndexError cause");
        };
        assert_eq!(error.duplicating_properties().len(), 2);
    }

    #[test]
    fn rebrand_dpns_domain_conflict_passes_through_other_errors() {
        let error = rebrand_dpns_domain_conflict(TaskError::DataContractNotFound);

        assert!(matches!(error, TaskError::DataContractNotFound));
    }

    /// A user identity with no names, alias, or keys.
    fn bare_identity(byte: u8) -> crate::model::qualified_identity::QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        QualifiedIdentity {
            identity: dash_sdk::platform::Identity::create_basic_identity(
                [byte; 32].into(),
                dash_sdk::dpp::version::PlatformVersion::latest(),
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: Vec::new(),
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
    }

    /// Once the documents are broadcast and paid for, failed re-reads must not
    /// turn into an error that re-enables Pay; the request is recorded and no
    /// automatic alias hides the user's chosen main name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paid_registration_succeeds_when_rereads_fail() {
        use crate::model::dpns::DpnsRegistrationOutcome;
        use crate::model::dpns_usernames::RequestPhase;
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        // No expectations: every network re-read fails.
        let sdk = Sdk::new_mock();
        let identity = bare_identity(7);
        let id = identity.identity.id();

        let pending = ctx
            .finish_username_registration(
                &sdk,
                identity.clone(),
                "alice",
                DpnsRegistrationOutcome::PendingCommunityVote,
                None,
                1_000,
                200,
            )
            .await;
        let BackendTaskSuccessResult::RegisteredDpnsName {
            outcome,
            fee_result,
        } = pending
        else {
            panic!("expected a successful registration, got {pending:?}");
        };
        assert_eq!(outcome, DpnsRegistrationOutcome::PendingCommunityVote);
        assert_eq!(fee_result.estimated_fee, 200);
        let requests = ctx.username_requests_for(&id);
        assert_eq!(requests.len(), 1, "the paid request must be listed");
        assert_eq!(requests[0].phase, RequestPhase::Joinable);

        let registered = ctx
            .finish_username_registration(
                &sdk,
                identity,
                "bob",
                DpnsRegistrationOutcome::Registered,
                None,
                1_000,
                100,
            )
            .await;
        assert!(matches!(
            registered,
            BackendTaskSuccessResult::RegisteredDpnsName { .. }
        ));
    }

    /// USR-TC-018 backend half: availability is re-checked before anything is
    /// broadcast, so a failed or changed check spends nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_rechecks_availability_before_spending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        // No expectations: the first network call (the re-check) fails.
        let result = ctx
            .register_dpns_name(
                &Sdk::new_mock(),
                RegisterDpnsNameInput {
                    qualified_identity: bare_identity(8),
                    name_input: "alice".to_owned(),
                    signing_key_id: None,
                },
            )
            .await;
        assert!(
            matches!(
                result,
                Err(TaskError::UsernameAvailabilityCheckFailed { .. })
            ),
            "expected the re-check to stop the registration, got {result:?}"
        );
    }
}
