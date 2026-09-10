mod edit_scheduled_vote;
mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::{DapiAddressAvailability, TaskError};
use crate::context::AppContext;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsScheduleEditValidationError, DpnsVoteFailure, DpnsVoteOperation,
    DpnsVoteOperationId, DpnsVoteTarget, DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
    dpns_schedule_is_overdue, failed_before_broadcast_outcome, unavailable_preflight_outcome,
    validate_dpns_schedule_edit, validate_dpns_schedule_time,
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

#[cfg(test)]
use crate::model::dpns_voting::SCHEDULED_VOTE_MAX_LATENESS_MS;

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
    EditScheduledDpnsVote {
        operation_id: Option<DpnsVoteOperationId>,
        key: DpnsVoteTargetKey,
        expected_choice: ResourceVoteChoice,
        expected_timestamp: u64,
        choice: ResourceVoteChoice,
        unix_timestamp: u64,
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
    timing: VoteTiming,
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
        Ok(vote_on_dpns_name::DpnsVoteAttempt::FailedBeforeSubmission(_)) | Err(_) => {
            failed_before_broadcast_outcome(timing)
        }
    }
}

fn missing_voter_outcome(
    timing: VoteTiming,
) -> (DpnsVoteTargetStatus, Option<DpnsVoteFailure>, bool) {
    let (status, failure) = failed_before_broadcast_outcome(timing);
    (status, failure, matches!(timing, VoteTiming::Scheduled(_)))
}

