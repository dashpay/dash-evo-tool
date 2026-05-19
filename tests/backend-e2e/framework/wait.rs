//! Polling helpers for waiting on async state changes.

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Wait until a wallet has at least `min_balance` total duffs (including unconfirmed),
/// polling every 2s. Triggers SPV reconciliation on each poll.
pub async fn wait_for_balance(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    min_balance: u64,
    wait_timeout: Duration,
) -> Result<u64, String> {
    let start = std::time::Instant::now();
    timeout(wait_timeout, async {
        let mut poll_count = 0u32;
        loop {
            let balance = Some(app_context.snapshot_balance(&wallet_hash).total);
            poll_count += 1;
            if let Some(b) = balance
                && b >= min_balance
            {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    balance = b,
                    "wait_for_balance: satisfied"
                );
                return b;
            }
            if poll_count.is_multiple_of(5) {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    current = ?balance,
                    target = min_balance,
                    "wait_for_balance: polling..."
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out waiting for total balance >= {} duffs",
            min_balance
        )
    })
}

/// Wait until a wallet has at least `min_balance` **spendable** (confirmed/IS-locked) duffs.
///
/// This is stricter than `wait_for_balance()` — it ensures the funds are actually
/// available for transaction building, not just visible as unconfirmed balance.
/// Triggers SPV reconciliation on each poll.
pub async fn wait_for_spendable_balance(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    min_balance: u64,
    wait_timeout: Duration,
) -> Result<u64, String> {
    let start = std::time::Instant::now();
    timeout(wait_timeout, async {
        let mut poll_count = 0u32;
        loop {
            let balance = Some(app_context.snapshot_balance(&wallet_hash).confirmed);
            poll_count += 1;
            if let Some(b) = balance
                && b >= min_balance
            {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    balance = b,
                    "wait_for_spendable_balance: satisfied"
                );
                return b;
            }
            if poll_count.is_multiple_of(5) {
                tracing::trace!(
                    elapsed_ms = start.elapsed().as_millis(),
                    polls = poll_count,
                    current = ?balance,
                    target = min_balance,
                    "wait_for_spendable_balance: polling..."
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .map_err(|_| {
        // Report both confirmed and total for diagnostics
        let snap = app_context.snapshot_balance(&wallet_hash);
        let (confirmed, total) = (snap.confirmed, snap.total);
        format!(
            "Timed out waiting for spendable balance >= {} duffs \
             (confirmed: {}, total: {})",
            min_balance, confirmed, total
        )
    })
}

/// Wait until a wallet appears in the SPV subsystem.
// TODO(P0.5): re-enable in P2 — chain sync is owned by upstream
// platform-wallet; wallet registration is observed via the EventBridge.
pub async fn wait_for_wallet_in_spv(
    _app_context: &Arc<AppContext>,
    _wallet_hash: WalletSeedHash,
    _wait_timeout: Duration,
) -> Result<(), String> {
    Err("wait_for_wallet_in_spv is not wired until P2".to_string())
}

/// Wait for SPV to complete initial sync (all managers including masternodes).
///
/// `SpvStatus::Running` is set after `SyncComplete` fires, which means
/// MempoolManager is activated and bloom filter is built.
pub async fn wait_for_spv_running(
    app_context: &Arc<AppContext>,
    wait_timeout: Duration,
) -> Result<(), String> {
    use dash_evo_tool::model::spv_status::SpvStatus;
    timeout(wait_timeout, async {
        loop {
            if app_context.connection_status().spv_status() == SpvStatus::Running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out after {:?} waiting for SPV to reach Running state",
            wait_timeout
        )
    })
}

/// Wait for SPV to connect to at least one peer.
// TODO(P0.5): re-enable in P2 — peer count comes from upstream
// platform-wallet sync status via the EventBridge.
pub async fn wait_for_spv_peers(
    _app_context: &Arc<AppContext>,
    _wait_timeout: Duration,
) -> Result<(), String> {
    Err("wait_for_spv_peers is not wired until P2".to_string())
}
