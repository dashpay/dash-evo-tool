//! Devnet bootstrap. Testnet keeps the framework wallet's platform address
//! pre-funded; a fresh devnet needs this run once before the DPNS tests.

use crate::framework::funding::derive_platform_receive_address;
use crate::framework::harness::{ctx, e2e_network};
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};

/// Fund the framework wallet's first platform address with
/// `E2E_PLATFORM_FUND_DUFFS` (default 25 DASH) from its Core UTXOs.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn fund_framework_platform_address() {
    let amount = std::env::var("E2E_PLATFORM_FUND_DUFFS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_500_000_000);
    let ctx = ctx().await;
    let seed_hash = ctx.framework_wallet_hash;
    let destination =
        derive_platform_receive_address(&ctx.app_context, seed_hash, e2e_network(), false).await;
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
    .expect("FundPlatformAddressFromWalletUtxos should succeed");
    assert!(
        matches!(
            result,
            BackendTaskSuccessResult::PlatformAddressFunded { .. }
        ),
        "expected PlatformAddressFunded, got: {result:?}"
    );
}
