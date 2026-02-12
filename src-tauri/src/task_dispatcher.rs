//! Task dispatcher: bridges Tauri IPC commands to the existing BackendTask system.
//!
//! Provides a way to dispatch `BackendTask` instances from Tauri commands,
//! receive results asynchronously, and emit them as Tauri events to the frontend.
//!
//! ## Design
//!
//! The egui app used channels (`SenderAsync<TaskResult>`) to shuttle results
//! from the tokio runtime back to the UI thread. In Tauri, we replace this with:
//!
//! 1. **Direct command returns:** Short-lived commands (DB reads, config queries)
//!    return results directly from the `#[tauri::command]` function.
//!
//! 2. **Event-based async results:** Long-running operations (identity registration,
//!    token minting, wallet sync) are dispatched as BackendTasks. A tokio task
//!    runs the operation and emits `TaskResultEvent` or `TaskErrorEvent` when done.
//!
//! 3. **Polling events:** ZMQ messages, SPV status, and scheduled votes are
//!    forwarded as Tauri events from background loops.

use crate::commands::system::{convert_mn_list_diff, convert_qr_info};
use crate::dto::{
    ContractTokenInfoDto, ContractWithTokensDto, TaskChainLockSigDto, NetworkDto,
    ProfileSearchResultDto, TaskDomain, TaskResultPayloadDto, TokenSearchResultDto,
};
use crate::events::{
    ScheduledVoteExecutedEvent, SpvStatusDto, SpvStatusEvent, TaskErrorEvent, TaskResultEvent,
    WalletUpdatedEvent, ZmqChainLockedBlockEvent, ZmqConnectionStatusEvent,
    ZmqIsLockedTransactionEvent,
};
use crate::state::AppState;
use dash_evo_tool::app::TaskResult;
use dash_evo_tool::backend_task::contract::ContractResult;
use dash_evo_tool::backend_task::dashpay::DashPayResult;
use dash_evo_tool::backend_task::document::DocumentResult;
use dash_evo_tool::backend_task::identity::IdentityResult;
use dash_evo_tool::backend_task::mnlist::MnListResult;
use dash_evo_tool::backend_task::platform_info::{PlatformInfoTaskResult, PlatformResult};
use dash_evo_tool::backend_task::tokens::TokenResult;
use dash_evo_tool::backend_task::wallet::WalletResult;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
use dash_evo_tool::components::core_zmq_listener::{ZMQConnectionEvent, ZMQMessage};
use dash_evo_tool::utils::egui_mpsc::SenderAsync;
use dash_sdk::dpp::dashcore::consensus::Encodable;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::mpsc as tokiompsc;

/// Global task ID counter for unique task identification.
static TASK_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique task ID.
pub fn next_task_id() -> String {
    let id = TASK_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("task-{}", id)
}

