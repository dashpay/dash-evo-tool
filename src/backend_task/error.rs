//! Typed error envelope for backend tasks.
//!
//! `Display` → user-friendly text (shown in `MessageBanner`).
//! `Debug` → variant name + fields (logged and shown in collapsible details).
//! `From<String>` → backwards compatible with existing `Result<T, String>` code.

use thiserror::Error;

/// App-level error envelope for backend tasks.
#[derive(Debug, Error)]
pub enum TaskError {
    /// Legacy string error — backwards compatible with all existing code.
    #[error("{0}")]
    Generic(String),

    /// SPV subsystem errors.
    #[error("{0}")]
    Spv(#[from] crate::spv::SpvError),

    /// DashPay domain errors.
    #[error("{0}")]
    DashPay(#[from] crate::backend_task::dashpay::errors::DashPayError),

    /// Configuration errors.
    #[error("{0}")]
    Config(#[from] crate::config::ConfigError),

    /// GroveSTARK prover errors.
    #[error("{0}")]
    GroveStark(#[from] crate::model::grovestark_prover::GroveSTARKError),

    /// Wallet errors.
    #[error("{0}")]
    Wallet(#[from] crate::database::WalletError),
}

impl From<String> for TaskError {
    fn from(s: String) -> Self {
        TaskError::Generic(s)
    }
}
