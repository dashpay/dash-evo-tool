use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dash_sdk::dash_platform_queries::transition::validation::ensure_valid_state_transition_structure;
use dash_sdk::dpp::identity::PartialIdentity;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::signer::Signer;
use dash_sdk::dpp::prelude::IdentityNonce;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::state_transition::data_contract_create_transition::DataContractCreateTransition;
use dash_sdk::dpp::state_transition::data_contract_create_transition::methods::DataContractCreateTransitionMethodsV0;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::Fetch;
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

/// How long after submission an unconfirmed registration is looked for.
/// Platform creates a block, empty if need be, about every three minutes, so a
/// transition still unconfirmed five minutes after submission was dropped and
/// will not execute; paying again is then safe.
const REGISTRATION_CONFIRMATION_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Pause between lookups of an unconfirmed registration.
const REGISTRATION_RECHECK_INTERVAL: Duration = Duration::from_secs(30);

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
        let alias = (!alias.is_empty()).then_some(alias);
        ensure_private_document_entries_fit(&data_contract, None)?;
        let identity_nonce = sdk
            .get_identity_nonce(data_contract.owner_id(), true, None)
            .await?;
        self.register_signed_contract(
            data_contract,
            identity_nonce,
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
            |contract_id, submitted_at| {
                self.find_unconfirmed_registration(sdk, contract_id, submitted_at, &sender)
            },
        )
        .await
    }

    /// [`Self::register_data_contract`] once the identity nonce is known, with
    /// the network steps passed in: `submit` sends the signed transition and
    /// waits for its result; `find` looks a registration whose result could
    /// not be read up under its Platform-assigned ID.
    #[expect(
        clippy::too_many_arguments,
        reason = "the transition's inputs plus the two network steps"
    )]
    async fn register_signed_contract<S, Submitted, Found>(
        &self,
        data_contract: DataContract,
        identity_nonce: IdentityNonce,
        signing_key: IdentityPublicKey,
        signer: &S,
        platform_version: &PlatformVersion,
        alias: Option<&str>,
        kind: RegistrationKind,
        submit: impl FnOnce(StateTransition) -> Submitted,
        find: impl FnOnce(Identifier, Instant) -> Found,
    ) -> Result<BackendTaskSuccessResult, TaskError>
    where
        S: Signer<IdentityPublicKey>,
        Submitted: Future<Output = Result<DataContract, SdkError>>,
        Found: Future<Output = Result<Option<DataContract>, TaskError>>,
    {
        let fee_result =
            FeeResult::estimated_only(self.fee_estimator().estimate_contract_create_base());
        let (transition, contract_id) = contract_create_transition(
            data_contract,
            identity_nonce,
            signing_key,
            signer,
            platform_version,
        )
        .await?;

        let submitted_at = Instant::now();
        match submit(transition).await {
            Ok(contract) => Ok(self
                .finish_confirmed_registration(&contract, alias, fee_result, kind)
                .await),
            Err(error) if may_have_registered(&error) => {
                let found = find(contract_id, submitted_at).await?;
                self.settle_unconfirmed_registration(
                    found,
                    contract_id,
                    error,
                    alias,
                    fee_result,
                    kind,
                )
                .await
            }
            Err(error) => Err(TaskError::from(error)),
        }
    }

    /// Looks for a registration whose result could not be read under its
    /// Platform-assigned ID until it is found or
    /// [`REGISTRATION_CONFIRMATION_WINDOW`] after `submitted_at` ends. The
    /// task runs throughout, so its form offers no second payment meanwhile.
    async fn find_unconfirmed_registration(
        &self,
        sdk: &Sdk,
        contract_id: Identifier,
        submitted_at: Instant,
        sender: &crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<Option<DataContract>, TaskError> {
        if let Some(contract) = self
            .refetch_contract_after_unread_result(sdk, contract_id, sender)
            .await?
        {
            return Ok(Some(contract));
        }
        tracing::info!(
            %contract_id,
            "Contract registration not confirmed yet; checking the network until the confirmation window ends"
        );
        let awaiting = TaskResult::interim_success(
            self.network,
            BackendTaskSuccessResult::ContractRegistrationAwaitingConfirmation,
        );
        if sender.send(awaiting).await.is_err() {
            tracing::debug!(%contract_id, "No frame loop took the registration's awaiting-confirmation report");
        }
        Ok(watch_for_registration(
            || async move { DataContract::fetch(sdk, contract_id).await.ok().flatten() },
            || submitted_at.elapsed(),
            tokio::time::sleep,
        )
        .await)
    }

    /// Settles a registration whose result could not be read, by the lookup
    /// under its Platform-assigned ID: found is confirmed; not found by the end
    /// of the confirmation window is reported as most likely not registered.
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

/// The pause before the next lookup of an unconfirmed registration submitted
/// `elapsed` ago, cut so the last lookup falls at the window's end; `None`
/// once the window is over.
fn next_registration_check(elapsed: Duration) -> Option<Duration> {
    let remaining = REGISTRATION_CONFIRMATION_WINDOW
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())?;
    Some(remaining.min(REGISTRATION_RECHECK_INTERVAL))
}

