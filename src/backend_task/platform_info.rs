use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use dash_sdk::Error as SdkError;
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::block::extended_epoch_info::{
    ExtendedEpochInfo, v0::ExtendedEpochInfoV0Getters,
};
use dash_sdk::dpp::core_types::validator_set::v0::ValidatorSetV0Getters;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, Network, ScriptBuf};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contracts::SystemDataContract;
use dash_sdk::dpp::data_contracts::withdrawals_contract::WithdrawalStatus;
use dash_sdk::dpp::data_contracts::withdrawals_contract::v1::document_types::withdrawal::properties::{
    AMOUNT, STATUS, TRANSACTION_INDEX,
};
use dash_sdk::dpp::document::{Document, DocumentV0Getters};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::btreemap_extensions::BTreeValueMapHelper;
use dash_sdk::dpp::state_transition::identity_credit_withdrawal_transition::fields::OUTPUT_SCRIPT;
use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::withdrawal::daily_withdrawal_limit::daily_withdrawal_limit;
use dash_sdk::dpp::{dash_to_credits, version::ProtocolVersionVoteCount};
use dash_sdk::drive::query::{SelectProjection, OrderClause, WhereClause, WhereOperator};
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::platform::fetch_current_no_parameters::FetchCurrent;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{
    DataContract, DocumentQuery, Fetch, FetchMany, FetchUnproved, Identifier,
};
use dash_sdk::query_types::{
    AddressInfo, CurrentQuorumsInfo, NoParamQuery, ProtocolVersionUpgrades, TotalCreditsInPlatform,
};
use std::sync::Arc;
use chrono::{LocalResult, prelude::*};
use chrono_humanize::{Accuracy, HumanTime, Tense};

#[derive(Debug, Clone, PartialEq)]
pub enum PlatformInfoTaskRequestType {
    CurrentEpochInfo,
    TotalCreditsOnPlatform,
    CurrentVersionVotingState,
    CurrentValidatorSetInfo,
    CurrentWithdrawalsInQueue,
    RecentlyCompletedWithdrawals,
    /// Structured, paginated withdrawal query for programmatic clients (MCP /
    /// CLI). Unlike the text variants above, this returns one
    /// [`WithdrawalRecord`] per document plus a continuation cursor.
    Withdrawals {
        /// Query completed/expired withdrawals when `true`, the in-queue set
        /// when `false`.
        completed: bool,
        /// Maximum documents to return. `None` uses the platform default.
        limit: Option<u32>,
        /// Continuation cursor: the document id to start after, as returned in
        /// a prior response's `next_cursor`.
        start_after: Option<Identifier>,
    },
    BasicPlatformInfo,
    ShieldedPoolState,
    FetchAddressBalance(String),
}

/// One withdrawal document flattened into the fields programmatic clients need.
/// Credits are atomic units (1 Dash = `dash_to_credits!(1)` credits); timestamps
/// are Unix milliseconds straight from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalRecord {
    /// Withdrawal document id (base58-encodable handle, also the page cursor).
    pub document_id: Identifier,
    /// Identity that requested the withdrawal.
    pub owner_id: Identifier,
    /// Amount in credits (atomic units).
    pub amount_credits: u64,
    /// Withdrawal status: `"queued"`, `"pooled"`, `"broadcasted"`,
    /// `"complete"`, or `"expired"`.
    pub status: String,
    /// Destination Dash address decoded from the output script, or `None` when
    /// the script does not map to a standard address on this network.
    pub address: Option<String>,
    /// Sequential on-chain transaction index, when present on the document.
    pub transaction_index: Option<u64>,
    /// Document creation time (Unix ms), when present.
    pub created_at_ms: Option<u64>,
    /// Document last-update time (Unix ms), when present.
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum PlatformInfoTaskResult {
    BasicPlatformInfo {
        platform_version: &'static PlatformVersion,
        core_chain_lock_height: Option<u32>,
        network: dash_sdk::dpp::dashcore::Network,
    },
    TextResult(String),
    AddressBalance {
        address: String,
        balance: u64,
        nonce: u32,
    },
    Withdrawals {
        records: Vec<WithdrawalRecord>,
        /// Sum of all returned records' `amount_credits`.
        total_amount_credits: u64,
        /// Pass as the next request's `start_after` to fetch the following
        /// page. `None` when this page was not full (no more results).
        next_cursor: Option<Identifier>,
    },
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
            (
                PlatformInfoTaskResult::AddressBalance {
                    address: addr1,
                    balance: bal1,
                    nonce: n1,
                },
                PlatformInfoTaskResult::AddressBalance {
                    address: addr2,
                    balance: bal2,
                    nonce: n2,
                },
            ) => addr1 == addr2 && bal1 == bal2 && n1 == n2,
            (
                PlatformInfoTaskResult::Withdrawals {
                    records: r1,
                    total_amount_credits: t1,
                    next_cursor: c1,
                },
                PlatformInfoTaskResult::Withdrawals {
                    records: r2,
                    total_amount_credits: t2,
                    next_cursor: c2,
                },
            ) => r1 == r2 && t1 == t2 && c1 == c2,
            _ => false,
        }
    }
}