/// Dispatch a single backend task asynchronously.
///
/// Returns a task ID immediately. The result will be emitted as a
/// `TaskResultEvent` or `TaskErrorEvent` via Tauri events.
pub fn dispatch_task(app_handle: &AppHandle, app_state: &AppState, task: BackendTask) -> String {
    let task_id = next_task_id();
    let app_context = app_state.current_context().clone();
    let handle = app_handle.clone();
    let tid = task_id.clone();
    let domain = task_domain(&task);

    // Create a headless sender — no egui context needed in Tauri.
    // The channel forwards intermediate progress messages from the backend
    // task system; we listen on the receiver and emit Tauri events.
    let (tx, mut rx) = tokiompsc::channel::<TaskResult>(64);
    let sender = SenderAsync::new_headless(tx);

    // Forward intermediate progress/refresh messages as events.
    // Only Refresh is handled here — final Success/Error results come from
    // the outer completion handler below, avoiding duplicate emissions.
    let handle_progress = handle.clone();
    let tid_progress = tid.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(task_result) = rx.recv().await {
            match task_result {
                TaskResult::Refresh => {
                    if let Err(e) = (TaskResultEvent {
                        task_id: tid_progress.clone(),
                        result: TaskResultPayloadDto::Refresh,
                    })
                    .emit(&handle_progress)
                    {
                        tracing::error!(task_id = %tid_progress, error = %e, "Failed to emit Refresh event");
                    }
                }
                // Success and Error are handled by the outer completion handler;
                // ignore them here to prevent duplicate event emission.
                TaskResult::Success(_) | TaskResult::Error(_) => {}
            }
        }
    });

    // Spawn the actual task inside a nested tokio::spawn so we can catch panics
    tauri::async_runtime::spawn(async move {
        let inner = tokio::spawn(async move {
            app_context.run_backend_task(task, sender).await
        });

        match inner.await {
            Ok(Ok(success)) => {
                let payload = classify_success_result(&success);
                tracing::info!(task_id = %tid, domain = ?domain, result_type = ?std::mem::discriminant(&payload), "Emitting TaskResultEvent");
                if let Err(e) = (TaskResultEvent {
                    task_id: tid.clone(),
                    result: payload,
                })
                .emit(&handle)
                {
                    tracing::error!(task_id = %tid, error = %e, "Failed to emit TaskResultEvent");
                }
            }
            Ok(Err(error)) => {
                tracing::info!(task_id = %tid, domain = ?domain, message = %error.user_message(), "Emitting TaskErrorEvent");
                if let Err(e) = (TaskErrorEvent {
                    task_id: tid.clone(),
                    domain,
                    message: error.user_message(),
                    details: error.technical_details(),
                    recoverable: error.is_recoverable(),
                })
                .emit(&handle)
                {
                    tracing::error!(task_id = %tid, error = %e, "Failed to emit TaskErrorEvent");
                }
            }
            Err(join_error) => {
                // Task panicked or was cancelled
                let panic_msg = if join_error.is_panic() {
                    match join_error.into_panic().downcast::<String>() {
                        Ok(s) => *s,
                        Err(p) => match p.downcast::<&str>() {
                            Ok(s) => s.to_string(),
                            Err(_) => "Unknown panic".to_string(),
                        },
                    }
                } else {
                    "Task was cancelled".to_string()
                };
                tracing::error!(task_id = %tid, panic = %panic_msg, "Backend task panicked");
                if let Err(e) = (TaskErrorEvent {
                    task_id: tid.clone(),
                    domain,
                    message: format!("Internal error: {}", panic_msg),
                    details: panic_msg,
                    recoverable: false,
                })
                .emit(&handle)
                {
                    tracing::error!(task_id = %tid, error = %e, "Failed to emit TaskErrorEvent after panic");
                }
            }
        }
    });

    task_id
}

/// Derive the task domain from the input `BackendTask`.
///
/// This is captured before the task is moved into the spawned future so
/// that error events can carry the correct domain discriminant.
fn task_domain(task: &BackendTask) -> TaskDomain {
    match task {
        BackendTask::IdentityTask(_) => TaskDomain::Identity,
        BackendTask::WalletTask(_) => TaskDomain::Wallet,
        BackendTask::ContractTask(_) => TaskDomain::Contract,
        BackendTask::DocumentTask(_) => TaskDomain::Document,
        BackendTask::TokenTask(_) => TaskDomain::Token,
        BackendTask::ContestedResourceTask(_) => TaskDomain::Contest,
        BackendTask::DashPayTask(_) => TaskDomain::DashPay,
        BackendTask::PlatformInfo(_) => TaskDomain::Platform,
        BackendTask::MnListTask(_) => TaskDomain::MnList,
        BackendTask::CoreTask(_) => TaskDomain::Core,
        BackendTask::SystemTask(_) => TaskDomain::System,
        BackendTask::GroveSTARKTask(_) => TaskDomain::GroveStark,
        BackendTask::BroadcastStateTransition(_) => TaskDomain::General,
        BackendTask::None => TaskDomain::General,
    }
}

