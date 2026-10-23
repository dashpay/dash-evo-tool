use crate::model::identity_key_usability::SigningScope;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::{
    context::AppContext,
    model::{
        dpns::{
            DpnsNameValidationResult, DpnsRegistrationOutcome, classify_dpns_registration_outcome,
            normalize_dpns_label, validate_dpns_name,
        },
        fee_estimation::{contest_fee_credits, username_name_admission_reserve_credits},
        qualified_identity::{DPNSNameInfo, QualifiedIdentity},
    },
};
use dash_sdk::{
    Error as SdkError, Sdk,
    dpp::{
        data_contract::accessors::v0::DataContractV0Getters,
        identity::accessors::{IdentityGettersV0, IdentitySettersV0},
        version::PlatformVersion,
    },
    platform::Fetch,
    platform::dpns_usernames::{
        RegisterDpnsNameInput as SdkRegistrationInput, RegisterDpnsNameResult,
    },
    platform::{Document, Identifier, IdentityPublicKey},
};

use super::refresh_loaded_identities_dpns_names::name_acquired_at;
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

/// Whether the network refused the last broadcast attempt.
///
/// `broadcast` re-sends the same signed transition and reports only its last
/// attempt, so a refusal can follow an attempt that was applied: it is believed
/// only once the network does not show the request. Any other failure (a
/// timeout, a dropped connection, no reachable node) proves nothing either way.
fn broadcast_was_rejected(error: &SdkError) -> bool {
    match error {
        SdkError::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(_)) => true,
        SdkError::StateTransitionBroadcastError(broadcast_error) => broadcast_error.cause.is_some(),
        _ => false,
    }
}

/// Whether an approved community vote fee can be right for `outcome`
/// without asking the network: 0 for a name that opens no vote, at least the
/// contest's base fund for one that does. The fund to join only ever grows
/// past the base as requests pile up.
fn contest_fee_fits_outcome(
    approved_contest_fee: u64,
    outcome: DpnsRegistrationOutcome,
    platform_version: &PlatformVersion,
) -> bool {
    match outcome {
        DpnsRegistrationOutcome::PendingCommunityVote => {
            approved_contest_fee >= contest_fee_credits(platform_version)
        }
        DpnsRegistrationOutcome::Registered => approved_contest_fee == 0,
    }
}

/// A failed registration broadcast, by how far it got.
#[derive(Debug, thiserror::Error)]
enum BroadcastFailure {
    /// The first document was not confirmed, so the name request was never sent.
    #[error("the first registration document was not confirmed")]
    Preorder(#[source] Box<SdkError>),
    /// The first document was confirmed and paid for; the name request failed
    /// and may have been applied anyway.
    #[error("the name request failed after the first registration document was confirmed")]
    Domain(#[source] Box<SdkError>),
}

impl BroadcastFailure {
    /// Sort a failed broadcast by whether the first document had been confirmed.
    fn at_stage(preorder_confirmed: bool, error: SdkError) -> Self {
        if preorder_confirmed {
            Self::Domain(Box::new(error))
        } else {
            Self::Preorder(Box::new(error))
        }
    }
}

/// Broadcast the two registration documents for `label` through `register`,
/// the SDK's registration call, returning the confirmed name document.
///
/// The SDK reports one error for both documents; it calls back between them,
/// once the first is confirmed, which is what tells the two failures apart.
async fn broadcast_name_registration<F, Fut>(
    identity: &QualifiedIdentity,
    signing_key: IdentityPublicKey,
    label: &str,
    approved_contest_fee: u64,
    register: F,
) -> Result<Document, BroadcastFailure>
where
    F: FnOnce(SdkRegistrationInput<QualifiedIdentity>) -> Fut,
    Fut: Future<Output = Result<RegisterDpnsNameResult, SdkError>>,
{
    // Written once by the SDK's callback, read only after its call returns.
    let preorder_confirmed = Arc::new(AtomicBool::new(false));
    let confirmed = Arc::clone(&preorder_confirmed);
    let input = SdkRegistrationInput {
        contest_fund: Some(approved_contest_fee),
        label: label.to_owned(),
        identity: identity.identity.clone(),
        identity_public_key: signing_key,
        signer: identity.clone(),
        preorder_callback: Some(Box::new(move |_| confirmed.store(true, Ordering::Release))),
    };
    register(input)
        .await
        .map(|registered| registered.domain_document)
        .map_err(|error| {
            BroadcastFailure::at_stage(preorder_confirmed.load(Ordering::Acquire), error)
        })
}

impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        sdk: &Sdk,
        input: RegisterDpnsNameInput,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.register_dpns_name_through(sdk, input, |registration| {
            sdk.register_dpns_name(registration)
        })
        .await
    }

