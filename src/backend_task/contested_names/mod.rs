mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use dash_sdk::Sdk;
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

impl AppContext {
    pub async fn run_contested_resource_task(
        self: &Arc<Self>,
        task: ContestedResourceTask,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            ContestedResourceTask::QueryDPNSContests => self
                .query_dpns_contested_resources(sdk, sender)
                .await
                .map(|_| BackendTaskSuccessResult::None),
            ContestedResourceTask::VoteOnDPNSNames(votes, all_voters) => {
                // Create a vector of async closures that will vote on each name concurrently
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

                // Run all futures concurrently
                let results = join_all(futures).await;

                let final_results = results
                    .into_iter()
                    .flat_map(|(name, vote_choice, det_execution_result)| {
                        match det_execution_result {
                            Ok(BackendTaskSuccessResult::DPNSVoteResults(platform_results)) => {
                                // Voting succeeded in DET, return the Platform results
                                platform_results
                            }
                            Err(det_err_msg) => {
                                // Voting failed in DET, return the error message
                                vec![(name.clone(), *vote_choice, Err(det_err_msg))]
                            }
                            Ok(_) => {
                                // Got some other BackendTaskSuccessResult, this shouldn't occur
                                vec![(name.clone(), *vote_choice, Ok(()))]
                            }
                        }
                    })
                    .collect::<Vec<_>>();

                Ok(BackendTaskSuccessResult::DPNSVoteResults(final_results))
            }
            ContestedResourceTask::ScheduleDPNSVotes(scheduled_votes) => self
                .insert_scheduled_votes(scheduled_votes)
                .map(|_| BackendTaskSuccessResult::Message("Votes scheduled".to_string()))
                .map_err(|e| format!("Error inserting scheduled votes: {}", e)),
            ContestedResourceTask::CastScheduledVote(scheduled_vote, voter) => self
                .vote_on_dpns_name(
                    &scheduled_vote.contested_name,
                    scheduled_vote.choice,
                    &vec![(**voter).clone()],
                    sdk,
                    sender,
                )
                .await
                .map(|_| BackendTaskSuccessResult::CastScheduledVote(scheduled_vote.clone()))
                .map_err(|e| format!("Error casting scheduled vote: {}", e)),
            ContestedResourceTask::ClearAllScheduledVotes => self
                .clear_all_scheduled_votes()
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error clearing all scheduled votes: {}", e)),
            ContestedResourceTask::ClearExecutedScheduledVotes => self
                .clear_executed_scheduled_votes()
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error clearing executed scheduled votes: {}", e)),
            ContestedResourceTask::DeleteScheduledVote(voter_id, contested_name) => self
                .delete_scheduled_vote(voter_id.as_slice(), contested_name)
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error clearing scheduled vote: {}", e)),
        }
    }
}
