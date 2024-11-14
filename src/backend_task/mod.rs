use crate::app::TaskResult;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::document::DocumentTask;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::withdrawal_statuses::{WithdrawStatusPartialData, WithdrawalsTask};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::query_types::Documents;
use derive_more::From;
use std::sync::Arc;
use tokio::sync::mpsc;

pub mod contested_names;
pub mod contract;
pub mod core;
mod document;
pub mod identity;
pub mod withdrawal_statuses;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendTask {
    IdentityTask(IdentityTask),
    DocumentTask(DocumentTask),
    ContractTask(ContractTask),
    ContestedResourceTask(ContestedResourceTask),
    CoreTask(CoreTask),
    WithdrawalTask(WithdrawalsTask),
}

#[derive(Debug, Clone, PartialEq, From)]
pub(crate) enum BackendTaskSuccessResult {
    None,
    Message(String),
    Documents(Documents),
    CoreItem(CoreItem),
    RegisteredIdentity(QualifiedIdentity),
    SuccessfulVotes(Vec<Vote>),
    WithdrawalStatus(WithdrawStatusPartialData),
}

impl BackendTaskSuccessResult {}

impl AppContext {
    pub async fn run_backend_tasks(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        for task in tasks {
            self.run_backend_task(task, sender.clone()).await?;
        }
        Ok(())
    }

    pub async fn run_backend_task(
        self: &Arc<Self>,
        task: BackendTask,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = self.sdk.clone();
        match task {
            BackendTask::ContractTask(contract_task) => self
                .run_contract_task(contract_task, &sdk)
                .await
                .map(|_| BackendTaskSuccessResult::None),
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
            BackendTask::WithdrawalTask(withdrawal_task) => {
                self.run_withdraws_task(withdrawal_task, &sdk).await
            }
        }
    }
}
