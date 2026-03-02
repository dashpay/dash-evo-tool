//! Polling helpers for waiting on async state changes.

use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Wait until a wallet has at least `min_balance` duffs, polling every 2s.
pub async fn wait_for_balance(
    app_context: &Arc<AppContext>,
    wallet_hash: WalletSeedHash,
    min_balance: u64,
    wait_timeout: Duration,
) -> Result<u64, String> {
    timeout(wait_timeout, async {
        loop {
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
    .map_err(|_| format!("Timed out waiting for balance >= {} duffs", min_balance))
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
