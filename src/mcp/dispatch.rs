//! Dispatch backend tasks from MCP tool handlers.

use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use rmcp::model::ErrorData;
use std::sync::Arc;

/// Run a single backend task and return its result.
///
/// Creates a throwaway channel (the receiver is never read).
/// Same pattern as `tests/backend-e2e/framework/task_runner.rs`.
pub(crate) async fn dispatch_task(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, egui::Context::default());
    app_context.run_backend_task(task, sender).await
}

/// Convert a `TaskError` into an MCP error response.
pub(crate) fn task_error_to_mcp(e: TaskError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Resolve a wallet identifier (alias or hex seed hash) to a `WalletSeedHash`.
///
/// Accepts either:
/// - A 64-char hex string parsed as `WalletSeedHash`
/// - Any other string matched against wallet aliases
pub(crate) fn resolve_wallet(
    app_context: &Arc<AppContext>,
    wallet_id: &str,
) -> Result<WalletSeedHash, ErrorData> {
    // Try hex parse first
    if wallet_id.len() == 64
        && let Ok(bytes) = hex::decode(wallet_id)
        && let Ok(hash) = <[u8; 32]>::try_from(bytes.as_slice())
    {
        return Ok(hash);
    }

    // Try alias match (wallets is RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>>)
    let wallets = app_context
        .wallets
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut available: Vec<String> = Vec::new();

    for (seed_hash, wallet_arc) in wallets.iter() {
        let wallet = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
        let hex_prefix = hex::encode(&seed_hash[..4]);
        if let Some(alias) = &wallet.alias {
            if alias == wallet_id {
                return Ok(*seed_hash);
            }
            available.push(format!("  - \"{alias}\" ({hex_prefix}...)"));
        } else {
            available.push(format!("  - ({hex_prefix}...)"));
        }
    }

    let msg = if available.is_empty() {
        "No wallets loaded".to_string()
    } else {
        format!(
            "Wallet \"{wallet_id}\" not found. Available wallets:\n{}",
            available.join("\n")
        )
    };

    Err(ErrorData::invalid_params(msg, None))
}
