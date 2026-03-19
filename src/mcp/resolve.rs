//! Parameter resolution helpers for MCP tools.

use crate::context::AppContext;
use crate::context::connection_status::OverallConnectionState;
use crate::mcp::error::McpToolError;
use crate::mcp::server::network_display_name;
use crate::model::wallet::WalletSeedHash;
use std::sync::{Arc, RwLock};

/// Poll interval for waiting on SPV connection -- matches ConnectionStatus throttle.
const SPV_WAIT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
/// Initial SPV sync (headers, masternodes, filters, blocks) can take several minutes.
const SPV_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Verify that the expected network matches the server's active network.
///
/// If `expected` is `None`, validation is skipped (backwards compatible).
pub(crate) fn verify_network(
    app_context: &AppContext,
    expected: Option<&str>,
) -> Result<(), McpToolError> {
    if let Some(expected) = expected {
        let actual = network_display_name(app_context.network);
        if !expected.eq_ignore_ascii_case(actual) {
            return Err(McpToolError::InvalidParams(format!(
                "Network mismatch: expected '{expected}' but server is on '{actual}'"
            )));
        }
    }
    Ok(())
}

/// Resolve a wallet identifier (alias or 64-char hex seed hash) to a `WalletSeedHash`.
pub(crate) fn wallet(ctx: &AppContext, wallet_id: &str) -> Result<WalletSeedHash, McpToolError> {
    // Try hex parse first
    if wallet_id.len() == 64
        && let Ok(bytes) = hex::decode(wallet_id)
        && let Ok(hash) = <[u8; 32]>::try_from(bytes.as_slice())
    {
        return Ok(hash);
    }

    let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
    let mut available: Vec<String> = Vec::new();

    for (seed_hash, wallet_arc) in wallets.iter() {
        let w = wallet_arc.read().unwrap_or_else(|e| e.into_inner());
        let hex_prefix = hex::encode(&seed_hash[..4]);
        if let Some(alias) = &w.alias {
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

    Err(McpToolError::WalletNotFound(msg))
}

/// Get the `Arc<RwLock<Wallet>>` for a given seed hash.
pub(crate) fn wallet_arc(
    ctx: &AppContext,
    seed_hash: WalletSeedHash,
) -> Result<Arc<RwLock<crate::model::wallet::Wallet>>, McpToolError> {
    let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());
    wallets
        .get(&seed_hash)
        .cloned()
        .ok_or_else(|| McpToolError::WalletNotFound("Wallet not found".to_string()))
}

/// Wait for SPV to reach fully-synced (green) state.
pub(crate) async fn ensure_spv_synced(ctx: &AppContext) -> Result<(), McpToolError> {
    let deadline = tokio::time::Instant::now() + SPV_WAIT_TIMEOUT;
    loop {
        let _ = ctx.connection_status.trigger_refresh(ctx);
        let state = ctx.connection_status.overall_state();
        if state == OverallConnectionState::Synced {
            return Ok(());
        }
        if state == OverallConnectionState::Error {
            return Err(McpToolError::SpvSyncFailed(
                "SPV connection failed. Check your network configuration.".to_string(),
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(McpToolError::SpvSyncFailed(format!(
                "SPV sync timed out after {} seconds (state: {state:?}). Check your network.",
                SPV_WAIT_TIMEOUT.as_secs()
            )));
        }
        tokio::time::sleep(SPV_WAIT_POLL_INTERVAL).await;
    }
}
