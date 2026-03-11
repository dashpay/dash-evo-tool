//! Typed error envelope for backend tasks.
//!
//! `Display` → user-friendly text (shown in `MessageBanner`).
//! `Debug` → variant name + fields (logged and shown in collapsible details).
//! `From<String>` → backwards compatible with existing `Result<T, String>` code.
//!   Parses known error patterns into typed variants automatically.

use dash_sdk::dashcore_rpc;
use thiserror::Error;

/// Dash Core RPC error code: wallet file not specified (multi-wallet node).
const RPC_WALLET_NOT_SPECIFIED: i32 = -19;

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

    /// SQLite errors.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// Tokio task join errors.
    #[error(transparent)]
    JoinError(#[from] tokio::task::JoinError),

    /// Core wallet not configured for this wallet on a multi-wallet Core node.
    #[error(
        "Core wallet not configured for this wallet. Go to the Wallets screen and refresh to auto-detect the Core wallet association."
    )]
    CoreWalletNotConfigured,

    /// The operation's prerequisite was auto-fixed (e.g., Core wallet detected).
    /// Callers should retry the failed operation.
    #[error("{0}")]
    MustRetry(String),

    /// Duplicate identity public key — the key data already exists on the platform.
    #[error(
        "This public key is already registered on the platform. Each key must be globally unique — try a different key."
    )]
    DuplicateIdentityPublicKey,

    /// Duplicate identity public key ID — the key hash is already taken platform-wide.
    #[error(
        "This key hash is already registered on the platform. Each key must be globally unique — try a different key."
    )]
    DuplicateIdentityPublicKeyId,

    /// Identity public key conflicts with an existing key's unique contract bounds.
    #[error(
        "This key conflicts with an existing key bound to contract {contract_id}. Use a different key or purpose."
    )]
    IdentityPublicKeyContractBoundsConflict { contract_id: String },
}

impl From<String> for TaskError {
    fn from(s: String) -> Self {
        if s.contains("Wallet file not specified") {
            TaskError::CoreWalletNotConfigured
        } else {
            TaskError::Generic(s)
        }
    }
}

impl From<dashcore_rpc::Error> for TaskError {
    fn from(e: dashcore_rpc::Error) -> Self {
        if let dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(ref rpc_err)) =
            e
            && rpc_err.code == RPC_WALLET_NOT_SPECIFIED
        {
            return TaskError::CoreWalletNotConfigured;
        }
        TaskError::Generic(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_string_detects_rpc_error_minus_19() {
        let err: TaskError = "JSON-RPC error: RPC error response: RpcError { code: -19, message: \"Wallet file not specified\" }".to_string().into();
        assert!(
            matches!(err, TaskError::CoreWalletNotConfigured),
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
        let msg = TaskError::CoreWalletNotConfigured.to_string();
        assert!(msg.contains("Wallets screen"));
        assert!(msg.contains("refresh"));
    }

    #[test]
    fn must_retry_displays_inner_message() {
        let err = TaskError::MustRetry("Auto-detected Core wallet 'mywallet'".to_string());
        assert_eq!(err.to_string(), "Auto-detected Core wallet 'mywallet'");
    }

    #[test]
    fn rpc_error_code_neg19_converts_to_core_wallet_not_configured() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -19,
            message: "Wallet file not specified".to_string(),
            data: None,
        };
        let err: TaskError =
            dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err)).into();
        assert!(
            matches!(err, TaskError::CoreWalletNotConfigured),
            "Expected CoreWalletNotConfigured, got: {err:?}"
        );
    }

    #[test]
    fn other_rpc_error_converts_to_generic() {
        let rpc_err = dashcore_rpc::jsonrpc::error::RpcError {
            code: -1,
            message: "Some other error".to_string(),
            data: None,
        };
        let err: TaskError =
            dashcore_rpc::Error::JsonRpc(dashcore_rpc::jsonrpc::error::Error::Rpc(rpc_err)).into();
        assert!(
            matches!(err, TaskError::Generic(_)),
            "Expected Generic, got: {err:?}"
        );
    }
}