/// Classify a `BackendTaskSuccessResult` into a typed `TaskResultPayloadDto`.
fn classify_success_result(result: &BackendTaskSuccessResult) -> TaskResultPayloadDto {
    match result {
        BackendTaskSuccessResult::None => TaskResultPayloadDto::None,
        BackendTaskSuccessResult::Refresh => TaskResultPayloadDto::Refresh,
        BackendTaskSuccessResult::Message(msg) => TaskResultPayloadDto::Message {
            text: msg.clone(),
        },
        BackendTaskSuccessResult::Identity(identity_result) => {
            classify_identity_result(identity_result)
        }
        BackendTaskSuccessResult::Wallet(wallet_result) => {
            classify_wallet_result(wallet_result)
        }
        BackendTaskSuccessResult::Core(_) => TaskResultPayloadDto::CoreCompleted,
        BackendTaskSuccessResult::Document(doc_result) => classify_document_result(doc_result),
        BackendTaskSuccessResult::Contract(contract_result) => {
            classify_contract_result(contract_result)
        }
        BackendTaskSuccessResult::Contest(_) => TaskResultPayloadDto::ContestCompleted,
        BackendTaskSuccessResult::System(_) => TaskResultPayloadDto::SystemCompleted,
        BackendTaskSuccessResult::Platform(platform_result) => {
            classify_platform_result(platform_result)
        }
        BackendTaskSuccessResult::DashPay(dashpay_result) => {
            classify_dashpay_result(dashpay_result)
        }
        BackendTaskSuccessResult::GroveSTARK(_) => TaskResultPayloadDto::GroveStarkCompleted,
        BackendTaskSuccessResult::MnList(mnlist_result) => classify_mnlist_result(mnlist_result),
        BackendTaskSuccessResult::Token(token_result) => {
            classify_token_result(token_result)
        }
        BackendTaskSuccessResult::BroadcastedStateTransition => {
            TaskResultPayloadDto::BroadcastedStateTransition
        }
    }
}

/// Extract the identity ID (if available) from an `IdentityResult`.
fn classify_identity_result(result: &IdentityResult) -> TaskResultPayloadDto {
    let identity_id = match result {
        IdentityResult::RegisteredIdentity(qi, _)
        | IdentityResult::ToppedUpIdentity(qi, _)
        | IdentityResult::RefreshedIdentity(qi)
        | IdentityResult::LoadedIdentity(qi)
        | IdentityResult::DisabledKeys(qi, _)
        | IdentityResult::ReplacedKey(qi, _) => {
            Some(hex::encode(qi.identity.id().to_buffer()))
        }
        IdentityResult::AddedKeyToIdentity(_)
        | IdentityResult::TransferredCredits(_)
        | IdentityResult::WithdrewFromIdentity(_)
        | IdentityResult::RegisteredDpnsName { .. } => None,
    };
    TaskResultPayloadDto::IdentityCompleted { identity_id }
}

/// Classify a `WalletResult` into a typed payload.
///
/// `GeneratedReceiveAddress` carries the real address for the frontend.
/// All other wallet results signal generic completion.
fn classify_wallet_result(result: &WalletResult) -> TaskResultPayloadDto {
    match result {
        WalletResult::GeneratedReceiveAddress { address, .. } => {
            TaskResultPayloadDto::WalletGeneratedAddress {
                address: address.clone(),
            }
        }
        _ => TaskResultPayloadDto::WalletCompleted,
    }
}

/// Classify a `DashPayResult` into a typed payload.
///
/// Most DashPay results trigger a generic reload from the local database.
/// `ProfileSearchResults` is special — the results are transient (not stored in DB),
/// so we serialize them here for the frontend to consume directly.
fn classify_dashpay_result(result: &DashPayResult) -> TaskResultPayloadDto {
    use dash_sdk::dpp::document::{Document, DocumentV0Getters};
    match result {
        DashPayResult::ProfileSearchResults(results) => {
            let items: Vec<ProfileSearchResultDto> = results
                .iter()
                .map(|(identity_id, profile_doc, username)| {
                    let (display_name, public_message, avatar_url) =
                        if let Some(document) = profile_doc {
                            let properties = match document {
                                Document::V0(doc_v0) => doc_v0.properties(),
                            };
                            (
                                properties
                                    .get("displayName")
                                    .and_then(|v| v.as_text())
                                    .map(|s| s.to_string()),
                                properties
                                    .get("publicMessage")
                                    .and_then(|v| v.as_text())
                                    .map(|s| s.to_string()),
                                properties
                                    .get("avatarUrl")
                                    .and_then(|v| v.as_text())
                                    .map(|s| s.to_string()),
                            )
                        } else {
                            (None, None, None)
                        };

                    ProfileSearchResultDto {
                        identity_id: hex::encode(identity_id.to_buffer()),
                        username: username.clone(),
                        display_name,
                        public_message,
                        avatar_url,
                    }
                })
                .collect();
            TaskResultPayloadDto::DashPayProfileSearchResults { results: items }
        }
        _ => TaskResultPayloadDto::DashPayCompleted,
    }
}

