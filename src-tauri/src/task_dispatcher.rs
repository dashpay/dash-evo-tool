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

use crate::dto::NetworkDto;
use crate::events::{
    ScheduledVoteExecutedEvent, SpvStatusDto, SpvStatusEvent, TaskErrorEvent, TaskResultEvent,
    ZmqChainLockedBlockEvent, ZmqConnectionStatusEvent, ZmqIsLockedTransactionEvent,
};
use crate::state::AppState;
use dash_evo_tool::app::TaskResult;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::BackendTaskSuccessResult;
use dash_evo_tool::components::core_zmq_listener::{ZMQConnectionEvent, ZMQMessage};
use dash_evo_tool::utils::egui_mpsc::SenderAsync;
use dash_sdk::dpp::dashcore::consensus::Encodable;
use dash_sdk::dpp::dashcore::Network;
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

    // Create a headless sender — no egui context needed in Tauri.
    // The channel forwards intermediate progress messages from the backend
    // task system; we listen on the receiver and emit Tauri events.
    let (tx, mut rx) = tokiompsc::channel::<TaskResult>(64);
    let sender = SenderAsync::new_headless(tx);

    // Forward intermediate progress/refresh messages as events
    let handle_progress = handle.clone();
    let tid_progress = tid.clone();
    tokio::spawn(async move {
        while let Some(task_result) = rx.recv().await {
            match task_result {
                TaskResult::Refresh => {
                    let _ = TaskResultEvent {
                        task_id: tid_progress.clone(),
                        result_type: "Refresh".to_string(),
                        payload: None,
                    }
                    .emit(&handle_progress);
                }
                TaskResult::Success(result) => {
                    let (result_type, payload) = classify_success_result(&result);
                    let _ = TaskResultEvent {
                        task_id: tid_progress.clone(),
                        result_type,
                        payload,
                    }
                    .emit(&handle_progress);
                }
                TaskResult::Error(error) => {
                    let _ = TaskErrorEvent {
                        task_id: tid_progress.clone(),
                        message: error.user_message(),
                        details: error.technical_details(),
                        recoverable: error.is_recoverable(),
                    }
                    .emit(&handle_progress);
                }
            }
        }
    });

    // Spawn the actual task
    tokio::spawn(async move {
        let result = app_context.run_backend_task(task, sender).await;

        match result {
            Ok(success) => {
                let (result_type, payload) = classify_success_result(&success);
                let _ = TaskResultEvent {
                    task_id: tid,
                    result_type,
                    payload,
                }
                .emit(&handle);
            }
            Err(error) => {
                let _ = TaskErrorEvent {
                    task_id: tid,
                    message: error.user_message(),
                    details: error.technical_details(),
                    recoverable: error.is_recoverable(),
                }
                .emit(&handle);
            }
        }
    });

    task_id
}

