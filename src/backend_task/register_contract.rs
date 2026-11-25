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
    dpp::data_contract::INITIAL_DATA_CONTRACT_VERSION,
    dpp::data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters},
    dpp::data_contract::accessors::v1::DataContractV1Setters,
    dpp::serialization::{
        PlatformDeserializableWithPotentialValidationFromVersionedStructureUntrusted,
        PlatformSerializableWithPlatformVersion,
    },
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

/// A pending registration with every attempt under its nonce that may still
/// run, so a contract found under its ID can be matched to the request that
/// sent it, or told apart from one registered elsewhere.
#[derive(Debug)]
pub(crate) struct PendingContractAttempts {
    registration: PendingContractRegistration,
    sent: Vec<SentAttempt>,
}

/// One request's contract sent under a pending nonce.
#[derive(Debug, Clone)]
struct SentAttempt {
    contract: DataContract,
    kind: RegistrationKind,
    alias: Option<String>,
}

/// Whose registration a contract found under the assigned ID is.
enum FoundRegistration {
    /// The current request's, as sent now or, if `earlier_version`, as an
    /// earlier attempt of the same request sent it.
    ThisRequest { earlier_version: bool },
    /// An earlier attempt of another request: a different kind or name.
    EarlierRequest { alias: Option<String> },
    /// No attempt from this app: registered elsewhere with the same nonce.
    Elsewhere,
}

/// Marks an owner's registration as being sent while alive; dropping it, on
/// any outcome, cancellation included, lets the owner register again.
struct RegistrationBeingSent<'a> {
    context: &'a AppContext,
    owner: Identifier,
}