/// Classify a `TokenResult` into a typed payload.
///
/// Transient results (pricing, search, estimates) carry their data directly.
/// Mutation results (mint, burn, etc.) signal generic completion for re-fetch.
fn classify_token_result(result: &TokenResult) -> TaskResultPayloadDto {
    match result {
        TokenResult::FrozenIdentities(ids) => {
            let identity_ids: Vec<String> =
                ids.iter().map(|id| hex::encode(id.to_buffer())).collect();
            TaskResultPayloadDto::TokenFrozenIdentities { identity_ids }
        }
        TokenResult::TokenPricing { token_id, prices } => {
            TaskResultPayloadDto::TokenPricing {
                token_id: hex::encode(token_id.to_buffer()),
                prices: prices
                    .as_ref()
                    .and_then(|p| serde_json::to_value(p).ok()),
            }
        }
        TokenResult::EstimatedDistributionRewards(iti, amount, explanation) => {
            TaskResultPayloadDto::TokenRewardEstimate {
                token_id: hex::encode(iti.token_id.to_buffer()),
                identity_id: hex::encode(iti.identity_id.to_buffer()),
                amount: amount.to_string(),
                explanation: explanation.detailed_explanation(),
            }
        }
        TokenResult::DescriptionsByKeyword(descriptions, cursor) => {
            let results: Vec<TokenSearchResultDto> = descriptions
                .iter()
                .map(|desc| TokenSearchResultDto {
                    contract_id: hex::encode(desc.data_contract_id.to_buffer()),
                    description: desc.description.clone(),
                })
                .collect();
            TaskResultPayloadDto::TokenSearchResults {
                results,
                has_more: cursor.is_some(),
            }
        }
        TokenResult::TokenNotFound => TaskResultPayloadDto::TokenNotFound,
        TokenResult::FetchedTokenBalances => TaskResultPayloadDto::TokenBalancesLoaded,
        _ => TaskResultPayloadDto::TokenCompleted,
    }
}

/// Classify a `ContractResult` into a typed payload.
///
/// `WithDescriptions` carries transient data (not stored in DB).
/// All other contract results signal generic completion.
fn classify_contract_result(result: &ContractResult) -> TaskResultPayloadDto {
    match result {
        ContractResult::WithDescriptions(map) => {
            let contracts: Vec<ContractWithTokensDto> = map
                .iter()
                .map(|(contract_id, (desc_info, tokens))| {
                    let description = desc_info.as_ref().map(|d| d.description.clone());
                    let token_dtos: Vec<ContractTokenInfoDto> = tokens
                        .iter()
                        .map(|t| {
                            let config_json =
                                serde_json::to_value(&t.token_configuration).ok();
                            ContractTokenInfoDto {
                                token_id: hex::encode(t.token_id.to_buffer()),
                                name: t.token_name.clone(),
                                description: t.description.clone(),
                                token_position: t.token_position,
                                configuration_json: config_json,
                            }
                        })
                        .collect();

                    ContractWithTokensDto {
                        contract_id: hex::encode(contract_id.to_buffer()),
                        description,
                        tokens: token_dtos,
                    }
                })
                .collect();

            TaskResultPayloadDto::ContractWithDescriptions { contracts }
        }
        _ => TaskResultPayloadDto::ContractCompleted,
    }
}

/// Classify a `DocumentResult` into a typed payload.
///
/// `Page` and `Fetched` results carry documents for direct frontend consumption.
/// Other document results (Broadcasted, Deleted, etc.) just signal completion.
fn classify_document_result(result: &DocumentResult) -> TaskResultPayloadDto {
    fn serialize_doc_page<'a>(
        entries: impl Iterator<
            Item = (
                &'a dash_sdk::platform::Identifier,
                &'a Option<dash_sdk::platform::Document>,
            ),
        >,
        has_more: bool,
    ) -> TaskResultPayloadDto {
        let documents: Vec<serde_json::Value> = entries
            .filter_map(|(_, doc_opt)| {
                doc_opt
                    .as_ref()
                    .and_then(|doc| serde_json::to_value(doc).ok())
            })
            .collect();

        TaskResultPayloadDto::DocumentPage {
            documents,
            has_more,
        }
    }

    match result {
        DocumentResult::Page(page_docs, next_cursor) => {
            serialize_doc_page(page_docs.iter(), next_cursor.is_some())
        }
        DocumentResult::Fetched(docs) => serialize_doc_page(docs.iter(), false),
        _ => TaskResultPayloadDto::DocumentCompleted,
    }
}

