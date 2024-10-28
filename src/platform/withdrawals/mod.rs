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
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::query_types::{Documents, KeysInPath, TotalCreditsInPlatform};
use dash_sdk::Sdk;
use crate::context::AppContext;
use crate::platform::BackendTaskSuccessResult;

/// constant id for subtree containing the sum of withdrawals
pub const WITHDRAWAL_TRANSACTIONS_SUM_AMOUNT_TREE_KEY: [u8; 1] = [2];

pub type WithdrawalsPerPage = u32;

pub type StartAfterIdentifier = Identifier;

pub type RequestRecentSumOfWithdrawals = bool;
pub type RequestTotalCreditsInPlatform = bool;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WithdrawalsTask {
    QueryWithdrawals(
        Vec<WithdrawalStatus>,
        WithdrawalsPerPage,
        Option<StartAfterIdentifier>,
        RequestRecentSumOfWithdrawals,
        RequestTotalCreditsInPlatform,
    ),
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithdrawRecord {
    pub date_time: DateTime<Local>,
    pub status: WithdrawalStatus,
    pub amount: Credits,
    pub owner_id: Identifier,
    pub document_id: Identifier,
    pub address: Address,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithdrawStatusPartialData {
    pub withdrawals: Vec<WithdrawRecord>,
    pub total_amount: Credits,
    pub recent_withdrawal_amounts: Option<SumValue>,
    pub daily_withdrawal_limit: Option<Credits>,
    pub total_credits_on_platform: Option<Credits>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WithdrawStatusData {
    pub withdrawals: Vec<WithdrawRecord>,
    pub total_amount: Credits,
    pub recent_withdrawal_amounts: SumValue,
    pub daily_withdrawal_limit: Credits,
    pub total_credits_on_platform: Credits,
}

impl WithdrawStatusData {
    pub fn merge_with_data(&mut self, partial: WithdrawStatusPartialData) {
        // Merge withdrawals (append new ones)
        self.withdrawals.extend(partial.withdrawals);

        // Reset total amount
        self.total_amount = self
            .withdrawals
            .iter()
            .map(|withdrawal| withdrawal.amount)
            .sum();

        // Merge recent withdrawal amounts if available
        if let Some(recent_amounts) = partial.recent_withdrawal_amounts {
            self.recent_withdrawal_amounts = recent_amounts;
        }

        // Merge daily withdrawal limit if available
        if let Some(daily_limit) = partial.daily_withdrawal_limit {
            self.daily_withdrawal_limit = daily_limit;
        }

        // Merge total credits on platform if available
        if let Some(total_credits) = partial.total_credits_on_platform {
            self.total_credits_on_platform = total_credits;
        }
    }
}

use std::convert::TryFrom;

#[derive(Debug)]
pub enum ConversionError {
    MissingField(&'static str),
}

impl TryFrom<WithdrawStatusPartialData> for WithdrawStatusData {
    type Error = ConversionError;

    fn try_from(partial: WithdrawStatusPartialData) -> Result<Self, Self::Error> {
        Ok(WithdrawStatusData {
            withdrawals: partial.withdrawals,
            total_amount: partial.total_amount,
            recent_withdrawal_amounts: partial
                .recent_withdrawal_amounts
                .ok_or(ConversionError::MissingField("recent_withdrawal_amounts"))?,
            daily_withdrawal_limit: partial
                .daily_withdrawal_limit
                .ok_or(ConversionError::MissingField("daily_withdrawal_limit"))?,
            total_credits_on_platform: partial
                .total_credits_on_platform
                .ok_or(ConversionError::MissingField("total_credits_on_platform"))?,
        })
    }
}

impl AppContext {
    pub async fn run_withdraws_task(
        self: &Arc<Self>,
        task: WithdrawalsTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = sdk.clone();
        match &task {
            WithdrawalsTask::QueryWithdrawals(
                status_filter,
                per_page_count,
                start_after,
                request_recent_sum_of_withdrawals,
                request_total_credits_in_platform,
            ) => {
                self.query_withdrawals(
                    sdk,
                    status_filter,
                    *per_page_count,
                    start_after,
                    *request_recent_sum_of_withdrawals,
                    *request_total_credits_in_platform,
                )
                .await
            }
        }
    }

    pub(super) async fn query_withdrawals(
        self: &Arc<Self>,
        sdk: Sdk,
        status_filter: &[WithdrawalStatus],
        per_page_count: WithdrawalsPerPage,
        start_after: &Option<StartAfterIdentifier>,
        request_recent_sum_of_withdrawals: bool,
        request_total_credits_in_platform: bool,
    ) -> Result<BackendTaskSuccessResult, String> {
        let queued_document_query = DocumentQuery {
            data_contract: self.withdraws_contract.clone(),
            document_type_name: "withdrawal".to_string(),
            where_clauses: vec![WhereClause {
                field: "status".to_string(),
                operator: WhereOperator::In,
                value: Value::Array(
                    status_filter
                        .iter()
                        .map(|status| (*status as u8).into())
                        .collect(),
                ),
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
            limit: per_page_count,
            start: start_after.map(|start| Start::StartAfter(start.to_vec())),
        };

        let documents = Document::fetch_many(&sdk, queued_document_query.clone())
            .await
            .map_err(|e| e.to_string())?;

        let recent_sum_of_withdrawals = if request_recent_sum_of_withdrawals {
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
                return Err(
                    "could not get sum tree value for current withdrawal maximum".to_string(),
                );
            };

            Some(*value)
        } else {
            None
        };

        let total_credits = if request_total_credits_in_platform {
            Some(
                TotalCreditsInPlatform::fetch_current(&sdk)
                    .await
                    .map_err(|e| e.to_string())?
                    .0,
            )
        } else {
            None
        };

        Ok(BackendTaskSuccessResult::WithdrawalStatus(
            util_transform_documents_to_withdrawal_status(
                recent_sum_of_withdrawals,
                total_credits,
                documents.into_values().filter_map(|a| a).collect(),
                sdk.network,
            )?,
        ))
    }
}

fn util_transform_documents_to_withdrawal_status(
    recent_withdrawal_amounts: Option<SumValue>,
    total_credits_on_platform: Option<Credits>,
    withdrawal_documents: Vec<Document>,
    network: Network,
) -> Result<WithdrawStatusPartialData, String> {
    let total_amount = withdrawal_documents.iter().try_fold(0, |acc, document| {
        document
            .properties()
            .get_integer::<Credits>(AMOUNT)
            .map_err(|_| "expected amount on withdrawal".to_string())
            .map(|amount| acc + amount)
    })?;

    let daily_withdrawal_limit = total_credits_on_platform
        .map(|total_credits_on_platform| {
            daily_withdrawal_limit(total_credits_on_platform, PlatformVersion::latest())
                .map_err(|_| "expected to get daily withdrawal limit".to_string())
        })
        .transpose()?;

    let mut vec_withdraws = vec![];
    for document in withdrawal_documents {
        vec_withdraws.push(util_convert_document_to_record(&document, network)?);
    }

    Ok(WithdrawStatusPartialData {
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
        document_id: document.id(),
        address,
    })
}
