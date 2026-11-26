//! Devnet bootstrap. Testnet keeps the framework wallet's platform address
//! pre-funded; a fresh devnet needs this run once before the DPNS tests.
//!
//! It moves funds, so it does nothing unless `E2E_NETWORK=devnet` and
//! `E2E_DEVNET_BOOTSTRAP=1` are both set: a full `--ignored` run on testnet,
//! or a later devnet run, never repeats the transfer.

use crate::framework::funding::derive_platform_receive_address;
use crate::framework::harness::{ctx, e2e_network};
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::dashcore::Network;

/// Default amount moved to the platform address: 25 DASH.
const DEFAULT_PLATFORM_FUND_DUFFS: u64 = 2_500_000_000;

/// Fund the framework wallet's first platform address with
/// `E2E_PLATFORM_FUND_DUFFS` (default 25 DASH) from its Core UTXOs.
/// Opt-in only, see the module documentation.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn fund_framework_platform_address() {
    if e2e_network() != Network::Devnet
        || std::env::var("E2E_DEVNET_BOOTSTRAP").as_deref() != Ok("1")
    {
        eprintln!(
            "devnet_setup: skipped; set E2E_NETWORK=devnet and E2E_DEVNET_BOOTSTRAP=1 to fund the platform address"
        );
        return;
    }
    let amount = match std::env::var("E2E_PLATFORM_FUND_DUFFS") {
        Err(std::env::VarError::NotPresent) => DEFAULT_PLATFORM_FUND_DUFFS,
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("E2E_PLATFORM_FUND_DUFFS must be a whole number of duffs, got {value:?}")
        }),
        Err(error) => panic!("E2E_PLATFORM_FUND_DUFFS is not valid UTF-8: {error}"),
    };
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
