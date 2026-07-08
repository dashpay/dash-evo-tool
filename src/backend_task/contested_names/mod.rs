mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::request_type::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use futures::future::join_all;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Widest window past a scheduled vote's time that the sweep will still cast it.
/// Beyond this a due vote is considered stale and left for the user to reschedule
/// rather than cast late (mirrors the original 2-minute UI-poll grace).
const SCHEDULED_VOTE_MAX_LATENESS_MS: u64 = 120_000;

#[derive(Debug, Clone, PartialEq)]
pub enum ContestedResourceTask {
    QueryDPNSContests,
    VoteOnDPNSNames(Vec<(String, ResourceVoteChoice)>, Vec<QualifiedIdentity>),
    ScheduleDPNSVotes(Vec<ScheduledDPNSVote>),
    CastScheduledVote(ScheduledDPNSVote, Box<QualifiedIdentity>),
    /// Sweep the scheduled-vote table and cast every vote that is now due. The
    /// periodic UI tick dispatches this so the DB query, identity load, and
    /// casting all run off the frame thread.
    CastDueScheduledVotes,
    ClearAllScheduledVotes,
    ClearExecutedScheduledVotes,
    DeleteScheduledVote(Identifier, String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledDPNSVote {
    pub contested_name: String,
    pub voter_id: Identifier,
    pub choice: ResourceVoteChoice,
    pub unix_timestamp: u64,
    pub executed_successfully: bool,
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
            ContestedResourceTask::VoteOnDPNSNames(votes, all_voters) => {
                let all_voters = &all_voters;
                let futures = votes
                    .iter()
                    .map(|(name, choice)| {
                        let cloned_sender = sender.clone();
                        let app_context = self.clone();

                        async move {
                            let result = app_context
                                .vote_on_dpns_name(name, *choice, all_voters, sdk, cloned_sender)
                                .await;

                            (name, choice, result)
                        }
                    })
                    .collect::<Vec<_>>();

                let results = join_all(futures).await;

                let final_results = results
                    .into_iter()
                    .flat_map(
                        |(name, vote_choice, det_execution_result)| match det_execution_result {
                            Ok(BackendTaskSuccessResult::DPNSVoteResults(platform_results)) => {
                                platform_results
                            }
                            Err(det_err) => {
                                vec![(
                                    name.clone(),
                                    *vote_choice,
                                    Err(std::sync::Arc::new(det_err)),
                                )]
                            }
                            Ok(_) => {
                                vec![(name.clone(), *vote_choice, Ok(()))]
                            }
                        },
                    )
                    .collect::<Vec<_>>();

                Ok(BackendTaskSuccessResult::DPNSVoteResults(final_results))
            }
            ContestedResourceTask::ScheduleDPNSVotes(scheduled_votes) => {
                self.insert_scheduled_votes(&scheduled_votes)?;
                Ok(BackendTaskSuccessResult::ScheduledVotes)
            }
            ContestedResourceTask::CastScheduledVote(scheduled_vote, voter) => {
                self.vote_on_dpns_name(
                    &scheduled_vote.contested_name,
                    scheduled_vote.choice,
                    &[*voter],
                    sdk,
                    sender,
                )
                .await?;
                Ok(BackendTaskSuccessResult::CastScheduledVote(scheduled_vote))
            }
            ContestedResourceTask::CastDueScheduledVotes => {
                self.cast_due_scheduled_votes(sdk, sender).await
            }
            ContestedResourceTask::ClearAllScheduledVotes => {
                self.clear_all_scheduled_votes()?;
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
        }
    }

    /// Cast every scheduled vote that is now due, off the UI thread.
    ///
    /// Queries the scheduled-vote table, keeps the votes whose time has arrived
    /// (and are not already executed or stale beyond
    /// [`SCHEDULED_VOTE_MAX_LATENESS_MS`]), pairs each with its local voting
    /// identity, and casts them independently so one failure cannot abort the
    /// rest. Emits [`ScheduledVotesInProgress`] before casting and a
    /// [`CastScheduledVote`] per success so the Scheduled Votes screen can
    /// reflect progress via `display_task_result`.
    ///
    /// [`ScheduledVotesInProgress`]: BackendTaskSuccessResult::ScheduledVotesInProgress
    /// [`CastScheduledVote`]: BackendTaskSuccessResult::CastScheduledVote
    async fn cast_due_scheduled_votes(
        self: &Arc<Self>,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let due: Vec<ScheduledDPNSVote> = self
            .get_scheduled_votes()?
            .into_iter()
            .filter(|v| {
                !v.executed_successfully
                    && v.unix_timestamp <= now_ms
                    && v.unix_timestamp + SCHEDULED_VOTE_MAX_LATENESS_MS >= now_ms
            })
            .collect();
        if due.is_empty() {
            return Ok(BackendTaskSuccessResult::None);
        }

        let voters = self.load_local_voting_identities()?;
        let mut castable: Vec<(ScheduledDPNSVote, QualifiedIdentity)> = Vec::new();
        for vote in due {
            match voters.iter().find(|i| i.identity.id() == vote.voter_id) {
                Some(voter) => castable.push((vote, voter.clone())),
                None => {
                    tracing::warn!(
                        contested_name = %vote.contested_name,
                        "No local voting identity for a scheduled vote; skipping it"
                    );
                }
            }
        }
        if castable.is_empty() {
            return Ok(BackendTaskSuccessResult::None);
        }

        // Tell the Scheduled Votes screen which votes are now in flight.
        let in_progress = castable.iter().map(|(v, _)| v.clone()).collect();
        let _ = sender
            .send(TaskResult::Success(Box::new(
                BackendTaskSuccessResult::ScheduledVotesInProgress(in_progress),
            )))
            .await;

        for (vote, voter) in castable {
            match self
                .vote_on_dpns_name(
                    &vote.contested_name,
                    vote.choice,
                    &[voter],
                    sdk,
                    sender.clone(),
                )
                .await
            {
                Ok(_) => {
                    let _ = sender
                        .send(TaskResult::Success(Box::new(
                            BackendTaskSuccessResult::CastScheduledVote(vote),
                        )))
                        .await;
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        contested_name = %vote.contested_name,
                        "Failed to cast a due scheduled vote; leaving it for the next sweep"
                    );
                }
            }
        }
        Ok(BackendTaskSuccessResult::None)
    }
}
