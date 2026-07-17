mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTarget,
    DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::request_type::RequestType;
use dash_sdk::Sdk;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
use dash_sdk::dpp::voting::votes::resource_vote::accessors::v0::ResourceVoteGettersV0;
use dash_sdk::drive::query::contested_resource_votes_given_by_identity_query::ContestedResourceVotesGivenByIdentityQuery;
use dash_sdk::platform::{FetchMany, Identifier};
use futures::{StreamExt, stream};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Widest window past a scheduled vote's time that the sweep will still cast it.
/// Beyond this a due vote is considered stale and left for the user to reschedule
/// rather than cast late (mirrors the original 2-minute UI-poll grace).
const SCHEDULED_VOTE_MAX_LATENESS_MS: u64 = 120_000;

#[derive(Debug, Clone, PartialEq)]
pub enum ContestedResourceTask {
    QueryDPNSContests,
    SubmitDpnsVoteOperation(
        DpnsVoteOperation,
        Vec<QualifiedIdentity>,
        Option<DpnsVoteTargetKey>,
        Network,
    ),
    ReconcileDpnsVoteOperation(DpnsVoteOperationId, Network),
    CastScheduledVote(ScheduledDPNSVote, Box<QualifiedIdentity>),
    /// Sweep the scheduled-vote table and cast every vote that is now due.
    /// `preserve_eligibility_since_ms` keeps a vote eligible when its normal
    /// grace window overlapped a migration that deferred the sweep.
    CastDueScheduledVotes {
        preserve_eligibility_since_ms: Option<u64>,
    },
    ClearAllScheduledVotes,
    ClearExecutedScheduledVotes,
    DeleteScheduledVote(Identifier, String),
    CancelScheduledDpnsVote {
        operation_id: DpnsVoteOperationId,
        key: DpnsVoteTargetKey,
        contested_name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledDPNSVote {
    pub contested_name: String,
    pub voter_id: Identifier,
    pub choice: ResourceVoteChoice,
    pub unix_timestamp: u64,
    pub executed_successfully: bool,
}

fn classify_vote_attempt(
    attempt: &Result<vote_on_dpns_name::DpnsVoteAttempt, TaskError>,
) -> (DpnsVoteTargetStatus, Option<DpnsVoteFailure>) {
    match attempt {
        Ok(vote_on_dpns_name::DpnsVoteAttempt::Confirmed) => {
            (DpnsVoteTargetStatus::Confirmed, None)
        }
        Ok(vote_on_dpns_name::DpnsVoteAttempt::Unconfirmed(_)) => (
            DpnsVoteTargetStatus::Unconfirmed,
            Some(DpnsVoteFailure::ResultUnconfirmed),
        ),
        Ok(vote_on_dpns_name::DpnsVoteAttempt::Rejected(_)) => (
            DpnsVoteTargetStatus::Rejected,
            Some(DpnsVoteFailure::PlatformRejected),
        ),
        Err(_) => (
            DpnsVoteTargetStatus::FailedBeforeSubmission,
            Some(DpnsVoteFailure::SubmissionFailed),
        ),
    }
}

fn classify_reconciled_vote(
    observed: Option<ResourceVoteChoice>,
    requested: ResourceVoteChoice,
) -> Option<DpnsVoteTargetStatus> {
    (observed == Some(requested)).then_some(DpnsVoteTargetStatus::Confirmed)
}

fn persist_terminal_then_legacy_mirror(
    persist_terminal: impl FnOnce() -> Result<(), TaskError>,
    update_legacy_mirror: impl FnOnce() -> Result<(), TaskError>,
) -> Result<Option<TaskError>, TaskError> {
    persist_terminal()?;
    Ok(update_legacy_mirror().err())
}

fn wrap_scheduled_vote_sweep_result<T>(
    network: Network,
    result: Result<T, TaskError>,
) -> Result<T, TaskError> {
    result.map_err(|source| TaskError::ScheduledVoteSweepFailed {
        network,
        source: Box::new(source),
    })
}

/// Logs a Drive proof-verification failure raised by a contested-resource query.
///
/// No-op unless `e` is a [`dash_sdk::Error::Proof`] carrying a GroveDB proof
/// failure — the shape these read-only queries surface (distinct from the
/// `DriveProofError` shape handled by `AppContext::log_drive_proof_error`).
pub(super) fn log_contested_proof_error(e: &dash_sdk::Error, request_type: RequestType) {
    if let dash_sdk::Error::Proof(dash_sdk::ProofVerifierError::GroveDBError {
        proof_bytes,
        height,
        time_ms,
        error,
        ..
    }) = e
    {
        tracing::error!(
            target: "proof_log",
            request_type = ?request_type,
            height = *height,
            time_ms = *time_ms,
            proof_bytes_len = proof_bytes.len(),
            error = %error,
            "drive proof verification failed during contested-resource query",
        );
    }
}

impl AppContext {
    pub async fn run_contested_resource_task(
        self: &Arc<Self>,
        task: ContestedResourceTask,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            ContestedResourceTask::QueryDPNSContests => self
                .query_dpns_contested_resources(sdk, sender)
                .await
                .map(|_| BackendTaskSuccessResult::None),
            ContestedResourceTask::SubmitDpnsVoteOperation(
                operation,
                voters,
                replacing_scheduled_key,
                _,
            ) => {
                self.execute_dpns_vote_operation(operation, voters, replacing_scheduled_key, sdk)
                    .await
            }
            ContestedResourceTask::ReconcileDpnsVoteOperation(operation_id, _) => {
                self.reconcile_dpns_vote_operation(operation_id, sdk).await
            }
            ContestedResourceTask::CastScheduledVote(scheduled_vote, voter) => {
                let operation = self.operation_for_scheduled_vote(&scheduled_vote, &voter)?;
                self.execute_dpns_vote_operation(operation, vec![*voter], None, sdk)
                    .await
            }
            ContestedResourceTask::CastDueScheduledVotes {
                preserve_eligibility_since_ms,
            } => {
                let result = async {
                    self.ensure_dpns_vote_recovery(sdk).await?;
                    self.cast_due_scheduled_votes(sdk, sender, preserve_eligibility_since_ms)
                        .await
                }
                .await;
                wrap_scheduled_vote_sweep_result(self.network, result)
            }
            ContestedResourceTask::ClearAllScheduledVotes => {
                self.cancel_all_scheduled_dpns_vote_targets()?;
                if let Err(error) = self.clear_all_scheduled_votes() {
                    tracing::warn!(
                        ?error,
                        "Scheduled DPNS votes were cancelled but the legacy mirror could not be cleared"
                    );
                }
                Ok(BackendTaskSuccessResult::Refresh)
            }
            ContestedResourceTask::ClearExecutedScheduledVotes => {
                self.clear_executed_scheduled_votes()?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            ContestedResourceTask::DeleteScheduledVote(voter_id, contested_name) => {
                self.delete_scheduled_vote(voter_id.as_slice(), &contested_name)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            ContestedResourceTask::CancelScheduledDpnsVote {
                operation_id,
                key,
                contested_name,
            } => {
                self.cancel_scheduled_dpns_vote_target(operation_id, &key)?;
                if let Err(error) =
                    self.delete_scheduled_vote(key.voter_id.as_slice(), &contested_name)
                {
                    tracing::warn!(
                        ?error,
                        operation_id = %operation_id,
                        voter_id = %key.voter_id,
                        contested_name,
                        "Scheduled DPNS vote was cancelled but the legacy mirror could not be cleared"
                    );
                }
                Ok(BackendTaskSuccessResult::Refresh)
            }
        }
    }

    async fn ensure_dpns_vote_recovery(self: &Arc<Self>, sdk: &Sdk) -> Result<(), TaskError> {
        let mut recovered = self.dpns_vote_recovery.lock().await;
        if *recovered {
            return Ok(());
        }
        self.recover_interrupted_dpns_vote_operations()?;
        let queued = self
            .dpns_vote_operations()?
            .into_iter()
            .filter(|operation| {
                operation
                    .targets
                    .iter()
                    .any(|outcome| outcome.status == DpnsVoteTargetStatus::Queued)
            })
            .collect::<Vec<_>>();
        if !queued.is_empty() {
            let voters = self.load_local_voting_identities()?;
            for operation in queued {
                self.execute_dpns_vote_operation(operation, voters.clone(), None, sdk)
                    .await?;
            }
        }
        *recovered = true;
        Ok(())
    }

    fn dpns_vote_target(
        &self,
        voter: &QualifiedIdentity,
        name: &str,
        choice: ResourceVoteChoice,
        timing: VoteTiming,
        require_current_state: bool,
    ) -> Result<DpnsVoteTarget, TaskError> {
        let voter_id = voter.identity.id();
        let vote_poll_id = self.dpns_vote_poll_id(name)?;
        let current_choice = match self.dpns_current_vote_state(voter_id, vote_poll_id)? {
            DpnsCurrentVoteState::Available(choice) => choice,
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable
                if require_current_state =>
            {
                return Err(TaskError::DpnsCurrentVoteUnavailable);
            }
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => None,
        };
        Ok(DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: self.network,
                voter_id,
                vote_poll_id,
            },
            voter_alias: voter.alias.clone(),
            contested_name: name.to_owned(),
            requested_choice: choice,
            current_choice,
            timing,
        })
    }

    fn operation_for_scheduled_vote(
        &self,
        scheduled_vote: &ScheduledDPNSVote,
        voter: &QualifiedIdentity,
    ) -> Result<DpnsVoteOperation, TaskError> {
        if let Some(mut operation) = self.dpns_vote_operations()?.into_iter().find(|operation| {
            operation.targets.iter().any(|outcome| {
                outcome.target.key.voter_id == scheduled_vote.voter_id
                    && outcome.target.contested_name == scheduled_vote.contested_name
            })
        }) {
            let mut target_queued = false;
            for outcome in &mut operation.targets {
                if outcome.target.key.voter_id != scheduled_vote.voter_id
                    || outcome.target.contested_name != scheduled_vote.contested_name
                {
                    continue;
                }
                match outcome.status {
                    DpnsVoteTargetStatus::Scheduled => {
                        outcome.status = DpnsVoteTargetStatus::Queued;
                        target_queued = true;
                    }
                    DpnsVoteTargetStatus::Unconfirmed
                    | DpnsVoteTargetStatus::Queued
                    | DpnsVoteTargetStatus::Submitting
                    | DpnsVoteTargetStatus::Confirming => {
                        return Err(TaskError::DpnsVoteTargetBusy);
                    }
                    DpnsVoteTargetStatus::Confirmed => {
                        self.mark_vote_executed(
                            scheduled_vote.voter_id.as_slice(),
                            scheduled_vote.contested_name.clone(),
                        )?;
                        return Ok(operation);
                    }
                    DpnsVoteTargetStatus::Rejected
                    | DpnsVoteTargetStatus::FailedBeforeSubmission
                    | DpnsVoteTargetStatus::NotApplied => {
                        // An explicit Cast now action is a deliberate retry and
                        // may create a new operation below.
                    }
                }
            }
            if target_queued {
                return Ok(operation);
            }
        }

        let target = self.dpns_vote_target(
            voter,
            &scheduled_vote.contested_name,
            scheduled_vote.choice,
            VoteTiming::Scheduled(scheduled_vote.unix_timestamp),
            false,
        )?;
        let mut operation = DpnsVoteOperation::new(vec![target]);
        operation.targets[0].status = DpnsVoteTargetStatus::Queued;
        Ok(operation)
    }

    async fn execute_dpns_vote_operation(
        self: &Arc<Self>,
        mut operation: DpnsVoteOperation,
        voters: Vec<QualifiedIdentity>,
        replacing_scheduled_key: Option<DpnsVoteTargetKey>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        if operation.targets.is_empty() {
            return Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
                network: self.network,
                operation_id: operation.id,
            });
        }
        let was_persisted = self.dpns_vote_operation(operation.id)?.is_some();
        if operation
            .targets
            .iter()
            .any(|outcome| outcome.status == DpnsVoteTargetStatus::Queued)
        {
            // A fresh proved snapshot is a submission precondition. This also
            // prevents a due schedule from replaying a vote already observed.
            self.refresh_dpns_vote_states(sdk).await;
            let queued_keys = operation
                .targets
                .iter()
                .filter(|outcome| outcome.status == DpnsVoteTargetStatus::Queued)
                .map(|outcome| outcome.target.key.clone())
                .collect::<Vec<_>>();
            for key in queued_keys {
                let state = self.dpns_current_vote_state(key.voter_id, key.vote_poll_id)?;
                if was_persisted {
                    self.revalidate_queued_dpns_vote_target(operation.id, &key, state)?;
                    continue;
                }
                let Some(outcome) = operation
                    .targets
                    .iter_mut()
                    .find(|outcome| outcome.target.key == key)
                else {
                    continue;
                };
                match state {
                    DpnsCurrentVoteState::Available(current) => {
                        outcome.target.current_choice = current;
                        if current == Some(outcome.target.requested_choice) {
                            outcome.status = DpnsVoteTargetStatus::Confirmed;
                            outcome.failure = None;
                        }
                    }
                    DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => {
                        return Err(TaskError::DpnsCurrentVoteUnavailable);
                    }
                }
            }
        }

