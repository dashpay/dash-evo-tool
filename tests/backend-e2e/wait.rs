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
    timeout(wait_timeout, async {
        loop {
            // Trigger reconcile so DET wallet model reflects latest SPV state
            if let Err(e) = app_context.reconcile_spv_wallets().await {
                eprintln!("  Warning: reconcile_spv_wallets failed: {e}");
            }

            let balance = {
                let wallets = app_context.wallets().read().expect("wallets lock");
                wallets.get(&wallet_hash).map(|wallet_arc| {
                    let wallet = wallet_arc.read().expect("wallet lock");
                    wallet.total_balance_duffs()
                })
            };
            if let Some(b) = balance
                && b >= min_balance
            {
                return b;
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
    timeout(wait_timeout, async {
        loop {
            // Trigger reconcile so DET wallet model reflects latest SPV state
            if let Err(e) = app_context.reconcile_spv_wallets().await {
                eprintln!("  Warning: reconcile_spv_wallets failed: {e}");
            }

            let balance = {
                let wallets = app_context.wallets().read().expect("wallets lock");
                wallets.get(&wallet_hash).map(|wallet_arc| {
                    let wallet = wallet_arc.read().expect("wallet lock");
                    wallet.confirmed_balance_duffs()
                })
            };
            if let Some(b) = balance
                && b >= min_balance
            {
                return b;
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
                        wallet.confirmed_balance_duffs(),
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

/// Wait until a wallet appears in the SPV subsystem.
pub async fn wait_for_wallet_in_spv(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    wait_timeout: Duration,
) -> Result<(), String> {
    timeout(wait_timeout, async {
        loop {
            let snapshot = app_context.spv_manager().det_wallets_snapshot();
            if snapshot.contains_key(&wallet_hash) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| "Timed out waiting for wallet in SPV".to_string())
}

/// Wait for SPV to connect to at least one peer.
pub async fn wait_for_spv_peers(
    app_context: &Arc<AppContext>,
    wait_timeout: Duration,
) -> Result<(), String> {
    let spv = app_context.spv_manager().clone();
    timeout(wait_timeout, async move {
        loop {
            let snapshot = spv.status_async().await;
            if snapshot.connected_peers > 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .map_err(|_| format!("Timed out after {:?} waiting for SPV peers", wait_timeout))
}
