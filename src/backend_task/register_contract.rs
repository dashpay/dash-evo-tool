use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dash_sdk::dash_platform_queries::transition::validation::ensure_valid_state_transition_structure;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::state::state_error::StateError;
use dash_sdk::dpp::identity::PartialIdentity;
use dash_sdk::dpp::identity::identity_nonce::MergeIdentityNonceResult;
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
    dpp::data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters},
    dpp::data_contract::accessors::v1::DataContractV1Setters,
    dpp::serialization::PlatformSerializableWithPlatformVersion,
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

/// An identity's contract registration that was sent but not settled. A
/// retry reuses its nonce, so every attempt carries the same contract ID and
/// Platform executes at most one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingContractRegistration {
    pub(crate) identity_nonce: IdentityNonce,
    pub(crate) contract_id: Identifier,
}

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
        earlier_version: bool,
    ) -> BackendTaskSuccessResult {
        match self {
            Self::Contract => BackendTaskSuccessResult::RegisteredContract {
                fee_result,
                save_warning,
                earlier_version,
            },
            Self::Token => BackendTaskSuccessResult::RegisteredTokenContract {
                save_warning,
                earlier_version,
            },
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
        let alias = (!alias.is_empty()).then_some(alias);
        ensure_private_document_entries_fit(&data_contract, None)?;
        let owner_id = data_contract.owner_id();
        self.register_signed_contract(
            data_contract,
            || async move { sdk.get_identity_nonce(owner_id, true, None).await },
            signing_key,
            &identity,
            sdk.version(),
            alias.as_deref(),
            kind,
            |transition| async move {
                transition
                    .broadcast_and_wait::<DataContract>(sdk, None)
                    .await
            },
            |contract_id| self.refetch_contract_after_unread_result(sdk, contract_id, &sender),
        )
        .await
    }

    /// [`Self::register_data_contract`] with the network steps passed in:
    /// `fresh_nonce` reads and bumps the owner's identity nonce, `submit` sends
    /// the signed transition and waits for its result, and `find` looks a
    /// registration whose result could not be read up under its
    /// Platform-assigned ID. While the owner has a registration pending, its
    /// nonce is reused instead of a fresh one, so at most one attempt executes.
    #[expect(
        clippy::too_many_arguments,
        reason = "the transition's inputs plus the two network steps"
    )]
    async fn register_signed_contract<S, Nonce, Submitted, Found>(
        &self,
        data_contract: DataContract,
        fresh_nonce: impl FnOnce() -> Nonce,
        signing_key: IdentityPublicKey,
        signer: &S,
        platform_version: &PlatformVersion,
        alias: Option<&str>,
        kind: RegistrationKind,
        submit: impl FnOnce(StateTransition) -> Submitted,
        find: impl FnOnce(Identifier) -> Found,
    ) -> Result<BackendTaskSuccessResult, TaskError>
    where
        S: Signer<IdentityPublicKey>,
        Nonce: Future<Output = Result<IdentityNonce, SdkError>>,
        Submitted: Future<Output = Result<DataContract, SdkError>>,
        Found: Future<Output = Result<Option<DataContract>, TaskError>>,
    {
        let fee_result =
            FeeResult::estimated_only(self.fee_estimator().estimate_contract_create_base());
        let owner_id = data_contract.owner_id();
        let submitted = data_contract.clone();
        let identity_nonce = match self.pending_contract_registration(&owner_id) {
            Some(pending) => {
                tracing::info!(
                    %owner_id,
                    identity_nonce = pending.identity_nonce,
                    "Retrying a pending contract registration with its identity nonce"
                );
                pending.identity_nonce
            }
            None => fresh_nonce().await?,
        };
        let (transition, contract_id) = contract_create_transition(
            data_contract,
            identity_nonce,
            signing_key,
            signer,
            platform_version,
        )
        .await?;
        let pending = PendingContractRegistration {
            identity_nonce,
            contract_id,
        };
        self.pending_registrations().insert(owner_id, pending);

        match submit(transition).await {
            Ok(contract) => {
                self.settle_pending_contract_registration(owner_id, pending);
                Ok(self
                    .finish_confirmed_registration(&contract, alias, fee_result, kind, false)
                    .await)
            }
            Err(error)
                if may_have_registered(&error)
                    || nonce_refusal(&error) == Some(NonceRefusal::AlreadyUsed) =>
            {
                // Not found, the registration stays pending and unresolved,
                // never a failure inviting a second registration.
                let Some(registered) = find(contract_id).await? else {
                    return Err(kind.unconfirmed(contract_id, error));
                };
                self.settle_pending_contract_registration(owner_id, pending);
                let earlier_version =
                    !is_submitted_contract(&submitted, &registered, platform_version);
                Ok(self
                    .finish_confirmed_registration(
                        &registered,
                        alias,
                        fee_result,
                        kind,
                        earlier_version,
                    )
                    .await)
            }
            Err(error) => {
                if nonce_refusal(&error) == Some(NonceRefusal::Unusable) {
                    self.settle_pending_contract_registration(owner_id, pending);
                }
                Err(TaskError::from(error))
            }
        }
    }

    /// Saves a contract Platform confirmed and reports the registration as
    /// done. The fee is already paid, so a save that keeps failing becomes a
    /// warning on the success, never an error inviting a second registration.
    /// `earlier_version`: the contract is an earlier attempt's, not the one
    /// last submitted.
    async fn finish_confirmed_registration(
        &self,
        contract: &DataContract,
        alias: Option<&str>,
        fee_result: FeeResult,
        kind: RegistrationKind,
        earlier_version: bool,
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
        kind.registered(fee_result, save_warning, earlier_version)
    }

    /// `owner`'s registration sent but not settled, if any.
    pub(crate) fn pending_contract_registration(
        &self,
        owner: &Identifier,
    ) -> Option<PendingContractRegistration> {
        self.pending_registrations().get(owner).copied()
    }

    /// Forgets `owner`'s pending registration once it is settled, unless a
    /// newer attempt has replaced it.
    fn settle_pending_contract_registration(
        &self,
        owner: Identifier,
        settled: PendingContractRegistration,
    ) {
        let mut registrations = self.pending_registrations();
        if registrations.get(&owner) == Some(&settled) {
            registrations.remove(&owner);
        }
    }

    fn pending_registrations(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<Identifier, PendingContractRegistration>>
    {
        self.pending_contract_registrations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// Whether `registered`, as Platform holds it, is the contract in `submitted`.
/// The fields Platform sets (ID, owner, creation times) are left out; a pair
/// that cannot be compared counts as the same.
fn is_submitted_contract(
    submitted: &DataContract,
    registered: &DataContract,
    platform_version: &PlatformVersion,
) -> bool {
    let comparable = |contract: &DataContract| {
        let mut contract = contract.clone();
        contract.set_id(registered.id());
        contract.set_owner_id(registered.owner_id());
        contract.set_created_at(None);
        contract.set_updated_at(None);
        contract.set_created_at_block_height(None);
        contract.set_updated_at_block_height(None);
        contract.set_created_at_epoch(None);
        contract.set_updated_at_epoch(None);
        contract
            .serialize_to_bytes_with_platform_version(platform_version)
            .ok()
    };
    match (comparable(submitted), comparable(registered)) {
        (Some(submitted), Some(registered)) => submitted == registered,
        _ => true,
    }
}

/// Whether a failed registration may still have been executed: its result
/// could not be read or proved, so only a lookup can tell.
fn may_have_registered(error: &SdkError) -> bool {
    matches!(error, SdkError::DriveProofError(..)) || error.is_state_transition_result_unconfirmed()
}

/// Why Platform refused a transition's identity nonce, where it matters to a
/// pending registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonceRefusal {
    /// The nonce, or the contract ID it gives, is already used: an earlier
    /// attempt may have executed.
    AlreadyUsed,
    /// The nonce can no longer be accepted, so no attempt carrying it will
    /// ever execute.
    Unusable,
}

/// The nonce refusal behind `error`, read from its typed consensus cause.
fn nonce_refusal(error: &SdkError) -> Option<NonceRefusal> {
    match error.consensus_error()? {
        ConsensusError::StateError(StateError::InvalidIdentityNonceError(refusal)) => {
            Some(match refusal.error() {
                MergeIdentityNonceResult::NonceAlreadyPresentAtTip
                | MergeIdentityNonceResult::NonceAlreadyPresentInPast(_) => {
                    NonceRefusal::AlreadyUsed
                }
                _ => NonceRefusal::Unusable,
            })
        }
        ConsensusError::StateError(StateError::DataContractAlreadyPresentError(_)) => {
            Some(NonceRefusal::AlreadyUsed)
        }
        _ => None,
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
    use dash_sdk::dpp::consensus::state::identity::invalid_identity_contract_nonce_error::InvalidIdentityNonceError;
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

    /// What one registration attempt did, seen from its network steps.
    struct Attempt {
        outcome: Result<BackendTaskSuccessResult, TaskError>,
        /// The ID the lookup was asked for, if it ran.
        asked: Option<Identifier>,
        /// The identity nonce the sent transition carries.
        sent_nonce: Option<IdentityNonce>,
        /// Whether a fresh nonce was read.
        fresh_nonce_read: bool,
    }

    /// Drives [`AppContext::register_signed_contract`] for the DPNS contract
    /// as `kind`, offering `fresh_nonce` as the owner's next nonce and
    /// `submitted` as the network's answer; the lookup answers `found`.
    async fn attempt(
        ctx: &AppContext,
        kind: RegistrationKind,
        fresh_nonce: IdentityNonce,
        submitted: Result<DataContract, SdkError>,
        found: Option<DataContract>,
    ) -> Attempt {
        let contract = ctx.dpns_contract().as_ref().clone();
        let asked = std::cell::Cell::new(None);
        let sent_nonce = std::cell::Cell::new(None);
        let fresh_nonce_read = std::cell::Cell::new(false);
        let outcome = ctx
            .register_signed_contract(
                contract,
                || {
                    fresh_nonce_read.set(true);
                    std::future::ready(Ok(fresh_nonce))
                },
                signing_key(),
                &FixedSigner,
                ctx.platform_version(),
                None,
                kind,
                |transition| {
                    if let StateTransition::DataContractCreate(create) = &transition {
                        sent_nonce.set(Some(create.identity_nonce()));
                    }
                    std::future::ready(submitted)
                },
                |contract_id| {
                    asked.set(Some(contract_id));
                    std::future::ready(Ok(found))
                },
            )
            .await;
        Attempt {
            outcome,
            asked: asked.get(),
            sent_nonce: sent_nonce.get(),
            fresh_nonce_read: fresh_nonce_read.get(),
        }
    }

    /// [`attempt`] at a fresh nonce of 5, as most tests need.
    async fn register_with(
        ctx: &AppContext,
        submitted: Result<DataContract, SdkError>,
        found: Option<DataContract>,
    ) -> (
        Result<BackendTaskSuccessResult, TaskError>,
        Option<Identifier>,
    ) {
        let done = attempt(ctx, RegistrationKind::Contract, 5, submitted, found).await;
        (done.outcome, done.asked)
    }

    fn unread() -> SdkError {
        SdkError::StateTransitionResultUnconfirmed(Box::new(proof_error()))
    }

    /// Platform's refusal of a transition whose identity nonce is unusable
    /// for the reason `merge`.
    fn nonce_refused(merge: MergeIdentityNonceResult) -> SdkError {
        SdkError::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(Box::new(
            ConsensusError::StateError(StateError::InvalidIdentityNonceError(
                InvalidIdentityNonceError::new(Identifier::from([0x21; 32]), Some(5), 5, merge),
            )),
        )))
    }

    fn assigned_id(ctx: &AppContext, nonce: IdentityNonce) -> Identifier {
        DataContract::generate_data_contract_id_v0(ctx.dpns_contract().owner_id(), nonce)
    }

    /// A retry after an unresolved registration reuses its nonce instead of
    /// reading a fresh one, so both attempts carry one contract ID.
    #[tokio::test]
    async fn a_retry_after_an_unresolved_registration_reuses_its_nonce() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();

        let first = attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;
        assert!(first.fresh_nonce_read);
        assert_eq!(first.sent_nonce, Some(5));
        assert_eq!(
            ctx.pending_contract_registration(&owner),
            Some(PendingContractRegistration {
                identity_nonce: 5,
                contract_id: assigned_id(&ctx, 5),
            })
        );

        let retry = attempt(&ctx, RegistrationKind::Contract, 6, Err(unread()), None).await;
        assert!(!retry.fresh_nonce_read);
        assert_eq!(retry.sent_nonce, Some(5));
        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
    }

    /// A retry refused because its nonce is already used looks the contract up
    /// under the shared ID: found, the registration is confirmed and no longer
    /// pending.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_retry_refused_for_its_used_nonce_finds_the_earlier_registration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let owner = ctx.dpns_contract().owner_id();

        for used in [
            MergeIdentityNonceResult::NonceAlreadyPresentAtTip,
            MergeIdentityNonceResult::NonceAlreadyPresentInPast(2),
        ] {
            attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;
            let earlier = ctx.dpns_contract().as_ref().clone();
            let retry = attempt(
                &ctx,
                RegistrationKind::Contract,
                6,
                Err(nonce_refused(used)),
                Some(earlier),
            )
            .await;

            assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
            assert!(
                matches!(
                    retry.outcome,
                    Ok(BackendTaskSuccessResult::RegisteredContract { .. })
                ),
                "{:?}",
                retry.outcome
            );
            assert_eq!(ctx.pending_contract_registration(&owner), None);
        }
    }

    /// A contract already present under the shared ID is the earlier attempt's.
    #[tokio::test]
    async fn a_retry_refused_for_an_existing_contract_looks_it_up() {
        use dash_sdk::dpp::consensus::state::data_contract::data_contract_already_present_error::DataContractAlreadyPresentError;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;
        let present = SdkError::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(Box::new(
            ConsensusError::StateError(StateError::DataContractAlreadyPresentError(
                DataContractAlreadyPresentError::new(assigned_id(&ctx, 5)),
            )),
        )));

        let retry = attempt(&ctx, RegistrationKind::Contract, 6, Err(present), None).await;

        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
    }

    /// A used nonce whose contract is not found leaves the registration
    /// unresolved and still pending: a later retry must not pay again.
    #[tokio::test]
    async fn a_used_nonce_without_its_contract_stays_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let retry = attempt(
            &ctx,
            RegistrationKind::Contract,
            6,
            Err(nonce_refused(
                MergeIdentityNonceResult::NonceAlreadyPresentAtTip,
            )),
            None,
        )
        .await;

        assert!(
            matches!(
                &retry.outcome,
                Err(TaskError::ContractRegistrationUnconfirmed { contract_id, .. })
                    if *contract_id == assigned_id(&ctx, 5)
            ),
            "{:?}",
            retry.outcome
        );
        assert_eq!(
            ctx.pending_contract_registration(&owner)
                .map(|p| p.identity_nonce),
            Some(5)
        );
    }

    /// A nonce Platform can no longer accept means the earlier attempt can
    /// never execute: it is no longer pending and the next attempt reads a
    /// fresh nonce.
    #[tokio::test]
    async fn an_unusable_nonce_is_no_longer_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let retry = attempt(
            &ctx,
            RegistrationKind::Contract,
            6,
            Err(nonce_refused(MergeIdentityNonceResult::NonceTooFarInPast)),
            None,
        )
        .await;

        assert_eq!(retry.asked, None);
        assert!(
            !matches!(
                retry.outcome,
                Ok(_) | Err(TaskError::ContractRegistrationUnconfirmed { .. })
            ),
            "{:?}",
            retry.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), None);
        let next = attempt(&ctx, RegistrationKind::Contract, 7, Err(unread()), None).await;
        assert_eq!(next.sent_nonce, Some(7));
    }

    /// A confirmed registration is no longer pending: the next one reads a
    /// fresh nonce.
    #[tokio::test]
    async fn a_confirmed_registration_is_no_longer_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let confirmed = ctx.dpns_contract().as_ref().clone();
        attempt(&ctx, RegistrationKind::Contract, 6, Ok(confirmed), None).await;
        assert_eq!(ctx.pending_contract_registration(&owner), None);

        let next = attempt(&ctx, RegistrationKind::Contract, 7, Err(unread()), None).await;
        assert!(next.fresh_nonce_read);
        assert_eq!(next.sent_nonce, Some(7));
    }

    /// A registration whose result could not be read is looked up under the
    /// ID Platform assigns, never the contract's placeholder, and found there
    /// it completes as registered.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unread_registration_is_looked_up_under_its_assigned_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let placeholder = ctx.dpns_contract().id();
        let assigned =
            DataContract::generate_data_contract_id_v0(ctx.dpns_contract().owner_id(), 5);
        let registered = ctx.dpns_contract().as_ref().clone();

        let (outcome, asked) = register_with(
            &ctx,
            Err(SdkError::StateTransitionResultUnconfirmed(Box::new(
                proof_error(),
            ))),
            Some(registered),
        )
        .await;

        assert_eq!(asked, Some(assigned));
        assert_ne!(asked, Some(placeholder));
        assert!(
            matches!(
                outcome,
                Ok(BackendTaskSuccessResult::RegisteredContract {
                    save_warning: None,
                    ..
                })
            ),
            "{outcome:?}"
        );
    }

    /// Not found under its assigned ID, an unread registration ends as not
    /// confirmed, naming that ID.
    #[tokio::test]
    async fn an_unread_registration_not_found_names_its_assigned_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let assigned =
            DataContract::generate_data_contract_id_v0(ctx.dpns_contract().owner_id(), 5);

        let (outcome, asked) = register_with(&ctx, Err(proof_error()), None).await;

        assert_eq!(asked, Some(assigned));
        assert!(
            matches!(
                &outcome,
                Err(TaskError::ContractRegistrationUnconfirmed { contract_id, .. })
                    if *contract_id == assigned
            ),
            "{outcome:?}"
        );
    }

    /// A refusal before execution and a confirmed result need no lookup.
    #[tokio::test]
    async fn only_an_unread_registration_is_looked_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());

        let (outcome, asked) =
            register_with(&ctx, Err(SdkError::Generic("refused".to_string())), None).await;
        assert_eq!(asked, None);
        assert!(
            !matches!(
                outcome,
                Ok(_) | Err(TaskError::ContractRegistrationUnconfirmed { .. })
            ),
            "{outcome:?}"
        );

        let confirmed = ctx.dpns_contract().as_ref().clone();
        let (outcome, asked) = register_with(&ctx, Ok(confirmed), None).await;
        assert_eq!(asked, None);
        assert!(
            matches!(
                outcome,
                Ok(BackendTaskSuccessResult::RegisteredContract { .. })
            ),
            "{outcome:?}"
        );
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
    /// confirmed, saved, and reported as done, for both kinds; found as it
    /// was submitted, it is not an earlier version.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_registration_found_after_an_unread_result_is_confirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let registered = ctx.dpns_contract().as_ref().clone();

        let contract = attempt(
            &ctx,
            RegistrationKind::Contract,
            5,
            Err(unread()),
            Some(registered.clone()),
        )
        .await;
        assert!(
            matches!(
                contract.outcome,
                Ok(BackendTaskSuccessResult::RegisteredContract {
                    save_warning: None,
                    earlier_version: false,
                    ..
                })
            ),
            "{:?}",
            contract.outcome
        );
        assert!(
            ctx.get_contract_by_id(&registered.id())
                .expect("read saved contract")
                .is_some()
        );

        let token = attempt(
            &ctx,
            RegistrationKind::Token,
            6,
            Err(proof_error()),
            Some(registered),
        )
        .await;
        assert!(
            matches!(
                token.outcome,
                Ok(BackendTaskSuccessResult::RegisteredTokenContract {
                    save_warning: None,
                    earlier_version: false,
                })
            ),
            "{:?}",
            token.outcome
        );
    }

    /// Found under the shared ID in a version an earlier attempt sent, the
    /// registration is done, and the result says it is that earlier version;
    /// the contract saved here is the one Platform holds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_registration_found_in_an_earlier_version_says_so() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let earlier = ctx.dashpay_contract().as_ref().clone();

        for kind in [RegistrationKind::Contract, RegistrationKind::Token] {
            let done = attempt(&ctx, kind, 5, Err(unread()), Some(earlier.clone())).await;
            assert!(
                matches!(
                    done.outcome,
                    Ok(BackendTaskSuccessResult::RegisteredContract {
                        earlier_version: true,
                        ..
                    }) | Ok(BackendTaskSuccessResult::RegisteredTokenContract {
                        earlier_version: true,
                        ..
                    })
                ),
                "{kind:?}: {:?}",
                done.outcome
            );
        }
        assert!(
            ctx.get_contract_by_id(&earlier.id())
                .expect("read saved contract")
                .is_some()
        );
    }

    /// A registration not found after an unread result stays unresolved: the
    /// error names the assigned ID to check, and says trying again is safe.
    #[tokio::test]
    async fn a_registration_not_found_after_an_unread_result_stays_unresolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let assigned = assigned_id(&ctx, 5);

        for kind in [RegistrationKind::Contract, RegistrationKind::Token] {
            let error = attempt(&ctx, kind, 5, Err(unread()), None)
                .await
                .outcome
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
            assert_eq!(named_id, assigned);
            let message = error.to_string();
            assert!(message.contains(&format!("{assigned}")), "{message}");
            assert!(message.contains("safe"), "{message}");
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
                false,
            )
            .await;
        let BackendTaskSuccessResult::RegisteredContract {
            fee_result,
            save_warning: Some(ContractSaveWarning::Contract { contract_id, .. }),
            ..
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
                false,
            )
            .await;
        let BackendTaskSuccessResult::RegisteredTokenContract {
            save_warning: Some(warning @ ContractSaveWarning::Token { .. }),
            ..
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
                false,
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