    /// [`Self::register_dpns_name`] with the SDK's registration call passed in
    /// as `register`, so everything around the broadcast runs without a network.
    async fn register_dpns_name_through<F, Fut>(
        &self,
        sdk: &Sdk,
        input: RegisterDpnsNameInput,
        register: F,
    ) -> Result<BackendTaskSuccessResult, TaskError>
    where
        F: FnOnce(SdkRegistrationInput<QualifiedIdentity>) -> Fut,
        Fut: Future<Output = Result<RegisterDpnsNameResult, SdkError>>,
    {
        let validation = validate_dpns_name(&input.name_input);
        if validation != DpnsNameValidationResult::Valid {
            return Err(TaskError::InvalidDpnsName { validation });
        }

        let dpns_contract = self.dpns_contract();

        let qualified_identity = input.qualified_identity;

        let preorder_document_type = dpns_contract
            .document_type_for_name("preorder")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let domain_document_type = dpns_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let outcome = classify_dpns_registration_outcome(
            &domain_document_type,
            &input.name_input,
            sdk.version(),
        )
        .map_err(|error| SdkError::Protocol(*error))?;

        // The approval is checked against the network contract's own contest
        // rule, so it can never cover a different charge than Platform applies.
        if !contest_fee_fits_outcome(input.approved_contest_fee, outcome, sdk.version()) {
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
        // The fund to join grows with the requests already in the vote; one
        // stating less is turned away only after its first document was paid
        // for. Re-quoted here, so the approval matches what joins now.
        if outcome == DpnsRegistrationOutcome::PendingCommunityVote
            && input.approved_contest_fee
                != self
                    .username_contest_fund_to_join(sdk, &input.name_input)
                    .await?
        {
            return Err(TaskError::UsernameRegistrationTermsChanged);
        }

        let public_key = match input.signing_key_id {
            Some(key_id) => qualified_identity
                .identity
                .get_public_key_by_id(key_id)
                .filter(|key| {
                    key_can_sign_documents(key, dpns_signing_requirement(&self.dpns_contract()))
                }),
            None => qualified_identity.document_signing_key(
                SigningScope::ContractWide {
                    contract_id: dpns_contract.id(),
                },
                &preorder_document_type,
            ),
        }
        .ok_or(TaskError::NoDocumentSigningKey)?;

        let estimated_fee = self.fee_estimator().estimate_username_registration(
            input.name_input.chars().count(),
            outcome == DpnsRegistrationOutcome::PendingCommunityVote,
        );
        let balance_before = qualified_identity.identity.balance();
        // Refused here, not by the network: it would take the first document's
        // fee and only then turn the second document down. The admission
        // reserve is held, not charged, but the name document is refused
        // without it.
        let admission_reserve =
            username_name_admission_reserve_credits(domain_document_type, sdk.version());
        let required = estimated_fee
            .saturating_add(input.approved_contest_fee)
            .saturating_add(admission_reserve);
        if balance_before < required {
            return Err(TaskError::UsernameRegistrationBalanceTooLow {
                available: balance_before,
                required,
            });
        }

        let confirmed_at = match broadcast_name_registration(
            &qualified_identity,
            public_key.clone(),
            &input.name_input,
            input.approved_contest_fee,
            register,
        )
        .await
        {
            Ok(domain_document) => name_acquired_at(&domain_document),
            // Not rebranded: a preorder conflict never means someone else holds the username.
            // Before DPNS v3 the preorder's unique index is its random `saltedDomainHash`; from
            // v3 it is `$ownerId, nameHash`, this identity's own earlier request. Either way the
            // generic `PlatformEntryConflict` message stays.
            Err(BroadcastFailure::Preorder(error)) => return Err(TaskError::from(*error)),
            Err(BroadcastFailure::Domain(error)) => {
                self.resolve_failed_domain_broadcast(
                    sdk,
                    qualified_identity.identity.id(),
                    &input.name_input,
                    outcome,
                    *error,
                )
                .await?;
                None
            }
        };

        Ok(self
            .finish_username_registration(
                sdk,
                qualified_identity,
                &input.name_input,
                outcome,
                joined_until,
                balance_before,
                estimated_fee,
                confirmed_at,
            )
            .await)
    }

    /// Decide what a failed name-request broadcast means, given it may be paid for.
    ///
    /// `Ok(())` when the network already shows the request, so the registration
    /// carries on as a success, even after a refusal. A refusal keeps its own
    /// error only when a successful read does not show the request. Anything
    /// else, a failed read included, is reported as unconfirmed, never as a
    /// plain failure that invites paying again.
    async fn resolve_failed_domain_broadcast(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        name: &str,
        outcome: DpnsRegistrationOutcome,
        error: SdkError,
    ) -> Result<(), TaskError> {
        match self
            .username_request_reached_network(sdk, identity_id, name, outcome)
            .await
        {
            Ok(true) => {
                tracing::warn!(
                    ?error,
                    %identity_id,
                    "Username registration reported an error, but the network shows the request; continuing as submitted"
                );
                return Ok(());
            }
            Ok(false) if broadcast_was_rejected(&error) => {
                return Err(rebrand_dpns_domain_conflict(TaskError::from(error)));
            }
            Ok(false) => {}
            Err(read_error) => tracing::warn!(
                ?read_error,
                %identity_id,
                "Username request could not be looked up after its broadcast failed; reporting the registration as unconfirmed"
            ),
        }
        Err(TaskError::UsernameRegistrationUnconfirmed {
            source_error: Box::new(error),
        })
    }

    /// Whether the network shows `identity_id`'s request for `name`.
    ///
    /// # Errors
    ///
    /// The read failed, which says nothing about the request either way.
    async fn username_request_reached_network(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        name: &str,
        outcome: DpnsRegistrationOutcome,
    ) -> Result<bool, TaskError> {
        match outcome {
            DpnsRegistrationOutcome::PendingCommunityVote => {
                let vote = sdk
                    .get_contested_dpns_vote_state(&normalize_dpns_label(name), None)
                    .await?;
                Ok(vote.contenders.contains_key(&identity_id))
            }
            DpnsRegistrationOutcome::Registered => {
                let names = self.fetch_owned_dpns_names(sdk, identity_id).await?;
                Ok(names.iter().any(|owned| owned.name == name))
            }
        }
    }

    /// Wrap up a registration whose documents were already broadcast and paid for.
    ///
    /// `confirmed_at` is when the network confirmed the name document; `None`
    /// when the registration was accepted as sent without one.
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
        identity: QualifiedIdentity,
        name: &str,
        outcome: DpnsRegistrationOutcome,
        joined_until: Option<u64>,
        balance_before: u64,
        estimated_fee: u64,
        confirmed_at: Option<u64>,
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
            if outcome == DpnsRegistrationOutcome::Registered
                && !fresh.dpns_names.iter().any(|known| known.name == name)
            {
                fresh.dpns_names.push(DPNSNameInfo {
                    name: name.to_owned(),
                    acquired_at: confirmed_at.unwrap_or_else(crate::utils::time::now_ms),
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
    use dash_sdk::dpp::document::{DocumentV0, DocumentV0Getters};
    use dash_sdk::dpp::voting::contender_structs::{
        ContenderWithSerializedDocument, ContenderWithSerializedDocumentV0,
    };
    use dash_sdk::dpp::voting::vote_info_storage::contested_document_vote_poll_winner_info::ContestedDocumentVotePollWinnerInfo;
    use dash_sdk::drive::query::vote_poll_vote_state_query::{
        ContestedDocumentVotePollDriveQuery, ContestedDocumentVotePollDriveQueryResultType,
    };
    use dash_sdk::query_types::{Contenders, Documents};
    use std::collections::BTreeMap;

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
        crate::context::test_support::bare_user_identity(
            [byte; 32].into(),
            dash_sdk::dpp::dashcore::Network::Testnet,
        )
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
            None,
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

    /// The registered name and its date come from the confirmed registration
    /// itself, next to the names the identity already had, with no re-read.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registered_name_and_date_come_from_the_confirmed_registration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let mut identity = bare_identity(5);
        identity.dpns_names.push(DPNSNameInfo {
            name: "earlier-name-2".to_owned(),
            acquired_at: 7,
        });
        let id = identity.identity.id();
        ctx.update_local_qualified_identity(&identity).unwrap();
        // No expectations: nothing may depend on reading the names back.
        let sdk = Sdk::new_mock();

        ctx.finish_username_registration(
            &sdk,
            identity,
            "alice-123",
            DpnsRegistrationOutcome::Registered,
            None,
            1_000,
            100,
            Some(4_242),
        )
        .await;

        let saved = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
        let names: Vec<(&str, u64)> = saved
            .dpns_names
            .iter()
            .map(|n| (n.name.as_str(), n.acquired_at))
            .collect();
        assert_eq!(names, [("earlier-name-2", 7), ("alice-123", 4_242)]);
    }

    /// An identity with a key allowed to sign name documents. Nothing signs
    /// with it while the SDK's registration call is stood in for.
    fn identity_with_signing_key(byte: u8) -> QualifiedIdentity {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};

        let mut identity = bare_identity(byte);
        identity
            .identity
            .add_public_keys([IdentityPublicKey::V0(IdentityPublicKeyV0 {
                id: 1,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::HIGH,
                contract_bounds: None,
                key_type: KeyType::ECDSA_SECP256K1,
                read_only: false,
                data: vec![2; 33].into(),
                disabled_at: None,
            })]);
        identity
    }

    /// An identity that cannot cover both documents, plus the community vote
    /// fee when the name needs one, is refused before anything is broadcast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_is_refused_before_broadcast_when_the_balance_cannot_cover_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        // A network whose DPNS contract picks the first preorder of a name.
        let platform_version = PlatformVersion::latest();
        let mock_sdk = || {
            dash_sdk::SdkBuilder::new_mock()
                .with_version(platform_version)
                .build()
                .expect("mock sdk")
        };
        ctx.sdk.store(Arc::new(mock_sdk()));
        let vote_fee = crate::model::fee_estimation::contest_fee_credits(platform_version);
        let domain_contract = ctx.dpns_contract();
        let admission_reserve =
            crate::model::fee_estimation::username_name_admission_reserve_credits(
                domain_contract
                    .document_type_for_name("domain")
                    .expect("DPNS domain document type"),
                platform_version,
            );
        assert!(
            admission_reserve > 0,
            "the DPNS contract under test must pick the first preorder of a name"
        );
        // Short by the reserve: the balance covers both fees but not what Platform
        // holds back before admitting the name document. Short by one credit: just
        // under everything required.
        for (label, approved_contest_fee, short_by) in [
            ("alice-123", 0, admission_reserve),
            ("alice", vote_fee, admission_reserve),
            ("alice-123", 0, 1),
            ("alice", vote_fee, 1),
        ] {
            let needs_vote = approved_contest_fee > 0;
            let required = ctx
                .fee_estimator()
                .estimate_username_registration(label.len(), needs_vote)
                + approved_contest_fee
                + admission_reserve;
            let mut identity = identity_with_signing_key(3);
            identity.identity.set_balance(required - short_by);
            let mut sdk = mock_sdk();
            expect_domain_lookup(&mut sdk, &ctx, label, false).await;
            if needs_vote {
                expect_vote_state(&mut sdk, &ctx, label, Contenders::default()).await;
                expect_contender_count(&mut sdk, &ctx, label, 0).await;
            }
            let broadcast = AtomicBool::new(false);

            let result = ctx
                .register_dpns_name_through(
                    &sdk,
                    RegisterDpnsNameInput {
                        qualified_identity: identity,
                        name_input: label.to_owned(),
                        approved_contest_fee,
                        signing_key_id: Some(1),
                    },
                    |_| {
                        broadcast.store(true, Ordering::Release);
                        async { Err::<RegisterDpnsNameResult, _>(unproven_broadcast_failure()) }
                    },
                )
                .await;

            assert!(
                matches!(
                    result,
                    Err(TaskError::UsernameRegistrationBalanceTooLow { available, required: needed })
                        if available == required - short_by && needed == required
                ),
                "{label}: {result:?}"
            );
            assert!(
                !broadcast.load(Ordering::Acquire),
                "{label}: nothing may be broadcast"
            );
        }
    }

    /// The date saved for a newly registered name is the one on the name
    /// document the network confirmed, carried there by the registration itself.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_saves_the_date_of_the_confirmed_name_document() {
        use dash_sdk::dpp::document::DocumentV0Setters;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let mut identity = identity_with_signing_key(3);
        // Exactly what the registration needs: the smallest balance that is accepted.
        identity.identity.set_balance(
            ctx.fee_estimator()
                .estimate_username_registration("alice-123".len(), false),
        );
        let id = identity.identity.id();
        ctx.update_local_qualified_identity(&identity).unwrap();
        let mut sdk = Sdk::new_mock();
        expect_domain_lookup(&mut sdk, &ctx, "alice-123", false).await;
        let mut confirmed = domain_document(id, "alice-123");
        confirmed.set_created_at(Some(4_242));

        let result = ctx
            .register_dpns_name_through(
                &sdk,
                RegisterDpnsNameInput {
                    qualified_identity: identity,
                    name_input: "alice-123".to_owned(),
                    approved_contest_fee: 0,
                    signing_key_id: Some(1),
                },
                |_| async move {
                    Ok(RegisterDpnsNameResult {
                        preorder_document: confirmed.clone(),
                        domain_document: confirmed,
                        full_domain_name: "alice-123.dash".to_owned(),
                    })
                },
            )
            .await
            .expect("the registration succeeds");

        assert!(matches!(
            result,
            BackendTaskSuccessResult::RegisteredDpnsName {
                outcome: DpnsRegistrationOutcome::Registered,
                ..
            }
        ));
        let saved = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
        let names: Vec<(&str, u64)> = saved
            .dpns_names
            .iter()
            .map(|n| (n.name.as_str(), n.acquired_at))
            .collect();
        assert_eq!(names, [("alice-123", 4_242)]);
    }

