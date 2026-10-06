use std::collections::BTreeMap;

use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::{
    context::AppContext,
    model::{
        dpns::{
            DpnsNameValidationResult, DpnsRegistrationOutcome, classify_dpns_registration_outcome,
            normalize_dpns_label, validate_dpns_name,
        },
        fee_estimation::contest_fee_credits,
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
        identity::accessors::{IdentityGettersV0, IdentitySettersV0},
        platform_value::Bytes32,
        util::{hash::hash_double, strings::convert_to_homograph_safe_chars},
        version::PlatformVersion,
    },
    platform::Fetch,
    platform::{Document, Identifier, transition::put_document::PutDocument},
};

use super::{BackendTaskSuccessResult, RegisterDpnsNameInput};
use crate::model::dpns_usernames::{dpns_signing_requirement, key_can_sign_documents};

fn rebrand_dpns_domain_conflict(error: TaskError) -> TaskError {
    match error {
        TaskError::PlatformEntryConflict { source_error } => {
            TaskError::DpnsUsernameAlreadyTaken { source_error }
        }
        other => other,
    }
}

/// Whether the network refused the transition, which proves it was not applied.
///
/// Any other failure (a timeout, a dropped connection, no reachable node) can
/// follow a broadcast that did arrive, so it proves nothing either way.
fn broadcast_was_rejected(error: &SdkError) -> bool {
    match error {
        SdkError::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(_)) => true,
        SdkError::StateTransitionBroadcastError(broadcast_error) => broadcast_error.cause.is_some(),
        _ => false,
    }
}