        if was_persisted {
            operation = self
                .dpns_vote_operation(operation.id)?
                .ok_or(TaskError::DpnsVoteOperationRecordMissing)?;
        } else {
            let scheduled_votes = operation
                .targets
                .iter()
                .filter_map(|outcome| match outcome.target.timing {
                    VoteTiming::Scheduled(unix_timestamp) => Some(ScheduledDPNSVote {
                        contested_name: outcome.target.contested_name.clone(),
                        voter_id: outcome.target.key.voter_id,
                        choice: outcome.target.requested_choice,
                        unix_timestamp,
                        executed_successfully: false,
                    }),
                    VoteTiming::Now => None,
                })
                .collect::<Vec<_>>();
            self.insert_dpns_vote_operation(&mut operation, replacing_scheduled_key.as_ref())?;
            if !scheduled_votes.is_empty()
                && let Err(error) = self.insert_scheduled_votes(&scheduled_votes)
            {
                // The journal is authoritative. The legacy table is a
                // compatibility mirror, so its failure cannot turn a durable
                // schedule into a reported failure that invites a duplicate.
                tracing::warn!(
                    ?error,
                    operation_id = %operation.id,
                    "DPNS vote schedule was journaled but its legacy mirror could not be updated"
                );
            }
        }

        for outcome in operation.targets.iter().filter(|outcome| {
            outcome.status == DpnsVoteTargetStatus::Confirmed
                && matches!(outcome.target.timing, VoteTiming::Scheduled(_))
        }) {
            if let Err(error) = self.mark_vote_executed(
                outcome.target.key.voter_id.as_slice(),
                outcome.target.contested_name.clone(),
            ) {
                tracing::warn!(
                    ?error,
                    operation_id = %operation.id,
                    voter_id = %outcome.target.key.voter_id,
                    contested_name = %outcome.target.contested_name,
                    "Confirmed DPNS vote was journaled but its legacy mirror could not be updated"
                );
            }
        }

