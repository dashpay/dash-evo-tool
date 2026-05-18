//! SPV status display value types.
//!
//! These are DET-owned UI/connection display types, decoupled from any chain
//! sync engine. Chain sync is owned by upstream `platform-wallet`'s
//! `SpvRuntime`; P1 introduces an `EventBridge` that feeds these values from
//! upstream sync events. Until then they default to `Idle`/empty.

use dash_sdk::dash_spv::sync::SyncProgress as SpvSyncProgress;
use std::time::SystemTime;

/// High-level status of the SPV client runtime, for UI display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum SpvStatus {
    #[default]
    Idle = 0,
    Starting = 1,
    Syncing = 2,
    Running = 3,
    Stopping = 4,
    Stopped = 5,
    Error = 6,
}

impl SpvStatus {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            SpvStatus::Starting | SpvStatus::Syncing | SpvStatus::Running | SpvStatus::Stopping
        )
    }
}

impl std::fmt::Display for SpvStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpvStatus::Idle => write!(f, "Idle"),
            SpvStatus::Starting => write!(f, "Starting"),
            SpvStatus::Syncing => write!(f, "Syncing"),
            SpvStatus::Running => write!(f, "Running"),
            SpvStatus::Stopping => write!(f, "Stopping"),
            SpvStatus::Stopped => write!(f, "Stopped"),
            SpvStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<u8> for SpvStatus {
    fn from(value: u8) -> Self {
        match value {
            1 => SpvStatus::Starting,
            2 => SpvStatus::Syncing,
            3 => SpvStatus::Running,
            4 => SpvStatus::Stopping,
            5 => SpvStatus::Stopped,
            6 => SpvStatus::Error,
            _ => SpvStatus::Idle,
        }
    }
}

/// Snapshot of the SPV runtime state for UI consumption.
#[derive(Debug, Clone, Default)]
pub struct SpvStatusSnapshot {
    pub status: SpvStatus,
    pub sync_progress: Option<SpvSyncProgress>,
    pub last_error: Option<String>,
    pub started_at: Option<SystemTime>,
    pub last_updated: Option<SystemTime>,
    pub connected_peers: usize,
}