/// Classify a `PlatformResult` into a typed payload.
fn classify_platform_result(result: &PlatformResult) -> TaskResultPayloadDto {
    match result {
        PlatformResult::Info(info) => match info {
            PlatformInfoTaskResult::BasicPlatformInfo {
                platform_version,
                core_chain_lock_height,
                network,
            } => {
                let text = format!(
                    "Platform Version Information:\n\n\
                     • Protocol Version: {}\n\
                     • Fee Version: {:?}\n\
                     • Drive ABCI Version: {:?}\n\
                     • Drive Version: {:?}\n\
                     • DPP Version: {:?}\n\
                     • System Data Contracts:\n\
                       - DPNS: {}\n\
                       - Withdrawals: {}\n\n\
                     Network: {:?}\n\
                     Chain Lock Height: {}",
                    platform_version.protocol_version,
                    platform_version.fee_version,
                    platform_version.drive_abci,
                    platform_version.drive,
                    platform_version.dpp,
                    platform_version.system_data_contracts.dpns,
                    platform_version.system_data_contracts.withdrawals,
                    network,
                    core_chain_lock_height.map_or("Not available".to_string(), |h| h.to_string())
                );
                TaskResultPayloadDto::PlatformText {
                    title: "Basic Platform Information".to_string(),
                    data: text,
                }
            }
            PlatformInfoTaskResult::TextResult(text) => {
                let title = text
                    .split("\n\n")
                    .next()
                    .map(|s| s.trim_end_matches(':'))
                    .unwrap_or("Platform Information")
                    .to_string();
                TaskResultPayloadDto::PlatformText {
                    title,
                    data: text.clone(),
                }
            }
            PlatformInfoTaskResult::AddressBalance {
                address,
                balance,
                nonce,
            } => TaskResultPayloadDto::PlatformAddressBalance {
                address: address.clone(),
                balance: *balance,
                nonce: *nonce,
            },
        },
    }
}

/// Classify a `MnListResult` into a typed payload.
fn classify_mnlist_result(result: &MnListResult) -> TaskResultPayloadDto {
    match result {
        MnListResult::FetchedDiff {
            base_height,
            height,
            diff,
        } => {
            let diff_dto = convert_mn_list_diff(diff);
            TaskResultPayloadDto::MnListFetchedDiff {
                base_height: *base_height,
                height: *height,
                diff: serde_json::to_value(&diff_dto).unwrap_or_default(),
            }
        }
        MnListResult::FetchedQrInfo { qr_info } => {
            let qr_info_dto = convert_qr_info(qr_info);
            TaskResultPayloadDto::MnListFetchedQrInfo {
                qr_info: serde_json::to_value(&qr_info_dto).unwrap_or_default(),
            }
        }
        MnListResult::ChainLockSigs { entries } => {
            let items: Vec<TaskChainLockSigDto> = entries
                .iter()
                .map(|((height, hash), sig)| TaskChainLockSigDto {
                    height: *height,
                    block_hash: hash.to_string(),
                    signature: sig.as_ref().map(|s| hex::encode(s.as_bytes())),
                })
                .collect();
            TaskResultPayloadDto::MnListChainLockSigs { entries: items }
        }
        MnListResult::FetchedDiffs { items } => {
            let diffs: Vec<serde_json::Value> = items
                .iter()
                .map(|((base_h, h), diff)| {
                    let diff_dto = convert_mn_list_diff(diff);
                    serde_json::json!({
                        "baseHeight": base_h,
                        "height": h,
                        "diff": serde_json::to_value(&diff_dto).unwrap_or_default()
                    })
                })
                .collect();
            TaskResultPayloadDto::MnListFetchedDiffs { diffs }
        }
    }
}

