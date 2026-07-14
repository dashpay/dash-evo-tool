//! Parameter resolution helpers for MCP tools.

use crate::context::AppContext;
use crate::mcp::error::McpToolError;
use crate::mcp::server::network_display_name;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::spv_status::SpvStatus;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;
use std::sync::{Arc, RwLock};

/// Initial SPV sync (headers, masternodes, filters, blocks) can take several minutes.
const SPV_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Maximum wait for a cold-start storage migration to complete.
///
/// Migration is fast (typically < 5 s) but bounded at 60 s to guard
/// against a hung migrator.  If this elapses, `ensure_storage_ready`
/// returns [`McpToolError::StorageNotReady`] so the caller gets an
/// actionable error rather than a mysterious `WalletStorageNotReady`
/// from deep inside `run_backend_task`.
const STORAGE_MIGRATION_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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

/// Verify network is provided (non-blank) and matches (mandatory for
/// destructive ops).
///
/// A missing or blank `network` is rejected as "required" rather than compared
/// against the active network — an empty string means "not provided".
pub(crate) fn require_network(
    app_context: &AppContext,
    network: Option<&str>,
) -> Result<(), McpToolError> {
    let expected = network
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| McpToolError::InvalidParam {
            message: "The 'network' parameter is required for fund-sending operations to prevent accidental cross-network transfers.".to_owned(),
        })?;
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
///
/// Delegates the lookup + poison recovery to [`AppContext::wallet_arc`] (the
/// single source of truth) and re-wraps its only failure —
/// [`TaskError::WalletNotFound`] — as the id-bearing [`McpToolError`] MCP
/// clients expect.
pub(crate) fn wallet_arc(
    ctx: &AppContext,
    seed_hash: WalletSeedHash,
) -> Result<Arc<RwLock<crate::model::wallet::Wallet>>, McpToolError> {
    ctx.wallet_arc(&seed_hash)
        .map_err(|_| McpToolError::WalletNotFound {
            id: hex::encode(seed_hash),
        })
}