    /// A registration accepted as sent after an error has no confirmed
    /// document, so the name is listed with the local time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registered_name_without_a_confirmed_document_gets_the_local_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let identity = bare_identity(6);
        let id = identity.identity.id();
        ctx.update_local_qualified_identity(&identity).unwrap();
        let before = crate::utils::time::now_ms();

        ctx.finish_username_registration(
            &Sdk::new_mock(),
            identity,
            "alice-123",
            DpnsRegistrationOutcome::Registered,
            None,
            1_000,
            100,
            None,
        )
        .await;

        let saved = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
        assert_eq!(saved.dpns_names.len(), 1);
        assert_eq!(saved.dpns_names[0].name, "alice-123");
        assert!(saved.dpns_names[0].acquired_at >= before);
    }

    /// A broadcast that fails before the first document is confirmed has paid
    /// for nothing; one that fails after it has, and must never read as a
    /// plain failure.
    #[test]
    fn failed_broadcast_is_sorted_by_whether_the_first_document_was_confirmed() {
        assert!(matches!(
            BroadcastFailure::at_stage(false, unproven_broadcast_failure()),
            BroadcastFailure::Preorder(_)
        ));
        assert!(matches!(
            BroadcastFailure::at_stage(true, unproven_broadcast_failure()),
            BroadcastFailure::Domain(_)
        ));
    }

    /// The SDK calls back once the first document is confirmed. A failure it
    /// reports after that call is the paid-for stage, which holds only while
    /// the callback handed to the SDK sets the flag read once it returns.
    #[tokio::test]
    async fn failure_after_the_sdk_confirms_the_first_document_is_the_paid_stage() {
        let identity = bare_identity(4);
        let key = dash_sdk::platform::IdentityPublicKey::random_key(
            1,
            Some(4),
            PlatformVersion::latest(),
        );
        let confirmed_preorder = domain_document(identity.identity.id(), "alice-123");

        let failure =
            broadcast_name_registration(&identity, key, "alice-123", 0, |input| async move {
                // As the SDK does: report the confirmed first document if asked to.
                if let Some(report_confirmed) = input.preorder_callback {
                    report_confirmed(&confirmed_preorder);
                }
                Err(unproven_broadcast_failure())
            })
            .await
            .expect_err("the name request failed");

        assert!(
            matches!(failure, BroadcastFailure::Domain(_)),
            "got {failure:?}"
        );
    }

    #[tokio::test]
    async fn registration_never_auto_quotes_above_the_approved_contest_fund() {
        for approved in [0, 50_000_000_000] {
            let identity = bare_identity(4);
            let key = dash_sdk::platform::IdentityPublicKey::random_key(
                1,
                Some(4),
                PlatformVersion::latest(),
            );
            let _ = broadcast_name_registration(
                &identity,
                key,
                "alice",
                approved,
                |input| async move {
                    assert_eq!(
                        input.contest_fund,
                        Some(approved),
                        "The SDK must receive the exact approved cap, including zero."
                    );
                    Err(unproven_broadcast_failure())
                },
            )
            .await;
        }
    }

    /// With no node to confirm the first document, the broadcast stops there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broadcast_that_cannot_confirm_the_first_document_reports_that_stage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let sdk = crate::context::test_support::mock_sdk_for(&ctx);
        let identity = bare_identity(4);
        let key = dash_sdk::platform::IdentityPublicKey::random_key(
            1,
            Some(4),
            PlatformVersion::latest(),
        );

        let failure = broadcast_name_registration(&identity, key, "alice-123", 0, |input| {
            sdk.register_dpns_name(input)
        })
        .await
        .expect_err("nothing can be broadcast on a mock");

        assert!(
            matches!(failure, BroadcastFailure::Preorder(_)),
            "got {failure:?}"
        );
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
                None,
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
                None,
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
        for (label, approved_contest_fee) in [("alice", 0), ("alice", fee - 1), ("alice-123", fee)]
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

    /// The fee the user must have approved follows the contest rule's verdict:
    /// a name that opens a vote always requires at least the base fund, one
    /// that opens none requires nothing.
    #[test]
    fn contest_fee_fits_outcome_follows_the_contest_rule() {
        let version = PlatformVersion::latest();
        let fee = contest_fee_credits(version);
        assert!(fee > 0);
        let vote = DpnsRegistrationOutcome::PendingCommunityVote;
        let plain = DpnsRegistrationOutcome::Registered;
        assert!(contest_fee_fits_outcome(fee, vote, version));
        assert!(contest_fee_fits_outcome(fee * 2, vote, version));
        assert!(!contest_fee_fits_outcome(fee - 1, vote, version));
        assert!(!contest_fee_fits_outcome(0, vote, version));
        assert!(contest_fee_fits_outcome(0, plain, version));
        assert!(!contest_fee_fits_outcome(fee, plain, version));
    }

    /// Once the vote holds 250 requests the fund to join doubles. A request
    /// approved at the base fund would be turned away after its first
    /// document was paid for, so it is refused before anything is sent; one
    /// approved at the doubled fund passes the fee check and is weighed
    /// against the balance with it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn registration_requires_the_fund_to_join_a_crowded_vote() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let platform_version = PlatformVersion::latest();
        let mock_sdk = || {
            dash_sdk::SdkBuilder::new_mock()
                .with_version(platform_version)
                .build()
                .expect("mock sdk")
        };
        ctx.sdk.store(Arc::new(mock_sdk()));
        let base = contest_fee_credits(platform_version);
        let doubled = base * 2;
        for (approved_contest_fee, expect_terms_changed) in [(base, true), (doubled, false)] {
            let mut identity = identity_with_signing_key(5);
            identity.identity.set_balance(0);
            let mut sdk = mock_sdk();
            expect_domain_lookup(&mut sdk, &ctx, "alice", false).await;
            // The availability page reads no running vote; the count, read
            // page by page, finds 251 requests.
            expect_vote_state(&mut sdk, &ctx, "alice", Contenders::default()).await;
            expect_contender_count(&mut sdk, &ctx, "alice", 251).await;
            let broadcast = AtomicBool::new(false);

            let result = ctx
                .register_dpns_name_through(
                    &sdk,
                    RegisterDpnsNameInput {
                        qualified_identity: identity,
                        name_input: "alice".to_owned(),
                        approved_contest_fee,
                        signing_key_id: Some(1),
                    },
                    |_| {
                        broadcast.store(true, Ordering::Release);
                        async { Err::<RegisterDpnsNameResult, _>(unproven_broadcast_failure()) }
                    },
                )
                .await;

            if expect_terms_changed {
                assert!(
                    matches!(result, Err(TaskError::UsernameRegistrationTermsChanged)),
                    "base fund: {result:?}"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(TaskError::UsernameRegistrationBalanceTooLow { required, .. })
                            if required > doubled
                    ),
                    "doubled fund: {result:?}"
                );
            }
            assert!(
                !broadcast.load(Ordering::Acquire),
                "nothing may be broadcast"
            );
        }
    }

    /// A registered-name document as the network returns it for `label`.
    fn domain_document(owner: Identifier, label: &str) -> Document {
        Document::V0(DocumentV0 {
            moderated_at: None,
            moderated_by: None,
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
                        &ctx.dpns_contract(),
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

    /// The availability check quotes the fee the review must show: the
    /// doubled fund for a vote already holding 251 requests, nothing for a
    /// name that opens no vote.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn availability_check_quotes_the_fund_to_join() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let platform_version = PlatformVersion::latest();
        let base = contest_fee_credits(platform_version);
        for (label, contenders, expected) in [("alice", 251, base * 2), ("alice-123", 0, 0)] {
            let mut sdk = dash_sdk::SdkBuilder::new_mock()
                .with_version(platform_version)
                .build()
                .expect("mock sdk");
            expect_domain_lookup(&mut sdk, &ctx, label, false).await;
            if expected > 0 {
                expect_vote_state(&mut sdk, &ctx, label, Contenders::default()).await;
                expect_contender_count(&mut sdk, &ctx, label, contenders).await;
            }
            let result = ctx
                .check_username_availability(&sdk, Identifier::from([9; 32]), label.to_owned())
                .await;
            assert!(
                matches!(
                    result,
                    Ok(BackendTaskSuccessResult::UsernameAvailability { contest_fund, .. })
                        if contest_fund == expected
                ),
                "{label}: {result:?}"
            );
        }
    }

    /// Mock the paged count of the requests in the vote on `label`: pages of
    /// up to 100, each starting after the last request of the one before.
    async fn expect_contender_count(sdk: &mut Sdk, ctx: &AppContext, label: &str, count: u16) {
        use dash_sdk::drive::config::DEFAULT_QUERY_LIMIT;
        let vote_poll = crate::model::dpns_voting::dpns_vote_poll(
            &ctx.dpns_contract(),
            &normalize_dpns_label(label),
        )
        .expect("vote poll");
        let requester = |index: u16| {
            let mut id = [0x40; 32];
            id[..2].copy_from_slice(&index.to_be_bytes());
            Identifier::from(id)
        };
        let mut start_at = None;
        let mut first = 0;
        loop {
            let end = count.min(first + DEFAULT_QUERY_LIMIT);
            let page: Vec<Identifier> = (first..end).map(requester).collect();
            sdk.mock()
                .expect_fetch_many::<Identifier, ContenderWithSerializedDocument, _, Contenders>(
                    ContestedDocumentVotePollDriveQuery {
                        vote_poll: vote_poll.clone(),
                        result_type: ContestedDocumentVotePollDriveQueryResultType::VoteTally,
                        offset: None,
                        limit: Some(DEFAULT_QUERY_LIMIT),
                        start_at,
                        allow_include_locked_and_abstaining_vote_tally: false,
                    },
                    Some(running_vote(&page)),
                )
                .await
                .expect("contender count expectation");
            if page.len() < usize::from(DEFAULT_QUERY_LIMIT) {
                break;
            }
            start_at = page.last().map(|last| (last.to_buffer(), false));
            first = end;
        }
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

    fn refused_broadcast() -> SdkError {
        crate::test_support::duplicate_unique_index_broadcast_error(vec![
            "normalizedParentDomainName",
            "normalizedLabel",
        ])
    }

    /// A refusal the network does not contradict keeps its own error and is not
    /// turned into an "unconfirmed" warning.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refused_name_request_keeps_its_own_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let id = Identifier::from([8; 32]);

        // The vote is readable and shows only someone else's request.
        let mut sdk = Sdk::new_mock();
        expect_vote_state(
            &mut sdk,
            &ctx,
            "alice",
            running_vote(&[Identifier::from([0x77; 32])]),
        )
        .await;

        let result = ctx
            .resolve_failed_domain_broadcast(
                &sdk,
                id,
                "alice",
                DpnsRegistrationOutcome::PendingCommunityVote,
                refused_broadcast(),
            )
            .await;

        assert!(
            matches!(result, Err(TaskError::DpnsUsernameAlreadyTaken { .. })),
            "{result:?}"
        );
    }

    /// The broadcast is re-sent and only its last attempt is reported, so a
    /// refusal can follow an attempt that was applied and paid for. When the
    /// network shows the request, the registration carries on as submitted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refused_name_request_continues_when_the_request_is_on_the_network() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let id = Identifier::from([8; 32]);

        let mut sdk = Sdk::new_mock();
        expect_vote_state(&mut sdk, &ctx, "alice", running_vote(&[id])).await;

        let result = ctx
            .resolve_failed_domain_broadcast(
                &sdk,
                id,
                "alice",
                DpnsRegistrationOutcome::PendingCommunityVote,
                refused_broadcast(),
            )
            .await;

        assert!(result.is_ok(), "{result:?}");
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

    /// A refusal is only the last attempt's answer. When the read that would
    /// show whether an earlier attempt was applied fails, the request may be
    /// paid for, so the refusal is not reported as a plain failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refused_name_request_is_unconfirmed_when_the_network_cannot_be_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let id = Identifier::from([8; 32]);

        for (name, outcome) in [
            ("alice", DpnsRegistrationOutcome::PendingCommunityVote),
            ("alice-123", DpnsRegistrationOutcome::Registered),
        ] {
            // No expectations: the reconciling read itself fails.
            let unreachable = Sdk::new_mock();
            let result = ctx
                .resolve_failed_domain_broadcast(
                    &unreachable,
                    id,
                    name,
                    outcome,
                    refused_broadcast(),
                )
                .await;
            let error = match result {
                Err(error @ TaskError::UsernameRegistrationUnconfirmed { .. }) => error,
                other => panic!("{name}: expected an unconfirmed registration, got {other:?}"),
            };
            assert!(
                error.to_string().contains("Do not pay again yet."),
                "{error}"
            );
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
            None,
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
