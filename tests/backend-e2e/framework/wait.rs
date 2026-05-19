//! Polling helpers for waiting on async state changes.

use dash_evo_tool::backend_task::error::TaskError;
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

/// Ensure a DET-registered wallet is registered with the upstream wallet
/// backend so its addresses are monitored by the `SpvRuntime`.
///
/// Chain sync is owned by upstream `platform-wallet`. A wallet becomes
/// "tracked" once `WalletBackend::ensure_wallets_registered` has run
/// `create_wallet_from_seed_bytes` for it (idempotent), at which point its
/// derived addresses are watched and balance events flow back through the
/// `EventBridge`. This drives that registration and confirms the upstream
/// backend has a snapshot slot for the wallet.
pub async fn wait_for_wallet_in_spv(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    wait_timeout: Duration,
) -> Result<(), String> {
    let backend = app_context
        .wallet_backend()
        .map_err(|e| format!("Wallet backend not wired: {e}"))?;

    timeout(wait_timeout, async {
        loop {
            match backend.ensure_wallets_registered(app_context).await {
                Ok(()) => {
                    if backend.is_wallet_registered(&wallet_hash) {
                        return Ok(());
                    }
                }
                // `TaskError::WalletBackend` is the upstream wallet runtime's
                // documented "retry in a moment" signal — transient under the
                // serial suite's burst of registrations. Retry within the
                // existing timeout budget. Any other typed error is terminal.
                Err(TaskError::WalletBackend { .. }) => {}
                Err(e) => return Err(format!("ensure_wallets_registered failed: {e}")),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out after {:?} waiting for wallet to register with the upstream backend",
            wait_timeout
        )
    })?
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

/// Wait until the upstream `SpvRuntime` has connected to peers and begun
/// syncing.
///
/// Chain sync is upstream-owned; the only signal DET observes is the
/// push-based `ConnectionStatus` fed by the wallet-backend `EventBridge`.
/// `on_progress` / `on_sync_event` move the status to `Syncing` (or
/// `Running`) only once a peer is connected and header sync has started —
/// upstream cannot make sync progress without a peer. The dedicated
/// `PeersUpdated` peer-count atomic is not reliably populated by this
/// upstream revision, so an active sync status (not the raw count) is the
/// authoritative "we have peers" signal.
pub async fn wait_for_spv_peers(
    app_context: &Arc<AppContext>,
    wait_timeout: Duration,
) -> Result<(), String> {
    use dash_evo_tool::model::spv_status::SpvStatus;
    let cs = app_context.connection_status();
    timeout(wait_timeout, async {
        loop {
            // A non-zero peer count is the strongest signal when present,
            // but `Syncing`/`Running` already implies a connected peer.
            if cs.spv_connected_peers() > 0
                || matches!(cs.spv_status(), SpvStatus::Syncing | SpvStatus::Running)
            {
                return Ok(());
            }
            if cs.spv_status() == SpvStatus::Error {
                return Err(format!(
                    "SPV entered Error state while waiting for peers: {}",
                    cs.spv_last_error().unwrap_or_default()
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out after {:?} waiting for SPV to connect to a peer (status: {})",
            wait_timeout,
            cs.spv_status()
        )
    })?
}
