use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::block::extended_epoch_info::{v0::ExtendedEpochInfoV0Getters, ExtendedEpochInfo};
use dash_sdk::dpp::core_types::validator_set::v0::ValidatorSetV0Getters;
use dash_sdk::dpp::dashcore::{Address, Network, ScriptBuf};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::data_contracts::withdrawals_contract::v1::document_types::withdrawal::properties::{
    AMOUNT, STATUS, TRANSACTION_INDEX,
};
use dash_sdk::dpp::data_contracts::withdrawals_contract::WithdrawalStatus;
use dash_sdk::dpp::data_contracts::SystemDataContract;
use dash_sdk::dpp::document::{Document, DocumentV0Getters};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::platform_value::btreemap_extensions::BTreeValueMapHelper;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::state_transition::identity_credit_withdrawal_transition::fields::OUTPUT_SCRIPT;
use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::daily_withdrawal_limit::daily_withdrawal_limit;
use dash_sdk::dpp::{dash_to_credits, version::ProtocolVersionVoteCount};
use dash_sdk::drive::query::{OrderClause, WhereClause, WhereOperator};
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dash_sdk::platform::{DocumentQuery, FetchMany, FetchUnproved};
use dash_sdk::query_types::{
    CurrentQuorumsInfo, NoParamQuery, ProtocolVersionUpgrades, TotalCreditsInPlatform,
};
use itertools::Itertools;
use std::sync::Arc;
use chrono::{prelude::*, LocalResult};
use chrono_humanize::{Accuracy, HumanTime, Tense};

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformInfoTaskRequestType {
    CurrentEpochInfo,
    TotalCreditsOnPlatform,
    CurrentVersionVotingState,
    CurrentValidatorSetInfo,
    CurrentWithdrawalsInQueue,
    RecentlyCompletedWithdrawals,
    BasicPlatformInfo,
}

#[derive(Debug, Clone)]
pub enum PlatformInfoTaskResult {
    BasicPlatformInfo {
        platform_version: &'static PlatformVersion,
        core_chain_lock_height: Option<u32>,
        network: dash_sdk::dpp::dashcore::Network,
    },
    TextResult(String),
}

impl PartialEq for PlatformInfoTaskResult {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                PlatformInfoTaskResult::BasicPlatformInfo {
                    core_chain_lock_height: height1,
                    network: network1,
                    ..
                },
                PlatformInfoTaskResult::BasicPlatformInfo {
                    core_chain_lock_height: height2,
                    network: network2,
                    ..
                },
            ) => height1 == height2 && network1 == network2,
            (
                PlatformInfoTaskResult::TextResult(text1),
                PlatformInfoTaskResult::TextResult(text2),
            ) => text1 == text2,
            _ => false,
        }
    }
}

// Helper functions for formatting platform data
fn format_extended_epoch_info(
    epoch_info: ExtendedEpochInfo,
    network: Network,
    is_current: bool,
) -> String {
    let readable_epoch_start_time_as_time_away =
        match Utc.timestamp_millis_opt(epoch_info.first_block_time() as i64) {
            LocalResult::None => String::new(),
            LocalResult::Single(block_time) => {
                let now = Utc::now();
                let duration = now.signed_duration_since(block_time);
                HumanTime::from(duration).to_text_en(Accuracy::Precise, Tense::Past)
            }
            LocalResult::Ambiguous(..) => String::new(),
        };

    let epoch_estimated_time = match network {
        Network::Dash => 788_400_000,
        Network::Testnet => 3_600_000,
        Network::Devnet => 3_600_000,
        Network::Regtest => 1_200_000,
        _ => 3_600_000,
    };

    let readable_epoch_end_time = match Utc
        .timestamp_millis_opt(epoch_info.first_block_time() as i64 + epoch_estimated_time as i64)
    {
        LocalResult::None => String::new(),
        LocalResult::Single(block_time) => {
            let now = Utc::now();
            let duration = block_time.signed_duration_since(now);

            if duration.num_milliseconds() >= 0 {
                HumanTime::from(duration).to_text_en(Accuracy::Precise, Tense::Future)
            } else {
                HumanTime::from(-duration).to_text_en(Accuracy::Precise, Tense::Past)
            }
        }
        LocalResult::Ambiguous(..) => String::new(),
    };

    let in_string = if is_current { "Current " } else { "" };

    format!(
        "{}Epoch Information:\n\
         • Protocol Version: {}\n\
         • Epoch Index: {}\n\
         • Start Height: {}\n\
         • Start Core Height: {}\n\
         • Start Time: {} ({})\n\
         • Estimated End Time: {}\n\
         • Fee Multiplier: {}",
        in_string,
        epoch_info.protocol_version(),
        epoch_info.index(),
        epoch_info.first_block_height(),
        epoch_info.first_core_block_height(),
        epoch_info.first_block_time(),
        readable_epoch_start_time_as_time_away,
        readable_epoch_end_time,
        epoch_info.fee_multiplier_permille(),
    )
}