fn classify_reconciled_vote(
    observed: Option<ResourceVoteChoice>,
    requested: ResourceVoteChoice,
) -> Option<DpnsVoteTargetStatus> {
    match observed {
        Some(choice) if choice == requested => Some(DpnsVoteTargetStatus::Confirmed),
        // A different current choice does not establish that this transition cannot apply.
        Some(_) | None => None,
    }
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
    fn record_dpns_vote_diagnostic_with_dapi_context(
        &self,
        operation_id: DpnsVoteOperationId,
        key: DpnsVoteTargetKey,
        error: TaskError,
        sdk: &Sdk,
    ) {
        let error = error.contextualize_dapi_availability(DapiAddressAvailability::from_sdk(sdk));
        self.record_dpns_vote_diagnostic(operation_id, key, error);
    }

    pub async fn run_contested_resource_task(
        self: &Arc<Self>,
        task: ContestedResourceTask,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let is_scheduled_sweep =
            matches!(&task, ContestedResourceTask::CastDueScheduledVotes { .. });
        if !is_scheduled_sweep && !matches!(&task, ContestedResourceTask::QueryDPNSContests) {
            self.ensure_dpns_vote_recovery(sdk).await?;
        }
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
                self.execute_dpns_vote_operation_with_recovery(
                    operation,
                    voters,
                    replacing_scheduled_key,
                    sdk,
                )
                .await
            }
            ContestedResourceTask::ReconcileDpnsVoteOperation(operation_id, _) => {
                self.reconcile_dpns_vote_operation(operation_id, sdk).await
            }
            ContestedResourceTask::CastScheduledVote(scheduled_vote, voter) => {
                let operation = self.operation_for_scheduled_vote(&scheduled_vote, &voter)?;
                self.execute_dpns_vote_operation_with_recovery(operation, vec![*voter], None, sdk)
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
                let outcomes = self.clear_all_scheduled_dpns_votes()?;
                Ok(BackendTaskSuccessResult::ScheduledVotesCleared(outcomes))
            }
            ContestedResourceTask::ClearExecutedScheduledVotes => {
                self.clear_executed_scheduled_votes()?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            ContestedResourceTask::DeleteScheduledVote(voter_id, contested_name) => {
                let key = DpnsVoteTargetKey {
                    network: self.network,
                    voter_id,
                    vote_poll_id: self.dpns_vote_poll_id(&contested_name)?,
                };
                self.remove_scheduled_dpns_vote(None, &key, &contested_name)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            ContestedResourceTask::CancelScheduledDpnsVote {
                operation_id,
                key,
                contested_name,
            } => {
                self.cancel_scheduled_dpns_vote_target(operation_id, &key, &contested_name)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            ContestedResourceTask::EditScheduledDpnsVote {
                operation_id,
                key,
                expected_choice,
                expected_timestamp,
                choice,
                unix_timestamp,
            } => {
                self.edit_scheduled_dpns_vote(
                    crate::model::dpns_voting::DpnsScheduledVoteEdit {
                        operation_id,
                        key,
                        expected_choice,
                        expected_timestamp,
                        choice,
                        unix_timestamp,
                    },
                    sdk,
                    sender,
                )
                .await
            }
        }
    }

    async fn ensure_dpns_vote_recovery(self: &Arc<Self>, sdk: &Sdk) -> Result<(), TaskError> {
        let mut recovered = self.dpns_vote_recovery.lock().await;
        if *recovered {
            return Ok(());
        }
        self.migrate_dpns_vote_operations()?;
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
        let key = DpnsVoteTargetKey {
            network: self.network,
            voter_id: scheduled_vote.voter_id,
            vote_poll_id: self.dpns_vote_poll_id(&scheduled_vote.contested_name)?,
        };
        let operations = self.dpns_vote_operations()?;
        if let Some(operation) = preferred_operation_for_scheduled_key(&operations, &key)?.cloned()
        {
            for outcome in &operation.targets {
                if outcome.target.key != key {
                    continue;
                }
                match outcome.status {
                    DpnsVoteTargetStatus::Scheduled => {
                        if !self
                            .queue_scheduled_dpns_vote_target(operation.id, &outcome.target.key)?
                        {
                            return Err(TaskError::DpnsVoteTargetBusy);
                        }
                        return self
                            .dpns_vote_operation(operation.id)?
                            .ok_or(TaskError::DpnsVoteOperationRecordMissing);
                    }
                    DpnsVoteTargetStatus::Unconfirmed
                    | DpnsVoteTargetStatus::Queued
                    | DpnsVoteTargetStatus::Submitting
                    | DpnsVoteTargetStatus::Confirming => {
                        return Err(TaskError::DpnsVoteTargetBusy);
                    }
                    DpnsVoteTargetStatus::Confirmed
                    | DpnsVoteTargetStatus::Rejected
                    | DpnsVoteTargetStatus::FailedBeforeSubmission
                    | DpnsVoteTargetStatus::Cancelled
                    | DpnsVoteTargetStatus::NotApplied => {
                        // An explicit Cast now action is a deliberate retry and
                        // may create a new operation below.
                    }
                }
            }
        }

        let target = self.dpns_vote_target(
            voter,
            &scheduled_vote.contested_name,
            scheduled_vote.choice,
            VoteTiming::Scheduled(scheduled_vote.unix_timestamp),
            false,
        )?;
        // An explicit retry must reach fresh preflight even if the cached
        // choice matches. Preflight records an already-satisfied target durably.
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            current_choice: None,
            ..target
        }]);
        for outcome in &mut operation.targets {
            outcome.status = DpnsVoteTargetStatus::Queued;
        }
        Ok(operation)
    }

    fn revalidate_dpns_vote_preflight(
        &self,
        operation: &mut DpnsVoteOperation,
        was_persisted: bool,
        key: &DpnsVoteTargetKey,
        state: DpnsCurrentVoteState,
    ) -> Result<(), TaskError> {
        if was_persisted {
            self.revalidate_queued_dpns_vote_target(operation.id, key, state)?;
            return Ok(());
        }
        let Some(outcome) = operation
            .targets
            .iter_mut()
            .find(|outcome| outcome.target.key == *key)
        else {
            return Ok(());
        };
        match state {
            DpnsCurrentVoteState::Available(current) => {
                if outcome.target.timing == VoteTiming::Now
                    && outcome.target.current_choice.is_none()
                    && current.is_some()
                    && current != Some(outcome.target.requested_choice)
                {
                    return Err(TaskError::DpnsVoteReviewRequired);
                }
                outcome.target.current_choice = current;
                if current == Some(outcome.target.requested_choice) {
                    outcome.status = DpnsVoteTargetStatus::Confirmed;
                    outcome.failure = None;
                }
            }
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => {
                (outcome.status, outcome.failure) =
                    unavailable_preflight_outcome(outcome.target.timing);
            }
        }
        Ok(())
    }

    fn validate_new_dpns_schedules(&self, targets: &[DpnsVoteTarget]) -> Result<(), TaskError> {
        if targets.is_empty() {
            return Ok(());
        }
        let now_ms = UNIX_EPOCH.elapsed().unwrap_or_default().as_millis() as u64;
        // Reject invalid times even when the contest has not been cached yet.
        for target in targets {
            if let VoteTiming::Scheduled(timestamp) = target.timing {
                validate_dpns_schedule_time(timestamp, now_ms, None)
                    .map_err(|_| TaskError::DpnsScheduledVoteInvalidTime)?;
            }
        }
        let contests = self.all_contested_names()?;
        for target in targets {
            let VoteTiming::Scheduled(timestamp) = target.timing else {
                continue;
            };
            let contest = contests
                .iter()
                .find(|contest| contest.normalized_contested_name == target.contested_name)
                .ok_or_else(|| TaskError::VotePollNotFound {
                    name: target.contested_name.clone(),
                })?;
            validate_dpns_schedule_edit(target.requested_choice, timestamp, now_ms, contest)
                .map_err(|error| match error {
                    DpnsScheduleEditValidationError::Time => {
                        TaskError::DpnsScheduledVoteInvalidTime
                    }
                    DpnsScheduleEditValidationError::Choice => {
                        TaskError::DpnsScheduledVoteInvalidChoice
                    }
                    DpnsScheduleEditValidationError::Contest => TaskError::VotePollNotFound {
                        name: target.contested_name.clone(),
                    },
                })?;
        }
        Ok(())
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
        let new_schedules = operation
            .targets
            .iter()
            .filter(|outcome| !was_persisted && outcome.status == DpnsVoteTargetStatus::Scheduled)
            .map(|outcome| outcome.target.clone())
            .collect::<Vec<_>>();
        self.validate_new_dpns_schedules(&new_schedules)?;
        let mut preflight_error = None;
        if operation
            .targets
            .iter()
            .any(|outcome| outcome.status == DpnsVoteTargetStatus::Queued)
        {
            // A fresh proved snapshot is a submission precondition. This also
            // prevents a due schedule from replaying a vote already observed.
            let refreshed = self.refresh_dpns_vote_states(sdk).await.map_err(Arc::new);
            let queued_keys = operation
                .targets
                .iter()
                .filter(|outcome| outcome.status == DpnsVoteTargetStatus::Queued)
                .map(|outcome| outcome.target.key.clone())
                .collect::<Vec<_>>();
            for key in queued_keys {
                let snapshot = match &refreshed {
                    Ok(results) => results
                        .get(&key.voter_id)
                        .ok_or_else(|| Arc::new(TaskError::DpnsCurrentVoteUnavailable))
                        .and_then(|result| result.as_ref().map_err(Arc::clone)),
                    Err(error) => Err(Arc::clone(error)),
                };
                let validation = self.with_dpns_vote_preflight_state(
                    key.voter_id,
                    key.vote_poll_id,
                    snapshot,
                    |state| {
                        matches!(state, DpnsCurrentVoteState::Available(_)).then(|| {
                            self.revalidate_dpns_vote_preflight(
                                &mut operation,
                                was_persisted,
                                &key,
                                state,
                            )
                        })
                    },
                );
                let source = match validation {
                    Ok(Some(result)) => {
                        result?;
                        continue;
                    }
                    Ok(None) => Arc::new(TaskError::DpnsCurrentVoteUnavailable),
                    Err(source) => source,
                };
                self.record_dpns_vote_diagnostic_with_dapi_context(
                    operation.id,
                    key.clone(),
                    TaskError::DpnsVotePreflightFailed {
                        source: Arc::clone(&source),
                    },
                    sdk,
                );
                self.revalidate_dpns_vote_preflight(
                    &mut operation,
                    was_persisted,
                    &key,
                    DpnsCurrentVoteState::Unavailable,
                )?;
                preflight_error.get_or_insert(source);
            }
        }

        if was_persisted {
            operation = self
                .dpns_vote_operation(operation.id)?
                .ok_or(TaskError::DpnsVoteOperationRecordMissing)?;
        } else {
            self.validate_new_dpns_schedules(&new_schedules)?;
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
            if let Some(error) = self.insert_dpns_vote_operation_with_scheduled_mirror(
                &mut operation,
                replacing_scheduled_key.as_ref(),
                &scheduled_votes,
            )? {
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
                        let mut scheduled_voter_missing = false;
                        for target in targets {
                            if app_context.claim_dpns_vote_target(operation_id, &target.key)? {
                                let (status, failure, retryable) =
                                    missing_voter_outcome(target.timing);
                                app_context.update_dpns_vote_target(
                                    operation_id,
                                    &target.key,
                                    status,
                                    failure,
                                )?;
                                scheduled_voter_missing |= retryable;
                            }
                        }
                        if scheduled_voter_missing {
                            return Err(TaskError::IdentityNotFoundLocally);
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
                                operation_id,
                                &target.key,
                                &target.contested_name,
                                target.requested_choice,
                                &voter,
                                &sdk,
                            )
                            .await;
                        let (status, failure) = classify_vote_attempt(&attempt, target.timing);
                        let confirmed = matches!(
                            &attempt,
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::Confirmed)
                        );
                        let mut retryable_scheduled_error = None;
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
                                app_context.record_dpns_vote_diagnostic_with_dapi_context(
                                    operation_id,
                                    target.key.clone(),
                                    error,
                                    &sdk,
                                );
                            }
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::Rejected(error)) => {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %target.key.voter_id,
                                    contested_name = %target.contested_name,
                                    "Platform rejected a DPNS vote"
                                );
                                app_context.record_dpns_vote_diagnostic_with_dapi_context(
                                    operation_id,
                                    target.key.clone(),
                                    error,
                                    &sdk,
                                );
                            }
                            Ok(vote_on_dpns_name::DpnsVoteAttempt::FailedBeforeSubmission(
                                error,
                            )) => {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %target.key.voter_id,
                                    contested_name = %target.contested_name,
                                    "DPNS vote failed before it reached the network"
                                );
                                if matches!(target.timing, VoteTiming::Scheduled(_)) {
                                    retryable_scheduled_error = Some(error);
                                } else {
                                    app_context.record_dpns_vote_diagnostic_with_dapi_context(
                                        operation_id,
                                        target.key.clone(),
                                        error,
                                        &sdk,
                                    );
                                }
                            }
                            Err(error) => {
                                tracing::warn!(
                                    ?error,
                                    voter_id = %target.key.voter_id,
                                    contested_name = %target.contested_name,
                                    "DPNS vote failed before a confirmed submission"
                                );
                                if matches!(target.timing, VoteTiming::Scheduled(_)) {
                                    retryable_scheduled_error = Some(error);
                                } else {
                                    app_context.record_dpns_vote_diagnostic_with_dapi_context(
                                        operation_id,
                                        target.key.clone(),
                                        error,
                                        &sdk,
                                    );
                                }
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
                        if let Some(error) = retryable_scheduled_error {
                            return Err(error);
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

        if let Some(source) = preflight_error {
            return Err(TaskError::DpnsVotePreflightFailed { source });
        }

        Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: self.network,
            operation_id: operation.id,
        })
    }

    async fn execute_dpns_vote_operation_with_recovery(
        self: &Arc<Self>,
        operation: DpnsVoteOperation,
        voters: Vec<QualifiedIdentity>,
        replacing_scheduled_key: Option<DpnsVoteTargetKey>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let operation_id = operation.id;
        let result = self
            .execute_dpns_vote_operation(operation, voters, replacing_scheduled_key, sdk)
            .await;
        if let Err(error) = &result {
            self.recover_failed_dpns_vote_operation(operation_id, error)
                .await;
        }
        result
    }

    async fn recover_failed_dpns_vote_operation(
        &self,
        operation_id: DpnsVoteOperationId,
        original_error: &TaskError,
    ) {
        if let Err(recovery_error) = self.recover_interrupted_dpns_vote_operation(operation_id) {
            tracing::error!(
                error = %recovery_error,
                original_error = %original_error,
                operation_id = %operation_id,
                "Failed to persist recovery for an interrupted DPNS vote operation"
            );
            *self.dpns_vote_recovery.lock().await = false;
        }
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
                Ok(votes) => {
                    let observed = votes
                        .get(&poll_id)
                        .and_then(Option::as_ref)
                        .map(ResourceVoteGettersV0::resource_vote_choice);
                    let reconciled_status =
                        classify_reconciled_vote(observed, outcome.target.requested_choice);
                    let status = reconciled_status.unwrap_or(DpnsVoteTargetStatus::Unconfirmed);
                    if !self.update_dpns_vote_reconciliation(
                        operation_id,
                        &outcome.target.key,
                        outcome.target.current_choice,
                        observed,
                        status,
                    )? {
                        continue;
                    }
                    if status != DpnsVoteTargetStatus::Confirmed {
                        continue;
                    }
                    if matches!(outcome.target.timing, VoteTiming::Scheduled(_))
                        && let Err(error) = self.mark_vote_executed(
                            outcome.target.key.voter_id.as_slice(),
                            outcome.target.contested_name.clone(),
                        )
                    {
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
                Err(error) => {
                    let error = TaskError::from(error);
                    tracing::warn!(
                        ?error,
                        operation_id = %operation_id,
                        voter_id = %outcome.target.key.voter_id,
                        contested_name = %outcome.target.contested_name,
                        "Could not reconcile an unconfirmed DPNS vote"
                    );
                    self.record_dpns_vote_diagnostic_with_dapi_context(
                        operation_id,
                        outcome.target.key.clone(),
                        error,
                        sdk,
                    );
                }
            }
        }
        Ok(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: self.network,
            operation_id,
        })
    }

    /// Preserve durable due targets and catch up after unrelated reconciliation.
    async fn prepare_due_scheduled_votes(
        &self,
        clock: impl Fn() -> u64,
        preserve_eligibility_since_ms: Option<u64>,
        reconcile: impl std::future::Future<Output = Result<(), TaskError>>,
    ) -> Result<(Vec<DpnsVoteOperation>, Vec<ScheduledDPNSVote>), TaskError> {
        let started_at = clock();
        let eligibility_cutoff = preserve_eligibility_since_ms.unwrap_or(started_at);
        // Persist entry admissions before the await so errors or interruption
        // cannot turn an eligible target into a stale schedule on the next sweep.
        self.queue_due_scheduled_votes_at(started_at, eligibility_cutoff)?;
        let reconciliation = reconcile.await;
        // Also admit schedules that became due while reconciliation was waiting.
        let admitted = self.queue_due_scheduled_votes_at(clock(), eligibility_cutoff)?;
        reconciliation?;
        Ok(admitted)
    }

    fn queue_due_scheduled_votes_at(
        &self,
        now_ms: u64,
        eligibility_cutoff: u64,
    ) -> Result<(Vec<DpnsVoteOperation>, Vec<ScheduledDPNSVote>), TaskError> {
        let mut due_operations = Vec::new();
        let mut in_progress = Vec::new();
        for mut operation in self.dpns_vote_operations()? {
            let mut due = false;
            for outcome in &mut operation.targets {
                let VoteTiming::Scheduled(scheduled_at) = outcome.target.timing else {
                    continue;
                };
                if !scheduled_target_should_execute(
                    outcome.status,
                    scheduled_at,
                    now_ms,
                    Some(eligibility_cutoff),
                ) {
                    continue;
                }
                if outcome.status == DpnsVoteTargetStatus::Scheduled
                    && !self.queue_scheduled_dpns_vote_target(operation.id, &outcome.target.key)?
                {
                    continue;
                }
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
            if due {
                due_operations.push(operation);
            }
        }
        Ok((due_operations, in_progress))
    }

    /// Reconcile unresolved votes and execute each admitted scheduled operation.
    /// Emits progress before execution; terminal results remain in the journal.
    async fn cast_due_scheduled_votes(
        self: &Arc<Self>,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
        preserve_eligibility_since_ms: Option<u64>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let (due_operations, in_progress) = self
            .prepare_due_scheduled_votes(
                || {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0)
                },
                preserve_eligibility_since_ms,
                async {
                    for operation in
                        self.dpns_vote_operations()?
                            .into_iter()
                            .filter(|operation| {
                                operation.targets.iter().any(|outcome| {
                                    outcome.status == DpnsVoteTargetStatus::Unconfirmed
                                })
                            })
                    {
                        let result = self
                            .reconcile_dpns_vote_operation(operation.id, sdk)
                            .await?;
                        let _ = sender.send(TaskResult::unattributed_success(result)).await;
                    }

                    Ok(())
                },
            )
            .await?;
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
                        .execute_dpns_vote_operation_with_recovery(operation, voters, None, &sdk)
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

fn preferred_operation_for_scheduled_key<'a>(
    operations: &'a [DpnsVoteOperation],
    key: &DpnsVoteTargetKey,
) -> Result<Option<&'a DpnsVoteOperation>, TaskError> {
    let mut lock_holders = operations.iter().filter(|operation| {
        operation
            .outcome(key)
            .is_some_and(|outcome| outcome.status.holds_lock())
    });
    let lock_holder = lock_holders.next();
    if lock_holders.next().is_some() {
        return Err(TaskError::DpnsVoteTargetBusy);
    }
    Ok(lock_holder.or_else(|| {
        operations
            .iter()
            .filter(|operation| {
                operation
                    .outcome(key)
                    .is_some_and(|outcome| !outcome.status.holds_lock())
            })
            .max_by_key(|operation| (operation.created_at, operation.id))
    }))
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
        && !dpns_schedule_is_overdue(scheduled_at_ms, eligibility_cutoff_ms)
}

