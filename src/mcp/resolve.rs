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

/// Wire the wallet backend and finish any pending legacy-wallet migration so
/// every persisted wallet is available in memory before returning.
///
/// Unlike [`ensure_spv_synced`], this does not start SPV or wait for chain sync.
pub(crate) async fn ensure_wallets_hydrated(ctx: &Arc<AppContext>) -> Result<(), McpToolError> {
    let (tx, _) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, egui::Context::default());
    ctx.ensure_wallet_backend(sender)
        .await
        .map_err(McpToolError::TaskFailed)?;
    ensure_legacy_storage_migrated(ctx).await
}

/// Poll until the cold-start storage update is fully complete.
///
/// On a fresh standalone process, the MCP readiness gates dispatch a
/// legacy-data migration before the backend is fully usable.
/// `AppContext::run_backend_task` short-circuits wallet-touching tasks while
/// `migration_status().state().is_in_progress()`, returning
/// [`TaskError::WalletStorageNotReady`]. Waiting here covers an active run that
/// started between dispatch and this check.
///
/// Fast exit: returns immediately if migration is already done (the common
/// case after the first gated tool has already waited).
///
/// A terminal [`MigrationState::Failed`] is never ready: it is converted back
/// into its typed task error even when a concurrent retry changed state while
/// another caller was waiting.
async fn ensure_storage_ready(ctx: &Arc<AppContext>) -> Result<(), McpToolError> {
    let migration = ctx.migration_status();

    let poll = async {
        loop {
            let state = migration.state();
            match state.as_ref() {
                crate::context::migration_status::MigrationState::Failed { error } => {
                    return Err(McpToolError::TaskFailed(
                        crate::backend_task::migration::migration_task_error(Arc::clone(error)),
                    ));
                }
                state if state.is_in_progress() => {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                _ => return Ok(()),
            }
        }
    };

    match tokio::time::timeout(STORAGE_MIGRATION_WAIT_TIMEOUT, poll).await {
        Ok(Ok(())) => {
            tracing::info!("Cold-start storage migration complete.");
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => {
            tracing::warn!(
                timeout_secs = STORAGE_MIGRATION_WAIT_TIMEOUT.as_secs(),
                "Timed out waiting for cold-start storage migration"
            );
            Err(McpToolError::StorageNotReady)
        }
    }
}

/// Dispatch or join the legacy-data migration before wallet reads.
///
/// Standalone/headless MCP has no GUI frame loop to dispatch `FinishUnwire`.
/// Embedded MCP may race the GUI dispatch, so the AppContext run gate safely
/// joins that run and the task's sentinel keeps repeated dispatch idempotent.
async fn ensure_legacy_storage_migrated(ctx: &Arc<AppContext>) -> Result<(), McpToolError> {
    let migration_state = ctx.migration_status().state();
    if matches!(
        migration_state.as_ref(),
        crate::context::migration_status::MigrationState::Idle
            | crate::context::migration_status::MigrationState::Failed { .. }
    ) {
        use crate::backend_task::migration::MigrationTask;
        if let Err(e) = ctx.run_migration_task(MigrationTask::FinishUnwire).await {
            tracing::warn!(error = ?e, "Standalone cold-start storage update failed");
            return Err(McpToolError::TaskFailed(e));
        }
    }

    ensure_storage_ready(ctx).await
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
/// Also dispatches or joins any pending cold-start storage migration and waits
/// for a wallet-safe terminal state before polling SPV — this prevents the
/// `WalletStorageNotReady` fast-fail that `run_backend_task` applies while
/// migration is mid-flight.
///
/// Once synced, an unpopulated Platform protocol cache is refreshed so
/// headless feature gates evaluate the connected network rather than boot state.
/// Refresh failures remain best-effort here so Core-only tools can proceed.
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
    ensure_spv_ready(ctx, ProtocolRefresh::BestEffortIfUnpopulated).await
}

/// Require synced SPV and fresh epoch metadata before a shielded fund movement.
pub(crate) async fn ensure_shielded_operations_ready(
    ctx: &Arc<AppContext>,
) -> Result<(), McpToolError> {
    ensure_spv_ready(ctx, ProtocolRefresh::Required).await
}

#[derive(Clone, Copy)]
enum ProtocolRefresh {
    BestEffortIfUnpopulated,
    Required,
}

async fn ensure_spv_ready(
    ctx: &Arc<AppContext>,
    protocol_refresh: ProtocolRefresh,
) -> Result<(), McpToolError> {
    // A throwaway `TaskResult` sender: MCP/CLI has no GUI event loop consuming
    // it, so the receiver is dropped. The `EventBridge` only does non-blocking
    // `try_send`, so a closed channel is harmless. Mirrors `dispatch::dispatch_task`.
    let (tx, _) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, egui::Context::default());
    if let Err(e) = ctx.ensure_wallet_backend_and_start_spv(sender).await {
        tracing::warn!(error = %e, "wallet backend wiring / SPV start failed before sync wait");
        return Err(McpToolError::TaskFailed(e));
    }

    ensure_legacy_storage_migrated(ctx).await?;

    wait_for_spv_and_refresh_platform_info(ctx, protocol_refresh).await
}

async fn wait_for_spv_and_refresh_platform_info(
    ctx: &Arc<AppContext>,
    protocol_refresh: ProtocolRefresh,
) -> Result<(), McpToolError> {
    // Subscribe before reading the current value so no transition is lost.
    // `borrow_and_update` keeps later iterations asleep until a real change.
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
        Ok(result) => result?,
        Err(_elapsed) => {
            tracing::warn!(
                "SPV sync timed out after {} seconds (status: {:?})",
                SPV_WAIT_TIMEOUT.as_secs(),
                ctx.connection_status().spv_status()
            );
            return Err(McpToolError::SpvSyncFailed);
        }
    }

    match protocol_refresh {
        ProtocolRefresh::BestEffortIfUnpopulated if ctx.platform_protocol_version() == 0 => {
            let _ = refresh_platform_protocol_version(ctx).await;
            Ok(())
        }
        ProtocolRefresh::BestEffortIfUnpopulated => Ok(()),
        ProtocolRefresh::Required => refresh_platform_protocol_version(ctx).await,
    }
}

