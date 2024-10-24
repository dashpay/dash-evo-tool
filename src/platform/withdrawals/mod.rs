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
use crate::context::AppContext;
use crate::platform::BackendTaskSuccessResult;

/// constant id for subtree containing the sum of withdrawals
pub const WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY: [u8; 1] = [2];

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WithdrawalsTask {
    QueryWithdrawals(Vec<Value>),
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
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = sdk.clone();
        match &task {
            WithdrawalsTask::QueryWithdrawals(status_filter) => self.query_withdrawals(sdk, status_filter).await,
        }
    }

    pub(super) async fn query_withdrawals(
        self: &Arc<Self>,
        sdk: Sdk,
        status_filter: &Vec<Value>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let queued_document_query = DocumentQuery {
            data_contract: self.withdraws_contract.clone(),
            document_type_name: "withdrawal".to_string(),
            where_clauses: vec![WhereClause {
                field: "status".to_string(),
                operator: WhereOperator::In,
                value: Value::Array(status_filter.to_vec()),
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

        let documents = Document::fetch_many(&sdk, queued_document_query.clone())
            .await
            .map_err(|e| e.to_string())?;

        let keys_in_path = KeysInPath {
            path: vec![vec![RootTree::WithdrawalTransactions as u8]],
            keys: vec![WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY.to_vec()],
        };

        let elements = Element::fetch_many(&sdk, keys_in_path)
            .await
            .map_err(|e| e.to_string())?;

        let sum_tree_element_option = elements
            .get(&WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY.to_vec())
            .ok_or_else(|| {
                "could not get sum tree value for current withdrawal maximum".to_string()
            })?;

        let sum_tree_element = sum_tree_element_option.as_ref().ok_or_else(|| {
            "could not get sum tree value for current withdrawal maximum".to_string()
        })?;

        let value = if let Element::SumTree(_, value, _) = sum_tree_element {
            value
        } else {
            return Err("could not get sum tree value for current withdrawal maximum".to_string());
        };

        let total_credits = TotalCreditsInPlatform::fetch_current(&sdk)
            .await
            .map_err(|e| e.to_string())?;

        Ok(BackendTaskSuccessResult::WithdrawalStatus(
            util_transform_documents_to_withdrawal_status(
                *value,
                total_credits.0,
                &documents.values().filter_map(|a| a.clone()).collect(),
                sdk.network,
            )?,
        ))
    }
}

fn util_transform_documents_to_withdrawal_status(
    recent_withdrawal_amounts: SumValue,
    total_credits_on_platform: Credits,
    withdrawal_documents: &Vec<Document>,
    network: Network,
) -> Result<WithdrawStatusData, String> {
    let total_amount = withdrawal_documents.iter().try_fold(0, |acc, document| {
        document
            .properties()
            .get_integer::<Credits>(AMOUNT)
            .map_err(|_| "expected amount on withdrawal".to_string())
            .map(|amount| acc + amount)
    })?;

    let daily_withdrawal_limit =
        daily_withdrawal_limit(total_credits_on_platform, PlatformVersion::latest())
            .map_err(|_| "expected to get daily withdrawal limit".to_string())?;

    let mut vec_withdraws = vec![];
    for document in withdrawal_documents {
        vec_withdraws.push(util_convert_document_to_record(document, network)?);
    }

    Ok(WithdrawStatusData {
        withdrawals: vec_withdraws,
        total_amount,
        recent_withdrawal_amounts,
        daily_withdrawal_limit,
        total_credits_on_platform,
    })
}

fn util_convert_document_to_record(
    document: &Document,
    network: Network,
) -> Result<WithdrawRecord, String> {
    let index = document
        .created_at()
        .ok_or_else(|| "expected created at".to_string())?;

    let local_datetime: DateTime<Local> = DateTime::<Utc>::from_timestamp_millis(index as i64)
        .ok_or_else(|| "expected date time".to_string())?
        .into();

    let amount = document
        .properties()
        .get_integer::<Credits>(AMOUNT)
        .map_err(|_| "expected amount on withdrawal".to_string())?;

    let status_int = document
        .properties()
        .get_integer::<u8>(STATUS)
        .map_err(|_| "expected status on withdrawal".to_string())?;

    let status: WithdrawalStatus = status_int
        .try_into()
        .map_err(|_| "invalid withdrawal status".to_string())?;

    let owner_id = document.owner_id();

    let address_bytes = document
        .properties()
        .get_bytes(OUTPUT_SCRIPT)
        .map_err(|_| "expected output script".to_string())?;

    let output_script = ScriptBuf::from_bytes(address_bytes);

    let address = Address::from_script(&output_script, network)
        .map_err(|_| "expected a valid address".to_string())?;

    Ok(WithdrawRecord {
        date_time: local_datetime,
        status,
        amount,
        owner_id,
        address,
    })
}