        let voters_by_id: BTreeMap<Identifier, QualifiedIdentity> = voters
            .into_iter()
            .map(|voter| (voter.identity.id(), voter))
            .collect();
        let mut groups: BTreeMap<Identifier, Vec<_>> = BTreeMap::new();
        for outcome in operation
            .targets
            .iter()
            .filter(|outcome| outcome.status == DpnsVoteTargetStatus::Queued)
        {
            groups
                .entry(outcome.target.key.voter_id)
                .or_default()
                .push(outcome.target.clone());
        }

        const MAX_CONCURRENT_VOTERS: usize = 4;
        stream::iter(groups)
            .map(|(voter_id, targets)| {
                let app_context = Arc::clone(self);
                let sdk = sdk.clone();
                let voter = voters_by_id.get(&voter_id).cloned();
                let operation_id = operation.id;
                async move {
                    let Some(voter) = voter else {
                        for target in targets {
                            if app_context.claim_dpns_vote_target(operation_id, &target.key)? {
                                app_context.update_dpns_vote_target(
                                    operation_id,
                                    &target.key,
                                    DpnsVoteTargetStatus::FailedBeforeSubmission,
                                    Some(DpnsVoteFailure::SubmissionFailed),
                                )?;
                            }
                        }
                        return Ok::<(), TaskError>(());
                    };

                    // One voter's targets are deliberately sequential: PutVote
                    // obtains and consumes the same masternode nonce.
                    let _dispatch_guard = app_context.dpns_vote_dispatch.acquire(voter_id).await?;
                    for target in targets {
                        if !app_context.claim_dpns_vote_target(operation_id, &target.key)? {
                            continue;
                        }
                        let attempt = app_context
                            .submit_dpns_vote(
                                &target.contested_name,
                                target.requested_choice,
                                &voter,
                                &sdk,
                            )
                            .await;
                        let (status, failure) = classify_vote_attempt(&attempt);
                        let confirmed = matches!(
                            &attempt,
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::Confirmed)
                        );
                        match attempt {
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::Confirmed) => {
                            }
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::Unconfirmed(error)) => {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %target.key.voter_id,
                                    contested_name = %target.contested_name,
                                    "DPNS vote was submitted but remains unconfirmed"
                                );
                                app_context.record_dpns_vote_diagnostic(
                                    operation_id,
                                    target.key.clone(),
                                    error,
                                );
                            }
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::Rejected(error)) => {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %target.key.voter_id,
                                    contested_name = %target.contested_name,
                                    "Platform rejected a DPNS vote"
                                );
                                app_context.record_dpns_vote_diagnostic(
                                    operation_id,
                                    target.key.clone(),
                                    error,
                                );
                            }
                            Err(error) => {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %target.key.voter_id,
                                    contested_name = %target.contested_name,
                                    "DPNS vote failed before a confirmed submission"
                                );
                                app_context.record_dpns_vote_diagnostic(
                                    operation_id,
                                    target.key.clone(),
                                    error,
                                );
                            }
                        }
                        let mirror_error = persist_terminal_then_legacy_mirror(
                            || {
                                app_context.update_dpns_vote_target(
                                    operation_id,
                                    &target.key,
                                    status,
                                    failure,
                                )
                            },
                            || {
                                if confirmed
                                    && matches!(target.timing, VoteTiming::Scheduled(_))
                                {
                                    app_context.mark_vote_executed(
                                        target.key.voter_id.as_slice(),
                                        target.contested_name.clone(),
                                    )
                                } else {
                                    Ok(())
                                }
                            },
                        )?;
                        if let Some(error) = mirror_error {
                            tracing::warn!(
                                ?error,
                                operation_id = %operation_id,
                                voter_id = %target.key.voter_id,
                                contested_name = %target.contested_name,
                                "DPNS vote reached a terminal journal state but its legacy mirror could not be updated"
                            );
                        }
                        if confirmed {
                            app_context.cache_confirmed_dpns_vote(
                                target.key.voter_id,
                                target.key.vote_poll_id,
                                target.requested_choice,
                            )?;
                        }
                    }
                    Ok(())
                }
            })
            .buffer_unordered(MAX_CONCURRENT_VOTERS)
            .collect::<Vec<Result<(), TaskError>>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: self.network,
            operation_id: operation.id,
        })
    }

    async fn reconcile_dpns_vote_operation(
        &self,
        operation_id: DpnsVoteOperationId,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let Some(operation) = self.dpns_vote_operation(operation_id)? else {
            return Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
                network: self.network,
                operation_id,
            });
        };
        for outcome in operation
            .targets
            .iter()
            .filter(|outcome| outcome.status == DpnsVoteTargetStatus::Unconfirmed)
        {
            let poll_id = outcome.target.key.vote_poll_id;
            let query = ContestedResourceVotesGivenByIdentityQuery {
                identity_id: outcome.target.key.voter_id,
                offset: None,
                limit: Some(1),
                start_at: Some((poll_id.to_buffer(), true)),
                order_ascending: true,
            };
            match ResourceVote::fetch_many(sdk, query).await {
                Ok(votes)
                    if classify_reconciled_vote(
                        votes
                            .get(&poll_id)
                            .and_then(Option::as_ref)
                            .map(ResourceVoteGettersV0::resource_vote_choice),
                        outcome.target.requested_choice,
                    ) == Some(DpnsVoteTargetStatus::Confirmed) =>
                {
                    let mirror_error = persist_terminal_then_legacy_mirror(
                        || {
                            self.update_dpns_vote_target(
                                operation_id,
                                &outcome.target.key,
                                DpnsVoteTargetStatus::Confirmed,
                                None,
                            )
                        },
                        || {
                            if matches!(outcome.target.timing, VoteTiming::Scheduled(_)) {
                                self.mark_vote_executed(
                                    outcome.target.key.voter_id.as_slice(),
                                    outcome.target.contested_name.clone(),
                                )
                            } else {
                                Ok(())
                            }
                        },
                    )?;
                    if let Some(error) = mirror_error {
                        tracing::warn!(
                            ?error,
                            operation_id = %operation_id,
                            voter_id = %outcome.target.key.voter_id,
                            contested_name = %outcome.target.contested_name,
                            "Reconciled DPNS vote reached a terminal journal state but its legacy mirror could not be updated"
                        );
                    }
                    self.cache_confirmed_dpns_vote(
                        outcome.target.key.voter_id,
                        poll_id,
                        outcome.target.requested_choice,
                    )?;
                }
                Ok(_) => {}
                Err(error) => {
                    let error = TaskError::from(error);
                    tracing::warn!(
                        ?error,
                        operation_id = %operation_id,
                        voter_id = %outcome.target.key.voter_id,
                        contested_name = %outcome.target.contested_name,
                        "Could not reconcile an unconfirmed DPNS vote"
                    );
                    self.record_dpns_vote_diagnostic(
                        operation_id,
                        outcome.target.key.clone(),
                        error,
                    );
                }
            }
        }
        Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: self.network,
            operation_id,
        })
    }

    /// Cast every scheduled vote that is now due, off the UI thread.
    ///
    /// Queries the scheduled-vote table, keeps the votes whose time has arrived
    /// (and are not already executed or stale beyond
    /// [`SCHEDULED_VOTE_MAX_LATENESS_MS`]), pairs each with its local voting
    /// identity, and casts them independently so one failure cannot abort the
    /// rest. Emits [`ScheduledVotesInProgress`] before casting; terminal state
    /// is persisted in the shared operation journal and legacy executed flag.
    ///
    /// [`ScheduledVotesInProgress`]: BackendTaskSuccessResult::ScheduledVotesInProgress
    async fn cast_due_scheduled_votes(
        self: &Arc<Self>,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
        preserve_eligibility_since_ms: Option<u64>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        for operation in self
            .dpns_vote_operations()?
            .into_iter()
            .filter(|operation| {
                operation
                    .targets
                    .iter()
                    .any(|outcome| outcome.status == DpnsVoteTargetStatus::Unconfirmed)
            })
        {
            let result = self
                .reconcile_dpns_vote_operation(operation.id, sdk)
                .await?;
            let _ = sender.send(TaskResult::unattributed_success(result)).await;
        }

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut due_operations = Vec::new();
        let mut in_progress = Vec::new();
        for mut operation in self.dpns_vote_operations()? {
            let mut due = false;
            for outcome in &mut operation.targets {
                let VoteTiming::Scheduled(scheduled_at) = outcome.target.timing else {
                    continue;
                };
                if outcome.status == DpnsVoteTargetStatus::Scheduled
                    && scheduled_vote_is_due(
                        scheduled_at,
                        false,
                        now_ms,
                        preserve_eligibility_since_ms,
                    )
                    && self.queue_scheduled_dpns_vote_target(operation.id, &outcome.target.key)?
                {
                    outcome.status = DpnsVoteTargetStatus::Queued;
                    due = true;
                    in_progress.push(ScheduledDPNSVote {
                        contested_name: outcome.target.contested_name.clone(),
                        voter_id: outcome.target.key.voter_id,
                        choice: outcome.target.requested_choice,
                        unix_timestamp: scheduled_at,
                        executed_successfully: false,
                    });
                }
            }
            if due {
                due_operations.push(operation);
            }
        }
        if due_operations.is_empty() {
            return Ok(BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
                network: self.network,
                preserve_eligibility_since_ms,
            });
        }

        let voters = self.load_local_voting_identities()?;
        // Tell the Scheduled Votes screen which votes are now in flight.
        let _ = sender
            .send(TaskResult::unattributed_success(
                BackendTaskSuccessResult::ScheduledVotesInProgress(in_progress),
            ))
            .await;

        let results = stream::iter(due_operations)
            .map(|operation| {
                let app_context = Arc::clone(self);
                let sdk = sdk.clone();
                let voters = voters.clone();
                let operation_id = operation.id;
                async move {
                    let result = app_context
                        .execute_dpns_vote_operation(operation, voters, None, &sdk)
                        .await
                        .map(|_| ());
                    (operation_id, result)
                }
            })
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;
        let mut first_error = None;
        for (operation_id, result) in results {
            if let Err(error) = result {
                tracing::error!(
                    error = %error,
                    operation_id = %operation_id,
                    "Failed to execute a due DPNS vote operation; leaving it for recovery"
                );
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
                network: self.network,
                preserve_eligibility_since_ms,
            })
        }
    }
}