async fn refresh_platform_protocol_version(ctx: &Arc<AppContext>) -> Result<(), McpToolError> {
    tracing::trace!("Refreshing Platform epoch information for headless feature gating");
    crate::mcp::dispatch::dispatch_task(
        ctx,
        crate::backend_task::BackendTask::PlatformInfo(
            crate::backend_task::platform_info::PlatformInfoTaskRequestType::CurrentEpochInfo,
        ),
    )
    .await
    .map_err(|error| {
        tracing::warn!(
            error = ?error,
            "Platform epoch information refresh failed for headless feature gating"
        );
        McpToolError::TaskFailed(error)
    })?;

    tracing::trace!(
        protocol_version = ctx.platform_protocol_version(),
        "Platform epoch information refreshed for headless feature gating"
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::Sdk;
    use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
    use dash_sdk::dpp::version::LATEST_VERSION;
    use dash_sdk::platform::DataContract;

    /// Mocks the proved DPNS-contract fetch that drives the protocol-version
    /// ratchet — the only network read the epoch task still performs while
    /// dashpay/platform#4231 is unresolved. The mock reports `LATEST_VERSION` in
    /// the response metadata, so a successful fetch ratchets the SDK exactly as a
    /// live network would; a failed one leaves the version unconfirmed.
    async fn mock_sdk_with_dpns_contract(ctx: &Arc<AppContext>) -> Sdk {
        let contract = DataContract::clone(&ctx.dpns_contract);
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch(contract.id(), Some(contract))
            .await
            .expect("register DPNS contract response");
        sdk
    }

    #[tokio::test]
    async fn headless_sync_populates_protocol_version_after_spv_is_running() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::mcp::tests::legacy_wallet_context(temp_dir.path());
        ctx.sdk
            .store(Arc::new(mock_sdk_with_dpns_contract(&ctx).await));
        ctx.connection_status().set_spv_status(SpvStatus::Running);

        assert_eq!(ctx.platform_protocol_version(), 0, "boot value");

        wait_for_spv_and_refresh_platform_info(&ctx, ProtocolRefresh::BestEffortIfUnpopulated)
            .await
            .expect("headless sync completion");

        assert_eq!(ctx.platform_protocol_version(), LATEST_VERSION);
        assert_eq!(
            ctx.fee_multiplier_permille(),
            crate::model::fee_estimation::PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE
        );
    }

    #[tokio::test]
    async fn protocol_refresh_observes_activation_after_a_pre_activation_epoch() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::mcp::tests::legacy_wallet_context(temp_dir.path());
        ctx.set_platform_protocol_version(LATEST_VERSION - 1);
        ctx.sdk
            .store(Arc::new(mock_sdk_with_dpns_contract(&ctx).await));

        refresh_platform_protocol_version(&ctx)
            .await
            .expect("refresh platform metadata");

        assert_eq!(ctx.platform_protocol_version(), LATEST_VERSION);
    }

    #[tokio::test]
    async fn best_effort_protocol_refresh_retries_without_blocking_spv_readiness() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::mcp::tests::legacy_wallet_context(temp_dir.path());
        ctx.sdk.store(Arc::new(Sdk::new_mock()));
        ctx.connection_status().set_spv_status(SpvStatus::Running);

        wait_for_spv_and_refresh_platform_info(&ctx, ProtocolRefresh::BestEffortIfUnpopulated)
            .await
            .expect("SPV readiness survives a Platform metadata failure");
        assert_eq!(ctx.platform_protocol_version(), 0);

        ctx.sdk
            .store(Arc::new(mock_sdk_with_dpns_contract(&ctx).await));
        wait_for_spv_and_refresh_platform_info(&ctx, ProtocolRefresh::BestEffortIfUnpopulated)
            .await
            .expect("SPV readiness retries Platform metadata");

        assert_eq!(ctx.platform_protocol_version(), LATEST_VERSION);
    }

    #[tokio::test]
    async fn storage_ready_rejects_terminal_migration_failure() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::mcp::tests::legacy_wallet_context(temp_dir.path());
        let source =
            Arc::new(crate::backend_task::migration::MigrationError::WalletBackendUnavailable);
        ctx.migration_status().set_state(
            crate::context::migration_status::MigrationState::Failed {
                error: Arc::clone(&source),
            },
        );

        let error = ensure_storage_ready(&ctx)
            .await
            .expect_err("a failed migration is not wallet-ready");

        match error {
            McpToolError::TaskFailed(crate::backend_task::error::TaskError::MigrationFailed {
                source: actual,
            }) => assert!(Arc::ptr_eq(&actual, &source)),
            other => panic!("expected the typed migration failure, got {other:?}"),
        }
    }
}
