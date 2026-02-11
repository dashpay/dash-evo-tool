//! Tauri event types for async communication from backend to frontend.
//!
//! Replaces the egui channel-based polling system with Tauri's native event
//! system. Events are emitted via `AppHandle::emit()` and listened to on the
//! frontend via `listen()`.
//!
//! All event types derive `tauri_specta::Event` for automatic TypeScript
//! type generation and type-safe event listeners.

use crate::dto::NetworkDto;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

// ---------------------------------------------------------------------------
// Backend task events
// ---------------------------------------------------------------------------

/// Emitted when a backend task completes successfully.
///
/// The `result_type` field is a discriminant string (e.g. "Identity", "Wallet",
/// "Contract") that the frontend can match on. The `payload` is the serialized
/// result data as JSON value. This design avoids needing to represent the entire
/// `BackendTaskSuccessResult` enum in the TypeScript type system up front —
/// individual domain result types will be defined as the IPC commands are
/// implemented in phases 1.4–1.7.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct TaskResultEvent {
    /// Unique task ID assigned at dispatch time.
    pub task_id: String,
    /// The domain of the result (e.g. "Identity", "Wallet", "None", "Refresh").
    pub result_type: String,
    /// The serialized result payload. `null` for `None` and `Refresh` results.
    pub payload: Option<serde_json::Value>,
}

/// Emitted when a backend task fails.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct TaskErrorEvent {
    /// Unique task ID assigned at dispatch time.
    pub task_id: String,
    /// User-friendly error message suitable for display.
    pub message: String,
    /// Technical error details for debugging.
    pub details: String,
    /// Whether the operation can be retried.
    pub recoverable: bool,
}

// ---------------------------------------------------------------------------
// ZMQ events (Dash Core → Frontend)
// ---------------------------------------------------------------------------

/// Emitted when an InstantSend-locked transaction is received via ZMQ.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ZmqIsLockedTransactionEvent {
    /// The network this event belongs to.
    pub network: NetworkDto,
    /// Transaction ID (hex).
    pub txid: String,
    /// Serialized transaction data (hex).
    pub raw_tx: String,
    /// Serialized InstantLock data (hex).
    pub raw_is_lock: String,
    /// Number of UTXOs affected by this transaction.
    pub affected_utxo_count: u32,
    /// Whether the InstantSend lock signature was verified successfully.
    pub is_valid: bool,
}

/// Emitted when a chain-locked block is received via ZMQ.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ZmqChainLockedBlockEvent {
    /// The network this event belongs to.
    pub network: NetworkDto,
    /// Block height.
    pub block_height: u32,
    /// Block hash (hex).
    pub block_hash: String,
    /// Number of transactions in the block.
    pub tx_count: u32,
    /// Transaction IDs in this block (hex).
    pub tx_ids: Vec<String>,
    /// Serialized block data (hex).
    pub raw_block: String,
    /// Serialized ChainLock signature data (hex).
    pub raw_chain_lock: String,
    /// ChainLock signature (hex).
    pub signature: String,
    /// Whether the chain lock signature was verified successfully.
    pub is_valid: bool,
}

/// Emitted when ZMQ connection status changes.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ZmqConnectionStatusEvent {
    /// The network this status change belongs to.
    pub network: NetworkDto,
    /// Whether ZMQ is currently connected.
    pub connected: bool,
}

// ---------------------------------------------------------------------------
// SPV events
// ---------------------------------------------------------------------------

/// High-level SPV status values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SpvStatusDto {
    Idle,
    Starting,
    Syncing,
    Running,
    Stopping,
    Stopped,
    Error,
}

/// Emitted when SPV sync status changes.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct SpvStatusEvent {
    /// The network this SPV instance belongs to.
    pub network: NetworkDto,
    /// Current SPV status.
    pub status: SpvStatusDto,
    /// Sync progress as a percentage (0.0–100.0), if syncing.
    pub sync_progress_pct: Option<f64>,
    /// Current header height, if known.
    pub header_height: Option<u32>,
    /// Number of connected peers.
    pub connected_peers: u32,
    /// Error message, if status is Error.
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Wallet events
// ---------------------------------------------------------------------------

/// Emitted when a wallet's balance or state changes.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct WalletUpdatedEvent {
    /// Which wallet changed (seed hash, hex-encoded).
    pub wallet_seed_hash: String,
    /// The network this wallet belongs to.
    pub network: NetworkDto,
}

// ---------------------------------------------------------------------------
// Scheduled vote events
// ---------------------------------------------------------------------------

/// Emitted when a scheduled vote is automatically cast.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledVoteExecutedEvent {
    /// The contested name the vote was for.
    pub contested_name: String,
    /// Voter identity ID (hex).
    pub voter_id: String,
    /// Whether the vote was cast successfully.
    pub success: bool,
    /// Error message if the vote failed.
    pub error: Option<String>,
}