/// The community vote fee the user must have approved for `outcome`.
fn required_contest_fee(
    outcome: DpnsRegistrationOutcome,
    platform_version: &PlatformVersion,
) -> u64 {
    match outcome {
        DpnsRegistrationOutcome::PendingCommunityVote => contest_fee_credits(platform_version),
        DpnsRegistrationOutcome::Registered => 0,
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

        // The approval is checked against the document that is broadcast, so it
        // can never cover a different charge than the one Platform applies.
        if input.approved_contest_fee != required_contest_fee(outcome, sdk.version()) {
            return Err(TaskError::UsernameRegistrationTermsChanged);
        }

        // Authoritative re-check before any fee is spent: the name may have been
        // taken, locked, or closed to new requests since the user chose it.
        let availability = self
            .username_availability(sdk, qualified_identity.identity.id(), &input.name_input)
            .await?;
        if !availability.allows_registration() {
            return Err(TaskError::UsernameNoLongerAvailable { availability });
        }
        let joined_until = availability.join_deadline();

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

        if let Err(error) = domain_document
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
        {
            self.resolve_failed_domain_broadcast(
                sdk,
                qualified_identity.identity.id(),
                &input.name_input,
                outcome,
                error,
            )
            .await?;
        }

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

    /// Decide what a failed name-request broadcast means, given it may be paid for.
    ///
    /// `Ok(())` when the network already shows the request, so the registration
    /// carries on as a success. A refusal keeps its own error. Anything else is
    /// reported as unconfirmed, never as a plain failure that invites paying again.
    async fn resolve_failed_domain_broadcast(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        name: &str,
        outcome: DpnsRegistrationOutcome,
        error: SdkError,
    ) -> Result<(), TaskError> {
        if broadcast_was_rejected(&error) {
            return Err(rebrand_dpns_domain_conflict(TaskError::from(error)));
        }
        if self
            .username_request_reached_network(sdk, identity_id, name, outcome)
            .await
        {
            tracing::warn!(
                ?error,
                %identity_id,
                "Username registration reported an error, but the network shows the request; continuing as submitted"
            );
            return Ok(());
        }
        Err(TaskError::UsernameRegistrationUnconfirmed {
            source_error: Box::new(error),
        })
    }

    /// Whether the network shows `identity_id`'s request for `name`. A failed
    /// read counts as not shown.
    async fn username_request_reached_network(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        name: &str,
        outcome: DpnsRegistrationOutcome,
    ) -> bool {
        match outcome {
            DpnsRegistrationOutcome::PendingCommunityVote => sdk
                .get_contested_dpns_vote_state(&normalize_dpns_label(name), None)
                .await
                .is_ok_and(|vote| vote.contenders.contains_key(&identity_id)),
            DpnsRegistrationOutcome::Registered => self
                .fetch_owned_dpns_names(sdk, identity_id)
                .await
                .is_ok_and(|names| names.iter().any(|owned| owned.name == name)),
        }
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
        identity: crate::model::qualified_identity::QualifiedIdentity,
        name: &str,
        outcome: DpnsRegistrationOutcome,
        joined_until: Option<u64>,
        balance_before: u64,
        estimated_fee: u64,
    ) -> BackendTaskSuccessResult {
        // The name request is broadcast and paid for: from here on nothing may
        // report failure, or the user would be invited to pay again.
        let identity_id = identity.identity.id();
        if outcome == DpnsRegistrationOutcome::PendingCommunityVote
            && let Err(error) =
                self.record_submitted_username_request(sdk, &identity_id, name, joined_until)
        {
            tracing::warn!(
                ?error,
                "Submitted username request could not be stored locally"
            );
        }

        let refreshed_names = match self.fetch_owned_dpns_names(sdk, identity_id).await {
            Ok(names) => Some(names),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Registered names could not be re-read after registration"
                );
                None
            }
        };

        let mut refreshed_balance = None;
        let fee_result = match dash_sdk::platform::Identity::fetch_by_identifier(sdk, identity_id)
            .await
        {
            Ok(Some(refreshed_identity)) => {
                let actual_fee = balance_before.saturating_sub(refreshed_identity.balance());
                refreshed_balance = Some(refreshed_identity.balance());
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

        if let Err(error) = self.edit_local_qualified_identity(&identity_id, |fresh| {
            if let Some(names) = refreshed_names {
                fresh.dpns_names = names;
            }
            // A re-read served by a node that has not seen the new name yet
            // must not drop it: the registration is already paid for.
            if outcome == DpnsRegistrationOutcome::Registered
                && !fresh.dpns_names.iter().any(|known| known.name == name)
            {
                fresh.dpns_names.push(DPNSNameInfo {
                    name: name.to_owned(),
                    acquired_at: crate::utils::time::now_ms(),
                });
            }
            if let Some(balance) = refreshed_balance {
                fresh.identity.set_balance(balance);
            }
            let main_username = self.main_username(fresh);
            fresh.initialize_node_alias(None, main_username.as_deref());
            Ok(())
        }) {
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
    use crate::model::dpns_usernames::UsernameAvailability;
    use dash_sdk::dpp::block::block_info::BlockInfo;
    use dash_sdk::dpp::consensus::ConsensusError::StateError as ConsensusStateError;
    use dash_sdk::dpp::consensus::state::state_error::StateError;
    use dash_sdk::dpp::document::DocumentV0Getters;
    use dash_sdk::dpp::voting::contender_structs::{
        ContenderWithSerializedDocument, ContenderWithSerializedDocumentV0,
    };
    use dash_sdk::dpp::voting::vote_info_storage::contested_document_vote_poll_winner_info::ContestedDocumentVotePollWinnerInfo;
    use dash_sdk::drive::query::vote_poll_vote_state_query::{
        ContestedDocumentVotePollDriveQuery, ContestedDocumentVotePollDriveQueryResultType,
    };
    use dash_sdk::query_types::{Contenders, Documents};

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

    #[tokio::test]
    async fn review_regression_registration_preserves_concurrent_identity_edits() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let stale = bare_identity(8);
        let mut current = stale.clone();
        current.alias = Some("Updated during registration".into());
        let key = dash_sdk::platform::IdentityPublicKey::random_key(
            9,
            Some(8),
            dash_sdk::dpp::version::PlatformVersion::latest(),
        );
        current.private_keys.insert_at((crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity, 9), (
            crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey::from(key.clone()),
            crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData::InVault,
        ));
        ctx.update_local_qualified_identity(&current).unwrap();
        ctx.finish_username_registration(
            &Sdk::new_mock(),
            stale,
            "alice-123",
            DpnsRegistrationOutcome::Registered,
            None,
            1_000,
            100,
        )
        .await;
        let saved = ctx
            .get_local_qualified_identity(&current.identity.id())
            .unwrap()
            .unwrap();
        assert_eq!(saved.alias, current.alias);
        assert!(saved.private_keys.first_live_candidate(&key).is_some());
        assert!(saved.dpns_names.iter().any(|name| name.name == "alice-123"));
    }

    /// A re-read that succeeds but lags behind the registration must not drop
    /// the name the user just paid for from the identity.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registered_name_survives_a_lagging_reread() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let identity = bare_identity(5);
        let id = identity.identity.id();
        ctx.update_local_qualified_identity(&identity).unwrap();
        // The node answers, but only with a name registered earlier.
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.owned_dpns_names_query(id),
                Some(documents(Some(domain_document(id, "earlier-name-2")))),
            )
            .await
            .expect("owned names expectation");

        ctx.finish_username_registration(
            &sdk,
            identity,
            "alice-123",
            DpnsRegistrationOutcome::Registered,
            None,
            1_000,
            100,
        )
        .await;

        let saved = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
        let names: Vec<&str> = saved.dpns_names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["earlier-name-2", "alice-123"]);
    }

    /// Once the documents are broadcast and paid for, failed re-reads must not
    /// turn into an error that re-enables Pay; the request is recorded and no
    /// automatic alias hides the user's chosen main name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn paid_registration_succeeds_when_rereads_fail() {
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
        ctx.update_local_qualified_identity(&identity).unwrap();

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

    #[tokio::test]
    async fn registration_rejects_unapproved_vote_fee_before_network_or_payment() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let sdk = Sdk::new_mock();
        let fee = crate::model::fee_estimation::contest_fee_credits(sdk.version());
        for (label, approved_contest_fee) in [("alice", 0), ("alice", fee + 1), ("alice-123", fee)]
        {
            let result = ctx
                .register_dpns_name(
                    &sdk,
                    RegisterDpnsNameInput {
                        qualified_identity: bare_identity(9),
                        name_input: label.into(),
                        approved_contest_fee,
                        signing_key_id: None,
                    },
                )
                .await;
            assert!(
                matches!(result, Err(TaskError::UsernameRegistrationTermsChanged)),
                "{label}: {result:?}"
            );
        }
    }

    /// The fee the user must have approved follows the document's own
    /// classification: a document that opens a vote always requires the fee,
    /// whatever any other predicate said about its label.
    #[test]
    fn required_contest_fee_follows_the_broadcast_document() {
        let version = PlatformVersion::latest();
        let fee = contest_fee_credits(version);
        assert!(fee > 0);
        assert_eq!(
            required_contest_fee(DpnsRegistrationOutcome::PendingCommunityVote, version),
            fee
        );
        assert_eq!(
            required_contest_fee(DpnsRegistrationOutcome::Registered, version),
            0
        );
    }

    /// A registered-name document as the network returns it for `label`.
    fn domain_document(owner: Identifier, label: &str) -> Document {
        Document::V0(DocumentV0 {
            id: Identifier::from([0x5D; 32]),
            owner_id: owner,
            creator_id: None,
            properties: BTreeMap::from([("label".to_string(), label.into())]),
            revision: None,
            created_at: Some(100),
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        })
    }

    fn documents(found: Option<Document>) -> Documents {
        found
            .map(|document| (document.id(), Some(document)))
            .into_iter()
            .collect()
    }

    /// Mock the registered-name lookup for `label`.
    async fn expect_domain_lookup(sdk: &mut Sdk, ctx: &AppContext, label: &str, found: bool) {
        let found = found.then(|| domain_document(Identifier::from([0x5E; 32]), label));
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.dpns_domain_query(&normalize_dpns_label(label)),
                Some(documents(found)),
            )
            .await
            .expect("domain lookup expectation");
    }

    /// Mock the community-vote state for `label`.
    async fn expect_vote_state(sdk: &mut Sdk, ctx: &AppContext, label: &str, state: Contenders) {
        sdk.mock()
            .expect_fetch_many::<Identifier, ContenderWithSerializedDocument, _, Contenders>(
                ContestedDocumentVotePollDriveQuery {
                    vote_poll: crate::model::dpns_voting::dpns_vote_poll(
                        &ctx.dpns_contract,
                        &normalize_dpns_label(label),
                    )
                    .expect("vote poll"),
                    result_type:
                        ContestedDocumentVotePollDriveQueryResultType::DocumentsAndVoteTally,
                    allow_include_locked_and_abstaining_vote_tally: true,
                    start_at: None,
                    limit: None,
                    offset: None,
                },
                Some(state),
            )
            .await
            .expect("vote state expectation");
    }

    /// A running vote whose requests come from `requesters`.
    fn running_vote(requesters: &[Identifier]) -> Contenders {
        Contenders {
            contenders: requesters
                .iter()
                .map(|id| {
                    (
                        *id,
                        ContenderWithSerializedDocument::V0(ContenderWithSerializedDocumentV0 {
                            identity_id: *id,
                            serialized_document: None,
                            vote_tally: Some(0),
                        }),
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The pay-time re-check must stop a registration the network no longer
    /// accepts before the preorder or the name request is paid for. No broadcast
    /// is mocked, so reaching one would fail with a different error.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_stops_before_paying_when_the_name_is_no_longer_available() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let requester = bare_identity(8);
        let requester_id = requester.identity.id();
        let fee = contest_fee_credits(Sdk::new_mock().version());
        let locked = Contenders {
            winner: Some((
                ContestedDocumentVotePollWinnerInfo::Locked,
                BlockInfo::default(),
            )),
            ..Default::default()
        };
        let cases = [
            ("alice-123", 0, true, None, UsernameAvailability::Taken),
            (
                "alice",
                fee,
                false,
                Some(locked),
                UsernameAvailability::Locked,
            ),
            (
                "alice",
                fee,
                false,
                Some(running_vote(&[Identifier::from([0x77; 32])])),
                UsernameAvailability::JoinClosed,
            ),
            (
                "alice",
                fee,
                false,
                Some(running_vote(&[requester_id])),
                UsernameAvailability::AlreadyRequested,
            ),
        ];
        for (label, approved_contest_fee, registered, vote, expected) in cases {
            let mut sdk = Sdk::new_mock();
            expect_domain_lookup(&mut sdk, &ctx, label, registered).await;
            if let Some(vote) = vote {
                expect_vote_state(&mut sdk, &ctx, label, vote).await;
            }
            let result = ctx
                .register_dpns_name(
                    &sdk,
                    RegisterDpnsNameInput {
                        qualified_identity: requester.clone(),
                        name_input: label.to_owned(),
                        approved_contest_fee,
                        signing_key_id: None,
                    },
                )
                .await;
            assert!(
                matches!(
                    &result,
                    Err(TaskError::UsernameNoLongerAvailable { availability })
                        if *availability == expected
                ),
                "{label}: expected {expected:?}, got {result:?}"
            );
        }
    }

    fn unproven_broadcast_failure() -> SdkError {
        SdkError::TimeoutReached(std::time::Duration::from_secs(1), "no answer".to_owned())
    }

    /// A refusal by the network proves nothing was paid, so it keeps its own
    /// error and is not turned into an "unconfirmed" warning.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refused_name_request_keeps_its_own_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let refused = crate::test_support::duplicate_unique_index_broadcast_error(vec![
            "normalizedParentDomainName",
            "normalizedLabel",
        ]);

        let result = ctx
            .resolve_failed_domain_broadcast(
                &Sdk::new_mock(),
                Identifier::from([8; 32]),
                "alice",
                DpnsRegistrationOutcome::PendingCommunityVote,
                refused,
            )
            .await;

        assert!(
            matches!(result, Err(TaskError::DpnsUsernameAlreadyTaken { .. })),
            "{result:?}"
        );
    }

    /// A broadcast failure that proves nothing is reconciled first: when the
    /// network already shows the request, the registration carries on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unproven_broadcast_failure_continues_when_the_request_is_on_the_network() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let id = Identifier::from([8; 32]);

        let mut sdk = Sdk::new_mock();
        expect_vote_state(&mut sdk, &ctx, "alice", running_vote(&[id])).await;
        let contested = ctx
            .resolve_failed_domain_broadcast(
                &sdk,
                id,
                "alice",
                DpnsRegistrationOutcome::PendingCommunityVote,
                unproven_broadcast_failure(),
            )
            .await;
        assert!(contested.is_ok(), "{contested:?}");

        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.owned_dpns_names_query(id),
                Some(documents(Some(domain_document(id, "alice-123")))),
            )
            .await
            .expect("owned names expectation");
        let registered = ctx
            .resolve_failed_domain_broadcast(
                &sdk,
                id,
                "alice-123",
                DpnsRegistrationOutcome::Registered,
                unproven_broadcast_failure(),
            )
            .await;
        assert!(registered.is_ok(), "{registered:?}");
    }

    /// When nothing proves the outcome, the user is told the request may have
    /// gone through and must not be paid for again, never that it simply failed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unproven_broadcast_failure_is_reported_as_unconfirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let id = Identifier::from([8; 32]);

        // The vote is readable but does not show this identity's request yet.
        let mut lagging = Sdk::new_mock();
        expect_vote_state(
            &mut lagging,
            &ctx,
            "alice",
            running_vote(&[Identifier::from([0x77; 32])]),
        )
        .await;
        // No expectations: the reconciling read itself fails.
        let unreachable = Sdk::new_mock();

        for sdk in [lagging, unreachable] {
            let result = ctx
                .resolve_failed_domain_broadcast(
                    &sdk,
                    id,
                    "alice",
                    DpnsRegistrationOutcome::PendingCommunityVote,
                    unproven_broadcast_failure(),
                )
                .await;
            let error = match result {
                Err(error @ TaskError::UsernameRegistrationUnconfirmed { .. }) => error,
                other => panic!("expected an unconfirmed registration, got {other:?}"),
            };
            let message = error.to_string();
            assert!(message.contains("Do not pay again yet."), "{message}");
            assert!(!message.contains("failed"), "{message}");
        }
    }

    /// A request that joins a running vote is saved with that vote's deadlines,
    /// not with ones counted from the moment of payment.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn joined_request_is_saved_with_the_running_vote_deadlines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let sdk = Sdk::new_mock();
        let identity = bare_identity(6);
        let id = identity.identity.id();
        ctx.update_local_qualified_identity(&identity).unwrap();
        let durations = crate::model::dpns::contest_durations(ctx.network, sdk.version());
        let join = u64::try_from(durations.join.as_millis()).unwrap();
        let total = u64::try_from(durations.total.as_millis()).unwrap();
        // The vote opened ten minutes ago, so its join window closes that much sooner.
        let join_end = crate::utils::time::now_ms() + join - 600_000;

        ctx.finish_username_registration(
            &sdk,
            identity,
            "alice",
            DpnsRegistrationOutcome::PendingCommunityVote,
            Some(join_end),
            1_000,
            200,
        )
        .await;

        let requests = ctx.username_requests_for(&id);
        assert_eq!(requests.len(), 1, "the paid request must be listed");
        assert_eq!(requests[0].join_end, Some(join_end));
        assert_eq!(requests[0].end, Some(join_end - join + total));
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
                    approved_contest_fee: crate::model::fee_estimation::contest_fee_credits(
                        Sdk::new_mock().version(),
                    ),
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
