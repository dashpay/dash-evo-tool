//! ShieldedTask backend E2E tests (TC-074 to TC-083).
//!
//! All tests are guarded by `E2E_SKIP_SHIELDED` — set the env var to skip
//! these compute-intensive ZK tests, and are `#[ignore]` (network-dependent).
//!
//! Phase D retired DET's home-grown shielded subsystem: the `WarmUpProvingKey`,
//! `InitializeShieldedWallet`, `SyncNotes` and `CheckNullifiers` tasks are gone
//! (the upstream coordinator owns proving-key warm-up, key binding, note sync
//! and nullifier scanning). Only the five fund-moving ops remain. The full
//! lifecycle chain is stubbed pending a Phase-F rewrite against the coordinator.

use crate::framework::harness::ctx;
use crate::framework::shielded_helpers;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::shielded::ShieldedTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::dpp::dashcore::Network;

// ---------------------------------------------------------------------------
// Lifecycle test — pending Phase-F rewrite
// ---------------------------------------------------------------------------

/// Shielded lifecycle E2E test (TC-074 … TC-082).
///
/// TODO(Phase F): rewrite for the upstream coordinator. The DET-owned
/// `WarmUpProvingKey` / `InitializeShieldedWallet` / `SyncNotes` /
/// `CheckNullifiers` tasks were removed in Phase D; the lifecycle now runs
/// through `ensure_shielded_bound` + the coordinator's `sync_now`, asserting
/// against the coordinator store rather than DET's deleted sidecar. Kept
/// `#[ignore]` and stubbed until ported.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_074_shielded_lifecycle() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }
    tracing::warn!(
        "tc_074_shielded_lifecycle: pending Phase-F rewrite for the upstream shielded coordinator"
    );
}

// ---------------------------------------------------------------------------
// Independent tests
// ---------------------------------------------------------------------------

/// TC-079: ShieldFromBalance
///
/// Shields credits from the wallet's platform balance into the shielded pool
/// (Type 15). Self-funds a platform address via
/// `FundPlatformAddressFromWalletUtxos` first; the upstream coordinator selects
/// the input addresses for the shield.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_079_shield_from_balance() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    if !shielded_helpers::is_shielded_available(app_context) {
        tracing::warn!(
            "tc_079: platform does not support shielded ops (FeatureGate check) — skipping"
        );
        return;
    }

    // Get a platform address from the wallet
    let platform_addr = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet_arc = wallets
            .get(&seed_hash)
            .expect("framework wallet must exist");
        let wallet = wallet_arc.read().expect("wallet lock");
        let addrs = wallet.platform_addresses(Network::Testnet);
        assert!(
            !addrs.is_empty(),
            "Wallet must have at least one platform address"
        );
        addrs[0].1
    };

    // Fund the platform address
    let fund_amount = 1_000_000; // 1M duffs = 0.01 DASH
    let fund_task = BackendTask::WalletTask(WalletTask::FundPlatformAddressFromWalletUtxos {
        seed_hash,
        amount: fund_amount,
        destination: platform_addr,
        fee_deduct_from_output: true,
    });
    run_task(app_context, fund_task)
        .await
        .expect("FundPlatformAddressFromWalletUtxos should succeed");

    tracing::info!("Platform address funded with {} duffs", fund_amount);

    // Fetch balances to confirm funding
    let balance_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let balance_result = run_task(app_context, balance_task)
        .await
        .expect("FetchPlatformAddressBalances should succeed");

    let available_credits = match &balance_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
            let (credits, _nonce) = balances
                .get(&platform_addr)
                .expect("funded address should appear in balances");
            assert!(*credits > 0, "Platform address balance should be > 0");
            tracing::info!("Platform address has {} credits", credits);
            *credits
        }
        other => panic!("Expected PlatformAddressBalances, got: {:?}", other),
    };

    // Shield a portion of the credits. The upstream coordinator selects the
    // input addresses — DET no longer supplies a `from_address`.
    let shield_amount = available_credits / 2;
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromBalance {
        seed_hash,
        amount: shield_amount,
    });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("TC-079: skipped — platform does not support shielded ops: {e}");
        }
        Err(e) => panic!("ShieldFromBalance failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedCreditsShielded {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, shield_amount, "shielded amount should match");
            tracing::info!("ShieldFromBalance: shielded {} credits", amount);
        }
        Ok(other) => panic!("Expected ShieldedCreditsShielded, got: {:?}", other),
    }
}

/// TC-083: ShieldedTask error — unknown wallet.
///
/// Dispatching a shielded op for a seed hash that has no loaded wallet must
/// return a typed error, not panic.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_083_error_unknown_wallet() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;

    // Use a fake seed hash that has no loaded wallet.
    let fake_seed_hash: WalletSeedHash = [0xDE; 32];

    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromBalance {
        seed_hash: fake_seed_hash,
        amount: 1,
    });
    let result = run_task(app_context, task).await;

    let err = result.expect_err("a shielded op on an unknown wallet should fail");

    // The wallet with this seed hash doesn't exist, so we expect WalletNotFound.
    // If shielded is unsupported on this platform, that's also acceptable.
    assert!(
        matches!(
            err,
            dash_evo_tool::backend_task::error::TaskError::WalletNotFound
        ) || shielded_helpers::is_platform_shielded_unsupported(&err),
        "TC-083: expected WalletNotFound or shielded-unsupported error, got: {:?}",
        err
    );

    tracing::info!(
        "Unknown wallet error (expected): {} (debug: {:?})",
        err,
        err
    );
}
