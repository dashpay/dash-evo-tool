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
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContestedResourceTask {
    QueryDPNSContestedResources,
    VoteOnDPNSName(String, ResourceVoteChoice, Vec<QualifiedIdentity>),
    ScheduleDPNSVotes(Vec<ScheduledDPNSVote>),
    CastScheduledVote(ScheduledDPNSVote, QualifiedIdentity),
    ClearAllScheduledVotes,
    ClearExecutedScheduledVotes,
    DeleteScheduledVote(Vec<u8>, String),
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
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            ContestedResourceTask::QueryDPNSContestedResources => self
                .query_dpns_contested_resources(sdk, sender)
                .await
                .map(|_| BackendTaskSuccessResult::None),
            ContestedResourceTask::VoteOnDPNSName(name, vote_choice, voters) => {
                self.vote_on_dpns_name(name, *vote_choice, voters, sdk, sender)
                    .await
            }
            ContestedResourceTask::ScheduleDPNSVotes(scheduled_votes) => self
                .db
                .insert_scheduled_votes(self, scheduled_votes)
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error inserting scheduled votes: {}", e.to_string())),
            ContestedResourceTask::CastScheduledVote(scheduled_vote, voter) => self
                .vote_on_dpns_name(
                    &scheduled_vote.contested_name,
                    scheduled_vote.choice,
                    &vec![voter.clone()],
                    sdk,
                    sender,
                )
                .await
                .map(|_| BackendTaskSuccessResult::CastScheduledVote(scheduled_vote.clone()))
                .map_err(|e| format!("Error casting scheduled vote: {}", e.to_string())),
            ContestedResourceTask::ClearAllScheduledVotes => self
                .db
                .clear_all_scheduled_votes(self)
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error clearing all scheduled votes: {}", e.to_string())),
            ContestedResourceTask::ClearExecutedScheduledVotes => self
                .db
                .clear_executed_past_scheduled_votes(self)
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error clearing executed scheduled votes: {}", e.to_string())),
            ContestedResourceTask::DeleteScheduledVote(voter_id, contested_name) => self
                .db
                .delete_scheduled_vote(voter_id, contested_name, self)
                .map(|_| BackendTaskSuccessResult::Refresh)
                .map_err(|e| format!("Error clearing scheduled vote: {}", e.to_string())),
        }
    }
}
