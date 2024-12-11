mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
pub mod schedule_dpns_vote;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::Sdk;
use schedule_dpns_vote::ScheduledDPNSVote;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContestedResourceTask {
    QueryDPNSContestedResources,
    QueryDPNSVoteContenders(String),
    ScheduleDPNSVote(Vec<ScheduledDPNSVote>),
    ExecuteScheduledVote(ScheduledDPNSVote, QualifiedIdentity),
    VoteOnDPNSName(String, ResourceVoteChoice, Vec<QualifiedIdentity>),
    ClearAllScheduledVotes,
    ClearExecutedScheduledVotes,
    DeleteScheduledVote(Vec<u8>, String),
}

impl AppContext {
    pub async fn run_contested_resource_task(
        self: &Arc<Self>,
        task: ContestedResourceTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            ContestedResourceTask::QueryDPNSContestedResources => self
                .query_dpns_contested_resources(sdk, sender)
                .await
                .map(|_| BackendTaskSuccessResult::None),
            ContestedResourceTask::QueryDPNSVoteContenders(name) => self
                .query_dpns_vote_contenders(name, sdk, sender)
                .await
                .map(|_| BackendTaskSuccessResult::None),
            ContestedResourceTask::ScheduleDPNSVote(scheduled_votes) => {
                self.schedule_dpns_vote(scheduled_votes).await
            }
            ContestedResourceTask::ExecuteScheduledVote(scheduled_vote, voter) => self
                .vote_on_dpns_name(
                    &scheduled_vote.contested_name,
                    scheduled_vote.choice,
                    &vec![voter.clone()],
                    sdk,
                    sender,
                )
                .await
                .map(|result| match result {
                    BackendTaskSuccessResult::SuccessfulVotes(_) => {
                        BackendTaskSuccessResult::CastScheduledVote(scheduled_vote.clone())
                    }
                    _ => BackendTaskSuccessResult::CastScheduledVote(scheduled_vote.clone()),
                })
                .map_err(|e| format!("Error casting scheduled vote: {}", e.to_string())),
            ContestedResourceTask::VoteOnDPNSName(name, vote_choice, voters) => {
                self.vote_on_dpns_name(name, *vote_choice, voters, sdk, sender)
                    .await
            }
            ContestedResourceTask::ClearAllScheduledVotes => self
                .db
                .clear_all_scheduled_votes(self)
                .map(|_| BackendTaskSuccessResult::SuccessfulVotes(vec![])) // this one refreshes
                .map_err(|e| format!("Error clearing all scheduled votes: {}", e.to_string())),
            ContestedResourceTask::ClearExecutedScheduledVotes => self
                .db
                .clear_executed_past_scheduled_votes(self)
                .map(|_| BackendTaskSuccessResult::SuccessfulVotes(vec![])) // this one refreshes
                .map_err(|e| format!("Error clearing executed scheduled votes: {}", e.to_string())),
            ContestedResourceTask::DeleteScheduledVote(voter_id, contested_name) => self
                .db
                .delete_scheduled_vote(voter_id, contested_name, self)
                .map(|_| BackendTaskSuccessResult::SuccessfulVotes(vec![])) // this one refreshes
                .map_err(|e| format!("Error clearing scheduled vote: {}", e.to_string())),
        }
    }
}