/// Classify a `BackendTaskSuccessResult` into a type discriminant and optional
/// JSON payload for the event system.
///
/// NOTE: Full payload serialization will be implemented as domain DTOs are built
/// in tasks 1.4–1.7. For now we serialize what we can and use the type string
/// as a discriminant for the frontend to match on.
fn classify_success_result(
    result: &BackendTaskSuccessResult,
) -> (String, Option<serde_json::Value>) {
    match result {
        BackendTaskSuccessResult::None => ("None".to_string(), None),
        BackendTaskSuccessResult::Refresh => ("Refresh".to_string(), None),
        BackendTaskSuccessResult::Message(msg) => (
            "Message".to_string(),
            Some(serde_json::Value::String(msg.clone())),
        ),
        BackendTaskSuccessResult::Identity(_) => ("Identity".to_string(), None),
        BackendTaskSuccessResult::Wallet(_) => ("Wallet".to_string(), None),
        BackendTaskSuccessResult::Core(_) => ("Core".to_string(), None),
        BackendTaskSuccessResult::Document(_) => ("Document".to_string(), None),
        BackendTaskSuccessResult::Contract(_) => ("Contract".to_string(), None),
        BackendTaskSuccessResult::Contest(_) => ("Contest".to_string(), None),
        BackendTaskSuccessResult::System(_) => ("System".to_string(), None),
        BackendTaskSuccessResult::Platform(_) => ("Platform".to_string(), None),
        BackendTaskSuccessResult::DashPay(_) => ("DashPay".to_string(), None),
        BackendTaskSuccessResult::GroveSTARK(_) => ("GroveSTARK".to_string(), None),
        BackendTaskSuccessResult::MnList(_) => ("MnList".to_string(), None),
        BackendTaskSuccessResult::Token(_) => ("Token".to_string(), None),
        BackendTaskSuccessResult::BroadcastedStateTransition => {
            ("BroadcastedStateTransition".to_string(), None)
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
                            let txid = format!("{}", tx.txid());
                            let _ = ZmqIsLockedTransactionEvent {
                                network: net_dto,
                                txid,
                                raw_tx: serialize_tx_hex(tx),
                                affected_utxo_count: utxos.len() as u32,
                            }
                            .emit(&handle_msg);
                        }
                        Err(e) => {
                            tracing::error!("Failed to process IS-locked transaction: {}", e);
                        }
                    }
                }
                ZMQMessage::ChainLockedLockedTransaction(ref tx, height) => {
                    if let Err(e) = ctx.received_transaction_finality(tx, None, Some(height)) {
                        tracing::error!("Failed to process chain-locked transaction: {}", e);
                    }
                }
                ZMQMessage::ChainLockedBlock(ref block, ref _chain_lock) => {
                    let block_hash = format!("{}", block.block_hash());
                    let _ = ZmqChainLockedBlockEvent {
                        network: net_dto,
                        block_height: block.bip34_block_height().unwrap_or(0) as u32,
                        block_hash,
                        tx_count: block.txdata.len() as u32,
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
    tokio::spawn(async move {
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
                    let (tx, _rx) = tokiompsc::channel::<TaskResult>(16);
                    let sender = SenderAsync::new_headless(tx);

                    let ctx_clone = ctx.clone();
                    let handle = app_handle.clone();
                    let vid = voter_id_hex.clone();
                    let cname = contested_name.clone();

                    tokio::spawn(async move {
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
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;

            for network in app_state.available_networks() {
                let ctx = app_state.context_for_network(network);
                let network_dto = NetworkDto::from_network(network);

                let spv_manager = ctx.spv_manager();
                let snapshot = spv_manager.status();

                let status = convert_spv_status(snapshot.status);

                // SyncProgress from dash-spv has `header_height` (absolute height)
                // rather than a total/current pair — we report it as-is.
                let sync_pct = None::<f64>; // Not computable without a target height
                let header_height = snapshot.sync_progress.as_ref().map(|p| p.header_height);

                let _ = SpvStatusEvent {
                    network: network_dto,
                    status,
                    sync_progress_pct: sync_pct,
                    header_height,
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

/// Serialize a transaction to hex string.
fn serialize_tx_hex(tx: &dash_sdk::dpp::dashcore::Transaction) -> String {
    let mut buf = Vec::new();
    tx.consensus_encode(&mut buf).unwrap_or_default();
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
        let (ty, payload) = classify_success_result(&BackendTaskSuccessResult::None);
        assert_eq!(ty, "None");
        assert!(payload.is_none());
    }

    #[test]
    fn classify_refresh_result() {
        let (ty, payload) = classify_success_result(&BackendTaskSuccessResult::Refresh);
        assert_eq!(ty, "Refresh");
        assert!(payload.is_none());
    }

    #[test]
    fn classify_message_result() {
        let (ty, payload) =
            classify_success_result(&BackendTaskSuccessResult::Message("hello".into()));
        assert_eq!(ty, "Message");
        assert_eq!(payload.unwrap(), serde_json::Value::String("hello".into()));
    }

    #[test]
    fn classify_broadcast_result() {
        let (ty, payload) =
            classify_success_result(&BackendTaskSuccessResult::BroadcastedStateTransition);
        assert_eq!(ty, "BroadcastedStateTransition");
        assert!(payload.is_none());
    }
}