impl Drop for RegistrationBeingSent<'_> {
    fn drop(&mut self) {
        self.context
            .contract_registrations_in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.owner);
    }
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

    fn replaced_by_earlier(self, contract_id: Identifier) -> TaskError {
        match self {
            Self::Contract => TaskError::ContractRegistrationReplacedByEarlier { contract_id },
            Self::Token => TaskError::TokenRegistrationReplacedByEarlier { contract_id },
        }
    }

    fn preempted(self, contract_id: Identifier, source: SdkError) -> TaskError {
        let source_error = Box::new(source);
        match self {
            Self::Contract => TaskError::ContractRegistrationPreempted {
                contract_id,
                source_error,
            },
            Self::Token => TaskError::TokenRegistrationPreempted {
                contract_id,
                source_error,
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
        let _sending = self.begin_contract_registration(owner_id)?;
        let submitted = data_contract.clone();
        let reused = self.pending_contract_registration(&owner_id);
        let identity_nonce = match reused {
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
        // Reserved before anything is sent, so a cancelled send still counts.
        let attempt = SentAttempt {
            contract: submitted,
            kind,
            alias: alias.map(str::to_owned),
        };
        self.pending_registrations()
            .entry(owner_id)
            .or_insert_with(|| PendingContractAttempts {
                registration: pending,
                sent: Vec::new(),
            })
            .sent
            .push(attempt.clone());

        let error = match submit(transition).await {
            Ok(contract) => {
                self.settle_pending_contract_registration(owner_id, pending);
                return Ok(self
                    .finish_confirmed_registration(&contract, alias, fee_result, kind, false)
                    .await);
            }
            Err(error) => error,
        };
        let refusal = nonce_refusal(&error);
        // A refused or unsent attempt cannot run; any other may still.
        let this_may_run = !(error.consensus_error().is_some() || was_never_sent(&error));
        let earlier_may_run = self.forget_last_attempt_unless(owner_id, pending, this_may_run);
        let look_up = may_have_registered(&error) || earlier_may_run;
        if look_up {
            match find(contract_id).await {
                // Updated since it was created, so what it was created as is
                // unknown: it may be this registration, changed later.
                Ok(Some(registered)) if registered.version() != INITIAL_DATA_CONTRACT_VERSION => {
                    tracing::warn!(
                        %contract_id,
                        version = registered.version(),
                        "A contract updated since its creation was found under a registration's assigned ID"
                    );
                    return Err(kind.unconfirmed(contract_id, error));
                }
                Ok(Some(registered)) => {
                    let found =
                        self.whose_registration(owner_id, &registered, &attempt, platform_version);
                    self.settle_pending_contract_registration(owner_id, pending);
                    return match found {
                        FoundRegistration::ThisRequest { earlier_version } => Ok(self
                            .finish_confirmed_registration(
                                &registered,
                                alias,
                                fee_result,
                                kind,
                                earlier_version,
                            )
                            .await),
                        FoundRegistration::EarlierRequest { alias } => {
                            if let Err(source) = self
                                .save_registered_contract(&registered, alias.as_deref())
                                .await
                            {
                                tracing::warn!(
                                    ?source,
                                    %contract_id,
                                    "An earlier request's registered contract could not be saved locally"
                                );
                            }
                            Err(kind.replaced_by_earlier(contract_id))
                        }
                        // The identity registered it elsewhere with the same
                        // nonce, so this registration cannot run.
                        FoundRegistration::Elsewhere => {
                            tracing::warn!(
                                %contract_id,
                                "A contract this app did not send was found under a registration's assigned ID"
                            );
                            Err(kind.preempted(contract_id, error))
                        }
                    };
                }
                Ok(None) => {}
                // A failed lookup tells nothing, so the registration stays pending.
                Err(lookup_error) => {
                    tracing::warn!(
                        ?lookup_error,
                        %contract_id,
                        "Could not look up a contract registration with an unknown result"
                    );
                    return Err(kind.unconfirmed(contract_id, error));
                }
            }
        }
        // Not found: an attempt that may still run keeps the registration
        // pending and unresolved, never a failure inviting a second one.
        match refusal {
            // No attempt carrying this nonce has executed or ever will.
            Some(NonceRefusal::Unusable) => {
                self.settle_pending_contract_registration(owner_id, pending);
                Err(TaskError::RegistrationNonceRefused {
                    source_error: Box::new(error),
                })
            }
            _ if look_up => Err(kind.unconfirmed(contract_id, error)),
            // Refused, with no attempt left that may run: nothing is pending.
            Some(_) => Err(TaskError::RegistrationNonceRefused {
                source_error: Box::new(error),
            }),
            None => Err(TaskError::from(error)),
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
        let saved = self.save_registered_contract(contract, alias).await;
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
        self.pending_registrations()
            .get(owner)
            .map(|attempts| attempts.registration)
    }

    /// Saves a contract Platform confirmed under `alias`, with its tokens.
    async fn save_registered_contract(
        &self,
        contract: &DataContract,
        alias: Option<&str>,
    ) -> Result<(), TaskError> {
        retry_local_save(|| {
            self.insert_contract_if_not_exists(contract, alias, AllTokensShouldBeAdded)
        })
        .await
    }

    /// Unless `may_run`, forgets the attempt just sent under `owner`'s
    /// pending registration, and the registration itself when no attempt is
    /// left. Returns whether an earlier attempt may still run.
    fn forget_last_attempt_unless(
        &self,
        owner: Identifier,
        pending: PendingContractRegistration,
        may_run: bool,
    ) -> bool {
        let mut registrations = self.pending_registrations();
        let Some(attempts) = registrations
            .get_mut(&owner)
            .filter(|attempts| attempts.registration == pending)
        else {
            return false;
        };
        let earlier = attempts.sent.len().saturating_sub(1);
        if !may_run {
            attempts.sent.pop();
            if attempts.sent.is_empty() {
                registrations.remove(&owner);
            }
        }
        earlier > 0
    }

    /// Whose registration `registered`, found under `owner`'s assigned ID, is:
    /// the current request's (`current`), an earlier request's sent from
    /// here under the same nonce, or one registered elsewhere.
    fn whose_registration(
        &self,
        owner: Identifier,
        registered: &DataContract,
        current: &SentAttempt,
        platform_version: &PlatformVersion,
    ) -> FoundRegistration {
        if is_submitted_contract(&current.contract, registered, platform_version) {
            return FoundRegistration::ThisRequest {
                earlier_version: false,
            };
        }
        let registrations = self.pending_registrations();
        let matching: Vec<&SentAttempt> = registrations
            .get(&owner)
            .map(|attempts| attempts.sent.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|sent| is_submitted_contract(&sent.contract, registered, platform_version))
            .collect();
        if matching
            .iter()
            .any(|sent| sent.kind == current.kind && sent.alias == current.alias)
        {
            return FoundRegistration::ThisRequest {
                earlier_version: true,
            };
        }
        match matching.first() {
            Some(sent) => FoundRegistration::EarlierRequest {
                alias: sent.alias.clone(),
            },
            None => FoundRegistration::Elsewhere,
        }
    }

    /// Marks `owner`'s registration as being sent until the returned guard
    /// drops; refuses a second one for the same owner meanwhile, so its
    /// pending nonce is never read or replaced by a concurrent registration.
    fn begin_contract_registration(
        &self,
        owner: Identifier,
    ) -> Result<RegistrationBeingSent<'_>, TaskError> {
        let newly_sending = self
            .contract_registrations_in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(owner);
        if !newly_sending {
            return Err(TaskError::ContractRegistrationInProgress);
        }
        Ok(RegistrationBeingSent {
            context: self,
            owner,
        })
    }

    /// Forgets `owner`'s pending registration once it is settled, unless a
    /// newer attempt has replaced it.
    fn settle_pending_contract_registration(
        &self,
        owner: Identifier,
        settled: PendingContractRegistration,
    ) {
        let mut registrations = self.pending_registrations();
        if registrations
            .get(&owner)
            .map(|attempts| attempts.registration)
            == Some(settled)
        {
            registrations.remove(&owner);
        }
    }

    fn pending_registrations(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<Identifier, PendingContractAttempts>>
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
/// Both are compared as Platform reads them back (which, for one, lowercases
/// keywords), without the fields Platform sets (ID, owner, creation times); a
/// pair that cannot be compared counts as the same.
fn is_submitted_contract(
    submitted: &DataContract,
    registered: &DataContract,
    platform_version: &PlatformVersion,
) -> bool {
    let comparable = |contract: &DataContract| {
        let bytes = contract
            .serialize_to_bytes_with_platform_version(platform_version)
            .ok()?;
        let mut contract =
            DataContract::versioned_deserialize_untrusted(&bytes, false, platform_version).ok()?;
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

/// Whether `error` came before the transition was sent to any node.
fn was_never_sent(error: &SdkError) -> bool {
    matches!(
        error,
        SdkError::DapiClientError(dash_sdk::dapi_client::DapiClientError::NoAvailableAddresses)
    )
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
    /// The nonce is ahead of what the node has seen. The node may lag, so an
    /// earlier attempt carrying it may still execute.
    NotYet,
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
                MergeIdentityNonceResult::NonceTooFarInFuture => NonceRefusal::NotYet,
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
    use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
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

    /// `contract` as Platform creates it: at the initial version.
    fn as_created(contract: &DataContract) -> DataContract {
        let mut contract = contract.clone();
        contract.set_version(INITIAL_DATA_CONTRACT_VERSION);
        contract
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
        let contract = as_created(&ctx.dpns_contract());
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
        attempt_with_lookup(ctx, kind, fresh_nonce, submitted, Ok(found)).await
    }

    /// [`attempt`] whose lookup answers `lookup`, which may be a failure.
    async fn attempt_with_lookup(
        ctx: &AppContext,
        kind: RegistrationKind,
        fresh_nonce: IdentityNonce,
        submitted: Result<DataContract, SdkError>,
        lookup: Result<Option<DataContract>, TaskError>,
    ) -> Attempt {
        let contract = as_created(&ctx.dpns_contract());
        attempt_registering(ctx, contract, None, kind, fresh_nonce, submitted, lookup).await
    }

    /// [`attempt_with_lookup`] registering `contract`.
    async fn attempt_registering(
        ctx: &AppContext,
        contract: DataContract,
        alias: Option<&str>,
        kind: RegistrationKind,
        fresh_nonce: IdentityNonce,
        submitted: Result<DataContract, SdkError>,
        lookup: Result<Option<DataContract>, TaskError>,
    ) -> Attempt {
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
                alias,
                kind,
                |transition| {
                    if let StateTransition::DataContractCreate(create) = &transition {
                        sent_nonce.set(Some(create.identity_nonce()));
                    }
                    std::future::ready(submitted)
                },
                |contract_id| {
                    asked.set(Some(contract_id));
                    std::future::ready(lookup)
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
            let earlier = as_created(&ctx.dpns_contract());
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

    /// A retry refused because Platform can no longer accept its nonce looks
    /// the earlier attempt up first: not found, that attempt can never
    /// execute, so it is no longer pending, the refusal is reported as such,
    /// and the next attempt reads a fresh nonce.
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

        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
        assert!(
            matches!(
                retry.outcome,
                Err(TaskError::RegistrationNonceRefused { .. })
            ),
            "{:?}",
            retry.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), None);
        let next = attempt(&ctx, RegistrationKind::Contract, 7, Err(unread()), None).await;
        assert_eq!(next.sent_nonce, Some(7));
    }

    /// A lookup that fails tells nothing: the registration stays unconfirmed
    /// and pending, after an unread result and after an unusable nonce alike,
    /// so a retry still cannot create a second one.
    #[tokio::test]
    async fn a_failed_lookup_leaves_the_registration_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let still_pending = Some(PendingContractRegistration {
            identity_nonce: 5,
            contract_id: assigned_id(&ctx, 5),
        });

        let first = attempt_with_lookup(
            &ctx,
            RegistrationKind::Contract,
            5,
            Err(unread()),
            Err(TaskError::InternalSendError),
        )
        .await;
        assert_eq!(first.asked, Some(assigned_id(&ctx, 5)));
        assert!(
            matches!(
                first.outcome,
                Err(TaskError::ContractRegistrationUnconfirmed { .. })
            ),
            "{:?}",
            first.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), still_pending);

        let retry = attempt_with_lookup(
            &ctx,
            RegistrationKind::Token,
            6,
            Err(nonce_refused(MergeIdentityNonceResult::NonceTooFarInPast)),
            Err(TaskError::InternalSendError),
        )
        .await;
        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
        assert!(
            matches!(
                retry.outcome,
                Err(TaskError::TokenRegistrationUnconfirmed { .. })
            ),
            "{:?}",
            retry.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), still_pending);
    }

    /// A contract found under the assigned ID that was updated since it was
    /// created may be this registration, changed later (for example from
    /// another device); what it was created as is unknown. It stays
    /// unconfirmed under that ID, and pending, never reported as taken by
    /// another registration, on a first attempt and on a retry.
    #[tokio::test]
    async fn an_updated_contract_under_the_assigned_id_stays_unconfirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let mut updated = as_created(&ctx.dashpay_contract());
        updated.set_version(2);
        let still_pending = Some(PendingContractRegistration {
            identity_nonce: 5,
            contract_id: assigned_id(&ctx, 5),
        });

        let first = attempt(
            &ctx,
            RegistrationKind::Contract,
            5,
            Err(unread()),
            Some(updated.clone()),
        )
        .await;
        assert!(
            matches!(
                &first.outcome,
                Err(TaskError::ContractRegistrationUnconfirmed { contract_id, .. })
                    if *contract_id == assigned_id(&ctx, 5)
            ),
            "{:?}",
            first.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), still_pending);

        let retry = attempt(
            &ctx,
            RegistrationKind::Token,
            6,
            Err(nonce_refused(
                MergeIdentityNonceResult::NonceAlreadyPresentAtTip,
            )),
            Some(updated),
        )
        .await;
        assert_eq!(retry.sent_nonce, Some(5));
        assert!(
            matches!(
                &retry.outcome,
                Err(TaskError::TokenRegistrationUnconfirmed { contract_id, .. })
                    if *contract_id == assigned_id(&ctx, 5)
            ),
            "{:?}",
            retry.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), still_pending);
    }

    /// A retry after an unresolved registration that fails before anything
    /// is sent still leaves the earlier attempt unresolved: it is looked up
    /// and, not found, reported as unconfirmed with its ID, and stays
    /// pending.
    #[tokio::test]
    async fn a_retry_failing_before_it_is_sent_keeps_the_earlier_attempt_unconfirmed() {
        use dash_sdk::dapi_client::DapiClientError;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let retry = attempt(
            &ctx,
            RegistrationKind::Contract,
            6,
            Err(SdkError::DapiClientError(
                DapiClientError::NoAvailableAddresses,
            )),
            None,
        )
        .await;

        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
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

    /// A token request that reused the nonce of an earlier, unresolved plain
    /// contract registration and finds that plain contract under the shared
    /// ID completes the plain registration under its own name, not as the
    /// token: the token request is told so, nothing stays pending, and the
    /// next attempt reads a fresh nonce.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_token_request_finding_an_earlier_contract_completes_that_contract() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let owner = ctx.dpns_contract().owner_id();
        let plain = as_created(&ctx.dpns_contract());
        let mut token_contract = as_created(&ctx.dashpay_contract());
        token_contract.set_owner_id(owner);

        attempt_registering(
            &ctx,
            plain.clone(),
            Some("plain"),
            RegistrationKind::Contract,
            5,
            Err(unread()),
            Ok(None),
        )
        .await;
        let token = attempt_registering(
            &ctx,
            token_contract,
            Some("MyToken"),
            RegistrationKind::Token,
            6,
            Err(unread()),
            Ok(Some(plain.clone())),
        )
        .await;

        assert_eq!(token.sent_nonce, Some(5));
        assert!(
            matches!(
                &token.outcome,
                Err(TaskError::TokenRegistrationReplacedByEarlier { contract_id })
                    if *contract_id == assigned_id(&ctx, 5)
            ),
            "{:?}",
            token.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), None);
        let saved = ctx
            .get_contract_by_id(&plain.id())
            .expect("read saved contract")
            .expect("the plain contract is saved");
        assert_eq!(saved.alias.as_deref(), Some("plain"));

        let next = attempt(&ctx, RegistrationKind::Token, 7, Err(unread()), None).await;
        assert_eq!(next.sent_nonce, Some(7));
    }

    /// Platform refusing a reused nonce as ahead of the network says nothing
    /// final: the node may lag, and the earlier attempt may still run. It is
    /// looked up and, not found, stays unconfirmed and pending.
    #[tokio::test]
    async fn a_nonce_ahead_of_the_network_keeps_the_earlier_attempt_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let retry = attempt(
            &ctx,
            RegistrationKind::Contract,
            6,
            Err(nonce_refused(MergeIdentityNonceResult::NonceTooFarInFuture)),
            None,
        )
        .await;

        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
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

    /// A first attempt refused outright, or never sent, cannot run, and no
    /// earlier attempt is unresolved: nothing stays pending, so the next
    /// attempt reads a fresh nonce.
    #[tokio::test]
    async fn a_refused_or_unsent_first_attempt_leaves_nothing_pending() {
        use dash_sdk::dapi_client::DapiClientError;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let refused_outright = || {
            SdkError::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(Box::new(
                ConsensusError::DefaultError,
            )))
        };

        for (nonce, error) in [
            (
                5,
                nonce_refused(MergeIdentityNonceResult::NonceTooFarInFuture),
            ),
            (6, refused_outright()),
            (
                7,
                SdkError::DapiClientError(DapiClientError::NoAvailableAddresses),
            ),
        ] {
            let first = attempt(&ctx, RegistrationKind::Contract, nonce, Err(error), None).await;
            assert!(first.fresh_nonce_read, "nonce {nonce}");
            assert_eq!(first.asked, None, "nonce {nonce}");
            assert!(
                !matches!(
                    first.outcome,
                    Ok(_) | Err(TaskError::ContractRegistrationUnconfirmed { .. })
                ),
                "nonce {nonce}: {:?}",
                first.outcome
            );
            assert_eq!(
                ctx.pending_contract_registration(&owner),
                None,
                "nonce {nonce}"
            );
        }
    }

    /// A retry that was never sent does not count as sent: a contract later
    /// found with its content was registered elsewhere, not by this app.
    #[tokio::test]
    async fn an_unsent_retry_is_not_counted_as_sent() {
        use dash_sdk::dapi_client::DapiClientError;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let mut unsent = as_created(&ctx.dashpay_contract());
        unsent.set_owner_id(owner);
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let never_sent = attempt_registering(
            &ctx,
            unsent.clone(),
            None,
            RegistrationKind::Contract,
            6,
            Err(SdkError::DapiClientError(
                DapiClientError::NoAvailableAddresses,
            )),
            Ok(None),
        )
        .await;
        assert!(
            matches!(
                never_sent.outcome,
                Err(TaskError::ContractRegistrationUnconfirmed { .. })
            ),
            "{:?}",
            never_sent.outcome
        );

        let retry = attempt(
            &ctx,
            RegistrationKind::Contract,
            7,
            Err(unread()),
            Some(unsent),
        )
        .await;
        assert!(
            matches!(
                retry.outcome,
                Err(TaskError::ContractRegistrationPreempted { .. })
            ),
            "{:?}",
            retry.outcome
        );
    }

    /// An earlier attempt found under the shared ID after its nonce became
    /// unusable is the registration: confirmed and no longer pending.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unusable_nonce_whose_registration_is_found_is_confirmed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = wired_test_app_context(dir.path()).await;
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let earlier = as_created(&ctx.dpns_contract());
        let retry = attempt(
            &ctx,
            RegistrationKind::Contract,
            6,
            Err(nonce_refused(MergeIdentityNonceResult::NonceTooFarInPast)),
            Some(earlier),
        )
        .await;

        assert_eq!(retry.asked, Some(assigned_id(&ctx, 5)));
        assert!(
            matches!(
                retry.outcome,
                Ok(BackendTaskSuccessResult::RegisteredContract {
                    earlier_version: false,
                    ..
                })
            ),
            "{:?}",
            retry.outcome
        );
        assert_eq!(ctx.pending_contract_registration(&owner), None);
    }

    /// A first attempt, with a fresh nonce, refused because that nonce or its
    /// contract ID is already used was not this app's registration: nothing is
    /// looked up or adopted, the refusal is reported, and nothing stays
    /// pending.
    #[tokio::test]
    async fn a_first_attempt_refused_as_already_used_adopts_nothing() {
        use dash_sdk::dpp::consensus::state::data_contract::data_contract_already_present_error::DataContractAlreadyPresentError;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let present = SdkError::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(Box::new(
            ConsensusError::StateError(StateError::DataContractAlreadyPresentError(
                DataContractAlreadyPresentError::new(assigned_id(&ctx, 5)),
            )),
        )));

        for refusal in [
            nonce_refused(MergeIdentityNonceResult::NonceAlreadyPresentAtTip),
            present,
        ] {
            let other = as_created(&ctx.dashpay_contract());
            let first = attempt(
                &ctx,
                RegistrationKind::Contract,
                5,
                Err(refusal),
                Some(other),
            )
            .await;

            assert!(first.fresh_nonce_read);
            assert_eq!(first.asked, None);
            assert!(
                matches!(
                    first.outcome,
                    Err(TaskError::RegistrationNonceRefused { .. })
                ),
                "{:?}",
                first.outcome
            );
            assert_eq!(ctx.pending_contract_registration(&owner), None);
        }
    }

    /// While one registration by an identity is being sent, a second one by
    /// the same identity is refused before it reads or reuses a nonce, so it
    /// cannot replace or share the first one's pending nonce.
    #[tokio::test]
    async fn a_second_registration_while_one_is_being_sent_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let (reached_tx, reached_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

        let first = ctx.register_signed_contract(
            as_created(&ctx.dpns_contract()),
            || std::future::ready(Ok(5)),
            signing_key(),
            &FixedSigner,
            ctx.platform_version(),
            None,
            RegistrationKind::Contract,
            move |_transition| async move {
                let _ = reached_tx.send(());
                let _ = release_rx.await;
                Err(unread())
            },
            |_contract_id| std::future::ready(Ok(None)),
        );
        let second = async {
            reached_rx.await.expect("the first registration is sent");
            let second = attempt(&ctx, RegistrationKind::Token, 9, Err(unread()), None).await;
            let _ = release_tx.send(());
            second
        };
        let (first, second) = tokio::join!(first, second);

        assert!(
            matches!(
                second.outcome,
                Err(TaskError::ContractRegistrationInProgress)
            ),
            "{:?}",
            second.outcome
        );
        assert!(!second.fresh_nonce_read);
        assert_eq!(second.sent_nonce, None);
        assert!(
            matches!(
                first,
                Err(TaskError::ContractRegistrationUnconfirmed { .. })
            ),
            "{first:?}"
        );
        assert_eq!(
            ctx.pending_contract_registration(&owner),
            Some(PendingContractRegistration {
                identity_nonce: 5,
                contract_id: assigned_id(&ctx, 5),
            })
        );

        let after = attempt(&ctx, RegistrationKind::Contract, 9, Err(unread()), None).await;
        assert_eq!(after.sent_nonce, Some(5));
    }

    /// A system contract with `keywords`, as the token creator builds it.
    fn contract_with_keywords(ctx: &AppContext, keywords: &[&str]) -> DataContract {
        let mut contract = as_created(&ctx.dpns_contract());
        contract.set_keywords(keywords.iter().map(|k| k.to_string()).collect());
        contract
    }

    /// Platform lowercases keywords when it reads a contract; the submitted
    /// keywords keep the user's case, which is no change.
    #[test]
    fn keywords_differing_only_in_case_are_the_submitted_contract() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let submitted = contract_with_keywords(&ctx, &["MyToken", "Dash"]);
        let registered = contract_with_keywords(&ctx, &["mytoken", "dash"]);
        assert_eq!(
            registered.keywords(),
            &vec!["mytoken".to_string(), "dash".to_string()]
        );

        assert!(is_submitted_contract(
            &submitted,
            &registered,
            ctx.platform_version()
        ));
    }

    /// The ID, owner and creation times Platform sets are no change.
    #[test]
    fn fields_platform_sets_are_not_a_change() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let submitted = as_created(&ctx.dpns_contract());
        let mut registered = submitted.clone();
        registered.set_id(Identifier::from([0x31; 32]));
        registered.set_owner_id(Identifier::from([0x32; 32]));
        registered.set_created_at(Some(1_700_000_000_000));
        registered.set_updated_at(Some(1_700_000_000_000));
        registered.set_created_at_block_height(Some(1_000));
        registered.set_updated_at_block_height(Some(1_000));
        registered.set_created_at_epoch(Some(3));
        registered.set_updated_at_epoch(Some(3));

        assert!(is_submitted_contract(
            &submitted,
            &registered,
            ctx.platform_version()
        ));
    }

    /// A token contract as the token creator builds it, with a capitalised
    /// keyword, a searchable name and a description, sent in a create
    /// transition, read the way Drive executes it (fully validated, with the
    /// creation fields set) and fetched back, is the contract that was sent.
    #[tokio::test]
    async fn a_token_contract_as_drive_stores_it_is_the_submitted_contract() {
        use crate::backend_task::tokens::TokenContractParams;
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
        use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::TokenKeepsHistoryRules;
        use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::v0::TokenKeepsHistoryRulesV0;
        use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
        use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
        use dash_sdk::dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;
        use dash_sdk::dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructureTrusted;

        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let platform_version = ctx.platform_version();
        let rules = || ChangeControlRules::V0(ChangeControlRulesV0::default());
        let params = TokenContractParams {
            token_names: vec![(
                "MyToken".to_string(),
                "MyTokens".to_string(),
                "en".to_string(),
            )],
            // The searchable name, as the token creator adds it, and a
            // capitalised keyword.
            contract_keywords: vec!["MyToken".to_string(), "Payments".to_string()],
            token_description: Some("A token for paying in the shop.".to_string()),
            should_capitalize: true,
            decimals: 8,
            base_supply: 1_000_000,
            max_supply: None,
            start_paused: false,
            allow_transfers_to_frozen_identities: false,
            keeps_history: TokenKeepsHistoryRules::V0(TokenKeepsHistoryRulesV0::default()),
            main_control_group: None,
            manual_minting_rules: rules(),
            manual_burning_rules: rules(),
            freeze_rules: rules(),
            unfreeze_rules: rules(),
            destroy_frozen_funds_rules: rules(),
            emergency_action_rules: rules(),
            max_supply_change_rules: rules(),
            conventions_change_rules: rules(),
            main_control_group_change_authorized: AuthorizedActionTakers::ContractOwner,
            distribution_rules: TokenDistributionRules::V0(TokenDistributionRulesV0 {
                perpetual_distribution: None,
                perpetual_distribution_rules: rules(),
                pre_programmed_distribution: None,
                new_tokens_destination_identity: None,
                new_tokens_destination_identity_rules: rules(),
                minting_allow_choosing_destination: false,
                minting_allow_choosing_destination_rules: rules(),
                change_direct_purchase_pricing_rules: rules(),
            }),
            groups: BTreeMap::new(),
            document_schemas: None,
            marketplace_rules: rules(),
            transferable: true,
        };
        let sent = ctx
            .build_data_contract_v1_with_one_token(ctx.dpns_contract().owner_id(), params)
            .expect("build the token contract");

        let (transition, contract_id) = contract_create_transition(
            sent.clone(),
            5,
            signing_key(),
            &FixedSigner,
            platform_version,
        )
        .await
        .expect("build the create transition");
        let StateTransition::DataContractCreate(create) = transition else {
            panic!("expected a contract create transition");
        };
        let mut stored = DataContract::try_from_platform_versioned(
            create.data_contract().clone(),
            true,
            &mut vec![],
            platform_version,
        )
        .expect("Drive reads the sent contract");
        stored.set_created_at(Some(1_700_000_000_000));
        stored.set_created_at_epoch(Some(3));
        stored.set_created_at_block_height(Some(1_000));
        assert_eq!(stored.id(), contract_id);
        let stored_bytes = stored
            .serialize_to_bytes_with_platform_version(platform_version)
            .expect("serialize the stored contract");
        let fetched =
            DataContract::versioned_deserialize_trusted(&stored_bytes, false, platform_version)
                .expect("read the fetched contract");
        assert_eq!(
            fetched.keywords(),
            &vec!["mytoken".to_string(), "payments".to_string()]
        );

        assert!(is_submitted_contract(&sent, &fetched, platform_version));
    }

    /// A different schema, or different keywords, is a change.
    #[test]
    fn a_different_contract_is_not_the_submitted_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let submitted = as_created(&ctx.dpns_contract());
        let other_schema = as_created(&ctx.dashpay_contract());
        assert!(!is_submitted_contract(
            &submitted,
            &other_schema,
            ctx.platform_version()
        ));

        let other_keywords = contract_with_keywords(&ctx, &["other"]);
        assert!(!is_submitted_contract(
            &contract_with_keywords(&ctx, &["mine"]),
            &other_keywords,
            ctx.platform_version()
        ));
    }

    /// A confirmed registration is no longer pending: the next one reads a
    /// fresh nonce.
    #[tokio::test]
    async fn a_confirmed_registration_is_no_longer_pending() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        attempt(&ctx, RegistrationKind::Contract, 5, Err(unread()), None).await;

        let confirmed = as_created(&ctx.dpns_contract());
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
        let registered = as_created(&ctx.dpns_contract());

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

        let confirmed = as_created(&ctx.dpns_contract());
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
        let registered = as_created(&ctx.dpns_contract());

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
        let mut earlier = as_created(&ctx.dashpay_contract());
        earlier.set_owner_id(ctx.dpns_contract().owner_id());

        for kind in [RegistrationKind::Contract, RegistrationKind::Token] {
            attempt_registering(
                &ctx,
                earlier.clone(),
                None,
                kind,
                5,
                Err(unread()),
                Ok(None),
            )
            .await;
            let done = attempt(&ctx, kind, 6, Err(unread()), Some(earlier.clone())).await;
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

    /// A contract found under the assigned ID that this app never sent there
    /// was registered elsewhere by the same identity: the registration is not
    /// done and no longer pending, so the next
    /// attempt reads a fresh nonce and gets a new ID. For both kinds, on a
    /// first attempt and on a retry.
    #[tokio::test]
    async fn a_different_contract_under_the_assigned_id_is_not_reported_as_done() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(dir.path());
        let owner = ctx.dpns_contract().owner_id();
        let other = as_created(&ctx.dashpay_contract());

        for kind in [RegistrationKind::Contract, RegistrationKind::Token] {
            let first = attempt(&ctx, kind, 5, Err(unread()), Some(other.clone())).await;
            let preempted_id = match (&first.outcome, kind) {
                (
                    Err(TaskError::ContractRegistrationPreempted { contract_id, .. }),
                    RegistrationKind::Contract,
                )
                | (
                    Err(TaskError::TokenRegistrationPreempted { contract_id, .. }),
                    RegistrationKind::Token,
                ) => *contract_id,
                (outcome, _) => {
                    panic!("{kind:?}: expected a preempted registration, got {outcome:?}")
                }
            };
            assert_eq!(preempted_id, assigned_id(&ctx, 5));
            assert_eq!(ctx.pending_contract_registration(&owner), None);

            let next = attempt(&ctx, kind, 7, Err(unread()), None).await;
            assert!(next.fresh_nonce_read);
            assert_eq!(next.sent_nonce, Some(7));

            let retry = attempt(
                &ctx,
                kind,
                8,
                Err(nonce_refused(
                    MergeIdentityNonceResult::NonceAlreadyPresentAtTip,
                )),
                Some(other.clone()),
            )
            .await;
            assert_eq!(retry.sent_nonce, Some(7));
            assert!(
                matches!(
                    retry.outcome,
                    Err(TaskError::ContractRegistrationPreempted { .. })
                        | Err(TaskError::TokenRegistrationPreempted { .. })
                ),
                "{kind:?}: {:?}",
                retry.outcome
            );
            assert_eq!(ctx.pending_contract_registration(&owner), None);
        }
    }

    /// A registration not found after an unread result stays unresolved: the
    /// error names the assigned ID to check, and promises no second
    /// registration only for a retry before the app closes, since the pending
    /// nonce lives in memory.
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
            assert!(message.contains("before closing the app"), "{message}");
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
        let contract = as_created(&ctx.dpns_contract());

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
        let contract = as_created(&ctx.dpns_contract());

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