fn format_current_quorums_info(current_quorums_info: &CurrentQuorumsInfo) -> String {
    let mut result = String::new();
    result.push_str("Current Validator Set Information:\n\n");

    for (i, validator_set) in current_quorums_info.validator_sets.iter().enumerate() {
        let quorum_hash = hex::encode(current_quorums_info.quorum_hashes[i]);
        result.push_str(&format!("Quorum Hash: {}\n", quorum_hash));

        for (pro_tx_hash, validator) in validator_set.members() {
            let pro_tx_hash_str = hex::encode(pro_tx_hash);
            if current_quorums_info.last_block_proposer == pro_tx_hash.to_byte_array()
                && current_quorums_info.current_quorum_hash
                    == validator_set.quorum_hash().to_byte_array()
            {
                result.push_str(&format!(
                    "  ---> {} - {} (LAST PROPOSER)\n",
                    pro_tx_hash_str, validator.node_ip
                ));
            } else {
                result.push_str(&format!(
                    "  • {} - {}\n",
                    pro_tx_hash_str, validator.node_ip
                ));
            }
        }
        result.push('\n');
    }

    result.push_str(&format!(
        "Last Platform Block Height: {}\n\
         Last Core Block Height: {}",
        current_quorums_info.last_platform_block_height,
        current_quorums_info.last_core_block_height
    ));

    result
}

fn format_withdrawal_documents_with_daily_limit(
    withdrawal_documents: &[Document],
    total_credits_on_platform: Credits,
    network: Network,
) -> String {
    let total_amount: Credits = withdrawal_documents
        .iter()
        .map(|document| {
            document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .expect("expected amount on withdrawal")
        })
        .sum();

    let amounts = withdrawal_documents
        .iter()
        .map(|document| {
            let index = document.created_at().expect("expected created at");
            let utc_datetime =
                DateTime::<Utc>::from_timestamp_millis(index as i64).expect("expected date time");
            let local_datetime: DateTime<Local> = utc_datetime.with_timezone(&Local);

            let amount = document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .expect("expected amount on withdrawal");
            let status: WithdrawalStatus = document
                .properties()
                .get_integer::<u8>(STATUS)
                .expect("expected status on withdrawal")
                .try_into()
                .expect("expected a withdrawal status");
            let owner_id = document.owner_id();
            let address_bytes = document
                .properties()
                .get_bytes(OUTPUT_SCRIPT)
                .expect("expected output script");
            let output_script = ScriptBuf::from_bytes(address_bytes);
            let address = Address::from_script(&output_script, network)
                .map(|addr| addr.to_string())
                .unwrap_or_else(|e| format!("Invalid Address: {}", e));
            format!(
                "{}: {:.8} Dash for {} towards {} ({})",
                local_datetime.format("%Y-%m-%d %H:%M:%S"),
                amount as f64 / (dash_to_credits!(1) as f64),
                owner_id,
                address,
                status,
            )
        })
        .join("\n    ");

    let daily_withdrawal_limit =
        daily_withdrawal_limit(total_credits_on_platform, PlatformVersion::latest())
            .expect("expected to get daily withdrawal limit");

    format!(
        "Withdrawal Information:\n\n\
         Total Amount: {:.8} Dash\n\
         Daily Withdrawal Limit: {:.8} Dash\n\
         Remaining Today: {:.8} Dash\n\n\
         Recent Withdrawals:\n    {}",
        total_amount as f64 / (dash_to_credits!(1) as f64),
        daily_withdrawal_limit as f64 / (dash_to_credits!(1) as f64),
        daily_withdrawal_limit.saturating_sub(0) as f64 / (dash_to_credits!(1) as f64), // We don't have 24h amount
        amounts
    )
}