fn scheduled_target_should_execute(
    status: DpnsVoteTargetStatus,
    scheduled_at_ms: u64,
    now_ms: u64,
    preserve_eligibility_since_ms: Option<u64>,
) -> bool {
    status == DpnsVoteTargetStatus::Queued
        || (status == DpnsVoteTargetStatus::Scheduled
            && scheduled_vote_is_due(
                scheduled_at_ms,
                false,
                now_ms,
                preserve_eligibility_since_ms,
            ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn dapi_connection_refused_error() -> TaskError {
        use dash_sdk::Error as SdkError;
        use dash_sdk::dapi_client::DapiClientError;
        use dash_sdk::dapi_client::transport::TransportError;

        let status = dash_sdk::dapi_grpc::tonic::Status::unavailable("tcp connect error");
        TaskError::from(SdkError::DapiClientError(DapiClientError::Transport(
            TransportError::Grpc(status),
        )))
    }

    fn qualified_identity(byte: u8) -> QualifiedIdentity {
        let identity = Identity::create_basic_identity(
            Identifier::from([byte; 32]),
            PlatformVersion::latest(),
        )
        .expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    fn r2_context() -> (tempfile::TempDir, Arc<AppContext>) {
        let temp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp.path());
        context.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        )));
        (temp, context)
    }

    fn r2_schedule(context: &AppContext, name: &str, timestamp: u64) -> DpnsVoteOperation {
        DpnsVoteOperation::new(vec![
            context
                .dpns_vote_target(
                    &qualified_identity(1),
                    name,
                    ResourceVoteChoice::Lock,
                    VoteTiming::Scheduled(timestamp),
                    false,
                )
                .unwrap(),
        ])
    }

    #[test]
    fn review_fixes_scheduled_retry_keeps_choice_after_different_immediate_success() {
        let (_temp, context) = r2_context();
        let mut previous = r2_schedule(&context, "alice", 42);
        previous.created_at = 1;
        previous.targets[0].status = DpnsVoteTargetStatus::Rejected;
        context
            .insert_dpns_vote_operation(&mut previous, None)
            .unwrap();
        let mut newer = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            requested_choice: ResourceVoteChoice::Abstain,
            timing: VoteTiming::Now,
            ..previous.targets[0].target.clone()
        }]);
        newer.created_at = 2;
        newer.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        context
            .insert_dpns_vote_operation(&mut newer, None)
            .unwrap();
        let retry = context
            .operation_for_scheduled_vote(
                &ScheduledDPNSVote {
                    voter_id: previous.targets[0].target.key.voter_id,
                    contested_name: "alice".into(),
                    choice: ResourceVoteChoice::Lock,
                    unix_timestamp: 42,
                    executed_successfully: false,
                },
                &qualified_identity(1),
            )
            .unwrap();
        assert_ne!(retry.id, newer.id);
        assert_eq!(
            retry.targets[0].target.requested_choice,
            ResourceVoteChoice::Lock
        );
        assert_eq!(retry.targets[0].status, DpnsVoteTargetStatus::Queued);
    }

    #[test]
    fn review_fixes_new_immediate_vote_requires_review_when_it_becomes_a_change() {
        let (_temp, context) = r2_context();
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            timing: VoteTiming::Now,
            ..r2_schedule(&context, "alice", 42).targets[0].target.clone()
        }]);
        let key = operation.targets[0].target.key.clone();
        assert!(
            context
                .revalidate_dpns_vote_preflight(
                    &mut operation,
                    false,
                    &key,
                    DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Abstain)),
                )
                .is_err(),
            "a newly required vote-change warning must return to review"
        );
        assert_eq!(operation.targets[0].target.current_choice, None);
        assert!(context.dpns_vote_operation(operation.id).unwrap().is_none());
    }

    #[tokio::test]
    async fn review_fixes_contest_query_does_not_require_readable_vote_journal() {
        use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
        use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
        use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
        use dash_sdk::query_types::{ContestedResource, ContestedResources};
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        let kv = crate::wallet_backend::DetKv::from_store(store.clone());
        let context = crate::context::test_support::test_app_context_with_kv(
            dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv);
        store.fail_next_gets_containing("det:dpns_vote_operations:v2:", 2);
        let mut sdk = Sdk::new_mock();
        let document_type = context
            .dpns_contract
            .document_type_for_name("domain")
            .unwrap();
        sdk.mock()
            .expect_fetch_many::<Identifier, ContestedResource, _, ContestedResources>(
                VotePollsByDocumentTypeQuery {
                    contract_id: context.dpns_contract.id(),
                    document_type_name: document_type.name().to_owned(),
                    index_name: document_type.find_contested_index().unwrap().name.clone(),
                    start_at_value: None,
                    start_index_values: vec!["dash".into()],
                    end_index_values: vec![],
                    limit: Some(100),
                    order_ascending: true,
                },
                Some(ContestedResources::default()),
            )
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());
        let result = context
            .run_contested_resource_task(ContestedResourceTask::QueryDPNSContests, &sdk, sender)
            .await;
        assert!(
            result.is_ok(),
            "independent contest reads must survive journal errors: {result:?}"
        );
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok()).any(|result| matches!(
                result, TaskResult::Success { result, .. }
                    if matches!(*result, BackendTaskSuccessResult::RefreshedDpnsContests)
            ))
        );
        assert!(
            context.ensure_dpns_vote_recovery(&sdk).await.is_err(),
            "vote recovery must still fail closed"
        );
    }

    #[test]
    fn review_fixes_preflight_preserves_matching_noop_and_reviewed_changes() {
        let (_temp, context) = r2_context();
        for (reviewed, observed, expected_status) in [
            (
                None,
                Some(ResourceVoteChoice::Lock),
                DpnsVoteTargetStatus::Confirmed,
            ),
            (
                Some(ResourceVoteChoice::Abstain),
                Some(ResourceVoteChoice::Abstain),
                DpnsVoteTargetStatus::Queued,
            ),
            (None, None, DpnsVoteTargetStatus::Queued),
        ] {
            let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
                timing: VoteTiming::Now,
                current_choice: reviewed,
                ..r2_schedule(&context, "alice", 42).targets[0].target.clone()
            }]);
            let key = operation.targets[0].target.key.clone();
            context
                .revalidate_dpns_vote_preflight(
                    &mut operation,
                    false,
                    &key,
                    DpnsCurrentVoteState::Available(observed),
                )
                .unwrap();
            assert_eq!(operation.targets[0].status, expected_status);
        }
    }

    #[test]
    fn r2_terminal_retry_with_matching_cache_reaches_durable_no_op_preflight() {
        let (_temp, context) = r2_context();
        let mut previous = r2_schedule(&context, "alice", 42);
        previous.targets[0].status = DpnsVoteTargetStatus::Rejected;
        context
            .insert_dpns_vote_operation(&mut previous, None)
            .unwrap();
        let key = previous.targets[0].target.key.clone();
        context
            .cache_confirmed_dpns_vote(key.voter_id, key.vote_poll_id, ResourceVoteChoice::Lock)
            .unwrap();
        let scheduled = ScheduledDPNSVote {
            voter_id: key.voter_id,
            contested_name: "alice".into(),
            choice: ResourceVoteChoice::Lock,
            unix_timestamp: 42,
            executed_successfully: false,
        };
        let mut retry = context
            .operation_for_scheduled_vote(&scheduled, &qualified_identity(1))
            .unwrap();
        assert_eq!(retry.targets.len(), 1);
        assert_eq!(
            retry.targets[0].status,
            DpnsVoteTargetStatus::Queued,
            "cache alone must not confirm a retry"
        );
        context
            .revalidate_dpns_vote_preflight(
                &mut retry,
                false,
                &key,
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock)),
            )
            .unwrap();
        context
            .insert_dpns_vote_operation_with_scheduled_mirror(&mut retry, None, &[scheduled])
            .unwrap();
        let saved = context.dpns_vote_operation(retry.id).unwrap().unwrap();
        assert_eq!(saved.targets[0].status, DpnsVoteTargetStatus::Confirmed);
        assert!(!saved.targets[0].status.holds_lock());
        assert_eq!(
            context
                .dpns_vote_operation(previous.id)
                .unwrap()
                .unwrap()
                .targets[0]
                .status,
            DpnsVoteTargetStatus::Rejected
        );
    }

    #[tokio::test]
    async fn r2_sweep_preserves_due_votes_across_slow_reconciliation() {
        let (_temp, context) = r2_context();
        let started = 1_000_000;
        let clock = std::cell::Cell::new(started);
        let mut operation = r2_schedule(&context, "alice", started);
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        let (due, _) = context
            .prepare_due_scheduled_votes(|| clock.get(), None, async {
                clock.set(started + SCHEDULED_VOTE_MAX_LATENESS_MS + 1);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(
            due.len(),
            1,
            "reconciliation must not expire a schedule eligible at sweep entry"
        );
        assert_eq!(
            context
                .dpns_vote_operation(operation.id)
                .unwrap()
                .unwrap()
                .targets[0]
                .status,
            DpnsVoteTargetStatus::Queued
        );
    }

    #[tokio::test]
    async fn r2_sweep_reconciliation_failure_retains_durable_due_admission() {
        let (_temp, context) = r2_context();
        let started = 1_000_000;
        let clock = std::cell::Cell::new(started);
        let mut due = r2_schedule(&context, "alice", started);
        let mut newly_due = r2_schedule(&context, "bob", started + 10);
        context.insert_dpns_vote_operation(&mut due, None).unwrap();
        context
            .insert_dpns_vote_operation(&mut newly_due, None)
            .unwrap();
        let result = context
            .prepare_due_scheduled_votes(|| clock.get(), None, async {
                clock.set(started + SCHEDULED_VOTE_MAX_LATENESS_MS * 2);
                Err(TaskError::DpnsVoteTargetBusy)
            })
            .await;
        assert!(matches!(result, Err(TaskError::DpnsVoteTargetBusy)));
        for id in [due.id, newly_due.id] {
            assert_eq!(
                context.dpns_vote_operation(id).unwrap().unwrap().targets[0].status,
                DpnsVoteTargetStatus::Queued,
                "failed reconciliation must retain entry and newly due admissions"
            );
        }
    }

    #[tokio::test]
    async fn r2_new_schedule_rejects_past_time_before_persistence() {
        let (_temp, context) = r2_context();
        let operation = r2_schedule(&context, "alice", 1);
        let result = context
            .execute_dpns_vote_operation(operation, vec![], None, &context.sdk())
            .await;
        assert!(matches!(
            result,
            Err(TaskError::DpnsScheduledVoteInvalidTime)
        ));
        assert!(context.dpns_vote_operations().unwrap().is_empty());
    }

    #[tokio::test]
    async fn r2_new_schedule_validates_known_deadline_and_unknown_deadline_policy() {
        let now = UNIX_EPOCH.elapsed().unwrap().as_millis() as u64;
        let end = now + 120_000;
        for (timestamp, deadline, expected_valid) in [
            (end - 1, Some(end), true),
            (end, Some(end), false),
            (end + 1, Some(end), false),
            (end, None, true),
        ] {
            let (_temp, context) = r2_context();
            context.seed_dpns_contest_for_test("alice", deadline, false);
            let operation = r2_schedule(&context, "alice", timestamp);
            let result = context
                .execute_dpns_vote_operation(operation, vec![], None, &context.sdk())
                .await;
            if expected_valid {
                result.unwrap();
                assert_eq!(context.dpns_vote_operations().unwrap().len(), 1);
            } else {
                assert!(matches!(
                    result,
                    Err(TaskError::DpnsScheduledVoteInvalidTime)
                ));
                assert!(context.dpns_vote_operations().unwrap().is_empty());
            }
        }
    }

    #[tokio::test]
    async fn r2_new_schedule_rejects_mixed_deadlines_atomically_and_missing_or_closed_contests() {
        let now = UNIX_EPOCH.elapsed().unwrap().as_millis() as u64;
        let timestamp = now + 60_000;
        let (_temp, context) = r2_context();
        context.seed_dpns_contest_for_test("alice", Some(timestamp + 1), false);
        context.seed_dpns_contest_for_test("bob", Some(timestamp), false);
        let mut targets = r2_schedule(&context, "alice", timestamp).targets;
        targets.extend(r2_schedule(&context, "bob", timestamp).targets);
        let operation = DpnsVoteOperation::new(targets.into_iter().map(|o| o.target).collect());
        let result = context
            .execute_dpns_vote_operation(operation, vec![], None, &context.sdk())
            .await;
        assert!(matches!(
            result,
            Err(TaskError::DpnsScheduledVoteInvalidTime)
        ));
        assert!(context.dpns_vote_operations().unwrap().is_empty());
        assert!(context.get_scheduled_votes().unwrap().is_empty());
        for name in ["missing", "closed"] {
            if name == "closed" {
                context.seed_dpns_contest_for_test(name, None, true);
            }
            let result = context
                .execute_dpns_vote_operation(
                    r2_schedule(&context, name, timestamp),
                    vec![],
                    None,
                    &context.sdk(),
                )
                .await;
            assert!(matches!(result, Err(TaskError::VotePollNotFound { .. })));
            assert!(context.dpns_vote_operations().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn r2_sweep_preserves_newly_due_but_excludes_already_stale_and_future_targets() {
        let (_temp, context) = r2_context();
        let started = 1_000_000;
        let finished = started + SCHEDULED_VOTE_MAX_LATENESS_MS * 2;
        let clock = std::cell::Cell::new(started);
        for (name, timestamp) in [
            ("newlydue", started + 10),
            ("stale", started - SCHEDULED_VOTE_MAX_LATENESS_MS - 1),
            ("future", finished + 1),
        ] {
            context
                .insert_dpns_vote_operation(&mut r2_schedule(&context, name, timestamp), None)
                .unwrap();
        }
        let (due, _) = context
            .prepare_due_scheduled_votes(|| clock.get(), None, async {
                clock.set(finished);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].targets[0].target.contested_name, "newlydue");
    }

    #[test]
    fn cast_now_durably_queues_an_existing_scheduled_operation() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let kv = crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        ));
        context.set_det_kv_override_for_test(kv);
        let voter = qualified_identity(1);
        let scheduled_vote = ScheduledDPNSVote {
            contested_name: "dominguez".to_owned(),
            voter_id: Identifier::from([1; 32]),
            choice: ResourceVoteChoice::Lock,
            unix_timestamp: 42,
            executed_successfully: false,
        };
        let target = context
            .dpns_vote_target(
                &voter,
                &scheduled_vote.contested_name,
                scheduled_vote.choice,
                VoteTiming::Scheduled(scheduled_vote.unix_timestamp),
                false,
            )
            .expect("scheduled target");
        let mut operation = DpnsVoteOperation::new(vec![target]);
        let operation_id = operation.id;
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .expect("persist scheduled operation");

        let returned = context
            .operation_for_scheduled_vote(&scheduled_vote, &voter)
            .expect("queue scheduled operation");
        let persisted = context
            .dpns_vote_operation(operation_id)
            .expect("load operation")
            .expect("persisted operation");

        assert_eq!(returned.targets[0].status, DpnsVoteTargetStatus::Queued);
        assert_eq!(persisted.targets[0].status, DpnsVoteTargetStatus::Queued);
    }

    #[test]
    fn scheduled_lookup_prefers_lock_holder_then_newest_terminal() {
        let key = DpnsVoteTargetKey {
            network: Network::Testnet,
            voter_id: Identifier::from([1; 32]),
            vote_poll_id: Identifier::from([2; 32]),
        };
        let target = DpnsVoteTarget {
            key: key.clone(),
            voter_alias: None,
            contested_name: "dominguez".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(42),
        };
        let mut older = DpnsVoteOperation::new(vec![target.clone()]);
        older.created_at = 1;
        older.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        let mut newer = DpnsVoteOperation::new(vec![target.clone()]);
        newer.created_at = 2;
        newer.targets[0].status = DpnsVoteTargetStatus::Cancelled;
        let mut lock_holder = DpnsVoteOperation::new(vec![target]);
        lock_holder.created_at = 0;
        lock_holder.targets[0].status = DpnsVoteTargetStatus::Scheduled;
        let operations = vec![older.clone(), lock_holder.clone(), newer.clone()];

        assert_eq!(
            preferred_operation_for_scheduled_key(&operations, &key)
                .unwrap()
                .unwrap()
                .id,
            lock_holder.id
        );

        let terminal_operations = vec![older, newer.clone()];
        assert_eq!(
            preferred_operation_for_scheduled_key(&terminal_operations, &key)
                .unwrap()
                .unwrap()
                .id,
            newer.id
        );
    }

    #[test]
    fn vote_diagnostic_contextualizes_exhausted_dapi_addresses() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = context.sdk();
        let addresses = sdk.address_list();
        let live_addresses = addresses.get_live_addresses();
        assert!(
            !live_addresses.is_empty(),
            "test SDK must have DAPI addresses"
        );
        for address in live_addresses {
            assert!(addresses.ban(&address));
        }

        let operation_id = DpnsVoteOperationId::from_bytes([3; 16]);
        let key = DpnsVoteTargetKey {
            network: Network::Testnet,
            voter_id: Identifier::from([4; 32]),
            vote_poll_id: Identifier::from([5; 32]),
        };
        context.record_dpns_vote_diagnostic_with_dapi_context(
            operation_id,
            key,
            dapi_connection_refused_error(),
            &sdk,
        );

        let diagnostics = context.dpns_vote_operation_diagnostics(operation_id);
        assert!(matches!(
            diagnostics.as_slice(),
            [error] if matches!(error.as_ref(), TaskError::DapiAllAddressesExhausted { .. })
        ));
    }

    #[test]
    fn preflight_diagnostics_preserve_network_cause_and_exhaustion_context() {
        let temp_dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = context.sdk();
        let addresses = sdk.address_list();
        for address in addresses.get_live_addresses() {
            assert!(addresses.ban(&address));
        }
        let source = Arc::new(dapi_connection_refused_error());
        let operation_id = DpnsVoteOperationId::from_bytes([3; 16]);
        context.record_dpns_vote_diagnostic_with_dapi_context(
            operation_id,
            DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([4; 32]),
                vote_poll_id: Identifier::from([5; 32]),
            },
            TaskError::DpnsVotePreflightFailed {
                source: Arc::clone(&source),
            },
            &sdk,
        );
        let diagnostics = context.dpns_vote_operation_diagnostics(operation_id);
        let TaskError::DapiAllAddressesExhausted {
            source: contextualized,
        } = diagnostics[0].as_ref()
        else {
            panic!("preflight diagnostics must explain exhausted addresses");
        };
        let TaskError::DpnsVotePreflightFailed { source: retained } = contextualized.as_ref()
        else {
            panic!("preflight diagnostics must retain the typed envelope");
        };
        assert!(Arc::ptr_eq(&source, retained));
        assert!(diagnostics[0].contains_dapi_reachability_failure());
    }

    /// VOTE-TC-033: an inner scheduled rejection is never classified as success.
    #[test]
    fn scheduled_inner_rejection_needs_attention() {
        let attempt = Ok(vote_on_dpns_name::DpnsVoteAttempt::Rejected(
            TaskError::DpnsVoteTargetBusy,
        ));
        assert_eq!(
            classify_vote_attempt(&attempt, VoteTiming::Scheduled(42)),
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
        let (status, _) = classify_vote_attempt(&attempt, VoteTiming::Scheduled(42));
        assert_eq!(status, DpnsVoteTargetStatus::Unconfirmed);
        assert!(status.holds_lock());
    }

    #[test]
    fn exact_reconciliation_confirms_only_the_requested_choice() {
        assert_eq!(
            classify_reconciled_vote(Some(ResourceVoteChoice::Lock), ResourceVoteChoice::Lock,),
            Some(DpnsVoteTargetStatus::Confirmed)
        );
        assert_eq!(
            classify_reconciled_vote(Some(ResourceVoteChoice::Abstain), ResourceVoteChoice::Lock,),
            None,
            "a mismatched row may predate the submitted transition and remains ambiguous"
        );
        assert_eq!(
            classify_reconciled_vote(None, ResourceVoteChoice::Lock,),
            None,
            "an absent exact row remains ambiguous and must not release its lock"
        );
    }

    #[test]
    fn repeated_mismatches_keep_an_ambiguous_vote_locked() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let kv = crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        ));
        context.set_det_kv_override_for_test(kv);
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: None,
            contested_name: "dominguez".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: Some(ResourceVoteChoice::Abstain),
            timing: VoteTiming::Now,
        }]);
        operation.targets[0].status = DpnsVoteTargetStatus::Confirming;
        let operation_id = operation.id;
        let key = operation.targets[0].target.key.clone();
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .expect("persist confirming operation");
        context
            .update_dpns_vote_target(
                operation_id,
                &key,
                DpnsVoteTargetStatus::Unconfirmed,
                Some(DpnsVoteFailure::ResultUnconfirmed),
            )
            .expect("persist unconfirmed operation");
        assert_eq!(
            context
                .dpns_vote_operation(operation_id)
                .unwrap()
                .unwrap()
                .targets[0]
                .target
                .current_choice,
            None,
            "the pre-broadcast choice must not count as reconciliation corroboration"
        );

        let observed = Some(ResourceVoteChoice::Abstain);
        let first_status = classify_reconciled_vote(observed, ResourceVoteChoice::Lock)
            .unwrap_or(DpnsVoteTargetStatus::Unconfirmed);
        assert!(
            context
                .update_dpns_vote_reconciliation(operation_id, &key, None, observed, first_status,)
                .expect("persist first observation")
        );
        assert_eq!(first_status, DpnsVoteTargetStatus::Unconfirmed);

        let persisted = context.dpns_vote_operation(operation_id).unwrap().unwrap();
        let second_status = classify_reconciled_vote(observed, ResourceVoteChoice::Lock)
            .unwrap_or(DpnsVoteTargetStatus::Unconfirmed);
        assert!(
            context
                .update_dpns_vote_reconciliation(
                    operation_id,
                    &key,
                    persisted.targets[0].target.current_choice,
                    observed,
                    second_status,
                )
                .expect("persist corroborated observation")
        );

        let target = &context
            .dpns_vote_operation(operation_id)
            .unwrap()
            .unwrap()
            .targets[0];
        assert_eq!(target.status, DpnsVoteTargetStatus::Unconfirmed);
        assert!(target.status.holds_lock());
        assert_eq!(
            context.dpns_vote_target_status(&key).unwrap(),
            Some(DpnsVoteTargetStatus::Unconfirmed)
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

    #[tokio::test]
    async fn scheduled_sweep_dispatch_wraps_recovery_failures() {
        use crate::app::TaskResult;
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::utils::tasks::TaskManager;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let database =
            Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("test database"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let context = AppContext::new(
            data_dir,
            Network::Testnet,
            database,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(1);
        let sender = SenderAsync::new(tx, context.egui_ctx().clone());

        let error = context
            .run_contested_resource_task(
                ContestedResourceTask::CastDueScheduledVotes {
                    preserve_eligibility_since_ms: None,
                },
                &context.sdk(),
                sender,
            )
            .await
            .expect_err("an unwired recovery must fail the scheduled sweep");

        assert!(matches!(
            error,
            TaskError::ScheduledVoteSweepFailed {
                network: Network::Testnet,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn additional_scenarios_failed_migration_retries_in_the_same_process() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        let kv = crate::wallet_backend::DetKv::from_store(store.clone());
        let context = crate::context::test_support::test_app_context_with_kv(
            dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv);
        store.fail_next_gets_containing("det:dpns_vote_operations:v2:", 1);
        assert!(
            context
                .ensure_dpns_vote_recovery(&context.sdk())
                .await
                .is_err()
        );
        assert!(!*context.dpns_vote_recovery.lock().await);
        context
            .ensure_dpns_vote_recovery(&context.sdk())
            .await
            .unwrap();
        assert!(*context.dpns_vote_recovery.lock().await);
    }

    #[tokio::test]
    async fn direct_dispatch_terminal_write_failure_rearms_global_recovery() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        let context = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(crate::wallet_backend::DetKv::from_store(store.clone())),
        );
        let (sender, _receiver) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(8);
        context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                sender,
                context.egui_ctx().clone(),
            ))
            .await
            .expect("wire wallet backend offline");
        context
            .set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(store.clone()));
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: None,
            contested_name: "dominguez".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Now,
        }]);
        operation.targets[0].status = DpnsVoteTargetStatus::Submitting;
        let operation_id = operation.id;
        let target_key = operation.targets[0].target.key.clone();
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .expect("persist in-flight operation");
        *context.dpns_vote_recovery.lock().await = true;
        store.fail_next_puts_containing(&operation_id.to_string(), 2);

        let terminal_error = context
            .update_dpns_vote_target(
                operation_id,
                &target_key,
                DpnsVoteTargetStatus::Confirmed,
                None,
            )
            .expect_err("terminal operation write must fail");
        context
            .recover_failed_dpns_vote_operation(operation_id, &terminal_error)
            .await;

        assert!(!*context.dpns_vote_recovery.lock().await);
        assert_eq!(
            context
                .dpns_vote_operation(operation_id)
                .unwrap()
                .unwrap()
                .targets[0]
                .status,
            DpnsVoteTargetStatus::Submitting
        );

        context
            .ensure_dpns_vote_recovery(&context.sdk())
            .await
            .expect("the re-armed recovery pass must succeed");
        assert_eq!(
            context
                .dpns_vote_operation(operation_id)
                .unwrap()
                .unwrap()
                .targets[0]
                .status,
            DpnsVoteTargetStatus::FailedBeforeSubmission
        );
    }

    #[test]
    fn scheduled_pre_broadcast_failure_remains_retryable() {
        let attempt = Err(TaskError::DpnsVoteTargetBusy);

        assert_eq!(
            classify_vote_attempt(&attempt, VoteTiming::Scheduled(42)),
            (
                DpnsVoteTargetStatus::Scheduled,
                Some(DpnsVoteFailure::SubmissionFailed),
            )
        );
        assert_eq!(
            classify_vote_attempt(&attempt, VoteTiming::Now),
            (
                DpnsVoteTargetStatus::FailedBeforeSubmission,
                Some(DpnsVoteFailure::SubmissionFailed),
            )
        );
    }

    #[test]
    fn scheduled_missing_voter_remains_retryable() {
        assert_eq!(
            missing_voter_outcome(VoteTiming::Scheduled(42)),
            (
                DpnsVoteTargetStatus::Scheduled,
                Some(DpnsVoteFailure::SubmissionFailed),
                true,
            )
        );
    }

    #[test]
    fn queued_schedule_remains_eligible_for_redrive() {
        let now_ms = 1_000_000;
        let stale_scheduled_at = now_ms - SCHEDULED_VOTE_MAX_LATENESS_MS - 1;

        assert!(scheduled_target_should_execute(
            DpnsVoteTargetStatus::Queued,
            stale_scheduled_at,
            now_ms,
            None,
        ));
        assert!(!scheduled_target_should_execute(
            DpnsVoteTargetStatus::Scheduled,
            stale_scheduled_at,
            now_ms,
            None,
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
