use crate::app::TaskResult;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use contested_names::ScheduledDPNSVote;
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, Identifier};
use dash_sdk::query_types::{Documents, IndexMap};
use futures::future::join_all;
use std::sync::Arc;
use tokio::sync::mpsc;

pub mod broadcast_state_transition;
pub mod contested_names;
pub mod contract;
pub mod core;
pub mod document;
pub mod identity;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendTask {
    IdentityTask(IdentityTask),
    DocumentTask(DocumentTask),
    ContractTask(ContractTask),
    ContestedResourceTask(ContestedResourceTask),
    CoreTask(CoreTask),
    BroadcastStateTransition(StateTransition),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendTaskSuccessResult {
    None,
    Refresh,
    Message(String),
    Documents(Documents),
    CoreItem(CoreItem),
    RegisteredIdentity(QualifiedIdentity),
    ToppedUpIdentity(QualifiedIdentity),
    SuccessfulVotes(Vec<Vote>),
    DPNSVoteResults(Vec<(String, ResourceVoteChoice, Result<(), String>)>),
    CastScheduledVote(ScheduledDPNSVote),
    FetchedContract(DataContract),
    FetchedContracts(Vec<Option<DataContract>>),
    PageDocuments(IndexMap<Identifier, Option<Document>>, Option<Start>),
}

impl BackendTaskSuccessResult {}

impl AppContext {
    /// Run backend tasks sequentially
    pub async fn run_backend_tasks_sequential(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: mpsc::Sender<TaskResult>,
    ) -> Vec<Result<BackendTaskSuccessResult, String>> {
        let mut results = Vec::new();
        for task in tasks {
            match self.run_backend_task(task, sender.clone()).await {
                Ok(result) => results.push(Ok(result)),
                Err(e) => results.push(Err(e)),
            };
        }
        results
    }

    /// Run backend tasks concurrently
    pub async fn run_backend_tasks_concurrent(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: mpsc::Sender<TaskResult>,
    ) -> Vec<Result<BackendTaskSuccessResult, String>> {
        let futures = tasks
            .into_iter()
            .map(|task| {
                let cloned_self = Arc::clone(self);
                let cloned_sender = sender.clone();
                async move { cloned_self.run_backend_task(task, cloned_sender).await }
            })
            .collect::<Vec<_>>();

        // Wait for all to finish before returning
        join_all(futures).await
    }

    pub async fn run_backend_task(
        self: &Arc<Self>,
        task: BackendTask,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = self.sdk.clone();
        match task {
            BackendTask::ContractTask(contract_task) => {
                self.run_contract_task(contract_task, &sdk).await
            }
            BackendTask::ContestedResourceTask(contested_resource_task) => {
                self.run_contested_resource_task(contested_resource_task, &sdk, sender)
                    .await
            }
            BackendTask::IdentityTask(identity_task) => {
                self.run_identity_task(identity_task, &sdk, sender).await
            }
            BackendTask::DocumentTask(document_task) => {
                self.run_document_task(document_task, &sdk).await
            }
            BackendTask::CoreTask(core_task) => self.run_core_task(core_task).await,
            BackendTask::BroadcastStateTransition(state_transition) => {
                self.broadcast_state_transition(state_transition, &sdk)
                    .await
            }
        }
    }
}
