//! Best-effort cleanup: return test wallet funds to the framework wallet.

use crate::harness::ctx;
use crate::identity_helpers::get_receive_address;
use crate::task_runner::run_task;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::model::wallet::WalletSeedHash;

/// Send all funds from non-framework wallets back to the framework wallet.
///
/// Logs errors but does not panic -- funds may already be spent.
#[allow(dead_code)]
pub async fn cleanup_test_wallets(framework_wallet_hash: WalletSeedHash) {
    let app_context = &ctx().await.app_context;

    // Framework wallet receive address
    let framework_address = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let framework_wallet = wallets
            .get(&framework_wallet_hash)
            .expect("framework wallet must exist");
        get_receive_address(app_context, framework_wallet)
    };

    // Collect non-framework wallet hashes
    let wallet_hashes: Vec<WalletSeedHash> = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .keys()
            .filter(|h| **h != framework_wallet_hash)
            .copied()
            .collect()
    };

    for hash in wallet_hashes {
        let wallet_arc = {
            let wallets = app_context.wallets().read().expect("wallets lock");
            match wallets.get(&hash) {
                Some(w) => w.clone(),
                None => continue,
            }
        };

        let balance = {
            let wallet = wallet_arc.read().expect("wallet lock");
            wallet.total_balance_duffs()
        };

        if balance == 0 {
            continue;
        }

        let request = WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: framework_address.clone(),
                amount_duffs: balance,
            }],
            subtract_fee_from_amount: true,
            memo: Some("E2E cleanup".to_string()),
            override_fee: None,
        };

        let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: wallet_arc,
            request,
        });

        match run_task(app_context, task).await {
            Ok(_) => println!(
                "  Cleanup: returned {} duffs from wallet {:?}",
                balance,
                &hash[..4]
            ),
            Err(e) => eprintln!(
                "  Cleanup warning: failed to return funds from wallet {:?}: {}",
                &hash[..4],
                e
            ),
        }
    }
}
