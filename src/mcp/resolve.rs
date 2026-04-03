//! Parameter resolution helpers for MCP tools.

use crate::context::AppContext;
use crate::context::connection_status::OverallConnectionState;
use crate::mcp::error::McpToolError;
use crate::mcp::server::network_display_name;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;
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
        let actual = network_display_name(app_context.network());
        if !expected.eq_ignore_ascii_case(actual) {
            return Err(McpToolError::NetworkMismatch {
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            });
        }
    }
    Ok(())
}

/// Verify network is provided and matches (mandatory for destructive ops).
pub(crate) fn require_network(
    app_context: &AppContext,
    network: Option<&str>,
) -> Result<(), McpToolError> {
    let Some(expected) = network else {
        return Err(McpToolError::InvalidParam {
            message: "The 'network' parameter is required for fund-sending operations to prevent accidental cross-network transfers.".to_owned(),
        });
    };
    let actual = network_display_name(app_context.network());
    if !expected.eq_ignore_ascii_case(actual) {
        return Err(McpToolError::NetworkMismatch {
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        });
    }
    Ok(())
}

/// Resolve a wallet identifier (alias or 64-char hex seed hash) to a `WalletSeedHash`.
pub(crate) fn wallet(ctx: &AppContext, wallet_id: &str) -> Result<WalletSeedHash, McpToolError> {
    let wallets = ctx.wallets.read().unwrap_or_else(|e| e.into_inner());

    // Try hex parse first — but only accept if the wallet is actually loaded.
    if wallet_id.len() == 64
        && let Ok(bytes) = hex::decode(wallet_id)
        && let Ok(hash) = <[u8; 32]>::try_from(bytes.as_slice())
        && wallets.contains_key(&hash)
    {
        return Ok(hash);
    }
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

    let id = if available.is_empty() {
        format!("\"{wallet_id}\" (no wallets loaded)")
    } else {
        format!("\"{wallet_id}\" — available: {}", available.join(", "))
    };

    Err(McpToolError::WalletNotFound { id })
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
        .ok_or_else(|| McpToolError::WalletNotFound {
            id: hex::encode(seed_hash),
        })
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
            return Err(McpToolError::SpvSyncFailed);
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                "SPV sync timed out after {} seconds (state: {state:?})",
                SPV_WAIT_TIMEOUT.as_secs()
            );
            return Err(McpToolError::SpvSyncFailed);
        }
        tokio::time::sleep(SPV_WAIT_POLL_INTERVAL).await;
    }
}

/// Validate amount for sending operations.
pub(crate) fn validate_amount(amount_duffs: u64) -> Result<(), McpToolError> {
    if amount_duffs == 0 {
        return Err(McpToolError::InvalidParam {
            message: "amount_duffs must be greater than zero".to_owned(),
        });
    }
    Ok(())
}

/// Basic format check for a Dash address.
///
/// Validates that the address is non-empty and starts with a character
/// typical for Dash addresses. This is a quick sanity check, not full
/// Base58Check validation (which happens at the backend layer).
pub(crate) fn validate_address(address: &str) -> Result<(), McpToolError> {
    if address.is_empty() {
        return Err(McpToolError::InvalidParam {
            message: "address must not be empty".to_owned(),
        });
    }
    // Dash mainnet: X (P2PKH), 7 (P2SH)
    // Dash testnet/devnet: y (P2PKH), 8 (P2SH), 9 (P2SH)
    let first = address.as_bytes()[0];
    if !matches!(first, b'X' | b'7' | b'y' | b'8' | b'9') {
        return Err(McpToolError::InvalidParam {
            message: format!(
                "address '{address}' does not look like a valid Dash address \
                 (expected to start with X, 7, y, 8, or 9)"
            ),
        });
    }
    Ok(())
}

/// Resolve an identity ID string to a QualifiedIdentity from the local database.
pub(crate) fn qualified_identity(
    ctx: &AppContext,
    identity_id_str: &str,
) -> Result<QualifiedIdentity, McpToolError> {
    let identifier = Identifier::from_string(identity_id_str, Encoding::Base58).map_err(|_| {
        McpToolError::InvalidParam {
            message: format!("Invalid identity ID: {identity_id_str}"),
        }
    })?;

    ctx.get_identity_by_id(&identifier)
        .map_err(|e| McpToolError::Internal(e.to_string()))?
        .ok_or_else(|| McpToolError::InvalidParam {
            message: format!(
                "Identity not found locally: {identity_id_str}. \
                 Load the identity first using the identity screen or CLI."
            ),
        })
}

/// Get the `PlatformWallet` for a given seed hash.
///
/// Returns `McpToolError::WalletNotFound` if no platform wallet is registered
/// for this seed hash (e.g. wallet not unlocked yet).
pub(crate) fn platform_wallet(
    ctx: &AppContext,
    seed_hash: WalletSeedHash,
) -> Result<crate::platform_wallet_bridge::PlatformWallet, McpToolError> {
    ctx.get_platform_wallet(&seed_hash)
        .ok_or_else(|| McpToolError::WalletNotFound {
            id: hex::encode(seed_hash),
        })
}

/// Validate amount in credits for sending operations.
pub(crate) fn validate_credits(amount_credits: u64) -> Result<(), McpToolError> {
    if amount_credits == 0 {
        return Err(McpToolError::InvalidParam {
            message: "amount_credits must be greater than zero".to_owned(),
        });
    }
    Ok(())
}
