//! Typed MCP tool errors with conversion to rmcp `ErrorData`.

use crate::backend_task::error::TaskError;
use rmcp::ErrorData as McpError;

/// Unified error type for MCP tool invocations.
pub enum McpToolError {
    /// Invalid or missing parameters.
    InvalidParams(String),
    /// Wallet not found by alias or seed hash.
    WalletNotFound(String),
    /// SPV sync failed or timed out.
    SpvSyncFailed(String),
    /// Backend task returned an error.
    TaskFailed(TaskError),
    /// Internal/infrastructure error.
    Internal(String),
}

impl From<McpToolError> for McpError {
    fn from(e: McpToolError) -> Self {
        match e {
            McpToolError::InvalidParams(msg) | McpToolError::WalletNotFound(msg) => {
                McpError::invalid_params(msg, None)
            }
            McpToolError::SpvSyncFailed(msg) | McpToolError::Internal(msg) => {
                McpError::internal_error(msg, None)
            }
            McpToolError::TaskFailed(e) => McpError::internal_error(e.to_string(), None),
        }
    }
}
