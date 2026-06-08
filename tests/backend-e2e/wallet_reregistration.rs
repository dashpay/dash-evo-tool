//! PROJ-010 regression: wallets must re-register with the upstream SPV
//! backend so received funds are visible.
//!
//! Background: commit `e6c6c017` replaced the seed-based re-registration
//! loader with a read-only seedless loader, leaving NO code that ever
//! populated the upstream `platform-wallet.sqlite` persistor. An empty
//! persistor means an empty SPV watch set, so received Core funds stayed
//! invisible at 100% sync. The fix re-introduces the persistor write at the
//! create/import (W1) and cold-boot (W2) seed-bearing moments, with a
//! genesis birth-height floor for imported/recovered wallets so deposits made
//! before registration are still found.
//!
//! These tests run against a live Dash testnet via SPV and require a funded
//! framework wallet; they are `#[ignore]` like the rest of the backend-e2e
//! suite. See `tests/backend-e2e/README.md`.

use crate::framework::harness;
use crate::framework::wait;
use std::time::Duration;

/// W1 below-tip visibility: a freshly created+funded wallet (registered with
/// the genesis birth-height floor via the `Imported` origin the harness uses)
/// must surface its received balance.
///
/// This is the miniature of the real-world 1.0 DASH-at-block-1492173 repro:
/// the deposit lands and, because the wallet's addresses are actually watched
/// (persistor populated by W1), the balance becomes visible. Pre-fix the
/// persistor was never written, the watch set was empty, and this balance
/// never appeared — `create_funded_test_wallet` would time out waiting for it.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn funded_wallet_balance_is_visible_after_registration() {
    let ctx = harness::ctx().await;

    // 100k duffs is comfortably above dust and below the framework wallet's
    // balance; enough to prove the watch set sees a real deposit.
    let amount_duffs: u64 = 100_000;
    let (seed_hash, _wallet_arc) = ctx.create_funded_test_wallet(amount_duffs).await;

    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the funded wallet must be registered with the upstream SPV backend"
    );

    // The balance must become visible — the core PROJ-010 assertion. The
    // deposit is below the SPV tip by the time it is matched, so this only
    // passes when the wallet's addresses are actually watched.
    let balance = wait::wait_for_balance(
        &ctx.app_context,
        seed_hash,
        amount_duffs,
        Duration::from_secs(120),
    )
    .await
    .expect("received funds must be visible once the wallet is registered (PROJ-010)");

    assert!(
        balance >= amount_duffs,
        "visible balance {balance} must be at least the funded amount {amount_duffs}"
    );
}

/// W2 cold-boot reconciliation idempotency on the live backend: re-driving the
/// registered-wallet reconciliation (`ensure_wallets_registered`, the
/// seedless load pass) against an already-registered, funded wallet leaves it
/// watched and does not disturb its visible balance.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn re_registration_is_idempotent_for_funded_wallet() {
    let ctx = harness::ctx().await;
    let seed_hash = ctx.framework_wallet_hash;

    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");

    // Framework wallet is funded historically (below the current tip). It must
    // already be registered and its balance visible.
    wait::wait_for_wallet_in_spv(&ctx.app_context, seed_hash, Duration::from_secs(30))
        .await
        .expect("framework wallet must be registered with the backend");
    let before = ctx.app_context.snapshot_balance(&seed_hash).total;

    // Re-run the reconciliation pass; it must not double-register or change the
    // visible balance.
    backend
        .ensure_wallets_registered(&ctx.app_context)
        .await
        .expect("re-registration pass must not error");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "the wallet must stay registered after a reconciliation pass"
    );
    let after = ctx.app_context.snapshot_balance(&seed_hash).total;
    assert_eq!(
        before, after,
        "a reconciliation pass must not disturb the visible balance"
    );
}
