//! Best-effort cleanup: return test wallet funds to the framework wallet.
//!
//! Called during setup to sweep orphaned wallets from previous runs
//! (e.g., a test panicked before cleanup). Wallets persist in the DB,
//! so AppContext loads them automatically on the next run.

use crate::framework::identity_helpers::get_receive_address;
use crate::framework::task_runner::run_task;
use crate::framework::wait;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use std::sync::Arc;
use std::time::Duration;

/// Send all funds from non-framework wallets back to the framework wallet.
///
/// Best-effort: logs errors but does not panic — funds may already be spent
/// or UTXOs may not yet be spendable.
pub async fn cleanup_test_wallets(
    app_context: &Arc<AppContext>,
    framework_wallet_hash: WalletSeedHash,
) {
    // Framework wallet receive address — extract Arc before awaiting to avoid
    // holding std::sync::RwLock across .await (deadlocks with SPV's tokio lock).
    let framework_wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };
    let framework_address = get_receive_address(app_context, &framework_wallet).await;

    // Collect non-framework wallet hashes
    let wallet_hashes: Vec<WalletSeedHash> = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .keys()
            .filter(|h| **h != framework_wallet_hash)
            .copied()
            .collect()
    };

    if wallet_hashes.is_empty() {
        return;
    }

    tracing::info!(
        "Sweeping {} orphaned test wallet(s) from previous run...",
        wallet_hashes.len()
    );

    for hash in wallet_hashes {
        let wallet_arc = {
            let wallets = app_context.wallets().read().expect("wallets lock");
            match wallets.get(&hash) {
                Some(w) => w.clone(),
                None => continue,
            }
        };

        // Wait briefly for SPV to sync this wallet's balance.
        let _ =
            wait::wait_for_spendable_balance(app_context, hash, 1, Duration::from_secs(10)).await;

        let balance = {
            let wallet = wallet_arc.read().expect("wallet lock");
            wallet.confirmed_balance_duffs()
        };

        if balance == 0 {
            continue;
        }

        // TODO(CMT-032): Also withdraw Platform credits from test identities back to
        // the framework wallet. Requires: enumerate identities owned by test wallets,
        // call IdentityTask::WithdrawFromIdentity for each, wait for Core balance.

        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: framework_address.clone(),
                amount_duffs: balance,
            }],
            subtract_fee_from_amount: true,
            memo: Some("E2E cleanup: sweep orphaned wallet".to_string()),
            override_fee: None,
        };

        let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: wallet_arc,
            request,
        });

        match run_task(app_context, task).await {
            Ok(_) => tracing::info!(
                "Cleanup: returned {} duffs from orphaned wallet {:?}",
                balance,
                &hash[..4]
            ),
            Err(e) => tracing::warn!("Cleanup: failed to sweep wallet {:?}: {}", &hash[..4], e),
        }
    }
}
