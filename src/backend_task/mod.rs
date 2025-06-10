use crate::app::TaskResult;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::system_task::SystemTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::tokens::tokens_screen::{
    ContractDescriptionInfo, IdentityTokenIdentifier, TokenInfo,
};
use crate::utils::egui_mpsc::SenderAsync;
use contested_names::ScheduledDPNSVote;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::group::group_action::GroupAction;
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, Identifier};
use dash_sdk::query_types::{Documents, IndexMap};
use futures::future::join_all;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokens::TokenTask;
use tokio::sync::mpsc;

pub mod broadcast_state_transition;
pub mod contested_names;
pub mod contract;
pub mod core;
pub mod document;
pub mod identity;
pub mod register_contract;
pub mod system_task;
pub mod tokens;
pub mod update_data_contract;

// TODO: Refactor how we handle errors and messages, and remove it from here
pub(crate) const NO_IDENTITIES_FOUND: &str = "No identities found";

#[derive(Debug, Clone, PartialEq)]
pub enum BackendTask {
    IdentityTask(IdentityTask),
    DocumentTask(Box<DocumentTask>),
    ContractTask(Box<ContractTask>),
    ContestedResourceTask(ContestedResourceTask),
    CoreTask(CoreTask),
    BroadcastStateTransition(StateTransition),
    TokenTask(Box<TokenTask>),
    SystemTask(SystemTask),
    None,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum BackendTaskSuccessResult {
    None,
    Refresh,
    Message(String),
    #[allow(dead_code)] // May be used for individual document operations
    Document(Document),
    Documents(Documents),
    BroadcastedDocument(Document),
    CoreItem(CoreItem),
    RegisteredIdentity(QualifiedIdentity),
    ToppedUpIdentity(QualifiedIdentity),
    #[allow(dead_code)] // May be used for reporting successful votes
    SuccessfulVotes(Vec<Vote>),
    DPNSVoteResults(Vec<(String, ResourceVoteChoice, Result<(), String>)>),
    CastScheduledVote(ScheduledDPNSVote),
    FetchedContract(DataContract),
    FetchedContracts(Vec<Option<DataContract>>),
    PageDocuments(IndexMap<Identifier, Option<Document>>, Option<Start>),
    #[allow(dead_code)] // May be used for token search results
    TokensByKeyword(Vec<TokenInfo>, Option<Start>),
    DescriptionsByKeyword(Vec<ContractDescriptionInfo>, Option<Start>),
    TokenEstimatedNonClaimedPerpetualDistributionAmount(IdentityTokenIdentifier, TokenAmount),
    ContractsWithDescriptions(
        BTreeMap<Identifier, (Option<ContractDescriptionInfo>, Vec<TokenInfo>)>,
    ),
    ActiveGroupActions(IndexMap<Identifier, GroupAction>),
    TokenPricing {
        token_id: Identifier,
        prices: Option<dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule>,
    },
}

impl BackendTaskSuccessResult {}

impl AppContext {
    /// Run backend tasks sequentially
    pub async fn run_backend_tasks_sequential(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: SenderAsync<TaskResult>,
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
        sender: SenderAsync<TaskResult>,
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
        sender: SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = {
            let guard = self.sdk.read().unwrap();
            guard.clone()
        };
        match task {
            BackendTask::ContractTask(contract_task) => {
                self.run_contract_task(*contract_task, &sdk, sender).await
            }
            BackendTask::ContestedResourceTask(contested_resource_task) => {
                self.run_contested_resource_task(contested_resource_task, &sdk, sender)
                    .await
            }
            BackendTask::IdentityTask(identity_task) => {
                self.run_identity_task(identity_task, &sdk, sender).await
            }
            BackendTask::DocumentTask(document_task) => {
                self.run_document_task(*document_task, &sdk).await
            }
            BackendTask::CoreTask(core_task) => self.run_core_task(core_task).await,
            BackendTask::BroadcastStateTransition(state_transition) => {
                self.broadcast_state_transition(state_transition, &sdk)
                    .await
            }
            BackendTask::TokenTask(token_task) => {
                self.run_token_task(*token_task, &sdk, sender).await
            }
            BackendTask::SystemTask(system_task) => self.run_system_task(system_task, sender).await,
            BackendTask::None => Ok(BackendTaskSuccessResult::None),
        }
    }
}
