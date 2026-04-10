//! Typed MCP tool errors with conversion to rmcp `ErrorData`.

use crate::backend_task::error::TaskError;
use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;

/// Unified error type for MCP tool invocations.
#[derive(Debug, thiserror::Error)]
pub enum McpToolError {
    #[error("Wallet not found: {id}")]
    WalletNotFound { id: String },
    #[error("Invalid parameter: {message}")]
    InvalidParam { message: String },
    #[error("Network mismatch: expected {expected}, got {actual}")]
    NetworkMismatch { expected: String, actual: String },
    #[error("SPV sync incomplete — please wait and retry")]
    SpvSyncFailed,
    #[error("Backend task failed: {0}")]
    TaskFailed(#[source] TaskError),
    #[error("{0}")]
    Internal(String),
}

/// MCP error codes for each variant (using JSON-RPC custom code range).
const CODE_WALLET_NOT_FOUND: i32 = -32001;
const CODE_INVALID_PARAM: i32 = -32602; // standard JSON-RPC invalid params
const CODE_NETWORK_MISMATCH: i32 = -32002;
const CODE_SPV_SYNC_FAILED: i32 = -32003;
const CODE_TASK_FAILED: i32 = -32004;
const CODE_INTERNAL: i32 = -32603; // standard JSON-RPC internal error

impl From<McpToolError> for McpError {
    fn from(e: McpToolError) -> Self {
        let (code, msg, data) = match &e {
            McpToolError::WalletNotFound { .. } => (CODE_WALLET_NOT_FOUND, e.to_string(), None),
            McpToolError::InvalidParam { .. } => (CODE_INVALID_PARAM, e.to_string(), None),
            McpToolError::NetworkMismatch { .. } => (CODE_NETWORK_MISMATCH, e.to_string(), None),
            McpToolError::SpvSyncFailed => (CODE_SPV_SYNC_FAILED, e.to_string(), None),
            McpToolError::TaskFailed(task_err) => {
                // Include the full Debug error chain so MCP clients can see
                // the underlying cause (e.g. SDK/DAPI errors) instead of just
                // the user-friendly Display message.
                let details = format!("{task_err:?}");
                (
                    CODE_TASK_FAILED,
                    e.to_string(),
                    Some(serde_json::Value::String(details)),
                )
            }
            McpToolError::Internal(_) => (CODE_INTERNAL, e.to_string(), None),
        };
        McpError {
            code: ErrorCode(code),
            message: msg.into(),
            data,
        }
    }
}