/// Poll until the cold-start storage update is no longer executing.
///
/// On a fresh standalone process, `ensure_wallet_backend_and_start_spv`
/// kicks off a legacy-data migration before the backend is fully usable.
/// `AppContext::run_backend_task` short-circuits wallet-touching tasks while
/// `migration_status().state().is_executing()`, returning
/// [`TaskError::WalletStorageNotReady`]. Waiting here covers an active run that
/// started between dispatch and this check.
///
/// Fast exit: returns immediately if migration is already done (the common
/// case after the first gated tool has already waited).
///
/// The inline dispatch surfaces a terminal failure before this function is
/// reached. A standalone password-protected install therefore fails promptly
/// with the desktop-app instruction and never enters this polling loop.
async fn ensure_storage_ready(ctx: &Arc<AppContext>) -> Result<(), McpToolError> {
    let migration = ctx.migration_status();
    // Fast path — not running; nothing to wait for.
    if !migration.state().is_executing() {
        return Ok(());
    }

    tracing::info!("Waiting for cold-start storage migration to complete…");

    let poll = async {
        loop {
            if !migration.state().is_executing() {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    };

    match tokio::time::timeout(STORAGE_MIGRATION_WAIT_TIMEOUT, poll).await {
        Ok(result) => {
            tracing::info!("Cold-start storage migration complete.");
            result
        }
        Err(_elapsed) => {
            tracing::warn!(
                timeout_secs = STORAGE_MIGRATION_WAIT_TIMEOUT.as_secs(),
                "Timed out waiting for cold-start storage migration"
            );
            Err(McpToolError::StorageNotReady)
        }
    }
}

/// Wait for SPV to reach the `Running` state (chain headers + filters synced).
///
/// Required for **all wallet-facing tools** — both core-chain (UTXOs, sending
/// Dash) and platform queries (address balances, withdrawals).  Even DAPI-only
/// operations need SPV because the SDK verifies DAPI proofs against quorum and
/// masternode list data from the synced chain.  When a second client is running,
/// SPV falls back to a tempdir and must sync before any proof verification works.
///
/// Only tools that make no network calls (e.g. `core_wallets_list`,
/// `network_info`, `tool_describe`) skip this gate.
///
/// Wires the wallet backend and starts chain sync on first call before waiting —
/// neither standalone (stdio) boot, the HTTP context swap, nor the
/// post-network-switch path eagerly wires the backend the way the GUI does, so
/// this is the single chokepoint that makes SPV actually start for every gated
/// tool. Both steps are idempotent, so repeated tool calls are cheap.
///
/// Also waits for any in-progress cold-start storage migration to finish
/// (see [`ensure_storage_ready`]) before polling SPV state — this prevents
/// the `WalletStorageNotReady` fast-fail that `run_backend_task` applies
/// while migration is mid-flight.
///
/// ## Why `SpvStatus::Running`, not `OverallConnectionState::Synced`
///
/// `OverallConnectionState::Synced` requires both SPV running **and**
/// `dapi_available == true`. In headless / MCP mode the DAPI availability
/// counter is only refreshed by the frame-loop `trigger_refresh` call, which
/// does not run here. Waiting for `Synced` would therefore block indefinitely
/// in headless mode even after the chain is fully synced — the symptom that
/// motivated this fix. `SpvStatus::Running` is push-based and sufficient: all
/// proof-verifying SDK calls only require a synced chain, not a live DAPI
/// counter at the `ensure_spv_synced` callsite.
pub(crate) async fn ensure_spv_synced(ctx: &Arc<AppContext>) -> Result<(), McpToolError> {
    // A throwaway `TaskResult` sender: MCP/CLI has no GUI event loop consuming
    // it, so the receiver is dropped. The `EventBridge` only does non-blocking
    // `try_send`, so a closed channel is harmless. Mirrors `dispatch::dispatch_task`.
    let (tx, _) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, egui::Context::default());
    if let Err(e) = ctx.ensure_wallet_backend_and_start_spv(sender).await {
        tracing::warn!(error = %e, "wallet backend wiring / SPV start failed before sync wait");
        return Err(McpToolError::TaskFailed(e));
    }

    // S7: In standalone/headless MCP mode the GUI frame-loop never runs, so
    // `MigrationTask::FinishUnwire` is never dispatched from `AppState`.
    // Dispatch it here. The AppContext-level run gate joins an existing GUI
    // dispatch when this is a shared context, so only one password waiter can
    // exist. In a standalone context the explicit prompt capability makes a
    // protected install fail immediately.
    let migration_state = ctx.migration_status().state();
    if migration_state.is_executing() {
        ensure_storage_ready(ctx).await?;
        if let crate::context::migration_status::MigrationState::Failed { error } =
            ctx.migration_status().state().as_ref()
        {
            return Err(McpToolError::TaskFailed(
                crate::backend_task::migration::migration_task_error(Arc::clone(error)),
            ));
        }
    } else if migration_state.is_awaiting_user_input() {
        tracing::debug!("The desktop storage update is awaiting a wallet password");
    } else if matches!(
        migration_state.as_ref(),
        crate::context::migration_status::MigrationState::Idle
            | crate::context::migration_status::MigrationState::Failed { .. }
    ) {
        use crate::backend_task::migration::MigrationTask;
        if let Err(e) = ctx.run_migration_task(MigrationTask::FinishUnwire).await {
            tracing::warn!(
                error = ?e,
                "Standalone cold-start storage update failed"
            );
            return Err(McpToolError::TaskFailed(e));
        }
    }

    // Wait for cold-start storage migration before polling SPV state.
    // `run_backend_task` rejects wallet-touching tasks while migration is
    // running; ensuring it finishes here makes cold-start tool calls
    // wait transparently rather than bouncing with WalletStorageNotReady.
    ensure_storage_ready(ctx).await?;

    // Subscribe BEFORE reading the current value so no transition is lost
    // between the `ensure_wallet_backend_and_start_spv` call above and the
    // first `borrow_and_update` below. borrow_and_update marks the current
    // value "seen", so the loop never spins — each iteration always sleeps on
    // a real change.
    let mut rx = ctx.connection_status().subscribe_spv_status();

    let wait = async {
        loop {
            let status = *rx.borrow_and_update();
            match status {
                SpvStatus::Running => return Ok(()),
                SpvStatus::Error => return Err(McpToolError::SpvSyncFailed),
                // Idle / Starting / Syncing / Stopping / Stopped — keep waiting.
                _ => {}
            }
            // changed() returns Err only if the sender is dropped, which is
            // app-lifetime, so this is effectively unreachable in practice.
            if rx.changed().await.is_err() {
                return Err(McpToolError::SpvSyncFailed);
            }
        }
    };

    match tokio::time::timeout(SPV_WAIT_TIMEOUT, wait).await {
        Ok(result) => result,
        Err(_elapsed) => {
            tracing::warn!(
                "SPV sync timed out after {} seconds (status: {:?})",
                SPV_WAIT_TIMEOUT.as_secs(),
                ctx.connection_status().spv_status()
            );
            Err(McpToolError::SpvSyncFailed)
        }
    }
}

/// Reject a zero send amount. `unit_label` names the JSON parameter's unit
/// (e.g. `"duffs"` or `"credits"`) so the message points at the right field.
pub(crate) fn validate_positive_amount(amount: u64, unit_label: &str) -> Result<(), McpToolError> {
    if amount == 0 {
        return Err(McpToolError::InvalidParam {
            message: format!("amount_{unit_label} must be greater than zero"),
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
