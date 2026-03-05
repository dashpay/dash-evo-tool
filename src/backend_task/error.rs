//! Typed error envelope for backend tasks.
//!
//! `Display` → user-friendly text (shown in `MessageBanner`).
//! `Debug` → variant name + fields (logged and shown in collapsible details).
//! `From<String>` → backwards compatible with existing `Result<T, String>` code.
//!   Parses known error patterns into typed variants automatically.

use thiserror::Error;

/// App-level error envelope for backend tasks.
#[derive(Debug, Error)]
pub enum TaskError {
    /// Legacy string error — backwards compatible with all existing code.
    #[error("{0}")]
    Generic(String),

    /// Boxed error — catch-all for errors without a dedicated variant.
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),

    /// SPV subsystem errors.
    #[error(transparent)]
    Spv(#[from] crate::spv::SpvError),

    /// DashPay domain errors.
    #[error(transparent)]
    DashPay(#[from] crate::backend_task::dashpay::errors::DashPayError),

    /// Configuration errors.
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),

    /// GroveSTARK prover errors.
    #[error(transparent)]
    GroveStark(#[from] crate::model::grovestark_prover::GroveSTARKError),

    /// Wallet errors.
    #[error(transparent)]
    Wallet(#[from] crate::database::WalletError),

    /// Core wallet not configured for this wallet on a multi-wallet Core node.
    #[error(
        "Core wallet not configured for this wallet. Go to the Wallets screen and refresh to auto-detect the Core wallet association."
    )]
    CoreWalletNotConfigured {
        /// Seed hash (HD) or key hash (single-key) identifying the wallet.
        wallet_seed_hash: [u8; 32],
    },
}

impl From<String> for TaskError {
    fn from(s: String) -> Self {
        if s.contains("Wallet file not specified") {
            TaskError::CoreWalletNotConfigured {
                wallet_seed_hash: [0; 32],
            }
        } else {
            TaskError::Generic(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_string_detects_rpc_error_minus_19() {
        let err: TaskError = "JSON-RPC error: RPC error response: RpcError { code: -19, message: \"Wallet file not specified\" }".to_string().into();
        assert!(
            matches!(err, TaskError::CoreWalletNotConfigured { .. }),
            "Expected CoreWalletNotConfigured, got: {err:?}",
        );
    }

    #[test]
    fn from_string_passes_through_other_errors() {
        let err: TaskError = "Connection refused".to_string().into();
        assert!(
            matches!(err, TaskError::Generic(ref s) if s == "Connection refused"),
            "Expected Generic, got: {err:?}",
        );
    }

    #[test]
    fn display_message_is_user_friendly() {
        let msg = TaskError::CoreWalletNotConfigured {
            wallet_seed_hash: [0; 32],
        }
        .to_string();
        assert!(msg.contains("Wallets screen"));
        assert!(msg.contains("refresh"));
    }
}