fn format_withdrawal_documents_to_bare_info(
    withdrawal_documents: &[Document],
    network: Network,
) -> String {
    let total_amount: Credits = withdrawal_documents
        .iter()
        .map(|document| {
            document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .expect("expected amount on withdrawal")
        })
        .sum();

    let amounts = withdrawal_documents
        .iter()
        .map(|document| {
            let index = document.created_at().expect("expected created at");
            let utc_datetime =
                DateTime::<Utc>::from_timestamp_millis(index as i64).expect("expected date time");
            let local_datetime: DateTime<Local> = utc_datetime.with_timezone(&Local);

            let amount = document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .expect("expected amount on withdrawal");
            let status: WithdrawalStatus = document
                .properties()
                .get_integer::<u8>(STATUS)
                .expect("expected status on withdrawal")
                .try_into()
                .expect("expected a withdrawal status");
            let owner_id = document.owner_id();
            let address_bytes = document
                .properties()
                .get_bytes(OUTPUT_SCRIPT)
                .expect("expected output script");
            let output_script = ScriptBuf::from_bytes(address_bytes);
            let address =
                Address::from_script(&output_script, network).expect("expected an address");
            format!(
                "{}: {:.8} Dash for {} towards {} ({})",
                local_datetime.format("%Y-%m-%d %H:%M:%S"),
                amount as f64 / (dash_to_credits!(1) as f64),
                owner_id,
                address,
                status,
            )
        })
        .join("\n    ");

    format!(
        "Withdrawal Information:\n\n\
         Total Amount: {:.8} Dash\n\n\
         Recent Withdrawals:\n    {}",
        total_amount as f64 / (dash_to_credits!(1) as f64),
        amounts
    )
}