/// Start the ZMQ event forwarding loop for a given network.
///
/// Spawns a background task that receives ZMQ messages from the `CoreZMQListener`
/// channel and emits them as Tauri events.
pub fn start_zmq_forwarding(
    app_handle: AppHandle,
    network: Network,
    zmq_receiver: crossbeam_channel::Receiver<(ZMQMessage, Network)>,
    zmq_status_receiver: crossbeam_channel::Receiver<ZMQConnectionEvent>,
    app_state: Arc<AppState>,
) {
    // Forward ZMQ messages (transactions, blocks)
    let handle_msg = app_handle.clone();
    let state_msg = app_state.clone();
    std::thread::spawn(move || {
        while let Ok((message, msg_network)) = zmq_receiver.recv() {
            let ctx = state_msg.context_for_network(msg_network);
            let net_dto = NetworkDto::from_network(msg_network);

            match message {
                ZMQMessage::ISLockedTransaction(ref tx, ref is_lock) => {
                    // Process the transaction finality in the backend
                    match ctx.received_transaction_finality(tx, Some(is_lock.clone()), None) {
                        Ok(utxos) => {
                            emit_wallet_updated_events(&handle_msg, &ctx, &utxos, net_dto);
                            let txid = format!("{}", tx.txid());
                            let _ = ZmqIsLockedTransactionEvent {
                                network: net_dto,
                                txid,
                                raw_tx: serialize_tx_hex(tx),
                                raw_is_lock: serialize_consensus_hex(is_lock),
                                affected_utxo_count: utxos.len() as u32,
                                is_valid: true, // Passed finality check
                            }
                            .emit(&handle_msg);
                        }
                        Err(e) => {
                            tracing::error!("Failed to process IS-locked transaction: {}", e);
                        }
                    }
                }
                ZMQMessage::ChainLockedLockedTransaction(ref tx, height) => {
                    match ctx.received_transaction_finality(tx, None, Some(height)) {
                        Ok(utxos) => {
                            emit_wallet_updated_events(&handle_msg, &ctx, &utxos, net_dto);
                        }
                        Err(e) => {
                            tracing::error!("Failed to process chain-locked transaction: {}", e);
                        }
                    }
                }
                ZMQMessage::ChainLockedBlock(ref block, ref chain_lock) => {
                    let block_hash = format!("{}", block.block_hash());
                    let tx_ids: Vec<String> = block
                        .txdata
                        .iter()
                        .map(|tx| format!("{}", tx.txid()))
                        .collect();
                    let signature = hex::encode(chain_lock.signature.to_bytes());
                    let _ = ZmqChainLockedBlockEvent {
                        network: net_dto,
                        block_height: block.bip34_block_height().unwrap_or(0) as u32,
                        block_hash,
                        tx_count: block.txdata.len() as u32,
                        tx_ids,
                        raw_block: serialize_consensus_hex(block),
                        raw_chain_lock: serialize_consensus_hex(chain_lock),
                        signature,
                        is_valid: true, // Received via ZMQ — assumed valid
                    }
                    .emit(&handle_msg);
                }
            }
        }
        tracing::info!(?network, "ZMQ message forwarding loop ended");
    });

    // Forward ZMQ connection status changes
    let status_network_dto = NetworkDto::from_network(network);
    std::thread::spawn(move || {
        while let Ok(event) = zmq_status_receiver.recv() {
            let connected = matches!(event, ZMQConnectionEvent::Connected);
            let _ = ZmqConnectionStatusEvent {
                network: status_network_dto,
                connected,
            }
            .emit(&app_handle);
        }
        tracing::info!(?network, "ZMQ status forwarding loop ended");
    });
}

