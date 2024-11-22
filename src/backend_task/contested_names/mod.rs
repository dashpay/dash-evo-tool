mod query_dpns_contested_resources;
mod query_dpns_vote_contenders;
mod query_ending_times;
mod vote_on_dpns_name;

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::Sdk;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContestedResourceTask {
    QueryDPNSContestedResources,
    QueryDPNSVoteContenders(String),
    VoteOnDPNSName(String, ResourceVoteChoice, Vec<QualifiedIdentity>),
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
            ContestedResourceTask::VoteOnDPNSName(name, vote_choice, voters) => {
                self.vote_on_dpns_name(name, *vote_choice, voters, sdk, sender)
                    .await
            }
        }
    }
}
