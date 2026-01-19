//! Error types for SPV operations.

use thiserror::Error;

/// Errors that can occur during SPV operations.
#[derive(Debug, Error, Clone)]
pub enum SpvError {
    /// A lock was poisoned (another thread panicked while holding it)
    #[error("SPV lock poisoned: {0}")]
    LockPoisoned(String),

    /// SPV client is not initialized
    #[error("SPV client not initialized")]
    ClientNotInitialized,

    /// SPV client is not running
    #[error("SPV client not running")]
    NotRunning,

    /// Sync operation failed
    #[error("SPV sync failed: {0}")]
    SyncFailed(String),

    /// Network operation failed
    #[error("SPV network error: {0}")]
    NetworkError(String),

    /// Wallet operation failed
    #[error("SPV wallet error: {0}")]
    WalletError(String),

    /// Configuration error
    #[error("SPV configuration error: {0}")]
    ConfigError(String),

    /// Channel communication error
    #[error("SPV channel error: {0}")]
    ChannelError(String),

    /// Generic error
    #[error("{0}")]
    Other(String),
}

impl From<String> for SpvError {
    fn from(s: String) -> Self {
        SpvError::Other(s)
    }
}

impl From<&str> for SpvError {
    fn from(s: &str) -> Self {
        SpvError::Other(s.to_string())
    }
}

/// Result type for SPV operations.
pub type SpvResult<T> = Result<T, SpvError>;
