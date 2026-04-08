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
            // Trigger reconcile so DET wallet model reflects latest SPV state
            if let Err(e) = app_context.reconcile_spv_wallets().await {
                tracing::warn!("reconcile_spv_wallets failed: {e}");
            }

            let balance = {
                let wallets = app_context.wallets().read().expect("wallets lock");
                wallets.get(&wallet_hash).map(|wallet_arc| {
                    let wallet = wallet_arc.read().expect("wallet lock");
                    wallet.total_balance_duffs()
                })
            };
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
            // Trigger reconcile so DET wallet model reflects latest SPV state
            if let Err(e) = app_context.reconcile_spv_wallets().await {
                tracing::warn!("reconcile_spv_wallets failed: {e}");
            }

            let balance = {
                let wallets = app_context.wallets().read().expect("wallets lock");
                wallets.get(&wallet_hash).and_then(|wallet_arc| {
                    let wallet = wallet_arc.read().expect("wallet lock");
                    wallet.spv_confirmed_balance()
                })
            };
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
        let (confirmed, total) = {
            let wallets = app_context.wallets().read().expect("wallets lock");
            wallets
                .get(&wallet_hash)
                .map(|wallet_arc| {
                    let wallet = wallet_arc.read().expect("wallet lock");
                    (
                        wallet.spv_confirmed_balance().unwrap_or(0),
                        wallet.total_balance_duffs(),
                    )
                })
                .unwrap_or((0, 0))
        };
        format!(
            "Timed out waiting for spendable balance >= {} duffs \
             (confirmed: {}, total: {})",
            min_balance, confirmed, total
        )
    })
}

/// Wait until a wallet appears in the SPV subsystem (registered with
/// PlatformWalletManager, as indicated by the wallet-ID mapping).
pub async fn wait_for_wallet_in_spv(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    wait_timeout: Duration,
) -> Result<(), String> {
    timeout(wait_timeout, async {
        loop {
            let registered = app_context
                .wallet_id_mapping()
                .lock()
                .map(|m| m.wallet_id_for_seed(&wallet_hash).is_some())
                .unwrap_or(false);
            if registered {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| "Timed out waiting for wallet in SPV".to_string())
}

/// Wait for SPV to reach at least Syncing state (filters active).
///
/// Accepts both `Syncing` and `Running`. Does NOT require masternode
/// sync which can fail on testnet (QRInfo errors). The wallet is fully
/// functional for transactions once filters are synced.
pub async fn wait_for_spv_syncing_or_running(
    app_context: &Arc<AppContext>,
    wait_timeout: Duration,
) -> Result<(), String> {
    use dash_evo_tool::spv::SpvStatus;
    timeout(wait_timeout, async {
        loop {
            let status = app_context.connection_status().spv_status();
            if matches!(status, SpvStatus::Syncing | SpvStatus::Running) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "Timed out after {:?} waiting for SPV to reach Syncing/Running state",
            wait_timeout
        )
    })
}

/// Wait for SPV to connect to at least one peer.
pub async fn wait_for_spv_peers(
    app_context: &Arc<AppContext>,
    wait_timeout: Duration,
) -> Result<(), String> {
    let bridge = app_context.spv_event_bridge().clone();
    timeout(wait_timeout, async move {
        loop {
            let snapshot = bridge.status();
            if snapshot.connected_peers > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| format!("Timed out after {:?} waiting for SPV peers", wait_timeout))
}
