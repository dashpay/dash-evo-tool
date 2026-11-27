//! Devnet bootstrap. Testnet keeps the framework wallet's platform address
//! pre-funded; a fresh devnet needs this run once before the DPNS tests.
//!
//! To move funds, opt in with `E2E_NETWORK=devnet` and
//! `E2E_DEVNET_BOOTSTRAP=1`. Unset the bootstrap flag after funding.

use crate::framework::funding::derive_platform_receive_address;
use crate::framework::harness::{ctx, devnet_bootstrap_amount};
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Network;

/// Fund the framework wallet's first platform address with
/// `E2E_PLATFORM_FUND_DUFFS` (default 25 DASH) from its Core UTXOs.
/// Opt-in only, see the module documentation.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn fund_framework_platform_address() {
    let Some(amount) = devnet_bootstrap_amount() else {
        eprintln!(
            "devnet_setup: skipped; set E2E_NETWORK=devnet and E2E_DEVNET_BOOTSTRAP=1 to fund the platform address"
        );
        return;
    };
    let ctx = ctx().await;
    let network = ctx.app_context.network();
    assert_eq!(
        network,
        Network::Devnet,
        "The active context is on a different network. Restart the tests with E2E_NETWORK=devnet."
    );
    let seed_hash = ctx.framework_wallet_hash;
    let destination =
        derive_platform_receive_address(&ctx.app_context, seed_hash, network, false).await;
    let result = run_task(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
            seed_hash,
            amount,
            destination,
            fee_deduct_from_output: true,
        }),
    )
    .await
    .inspect_err(|_| {
        tracing::warn!(
            "Devnet bootstrap reported an error. The transfer may have succeeded. Check the framework wallet's Platform address balance before rerunning bootstrap."
        );
    })
    .expect("FundPlatformAddressFromWalletUtxos should succeed");
    assert!(
        matches!(
            result,
            BackendTaskSuccessResult::PlatformAddressFunded { .. }
        ),
        "expected PlatformAddressFunded, got: {result:?}"
    );
}