fn scheduled_vote_is_due(
    scheduled_at_ms: u64,
    executed_successfully: bool,
    now_ms: u64,
    preserve_eligibility_since_ms: Option<u64>,
) -> bool {
    let eligibility_cutoff_ms = preserve_eligibility_since_ms.unwrap_or(now_ms);
    !executed_successfully
        && scheduled_at_ms <= now_ms
        && scheduled_at_ms.saturating_add(SCHEDULED_VOTE_MAX_LATENESS_MS) >= eligibility_cutoff_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// VOTE-TC-033: an inner scheduled rejection is never classified as success.
    #[test]
    fn scheduled_inner_rejection_needs_attention() {
        let attempt = Ok(vote_on_dpns_name::DpnsVoteAttempt::Rejected(
            TaskError::DpnsVoteTargetBusy,
        ));
        assert_eq!(
            classify_vote_attempt(&attempt),
            (
                DpnsVoteTargetStatus::Rejected,
                Some(DpnsVoteFailure::PlatformRejected)
            )
        );
    }

    /// VOTE-TC-034/052: an ambiguous post-broadcast result stays locked.
    #[test]
    fn cause_less_wait_failure_is_unconfirmed_not_retryable() {
        let attempt = Ok(vote_on_dpns_name::DpnsVoteAttempt::Unconfirmed(
            TaskError::DpnsVoteTargetBusy,
        ));
        let (status, _) = classify_vote_attempt(&attempt);
        assert_eq!(status, DpnsVoteTargetStatus::Unconfirmed);
        assert!(status.holds_lock());
    }

    #[test]
    fn exact_reconciliation_confirms_only_the_requested_choice() {
        assert_eq!(
            classify_reconciled_vote(Some(ResourceVoteChoice::Lock), ResourceVoteChoice::Lock),
            Some(DpnsVoteTargetStatus::Confirmed)
        );
        assert_eq!(
            classify_reconciled_vote(Some(ResourceVoteChoice::Abstain), ResourceVoteChoice::Lock),
            None,
            "a mismatched row may predate the submitted transition and remains ambiguous"
        );
        assert_eq!(
            classify_reconciled_vote(None, ResourceVoteChoice::Lock),
            None,
            "an absent exact row remains ambiguous and must not release its lock"
        );
    }

    #[test]
    fn terminal_journal_write_precedes_best_effort_legacy_mirror() {
        let events = RefCell::new(Vec::new());
        let mirror_error = persist_terminal_then_legacy_mirror(
            || {
                events.borrow_mut().push("journal");
                Ok(())
            },
            || {
                events.borrow_mut().push("legacy");
                Err(TaskError::DpnsVoteTargetBusy)
            },
        )
        .expect("the authoritative journal write succeeded");

        assert!(mirror_error.is_some());
        assert_eq!(events.into_inner(), vec!["journal", "legacy"]);
    }

    #[test]
    fn scheduled_terminal_or_unconfirmed_targets_are_not_due_for_rebroadcast() {
        for status in [
            DpnsVoteTargetStatus::Unconfirmed,
            DpnsVoteTargetStatus::Rejected,
            DpnsVoteTargetStatus::FailedBeforeSubmission,
            DpnsVoteTargetStatus::Confirmed,
        ] {
            assert_ne!(status, DpnsVoteTargetStatus::Scheduled);
        }
    }

    #[test]
    fn scheduled_sweep_wraps_recovery_failures() {
        let error = wrap_scheduled_vote_sweep_result::<BackendTaskSuccessResult>(
            Network::Testnet,
            Err(TaskError::DpnsCurrentVoteUnavailable),
        )
        .expect_err("a recovery failure must fail the scheduled sweep");

        assert!(matches!(
            error,
            TaskError::ScheduledVoteSweepFailed {
                network: Network::Testnet,
                source,
            } if matches!(source.as_ref(), TaskError::DpnsCurrentVoteUnavailable)
        ));
    }

    /// Migration extends only eligibility windows that overlap its wait.
    #[test]
    fn migration_wait_preserves_only_overlapping_vote_eligibility() {
        let migration_started_ms = 1_000_000;
        let now_ms = migration_started_ms + SCHEDULED_VOTE_MAX_LATENESS_MS * 2;

        assert!(
            scheduled_vote_is_due(
                migration_started_ms,
                false,
                now_ms,
                Some(migration_started_ms),
            ),
            "a vote due when migration began must remain eligible afterward",
        );
        assert!(
            !scheduled_vote_is_due(
                migration_started_ms - SCHEDULED_VOTE_MAX_LATENESS_MS - 1,
                false,
                now_ms,
                Some(migration_started_ms),
            ),
            "migration must not revive a vote already stale before it began",
        );
        assert!(
            !scheduled_vote_is_due(migration_started_ms, false, now_ms, None),
            "the normal sweep must retain the ordinary lateness limit",
        );
        assert!(
            !scheduled_vote_is_due(now_ms + 1, false, now_ms, Some(migration_started_ms)),
            "migration must not cast a vote before its scheduled time",
        );
        assert!(
            !scheduled_vote_is_due(
                migration_started_ms,
                true,
                now_ms,
                Some(migration_started_ms)
            ),
            "migration must not cast an already-executed vote again",
        );
    }
}
