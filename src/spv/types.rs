//! Shared SPV types used across the event bridge and manager.
//!
//! Extracted from `manager.rs` so they survive manager deletion during
//! the platform-wallet migration.

use dash_sdk::dash_spv::sync::SyncProgress as SpvSyncProgress;
use dash_sdk::dash_spv::sync::SyncState;
use std::time::SystemTime;

/// High-level status of the SPV client runtime.
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
            0 => SpvStatus::Idle,
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
/// Uses dash-spv's built-in progress types directly instead of duplicating.
#[derive(Debug, Clone, Default)]
pub struct SpvStatusSnapshot {
    pub status: SpvStatus,
    pub sync_progress: Option<SpvSyncProgress>,
    pub last_error: Option<String>,
    pub started_at: Option<SystemTime>,
    pub last_updated: Option<SystemTime>,
    pub connected_peers: usize,
}

/// Preferred backend for Core-level operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoreBackendMode {
    #[default]
    Rpc = 0,
    Spv = 1,
}

impl CoreBackendMode {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl From<u8> for CoreBackendMode {
    fn from(value: u8) -> Self {
        match value {
            1 => CoreBackendMode::Spv,
            _ => CoreBackendMode::Rpc,
        }
    }
}

/// Identify which sync phase failed (for error messages).
///
/// Extracted from `SpvManager::failed_manager_name` so it can be used
/// by the event bridge without depending on the manager.
pub fn failed_manager_name(progress: &SpvSyncProgress) -> &'static str {
    if progress
        .masternodes()
        .is_ok_and(|p| p.state() == SyncState::Error)
    {
        return "Masternodes";
    }
    if progress
        .headers()
        .is_ok_and(|p| p.state() == SyncState::Error)
    {
        return "Headers";
    }
    if progress
        .filter_headers()
        .is_ok_and(|p| p.state() == SyncState::Error)
    {
        return "Filter headers";
    }
    if progress
        .filters()
        .is_ok_and(|p| p.state() == SyncState::Error)
    {
        return "Filters";
    }
    if progress
        .blocks()
        .is_ok_and(|p| p.state() == SyncState::Error)
    {
        return "Blocks";
    }
    "unknown phase"
}