/// Why a withdrawal document could not be parsed for display or into a
/// [`WithdrawalRecord`].
///
/// Carried as the `#[source]` of [`TaskError::WithdrawalDocumentParsingError`]:
/// the user sees that variant's message while this enum preserves the technical
/// cause for logs and the collapsible details panel.
#[derive(Debug, thiserror::Error)]
pub enum WithdrawalParseError {
    /// A document field was missing or had an unexpected type.
    #[error("A withdrawal document field could not be read")]
    Field(#[from] dash_sdk::dpp::platform_value::Error),
    /// The document lacked a required created/updated timestamp.
    #[error("A withdrawal document is missing a required timestamp")]
    MissingTimestamp,
    /// A timestamp value fell outside the representable range.
    #[error("A withdrawal document has an out-of-range timestamp")]
    InvalidTimestamp,
    /// The status field held a value outside the known withdrawal states.
    #[error("A withdrawal document has an unrecognized status value {value}")]
    InvalidStatus { value: u8 },
    /// The daily withdrawal limit could not be computed for this platform version.
    #[error("The daily withdrawal limit could not be computed")]
    DailyLimit(#[source] Box<dash_sdk::dpp::ProtocolError>),
}

// Helper functions for formatting platform data
/// Kept for the restore path of the disabled live epoch fetch; see the
/// `TODO(platform#4231)` in the `CurrentEpochInfo` arm.
#[allow(dead_code)]
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
        Network::Mainnet => 788_400_000,
        Network::Testnet => 3_600_000,
        Network::Devnet => 3_600_000,
        Network::Regtest => 1_200_000,
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

/// `protocol_version` is `None` while the connected network has not confirmed one.
/// The fee multiplier is a fixed value, not a network reading — see the
/// `TODO(platform#4231)` in the `CurrentEpochInfo` arm.
fn format_hardcoded_current_epoch_info(
    protocol_version: Option<u32>,
    fee_multiplier_permille: u64,
) -> String {
    let fee_multiplier = fee_multiplier_permille as f64 / 1000.0;
    match protocol_version {
        Some(protocol_version) => format!(
            "Current Epoch Information:\n\
             • Protocol Version: {protocol_version}\n\
             • Fee Multiplier: {fee_multiplier}x (a fixed value, not read from the network)\n\n\
             Epoch details cannot be read while dashpay/platform#4231 is unresolved. The fee \
             multiplier shown is the one every network charges today, and it will be read live \
             again once that fix is released."
        ),
        None => format!(
            "Current Epoch Information:\n\
             • Protocol Version: the connected network has not confirmed one yet.\n\
             • Fee Multiplier: {fee_multiplier}x (a fixed value, not read from the network)\n\n\
             Epoch details cannot be read while dashpay/platform#4231 is unresolved. The fee \
             multiplier shown is the one every network charges today, and it will be read live \
             again once that fix is released."
        ),
    }
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
) -> Result<String, WithdrawalParseError> {
    let total_amount: Credits = withdrawal_documents
        .iter()
        .map(|document| {
            document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .map_err(WithdrawalParseError::Field)
        })
        .collect::<Result<Vec<Credits>, WithdrawalParseError>>()?
        .into_iter()
        .sum();

    let amounts: Vec<String> = withdrawal_documents
        .iter()
        .map(|document| format_withdrawal_line(document, network))
        .collect::<Result<Vec<String>, WithdrawalParseError>>()?;

    let daily_withdrawal_limit =
        daily_withdrawal_limit(total_credits_on_platform, PlatformVersion::latest())
            .map_err(|e| WithdrawalParseError::DailyLimit(Box::new(e)))?;

    Ok(format!(
        "Withdrawal Information:\n\n\
         Total Amount: {:.8} Dash\n\
         Daily Withdrawal Limit: {:.8} Dash\n\
         Remaining Today: N/A (24h usage data unavailable)\n\n\
         Recent Withdrawals:\n    {}",
        total_amount as f64 / (dash_to_credits!(1) as f64),
        daily_withdrawal_limit as f64 / (dash_to_credits!(1) as f64),
        amounts.join("\n    ")
    ))
}

fn format_withdrawal_documents_to_bare_info(
    withdrawal_documents: &[Document],
    network: Network,
) -> Result<String, WithdrawalParseError> {
    let total_amount: Credits = withdrawal_documents
        .iter()
        .map(|document| {
            document
                .properties()
                .get_integer::<Credits>(AMOUNT)
                .map_err(WithdrawalParseError::Field)
        })
        .collect::<Result<Vec<Credits>, WithdrawalParseError>>()?
        .into_iter()
        .sum();

    let amounts: Vec<String> = withdrawal_documents
        .iter()
        .map(|document| format_withdrawal_line(document, network))
        .collect::<Result<Vec<String>, WithdrawalParseError>>()?;

    Ok(format!(
        "Withdrawal Information:\n\n\
         Total Amount: {:.8} Dash\n\n\
         Recent Withdrawals:\n    {}",
        total_amount as f64 / (dash_to_credits!(1) as f64),
        amounts.join("\n    ")
    ))
}

/// Format one queued/in-flight withdrawal document as a single human-readable
/// line: `"<created-at>: <amount> Dash for <owner> towards <address> (<status>)"`.
fn format_withdrawal_line(
    document: &Document,
    network: Network,
) -> Result<String, WithdrawalParseError> {
    let index = document
        .created_at()
        .ok_or(WithdrawalParseError::MissingTimestamp)?;
    let utc_datetime = DateTime::<Utc>::from_timestamp_millis(index as i64)
        .ok_or(WithdrawalParseError::InvalidTimestamp)?;
    let local_datetime: DateTime<Local> = utc_datetime.with_timezone(&Local);

    let amount = document
        .properties()
        .get_integer::<Credits>(AMOUNT)
        .map_err(WithdrawalParseError::Field)?;
    let status_u8: u8 = document
        .properties()
        .get_integer::<u8>(STATUS)
        .map_err(WithdrawalParseError::Field)?;
    let status: WithdrawalStatus = status_u8
        .try_into()
        .map_err(|_| WithdrawalParseError::InvalidStatus { value: status_u8 })?;
    let owner_id = document.owner_id();
    let address_bytes = document
        .properties()
        .get_bytes(OUTPUT_SCRIPT)
        .map_err(WithdrawalParseError::Field)?;
    let output_script = ScriptBuf::from_bytes(address_bytes);
    let address = Address::from_script(&output_script, network)
        .map(|addr| addr.to_string())
        .unwrap_or_else(|e| format!("Invalid Address: {}", e));
    Ok(format!(
        "{}: {:.8} Dash for {} towards {} ({})",
        local_datetime.format("%Y-%m-%d %H:%M:%S"),
        amount as f64 / (dash_to_credits!(1) as f64),
        owner_id,
        address,
        status,
    ))
}

/// Format one completed/expired withdrawal document as a single line keyed by
/// on-chain transaction index and last-update time:
/// `"TX #<index>: <amount> Dash for <owner> to <address> (<status>) at <time>"`.
fn format_completed_withdrawal_line(
    document: &Document,
    network: Network,
) -> Result<String, WithdrawalParseError> {
    let index = document
        .updated_at()
        .ok_or(WithdrawalParseError::MissingTimestamp)?;
    let utc_datetime = DateTime::<Utc>::from_timestamp_millis(index as i64)
        .ok_or(WithdrawalParseError::InvalidTimestamp)?;
    let local_datetime: DateTime<Local> = utc_datetime.with_timezone(&Local);

    let amount = document
        .properties()
        .get_integer::<Credits>(AMOUNT)
        .map_err(WithdrawalParseError::Field)?;
    let status_u8: u8 = document
        .properties()
        .get_integer::<u8>(STATUS)
        .map_err(WithdrawalParseError::Field)?;
    let status: WithdrawalStatus = status_u8
        .try_into()
        .map_err(|_| WithdrawalParseError::InvalidStatus { value: status_u8 })?;
    let owner_id = document.owner_id();
    let address_bytes = document
        .properties()
        .get_bytes(OUTPUT_SCRIPT)
        .map_err(WithdrawalParseError::Field)?;
    let transaction_index = document
        .properties()
        .get_integer::<u64>(TRANSACTION_INDEX)
        .map_err(WithdrawalParseError::Field)?;
    let output_script = ScriptBuf::from_bytes(address_bytes);
    let address = Address::from_script(&output_script, network)
        .map(|addr| addr.to_string())
        .unwrap_or_else(|e| format!("Invalid Address: {}", e));
    Ok(format!(
        "TX #{}: {:.8} Dash for {} to {} ({}) at {}",
        transaction_index,
        amount as f64 / (dash_to_credits!(1) as f64),
        owner_id,
        address,
        status,
        local_datetime.format("%Y-%m-%d %H:%M:%S"),
    ))
}

/// Stable, lowercase status string for programmatic clients. Distinct from the
/// human-facing `Display` ("Queued", …) so machine consumers can match on it.
fn withdrawal_status_str(status: WithdrawalStatus) -> &'static str {
    match status {
        WithdrawalStatus::QUEUED => "queued",
        WithdrawalStatus::POOLED => "pooled",
        WithdrawalStatus::BROADCASTED => "broadcasted",
        WithdrawalStatus::COMPLETE => "complete",
        WithdrawalStatus::EXPIRED => "expired",
    }
}

/// Flatten one withdrawal [`Document`] into a [`WithdrawalRecord`].
fn extract_withdrawal_record(
    document: &Document,
    network: Network,
) -> Result<WithdrawalRecord, WithdrawalParseError> {
    let amount_credits = document
        .properties()
        .get_integer::<Credits>(AMOUNT)
        .map_err(WithdrawalParseError::Field)?;
    let status_u8 = document
        .properties()
        .get_integer::<u8>(STATUS)
        .map_err(WithdrawalParseError::Field)?;
    let status: WithdrawalStatus = status_u8
        .try_into()
        .map_err(|_| WithdrawalParseError::InvalidStatus { value: status_u8 })?;
    let address = document
        .properties()
        .get_bytes(OUTPUT_SCRIPT)
        .ok()
        .and_then(|bytes| Address::from_script(&ScriptBuf::from_bytes(bytes), network).ok())
        .map(|addr| addr.to_string());
    let transaction_index = document
        .properties()
        .get_integer::<u64>(TRANSACTION_INDEX)
        .ok();

    Ok(WithdrawalRecord {
        document_id: document.id(),
        owner_id: document.owner_id(),
        amount_credits,
        status: withdrawal_status_str(status).to_string(),
        address,
        transaction_index,
        created_at_ms: document.created_at(),
        updated_at_ms: document.updated_at(),
    })
}

/// Build the structured, paginated withdrawal result. `documents` are the page
/// already fetched; `page_limit` is the limit that was requested so the cursor
/// is only emitted when the page came back full (more results may exist).
fn build_withdrawals_result(
    documents: &[Document],
    page_limit: usize,
    network: Network,
) -> Result<PlatformInfoTaskResult, TaskError> {
    let records = documents
        .iter()
        .map(|doc| extract_withdrawal_record(doc, network))
        .collect::<Result<Vec<_>, _>>()?;
    let total_amount_credits = records.iter().map(|r| r.amount_credits).sum();
    let next_cursor = (documents.len() == page_limit)
        .then(|| records.last().map(|r| r.document_id))
        .flatten();
    Ok(PlatformInfoTaskResult::Withdrawals {
        records,
        total_amount_credits,
        next_cursor,
    })
}

impl AppContext {
    pub async fn run_platform_info_task(
        self: &Arc<Self>,
        request: PlatformInfoTaskRequestType,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match request {
            PlatformInfoTaskRequestType::BasicPlatformInfo => {
                let platform_version = sdk.version();

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
                // dashpay/platform#4231 breaks `ExtendedEpochInfo::fetch_current`, so the
                // network's version is learned from the ratchet a proved DPNS fetch drives.
                // Only a successful fetch proves it came from the network, not the local seed.
                match DataContract::fetch(sdk, self.dpns_contract.id()).await {
                    Ok(_) => self.set_platform_protocol_version(sdk.protocol_version_number()),
                    Err(error) => tracing::warn!(
                        %error,
                        "Protocol-version ratchet trigger (DPNS contract fetch) failed; \
                         the network's protocol version stays unconfirmed"
                    ),
                }

                // TODO(platform#4231): restore the commented-out live fetch below and drop the
                // hardcoded multiplier once https://github.com/dashpay/platform/pull/4231 merges
                // and this repo's platform pin (Cargo.toml/Cargo.lock rev a18bd158…) advances past
                // it. The proof verifier in that pin rejects the descending-epoch-without-start
                // query shape `fetch_current` sends, so the call fails identically on every DAPI
                // node: each attempt cycles the SDK's whole address pool and burns the shared
                // per-client request budget, which surfaced as `DapiAllAddressesExhausted` in
                // unrelated flows such as identity top-up. This task also runs automatically on
                // every SPV Syncing→Synced transition, so the cost is not user-paced.
                //
                // 1000 permille (1.0x) is what the network actually stores, not a placeholder: an
                // epoch's multiplier is written from
                // `platform_version.fee_version.uses_version_fee_multiplier_permille`
                // (rs-drive-abci/src/execution/platform_events/block_processing_end_events/
                // add_process_epoch_change_operations/v0/mod.rs:107) and both fee schedules in the
                // pinned crate declare `Some(1000)` — rs-platform-version/src/version/fee/v1.rs:13
                // and v2.rs:14, checked 2026-07-31 at rev a18bd158.
                //
                // match ExtendedEpochInfo::fetch_current(sdk).await {
                //     Ok(epoch_info) => {
                //         let fee_multiplier = epoch_info.fee_multiplier_permille();
                //         self.set_fee_multiplier_permille(fee_multiplier);
                //         self.set_platform_protocol_version(epoch_info.protocol_version());
                //
                //         let mut formatted =
                //             format_extended_epoch_info(epoch_info, self.network, true);
                //         formatted.push_str(&format!(
                //             "\n\n(Fee multiplier cache updated: {}x)",
                //             fee_multiplier as f64 / 1000.0
                //         ));
                //         Ok(BackendTaskSuccessResult::PlatformInfo(
                //             PlatformInfoTaskResult::TextResult(formatted),
                //         ))
                //     }
                //     // Restoring keeps a degraded arm here: log, then fall back to
                //     // `format_hardcoded_current_epoch_info` with the cached multiplier.
                //     Err(error) => { ... }
                // }
                let fee_multiplier = PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE;
                self.set_fee_multiplier_permille(fee_multiplier);

                let confirmed = match self.platform_protocol_version() {
                    0 => None,
                    version => Some(version),
                };
                Ok(BackendTaskSuccessResult::PlatformInfo(
                    PlatformInfoTaskResult::TextResult(format_hardcoded_current_epoch_info(
                        confirmed,
                        fee_multiplier,
                    )),
                ))
            }
            PlatformInfoTaskRequestType::TotalCreditsOnPlatform => {
                let total_credits = TotalCreditsInPlatform::fetch_current(sdk)
                    .await
                    .map_err(TaskError::from)?;

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
            PlatformInfoTaskRequestType::CurrentVersionVotingState => {
                let votes: ProtocolVersionUpgrades = ProtocolVersionVoteCount::fetch_many(sdk, ())
                    .await
                    .map_err(TaskError::from)?;

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
            PlatformInfoTaskRequestType::CurrentValidatorSetInfo => {
                match CurrentQuorumsInfo::fetch_unproved(sdk, NoParamQuery {}).await {
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
                    Err(e) => Err(TaskError::from(e)),
                }
            }
            PlatformInfoTaskRequestType::CurrentWithdrawalsInQueue => {
                let withdrawal_contract = load_system_data_contract(
                    SystemDataContract::Withdrawals,
                    PlatformVersion::latest(),
                )
                .map_err(|e| TaskError::from(SdkError::Protocol(e)))?;

                let queued_document_query = DocumentQuery {
                    select: SelectProjection::documents(),
                    data_contract: Arc::new(withdrawal_contract),
                    document_type_name: "withdrawal".to_string(),
                    where_clauses: vec![],
                    group_by: Vec::new(),
                    having: Vec::new(),
                    order_by_clauses: vec![],
                    limit: 50,
                    offset: None,
                    start: None,
                };

                let documents = Document::fetch_many(sdk, queued_document_query.clone())
                    .await
                    .map_err(TaskError::from)?;

                let withdrawal_docs: Vec<Document> =
                    documents.values().filter_map(|a| a.clone()).collect();

                match TotalCreditsInPlatform::fetch_current(sdk).await {
                    Ok(total_credits) => {
                        let formatted = format_withdrawal_documents_with_daily_limit(
                            &withdrawal_docs,
                            total_credits.0,
                            self.network,
                        )?;
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                    Err(_) => {
                        let formatted = format_withdrawal_documents_to_bare_info(
                            &withdrawal_docs,
                            self.network,
                        )?;
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                }
            }
            PlatformInfoTaskRequestType::RecentlyCompletedWithdrawals => {
                let withdrawal_contract = load_system_data_contract(
                    SystemDataContract::Withdrawals,
                    PlatformVersion::latest(),
                )
                .map_err(|e| TaskError::from(SdkError::Protocol(e)))?;

                let completed_document_query = DocumentQuery {
                    select: SelectProjection::documents(),
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
                    group_by: Vec::new(),
                    having: Vec::new(),
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
                    offset: None,
                    start: None,
                };

                let documents = Document::fetch_many(sdk, completed_document_query)
                    .await
                    .map_err(TaskError::from)?;

                let mut withdrawal_docs: Vec<Document> =
                    documents.values().filter_map(|a| a.clone()).collect();

                withdrawal_docs.sort_by(|a, b| {
                    b.updated_at()
                        .unwrap_or(0)
                        .cmp(&a.updated_at().unwrap_or(0))
                });

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
                                .map_err(WithdrawalParseError::Field)
                        })
                        .collect::<Result<Vec<Credits>, WithdrawalParseError>>()?
                        .into_iter()
                        .sum();

                    let amounts: Vec<String> = withdrawal_docs
                        .iter()
                        .map(|document| format_completed_withdrawal_line(document, self.network))
                        .collect::<Result<Vec<String>, WithdrawalParseError>>()?;

                    let formatted = format!(
                        "Recently Completed Withdrawals:\n\n\
                         Total Amount: {:.8} Dash\n\
                         Count: {} withdrawals\n\n\
                         Recent Transactions:\n    {}",
                        total_amount as f64 / (dash_to_credits!(1) as f64),
                        withdrawal_docs.len(),
                        amounts.join("\n    ")
                    );

                    Ok(BackendTaskSuccessResult::PlatformInfo(
                        PlatformInfoTaskResult::TextResult(formatted),
                    ))
                }
            }
            PlatformInfoTaskRequestType::Withdrawals {
                completed,
                limit,
                start_after,
            } => {
                let withdrawal_contract = load_system_data_contract(
                    SystemDataContract::Withdrawals,
                    PlatformVersion::latest(),
                )
                .map_err(|e| TaskError::from(SdkError::Protocol(e)))?;

                // `0` is the upstream sentinel for "default limit"; clamp the
                // requested page so the cursor heuristic has a known bound.
                let page_limit = limit.unwrap_or(50).clamp(1, 100);
                let start = start_after.map(|id| Start::StartAfter(id.to_buffer().to_vec()));

                let statuses = if completed {
                    vec![
                        Value::U8(WithdrawalStatus::COMPLETE as u8),
                        Value::U8(WithdrawalStatus::EXPIRED as u8),
                    ]
                } else {
                    vec![
                        Value::U8(WithdrawalStatus::QUEUED as u8),
                        Value::U8(WithdrawalStatus::POOLED as u8),
                        Value::U8(WithdrawalStatus::BROADCASTED as u8),
                    ]
                };
                let where_clauses = vec![WhereClause {
                    field: "status".to_string(),
                    operator: WhereOperator::In,
                    value: Value::Array(statuses),
                }];
                let order_by_clauses = vec![
                    OrderClause {
                        field: "status".to_string(),
                        ascending: true,
                    },
                    OrderClause {
                        field: "transactionIndex".to_string(),
                        ascending: true,
                    },
                ];

                let query = DocumentQuery {
                    select: SelectProjection::documents(),
                    data_contract: Arc::new(withdrawal_contract),
                    document_type_name: "withdrawal".to_string(),
                    where_clauses,
                    group_by: Vec::new(),
                    having: Vec::new(),
                    order_by_clauses,
                    limit: page_limit,
                    offset: None,
                    start,
                };

                let documents = Document::fetch_many(sdk, query)
                    .await
                    .map_err(TaskError::from)?;
                let withdrawal_docs: Vec<Document> =
                    documents.values().filter_map(|a| a.clone()).collect();

                let result =
                    build_withdrawals_result(&withdrawal_docs, page_limit as usize, self.network)?;
                Ok(BackendTaskSuccessResult::PlatformInfo(result))
            }
            PlatformInfoTaskRequestType::ShieldedPoolState => {
                use dash_sdk::query_types::ShieldedPoolState;

                match ShieldedPoolState::fetch_current(sdk).await {
                    Ok(pool_state) => {
                        let total_credits = pool_state.0;
                        let dash_amount = total_credits as f64 / (dash_to_credits!(1) as f64);
                        let formatted = format!(
                            "Shielded Pool State:\n\n\
                             • Total Balance: {} credits\n\
                             • Dash Equivalent: {:.8} DASH",
                            total_credits, dash_amount,
                        );
                        Ok(BackendTaskSuccessResult::PlatformInfo(
                            PlatformInfoTaskResult::TextResult(formatted),
                        ))
                    }
                    Err(e) => Err(TaskError::ShieldedSyncFailed(Box::new(e))),
                }
            }
            PlatformInfoTaskRequestType::FetchAddressBalance(address_string) => {
                let platform_address: PlatformAddress =
                    address_string
                        .parse()
                        .map_err(|_| TaskError::IdentifierParsingError {
                            input: address_string.clone(),
                        })?;

                let mut addresses = std::collections::BTreeSet::new();
                addresses.insert(platform_address);
                let address_infos = AddressInfo::fetch_many(sdk, addresses)
                    .await
                    .map_err(TaskError::from)?;

                let result: Option<&Option<AddressInfo>> = address_infos.get(&platform_address);
                if let Some(Some(info)) = result {
                    Ok(BackendTaskSuccessResult::PlatformInfo(
                        PlatformInfoTaskResult::AddressBalance {
                            address: address_string,
                            balance: info.balance,
                            nonce: info.nonce,
                        },
                    ))
                } else {
                    Ok(BackendTaskSuccessResult::PlatformInfo(
                        PlatformInfoTaskResult::AddressBalance {
                            address: address_string,
                            balance: 0,
                            nonce: 0,
                        },
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_workaround_reports_protocol_version_and_the_hardcoded_fee_multiplier() {
        assert_eq!(
            format_hardcoded_current_epoch_info(Some(12), 1000),
            "Current Epoch Information:\n\
             • Protocol Version: 12\n\
             • Fee Multiplier: 1x (a fixed value, not read from the network)\n\n\
             Epoch details cannot be read while dashpay/platform#4231 is unresolved. The fee \
             multiplier shown is the one every network charges today, and it will be read live \
             again once that fix is released."
        );
    }

    /// The multiplier is presented as fixed, never as a live reading: a user
    /// comparing it against a network that raised its fees must be able to see
    /// which of the two the app is charging by.
    #[test]
    fn epoch_workaround_never_presents_the_fee_multiplier_as_a_network_reading() {
        for protocol_version in [None, Some(12)] {
            let formatted = format_hardcoded_current_epoch_info(protocol_version, 1500);
            assert!(
                formatted
                    .contains("• Fee Multiplier: 1.5x (a fixed value, not read from the network)"),
                "the multiplier must be shown as fixed, got: {formatted}"
            );
        }
    }

    #[test]
    fn epoch_workaround_never_reports_an_unconfirmed_protocol_version_as_a_number() {
        let formatted = format_hardcoded_current_epoch_info(None, 1000);
        assert!(
            formatted
                .contains("• Protocol Version: the connected network has not confirmed one yet."),
            "an unconfirmed version must be named as such, got: {formatted}"
        );
    }

    /// Fee estimates must not keep running on whatever multiplier a previous
    /// refresh happened to leave behind: the task republishes the hardcoded one.
    #[tokio::test]
    async fn epoch_workaround_republishes_the_hardcoded_fee_multiplier() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = dash_sdk::Sdk::new_mock();
        ctx.set_fee_multiplier_permille(7_777);

        let result = ctx
            .run_platform_info_task(PlatformInfoTaskRequestType::CurrentEpochInfo, &sdk)
            .await
            .expect("the epoch workaround degrades to a text result");

        assert_eq!(
            ctx.fee_multiplier_permille(),
            PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE
        );
        let BackendTaskSuccessResult::PlatformInfo(PlatformInfoTaskResult::TextResult(text)) =
            result
        else {
            panic!("the epoch workaround returns a text result");
        };
        assert!(
            text.contains("• Fee Multiplier: 1x"),
            "the reported multiplier must match the cached one, got: {text}"
        );
    }

    /// A failed ratchet trigger leaves the SDK reporting its local seed, which is
    /// not a network observation: caching it would open the shielded capability
    /// gate and defeat the `0` retry sentinel `mcp::resolve` polls.
    #[tokio::test]
    async fn a_failed_ratchet_trigger_leaves_the_protocol_version_unconfirmed() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let sdk = dash_sdk::Sdk::new_mock();
        assert_ne!(
            sdk.protocol_version_number(),
            0,
            "precondition: the mock SDK seeds a local version of its own"
        );

        ctx.run_platform_info_task(PlatformInfoTaskRequestType::CurrentEpochInfo, &sdk)
            .await
            .expect("the epoch workaround degrades to a text result");

        assert_eq!(ctx.platform_protocol_version(), 0);
    }
}