impl AppContext {
    pub async fn run_platform_info_task(
        &self,
        request: PlatformInfoTaskRequestType,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = {
            let sdk_guard = self.sdk.read().unwrap();
            sdk_guard.clone()
        };

        match request {
            PlatformInfoTaskRequestType::BasicPlatformInfo => {
                // Get platform version from SDK
                let platform_version = sdk.version();

                // Try to get chain lock height from core
                let core_chain_lock_height = {
                    let core_client_guard = self.core_client.read();
                    if let Ok(guard) = core_client_guard {
                        match guard.get_best_chain_lock() {
                            Ok(chain_lock) => Some(chain_lock.block_height),
                            Err(_) => None,
                        }
                    } else {
                        None
                    }
                };

                Ok(BackendTaskSuccessResult::PlatformInfo(
                    PlatformInfoTaskResult::BasicPlatformInfo {
                        platform_version,
                        core_chain_lock_height,
                        network: self.network,
                    },
                ))
            }
            PlatformInfoTaskRequestType::CurrentEpochInfo => {
                match ExtendedEpochInfo::fetch_current(&sdk).await {
                    Ok(epoch_info) => {
                        let formatted = format_extended_epoch_info(epoch_info, self.network, true);
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                    Err(e) => Err(format!("Failed to fetch current epoch info: {}", e)),
                }
            }
            PlatformInfoTaskRequestType::TotalCreditsOnPlatform => {
                match TotalCreditsInPlatform::fetch_current(&sdk).await {
                    Ok(total_credits) => {
                        let dash_amount = total_credits.0 as f64 * 10f64.powf(-11.0);
                        let formatted = format!(
                            "Total Credits on Platform:\n\n\
                             • Credits: {}\n\
                             • Dash Equivalent: {:.4} Dash",
                            total_credits.0, dash_amount
                        );
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                    Err(e) => Err(format!("Failed to fetch total credits: {}", e)),
                }
            }
            PlatformInfoTaskRequestType::CurrentVersionVotingState => {
                match ProtocolVersionVoteCount::fetch_many(&sdk, ()).await {
                    Ok(votes) => {
                        let votes: ProtocolVersionUpgrades = votes;
                        let votes_info = votes
                            .into_iter()
                            .map(|(key, value): (u32, Option<u64>)| {
                                format!(
                                    "Version {} -> {}",
                                    key,
                                    value
                                        .map(|v| format!("{} votes", v))
                                        .unwrap_or("No votes".to_string())
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        let formatted = format!("Protocol Version Voting State:\n\n{}", votes_info);
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                    Err(e) => Err(format!("Failed to fetch version voting state: {}", e)),
                }
            }
            PlatformInfoTaskRequestType::CurrentValidatorSetInfo => {
                match CurrentQuorumsInfo::fetch_unproved(&sdk, NoParamQuery {}).await {
                    Ok(Some(current_quorums_info)) => {
                        let formatted = format_current_quorums_info(&current_quorums_info);
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                    Ok(None) => Ok(BackendTaskSuccessResult::PlatformInfo(
                        PlatformInfoTaskResult::TextResult(
                            "No current quorum information available".to_string(),
                        ),
                    )),
                    Err(e) => Err(format!("Failed to fetch validator set info: {}", e)),
                }
            }
            PlatformInfoTaskRequestType::CurrentWithdrawalsInQueue => {
                // Fetch withdrawal documents using the exact pattern from the working code
                let withdrawal_contract = load_system_data_contract(
                    SystemDataContract::Withdrawals,
                    PlatformVersion::latest(),
                )
                .expect("expected to get withdrawal contract");

                // Try the simplest possible query first - no where clauses or ordering
                let queued_document_query = DocumentQuery {
                    data_contract: Arc::new(withdrawal_contract),
                    document_type_name: "withdrawal".to_string(),
                    where_clauses: vec![], // No filtering - get all withdrawals to test basic query
                    order_by_clauses: vec![], // No ordering to avoid proof issues
                    limit: 50,             // Smaller limit to reduce proof size
                    start: None,
                };

                match Document::fetch_many(&sdk, queued_document_query.clone()).await {
                    Ok(documents) => {
                        let withdrawal_docs: Vec<Document> =
                            documents.values().filter_map(|a| a.clone()).collect();

                        // Try to get total credits for daily limit calculation
                        match TotalCreditsInPlatform::fetch_current(&sdk).await {
                            Ok(total_credits) => {
                                let formatted = format_withdrawal_documents_with_daily_limit(
                                    &withdrawal_docs,
                                    total_credits.0,
                                    self.network,
                                );
                                Ok(BackendTaskSuccessResult::PlatformInfo(
                                    PlatformInfoTaskResult::TextResult(formatted),
                                ))
                            }
                            Err(_) => {
                                // Fall back to simple format without daily limits
                                let formatted = format_withdrawal_documents_to_bare_info(
                                    &withdrawal_docs,
                                    self.network,
                                );
                                Ok(BackendTaskSuccessResult::PlatformInfo(
                                    PlatformInfoTaskResult::TextResult(formatted),
                                ))
                            }
                        }
                    }
                    Err(e) => Err(format!("Failed to fetch withdrawal documents: {}", e)),
                }
            }
            PlatformInfoTaskRequestType::RecentlyCompletedWithdrawals => {
                // Fetch completed withdrawal documents
                let withdrawal_contract = load_system_data_contract(
                    SystemDataContract::Withdrawals,
                    PlatformVersion::latest(),
                )
                .expect("expected to get withdrawal contract");

                let completed_document_query = DocumentQuery {
                    data_contract: Arc::new(withdrawal_contract),
                    document_type_name: "withdrawal".to_string(),
                    where_clauses: vec![WhereClause {
                        field: "status".to_string(),
                        operator: WhereOperator::In,
                        value: Value::Array(vec![
                            Value::U8(WithdrawalStatus::COMPLETE as u8),
                            Value::U8(WithdrawalStatus::EXPIRED as u8),
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

                match Document::fetch_many(&sdk, completed_document_query).await {
                    Ok(documents) => {
                        let mut withdrawal_docs: Vec<Document> =
                            documents.values().filter_map(|a| a.clone()).collect();

                        // Sort by updated_at descending to show most recent first
                        withdrawal_docs.sort_by(|a, b| {
                            b.updated_at()
                                .unwrap_or(0)
                                .cmp(&a.updated_at().unwrap_or(0))
                        });

                        // Take only the 50 most recent
                        withdrawal_docs.truncate(50);

                        if withdrawal_docs.is_empty() {
                            Ok(BackendTaskSuccessResult::PlatformInfo(
                                PlatformInfoTaskResult::TextResult(
                                    "No recently completed withdrawals found.".to_string(),
                                ),
                            ))
                        } else {
                            let total_amount: Credits = withdrawal_docs
                                .iter()
                                .map(|document| {
                                    document
                                        .properties()
                                        .get_integer::<Credits>(AMOUNT)
                                        .expect("expected amount on withdrawal")
                                })
                                .sum();

                            let amounts = withdrawal_docs
                                .iter()
                                .map(|document| {
                                    let index = document.updated_at().expect("expected updated at");
                                    let utc_datetime =
                                        DateTime::<Utc>::from_timestamp_millis(index as i64)
                                            .expect("expected date time");
                                    let local_datetime: DateTime<Local> =
                                        utc_datetime.with_timezone(&Local);

                                    let amount = document
                                        .properties()
                                        .get_integer::<Credits>(AMOUNT)
                                        .expect("expected amount on withdrawal");
                                    let status: WithdrawalStatus = document
                                        .properties()
                                        .get_integer::<u8>(STATUS)
                                        .expect("expected status on withdrawal")
                                        .try_into()
                                        .expect("expected a withdrawal status");
                                    let owner_id = document.owner_id();
                                    let address_bytes = document
                                        .properties()
                                        .get_bytes(OUTPUT_SCRIPT)
                                        .expect("expected output script");
                                    let transaction_index = document
                                        .properties()
                                        .get_integer::<u64>(TRANSACTION_INDEX)
                                        .expect("expected transaction index");
                                    let output_script = ScriptBuf::from_bytes(address_bytes);
                                    let address =
                                        Address::from_script(&output_script, self.network)
                                            .expect("expected an address");
                                    format!(
                                        "TX #{}: {:.8} Dash for {} to {} ({}) at {}",
                                        transaction_index,
                                        amount as f64 / (dash_to_credits!(1) as f64),
                                        owner_id,
                                        address,
                                        status,
                                        local_datetime.format("%Y-%m-%d %H:%M:%S"),
                                    )
                                })
                                .join("\n    ");

                            let formatted = format!(
                                "Recently Completed Withdrawals:\n\n\
                                 Total Amount: {:.8} Dash\n\
                                 Count: {} withdrawals\n\n\
                                 Recent Transactions:\n    {}",
                                total_amount as f64 / (dash_to_credits!(1) as f64),
                                withdrawal_docs.len(),
                                amounts
                            );

                            Ok(BackendTaskSuccessResult::PlatformInfo(
                                PlatformInfoTaskResult::TextResult(formatted),
                            ))
                        }
                    }
                    Err(e) => Err(format!(
                        "Failed to fetch completed withdrawal documents: {}",
                        e
                    )),
                }
            }
        }
    }
}