/// Start the scheduled vote polling loop.
///
/// Checks every 60 seconds for due scheduled votes and dispatches them as
/// BackendTasks. Results are emitted as `ScheduledVoteExecutedEvent`.
pub fn start_scheduled_vote_polling(app_handle: AppHandle, app_state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;

            let ctx = app_state.current_context().clone();

            // Query due votes
            let db_votes = match ctx.get_scheduled_votes() {
                Ok(votes) => votes,
                Err(e) => {
                    tracing::error!("Error querying scheduled votes: {}", e);
                    continue;
                }
            };

            let current_time = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let due_votes: Vec<_> = db_votes
                .into_iter()
                .filter(|v| {
                    v.unix_timestamp <= current_time
                        && !v.executed_successfully
                        && (v.unix_timestamp + 120000 >= current_time)
                })
                .collect();

            if due_votes.is_empty() {
                continue;
            }

            let local_identities = match ctx.load_local_voting_identities() {
                Ok(ids) => ids,
                Err(e) => {
                    tracing::error!("Error querying local voting identities: {}", e);
                    continue;
                }
            };

            for vote in due_votes {
                let voter_id_hex = hex::encode(vote.voter_id.as_slice());
                let contested_name = vote.contested_name.clone();

                if let Some(voter) = local_identities.iter().find(|i| {
                    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
                    i.identity.id() == vote.voter_id
                }) {
                    let task = BackendTask::ContestedResourceTask(
                        dash_evo_tool::backend_task::contested_names::ContestedResourceTask::CastScheduledVote(
                            vote.clone(),
                            Box::new(voter.clone()),
                        ),
                    );

                    // Create a headless sender for the task
                    let (tx, mut rx) = tokiompsc::channel::<TaskResult>(64);
                    let sender = SenderAsync::new_headless(tx);

                    // Drain intermediate progress — scheduled votes run headless
                    tauri::async_runtime::spawn(async move {
                        while rx.recv().await.is_some() {}
                    });

                    let ctx_clone = ctx.clone();
                    let handle = app_handle.clone();
                    let vid = voter_id_hex.clone();
                    let cname = contested_name.clone();

                    tauri::async_runtime::spawn(async move {
                        let result = ctx_clone.run_backend_task(task, sender).await;
                        match result {
                            Ok(_) => {
                                // Mark the vote as executed in the database
                                let _ = ctx_clone.mark_vote_executed(
                                    vote.voter_id.as_slice(),
                                    vote.contested_name.clone(),
                                );
                                let _ = ScheduledVoteExecutedEvent {
                                    contested_name: cname,
                                    voter_id: vid,
                                    success: true,
                                    error: None,
                                }
                                .emit(&handle);
                            }
                            Err(e) => {
                                let _ = ScheduledVoteExecutedEvent {
                                    contested_name: cname,
                                    voter_id: vid,
                                    success: false,
                                    error: Some(e.to_string()),
                                }
                                .emit(&handle);
                            }
                        }
                    });
                } else {
                    tracing::warn!(
                        voter_id = %voter_id_hex,
                        contested_name = %contested_name,
                        "Voter identity not found for scheduled vote"
                    );
                }
            }
        }
    });
}

/// SPV status polling loop — periodically reads SPV status and emits events.
pub fn start_spv_status_polling(app_handle: AppHandle, app_state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;

            for network in app_state.available_networks() {
                let ctx = app_state.context_for_network(network);
                let network_dto = NetworkDto::from_network(network);

                let spv_manager = ctx.spv_manager();
                let snapshot = spv_manager.status();

                let status = convert_spv_status(snapshot.status);

                // Use the overall sync percentage from the SPV progress watcher
                // (average of headers + filter_headers + filters, each 0.0–1.0).
                let sync_pct = if snapshot.sync_percentage > 0.0 {
                    Some(snapshot.sync_percentage * 100.0)
                } else {
                    None
                };
                let header_height = snapshot.sync_progress.as_ref().map(|p| p.header_height);

                // Per-category progress (0.0–100.0 scale), only when syncing
                let is_syncing = matches!(
                    snapshot.status,
                    dash_evo_tool::spv::SpvStatus::Syncing
                        | dash_evo_tool::spv::SpvStatus::Starting
                        | dash_evo_tool::spv::SpvStatus::Running
                );
                let bd = &snapshot.sync_breakdown;
                let (h_pct, mn_pct, fh_pct, f_pct, b_pct) = if is_syncing {
                    (
                        Some(bd.headers_pct * 100.0),
                        Some(bd.masternodes_pct * 100.0),
                        Some(bd.filter_headers_pct * 100.0),
                        Some(bd.filters_pct * 100.0),
                        Some(bd.blocks_pct * 100.0),
                    )
                } else {
                    (None, None, None, None, None)
                };

                let _ = SpvStatusEvent {
                    network: network_dto,
                    status,
                    sync_progress_pct: sync_pct,
                    header_height,
                    headers_progress_pct: h_pct,
                    masternodes_progress_pct: mn_pct,
                    filter_headers_progress_pct: fh_pct,
                    filters_progress_pct: f_pct,
                    blocks_progress_pct: b_pct,
                    connected_peers: snapshot.connected_peers as u32,
                    error: snapshot.last_error,
                }
                .emit(&app_handle);
            }
        }
    });
}

