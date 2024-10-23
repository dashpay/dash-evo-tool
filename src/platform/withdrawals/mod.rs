use std::sync::Arc;
use chrono::{DateTime, Local, Utc};
use dash_sdk::dashcore_rpc::dashcore::{Address, Network, ScriptBuf};
use dash_sdk::dpp::data_contracts::withdrawals_contract::v1::document_types::withdrawal::properties::{AMOUNT, OUTPUT_SCRIPT, STATUS};
use dash_sdk::dpp::data_contracts::withdrawals_contract::WithdrawalStatus;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::platform_value::btreemap_extensions::BTreeValueMapHelper;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::daily_withdrawal_limit::daily_withdrawal_limit;
use dash_sdk::drive::drive::RootTree;
use dash_sdk::drive::grovedb::Element;
use dash_sdk::drive::grovedb::element::SumValue;
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dash_sdk::query_types::{KeysInPath, TotalCreditsInPlatform};
use dash_sdk::Sdk;
use tokio::sync::mpsc;
use crate::app::TaskResult;
use crate::context::AppContext;
use crate::platform::BackendTaskSuccessResult;

/// constant key for transaction counter
pub const WITHDRAWAL_TRANSACTIONS_NEXT_INDEX_KEY: [u8; 1] = [0];
/// constant id for subtree containing transactions queue
pub const WITHDRAWAL_TRANSACTIONS_QUEUE_KEY: [u8; 1] = [1];
/// constant id for subtree containing the sum of withdrawals
pub const WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY: [u8; 1] = [2];
/// constant id for subtree containing the untied withdrawal transactions after they were broadcasted
pub const WITHDRAWAL_TRANSACTIONS_BROADCASTED_KEY: [u8; 1] = [3];

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WithdrawalsTask {
    QueryWithdrawals,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithdrawRecord {
    pub date_time: DateTime<Local>,
    pub status: WithdrawalStatus,
    pub amount: Credits,
    pub owner_id: Identifier,
    pub address: Address,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithdrawStatusData {
    pub withdrawals: Vec<WithdrawRecord>,
    pub total_amount: Credits,
    pub recent_withdrawal_amounts: SumValue,
    pub daily_withdrawal_limit: Credits,
    pub total_credits_on_platform: Credits,
}

impl AppContext {
    pub async fn run_withdraws_task(
        self: &Arc<Self>,
        task: WithdrawalsTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = sdk.clone();
        match &task {
            WithdrawalsTask::QueryWithdrawals => self.query_withdrawals(sdk, sender).await,
        }
    }

    pub(super) async fn query_withdrawals(
        self: &Arc<Self>,
        sdk: Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let queued_document_query = DocumentQuery {
            data_contract: self.withdraws_contract.clone(),
            document_type_name: "withdrawal".to_string(),
            where_clauses: vec![WhereClause {
                field: "status".to_string(),
                operator: WhereOperator::In,
                value: Value::Array(vec![
                    Value::U8(WithdrawalStatus::QUEUED as u8),
                    Value::U8(WithdrawalStatus::POOLED as u8),
                    Value::U8(WithdrawalStatus::BROADCASTED as u8),
                ]),
            }],
            order_by_clauses: vec![
                OrderClause {
                    field: "status".to_string(),
                    ascending: true,
                },
                OrderClause {
                    field: "transactionIndex".to_string(),
                    ascending: true,
                },
            ],
            limit: 100,
            start: None,
        };

        match Document::fetch_many(&sdk, queued_document_query.clone()).await {
            Ok(documents) => {
                let keys_in_path = KeysInPath {
                    path: vec![vec![RootTree::WithdrawalTransactions as u8]],
                    keys: vec![WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY.to_vec()],
                };
                match Element::fetch_many(&sdk, keys_in_path).await {
                    Ok(elements) => {
                        if let Some(Some(Element::SumTree(_, value, _))) =
                            elements.get(&WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY.to_vec())
                        {
                            match TotalCreditsInPlatform::fetch_current(&sdk).await {
                                Ok(total_credits) => {
                                    Ok(BackendTaskSuccessResult::WithdrawalStatus(
                                        util_transform_withdrawal_documents_to_bare_info(
                                            *value,
                                            total_credits.0,
                                            &documents.values().filter_map(|a| a.clone()).collect(),
                                            sdk.network,
                                        ),
                                    ))
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        } else {
                            Err(
                                "could not get sum tree value for current withdrawal maximum"
                                    .to_string(),
                            )
                        }
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

fn util_transform_withdrawal_documents_to_bare_info(
    recent_withdrawal_amounts: SumValue,
    total_credits_on_platform: Credits,
    withdrawal_documents: &Vec<Document>,
    network: Network,
) -> WithdrawStatusData {
    let total_amount: Credits = withdrawal_documents
        .iter()
        .map(|document| {
            document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .expect("expected amount on withdrawal")
        })
        .sum();
    let mut vec_withdraws = vec![];
    for document in withdrawal_documents.iter() {
        let index = document.created_at().expect("expected created at");
        // Convert the timestamp to a DateTime in UTC
        let utc_datetime =
            DateTime::<Utc>::from_timestamp_millis(index as i64).expect("expected date time");

        // Convert the UTC time to the local time zone
        let local_datetime: DateTime<Local> = utc_datetime.with_timezone(&Local);

        let amount = document
            .properties()
            .get_integer::<Credits>(AMOUNT)
            .expect("expected amount on withdrawal");
        let status: WithdrawalStatus = document
            .properties()
            .get_integer::<u8>(STATUS)
            .expect("expected amount on withdrawal")
            .try_into()
            .expect("expected a withdrawal status");
        let owner_id = document.owner_id();
        let address_bytes = document
            .properties()
            .get_bytes(OUTPUT_SCRIPT)
            .expect("expected output script");
        let output_script = ScriptBuf::from_bytes(address_bytes);
        let address = Address::from_script(&output_script, network).expect("expected an address");
        let withdraw_record = WithdrawRecord {
            date_time: local_datetime,
            status,
            amount,
            owner_id,
            address,
        };
        vec_withdraws.push(withdraw_record);
    }
    let daily_withdrawal_limit =
        daily_withdrawal_limit(total_credits_on_platform, PlatformVersion::latest())
            .expect("expected to get daily withdrawal limit");

    WithdrawStatusData {
        withdrawals: vec_withdraws,
        total_amount,
        recent_withdrawal_amounts,
        daily_withdrawal_limit,
        total_credits_on_platform,
    }
}
