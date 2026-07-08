mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use futures::future::join_all;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum ContestedResourceTask {
    QueryDPNSContests,
    VoteOnDPNSNames(Vec<(String, ResourceVoteChoice)>, Vec<QualifiedIdentity>),
    ScheduleDPNSVotes(Vec<ScheduledDPNSVote>),
    CastScheduledVote(ScheduledDPNSVote, Box<QualifiedIdentity>),
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
                                vec![(name.clone(), *vote_choice, Err(det_err.to_string()))]
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
}