/// Convert SPV status from the DET crate to the DTO enum.
fn convert_spv_status(status: dash_evo_tool::spv::SpvStatus) -> SpvStatusDto {
    match status {
        dash_evo_tool::spv::SpvStatus::Idle => SpvStatusDto::Idle,
        dash_evo_tool::spv::SpvStatus::Starting => SpvStatusDto::Starting,
        dash_evo_tool::spv::SpvStatus::Syncing => SpvStatusDto::Syncing,
        dash_evo_tool::spv::SpvStatus::Running => SpvStatusDto::Running,
        dash_evo_tool::spv::SpvStatus::Stopping => SpvStatusDto::Stopping,
        dash_evo_tool::spv::SpvStatus::Stopped => SpvStatusDto::Stopped,
        dash_evo_tool::spv::SpvStatus::Error => SpvStatusDto::Error,
    }
}

/// Emit `WalletUpdatedEvent` for each wallet that owns any of the given addresses.
fn emit_wallet_updated_events(
    app_handle: &AppHandle,
    ctx: &dash_evo_tool::context::AppContext,
    utxos: &[(dash_sdk::dpp::dashcore::OutPoint, dash_sdk::dpp::dashcore::TxOut, dash_sdk::dpp::dashcore::Address)],
    net_dto: NetworkDto,
) {
    use std::collections::BTreeSet;

    if utxos.is_empty() {
        return;
    }

    let addresses: BTreeSet<_> = utxos.iter().map(|(_, _, addr)| addr).collect();

    for (seed_hash, wallet_arc) in ctx.loaded_wallets() {
        let wallet = match wallet_arc.read() {
            Ok(w) => w,
            Err(_) => continue,
        };
        let affected = addresses
            .iter()
            .any(|addr| wallet.known_addresses.contains_key(addr));
        if affected {
            let _ = WalletUpdatedEvent {
                wallet_seed_hash: hex::encode(seed_hash),
                network: net_dto,
            }
            .emit(app_handle);
        }
    }
}

/// Serialize a transaction to hex string.
fn serialize_tx_hex(tx: &dash_sdk::dpp::dashcore::Transaction) -> String {
    let mut buf = Vec::new();
    tx.consensus_encode(&mut buf).unwrap_or_default();
    hex::encode(&buf)
}

/// Serialize any consensus-encodable type to hex string.
fn serialize_consensus_hex<T: Encodable>(item: &T) -> String {
    let mut buf = Vec::new();
    item.consensus_encode(&mut buf).unwrap_or_default();
    hex::encode(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_is_unique() {
        let id1 = next_task_id();
        let id2 = next_task_id();
        let id3 = next_task_id();
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert!(id1.starts_with("task-"));
    }

    #[test]
    fn classify_none_result() {
        let result = classify_success_result(&BackendTaskSuccessResult::None);
        assert!(matches!(result, TaskResultPayloadDto::None));
    }

    #[test]
    fn classify_refresh_result() {
        let result = classify_success_result(&BackendTaskSuccessResult::Refresh);
        assert!(matches!(result, TaskResultPayloadDto::Refresh));
    }

    #[test]
    fn classify_message_result() {
        let result =
            classify_success_result(&BackendTaskSuccessResult::Message("hello".into()));
        match result {
            TaskResultPayloadDto::Message { text } => assert_eq!(text, "hello"),
            other => panic!("Expected Message, got {:?}", other),
        }
    }

    #[test]
    fn classify_broadcast_result() {
        let result =
            classify_success_result(&BackendTaskSuccessResult::BroadcastedStateTransition);
        assert!(matches!(
            result,
            TaskResultPayloadDto::BroadcastedStateTransition
        ));
    }

    #[test]
    fn task_domain_maps_correctly() {
        assert_eq!(task_domain(&BackendTask::None), TaskDomain::General);
    }
}
