//! ShieldedTask backend E2E tests (TC-074 to TC-083).
//!
//! All tests are guarded by `E2E_SKIP_SHIELDED` — set the env var to skip
//! these compute-intensive ZK tests, and are `#[ignore]` (network-dependent).
//!
//! Phase D retired DET's home-grown shielded subsystem: the `WarmUpProvingKey`,
//! `InitializeShieldedWallet`, `SyncNotes` and `CheckNullifiers` tasks are gone
//! (the upstream coordinator owns proving-key warm-up, key binding, note sync
//! and nullifier scanning). Only the five fund-moving ops remain, and shielded
//! keys are bound automatically when the wallet is loaded/unlocked, so these
//! tests just dispatch the ops and assert on the typed result.
//!
//! Scope note: the coordinator-store balance readers (`shielded_balance_*`,
//! `shielded_default_address`) are `pub(crate)`, so this external test crate
//! cannot read the post-sync balance or build a self-transfer recipient. The
//! balance/sync/transfer verification therefore lives in the Phase-G det-cli
//! self-test, which drives the public MCP read tools (`shielded_sync`,
//! `shielded_balance_get`, `shielded_address_get`). Here we verify each op
//! dispatches and confirms through the upstream coordinator.

use crate::framework::harness::ctx;
use crate::framework::shielded_helpers;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::shielded::ShieldedTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::dpp::dashcore::Network;

// ---------------------------------------------------------------------------
// Lifecycle test — shield → transfer → unshield → withdraw, with balance checks
// ---------------------------------------------------------------------------

/// Full shielded lifecycle (TC-074 → TC-078 → TC-080 → TC-081 → TC-082):
/// shield core DASH into the pool via asset lock, self-transfer within the
/// pool, unshield part to a platform address, then withdraw part to a Core
/// address. Binding is automatic on wallet load (no explicit init step).
///
/// Each fund-moving op confirms through the upstream coordinator and returns
/// its typed result; between the shield and the unshield we force a coordinator
/// sync ([`shielded_helpers::force_shielded_sync`]) and assert the push-snapshot
/// balance moves in the expected direction — exercising the Phase-E writer
/// end-to-end (op → `sync_now` → `on_shielded_sync_completed` →
/// `AppContext::shielded_balances`).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_074_shielded_lifecycle() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    if !shielded_helpers::is_shielded_available(app_context) {
        tracing::warn!(
            "tc_074: platform does not support shielded ops (FeatureGate check) — skipping"
        );
        return;
    }

    // Baseline shielded balance after an initial sync (the wallet may already
    // hold shielded notes from a prior run).
    let baseline = shielded_helpers::force_shielded_sync(app_context, seed_hash).await;
    tracing::info!("tc_074: baseline shielded balance = {baseline} credits");

    // Step 1 (TC-078): shield core DASH into the pool via asset lock. The
    // recipient (the wallet's own default Orchard address) is resolved
    // internally by `run_shielded_task`. SPV-gated — can take minutes.
    let amount_duffs = 500_000; // 0.005 DASH
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
        seed_hash,
        amount_duffs,
    });
    match run_task(app_context, task).await {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("tc_074: shield-from-core skipped — platform unsupported: {e}");
            return;
        }
        Err(e) => panic!("ShieldFromAssetLock failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert!(amount > 0, "shielded amount should be > 0, got {amount}");
            tracing::info!("tc_074: shielded {amount} credits from core wallet");
        }
        Ok(other) => panic!("Expected ShieldedFromAssetLock, got: {other:?}"),
    }

    // Sync and assert the shielded balance increased (Phase-E push writer).
    let after_shield = shielded_helpers::force_shielded_sync(app_context, seed_hash).await;
    assert!(
        after_shield > baseline,
        "shielding must increase the shielded balance: {after_shield} !> {baseline}"
    );
    tracing::info!("tc_074: post-shield shielded balance = {after_shield} credits");

    // Step 2 (TC-080): private self-transfer within the pool. The recipient is
    // this wallet's own default Orchard address (raw 43-byte form).
    let recipient_address_bytes = app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .shielded_default_address(&seed_hash, 0)
        .await
        .expect("default shielded address read")
        .expect("wallet must be shielded-bound after a shield op")
        .to_vec();
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldedTransfer {
        seed_hash,
        amount: 50_000,
        recipient_address_bytes,
    });
    match run_task(app_context, task).await {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("tc_074: transfer skipped — platform unsupported: {e}");
            return;
        }
        Err(e) => panic!("ShieldedTransfer failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedTransferComplete {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash);
            assert_eq!(amount, 50_000);
            tracing::info!("tc_074: transferred {amount} credits privately");
        }
        Ok(other) => panic!("Expected ShieldedTransferComplete, got: {other:?}"),
    }

    // Step 3 (TC-081): unshield part of the pool back to a platform address.
    let platform_addr = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet_arc = wallets
            .get(&seed_hash)
            .expect("framework wallet must exist");
        let wallet = wallet_arc.read().expect("wallet lock");
        let addrs = wallet.platform_addresses(Network::Testnet);
        assert!(
            !addrs.is_empty(),
            "wallet must have at least one platform address"
        );
        addrs[0].1
    };
    let task = BackendTask::ShieldedTask(ShieldedTask::UnshieldCredits {
        seed_hash,
        amount: 30_000,
        to_platform_address: platform_addr,
    });
    match run_task(app_context, task).await {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("tc_074: unshield skipped — platform unsupported: {e}");
            return;
        }
        Err(e) => panic!("UnshieldCredits failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedCreditsUnshielded {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash);
            assert_eq!(amount, 30_000);
            tracing::info!("tc_074: unshielded {amount} credits to platform address");
        }
        Ok(other) => panic!("Expected ShieldedCreditsUnshielded, got: {other:?}"),
    }

    // Sync and assert the shielded balance decreased after unshielding.
    let after_unshield = shielded_helpers::force_shielded_sync(app_context, seed_hash).await;
    assert!(
        after_unshield < after_shield,
        "unshielding must decrease the shielded balance: {after_unshield} !< {after_shield}"
    );
    tracing::info!("tc_074: post-unshield shielded balance = {after_unshield} credits");

    // Step 4 (TC-082): withdraw part of the pool to a Core L1 address.
    let core_address = app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .next_receive_address(&seed_hash)
        .await
        .expect("get receive address")
        .parse::<dash_sdk::dpp::dashcore::Address<_>>()
        .expect("watched address parses")
        .assume_checked();
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldedWithdrawal {
        seed_hash,
        amount: 20_000,
        to_core_address: core_address.clone(),
    });
    match run_task(app_context, task).await {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("tc_074: withdrawal skipped — platform unsupported: {e}");
        }
        Err(e) => panic!("ShieldedWithdrawal failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedWithdrawalComplete {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash);
            assert_eq!(amount, 20_000);
            tracing::info!("tc_074: withdrew {amount} credits to {core_address}");
        }
        Ok(other) => panic!("Expected ShieldedWithdrawalComplete, got: {other:?}"),
    }
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

    // Baseline shielded balance before the shield (after a coordinator sync).
    let baseline = shielded_helpers::force_shielded_sync(app_context, seed_hash).await;

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

            // Sync and assert the shielded balance increased (Phase-E writer).
            let after = shielded_helpers::force_shielded_sync(app_context, seed_hash).await;
            assert!(
                after > baseline,
                "shield-from-balance must increase the shielded balance: {after} !> {baseline}"
            );
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