/// Runs `lookup` on the [`next_registration_check`] schedule until it finds
/// the registration or the window ends. `elapsed` and `sleep` are the clock.
async fn watch_for_registration<T, Found, Slept>(
    mut lookup: impl FnMut() -> Found,
    elapsed: impl Fn() -> Duration,
    mut sleep: impl FnMut(Duration) -> Slept,
) -> Option<T>
where
    Found: Future<Output = Option<T>>,
    Slept: Future<Output = ()>,
{
    while let Some(pause) = next_registration_check(elapsed()) {
        sleep(pause).await;
        if let Some(found) = lookup().await {
            return Some(found);
        }
    }
    None
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

    /// Drives [`AppContext::register_signed_contract`] for the DPNS contract at
    /// identity nonce 5, with `submitted` as the network's answer; returns the
    /// outcome and the ID the lookup was asked for, if any.
    async fn register_with(
        ctx: &AppContext,
        submitted: Result<DataContract, SdkError>,
        found: Option<DataContract>,
    ) -> (
        Result<BackendTaskSuccessResult, TaskError>,
        Option<Identifier>,
    ) {
        let contract = ctx.dpns_contract().as_ref().clone();
        let asked = std::cell::Cell::new(None);
        let outcome = ctx
            .register_signed_contract(
                contract,
                5,
                signing_key(),
                &FixedSigner,
                ctx.platform_version(),
                None,
                RegistrationKind::Contract,
                |_transition| std::future::ready(submitted),
                |contract_id, _submitted_at| {
                    asked.set(Some(contract_id));
                    std::future::ready(Ok(found))
                },
            )
            .await;
        (outcome, asked.get())
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

    /// A registration not found by the end of the confirmation window is
    /// reported as most likely not registered: the error names the assigned
    /// ID to check before registering again.
    #[tokio::test]
    async fn a_registration_not_found_in_the_window_is_reported_unconfirmed() {
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
            assert!(message.contains(&format!("{contract_id}")), "{message}");
            assert!(message.contains("five minutes"), "{message}");
            assert!(!message.contains("try again"), "{message}");
        }
    }

    /// Lookups come every recheck interval, the last one exactly when the
    /// confirmation window ends, and none after it.
    #[test]
    fn an_unconfirmed_registration_is_checked_until_the_window_ends() {
        assert_eq!(
            next_registration_check(Duration::ZERO),
            Some(REGISTRATION_RECHECK_INTERVAL)
        );
        assert_eq!(
            next_registration_check(REGISTRATION_CONFIRMATION_WINDOW - Duration::from_secs(10)),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            next_registration_check(REGISTRATION_CONFIRMATION_WINDOW),
            None
        );
        assert_eq!(
            next_registration_check(REGISTRATION_CONFIRMATION_WINDOW + Duration::from_secs(1)),
            None
        );
    }

    /// The watch stops at the first lookup that finds the registration.
    #[tokio::test]
    async fn the_watch_stops_once_the_registration_is_found() {
        use std::cell::Cell;
        let now = Cell::new(Duration::ZERO);
        let lookups = Cell::new(0u32);

        let found = watch_for_registration(
            || {
                lookups.set(lookups.get() + 1);
                std::future::ready((lookups.get() == 3).then_some(7u8))
            },
            || now.get(),
            |pause| {
                now.set(now.get() + pause);
                std::future::ready(())
            },
        )
        .await;

        assert_eq!(found, Some(7));
        assert_eq!(lookups.get(), 3);
        assert_eq!(now.get(), REGISTRATION_RECHECK_INTERVAL * 3);
    }

    /// A registration never found is looked for until the window ends, and
    /// no longer: the form then offers paying again.
    #[tokio::test]
    async fn the_watch_gives_up_when_the_window_ends() {
        use std::cell::Cell;
        let now = Cell::new(Duration::from_secs(40));
        let lookups = Cell::new(0u32);

        let found = watch_for_registration(
            || {
                lookups.set(lookups.get() + 1);
                std::future::ready(None::<u8>)
            },
            || now.get(),
            |pause| {
                now.set(now.get() + pause);
                std::future::ready(())
            },
        )
        .await;

        assert_eq!(found, None);
        assert_eq!(now.get(), REGISTRATION_CONFIRMATION_WINDOW);
        // 260 s left: eight 30 s pauses, then the last 20 s.
        assert_eq!(lookups.get(), 9);
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
