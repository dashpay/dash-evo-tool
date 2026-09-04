//! Live-network verification for cross-wallet identity top-ups (fixed in #954,
//! `resolve_top_up_route` in `src/backend_task/identity/top_up_identity.rs`).
//!
//! Offline tests (`src/context/wallet_lifecycle/tests.rs`) cover routing,
//! provisioning, and the corruption regression at the unit level. This module
//! covers what they cannot: the real foreign-identity top-up path — a fresh
//! asset lock, broadcast, IS proof, `TopUpIdentity` acceptance on Platform,
//! and the paying wallet's persister state afterward — against a live network.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::{run_task, run_task_with_nonce_retry};
use dash_evo_tool::backend_task::identity::{
    IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod,
};
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use platform_wallet::AssetLockFundingType;
use platform_wallet::wallet::asset_lock::tracked::AssetLockStatus;

/// Count `identity_keys` rows with no matching `identities` row for the same
/// `wallet_id` — the exact corruption shape a cross-wallet top-up must never
/// produce. Mirrors `orphaned_identity_key_rows` in
/// `src/context/wallet_lifecycle/tests.rs`, but points at the real wallet
/// persister this live e2e run actually wrote (`<workdir>/det-testnet.sqlite`),
/// not an offline fixture.
fn orphaned_identity_key_rows(data_dir: &std::path::Path) -> i64 {
    let path = data_dir.join("det-testnet.sqlite");
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open the live wallet persister read-only");
    connection
        .query_row(
            "SELECT COUNT(*) FROM identity_keys k \
             LEFT JOIN identities i \
               ON i.identity_id = k.identity_id AND i.wallet_id = k.wallet_id \
             WHERE i.identity_id IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("count orphaned identity_keys rows")
}

/// End-to-end cross-wallet top-up: an identity registered on `owner_wallet`
/// is topped up from `payer_wallet`'s own balance. Covers:
///
/// 1. The foreign top-up actually lands on Platform (balance increases,
///    confirmed independently via `RefreshIdentity`).
/// 2. The paying wallet's own identity/persister state is untouched — no
///    `identity_keys` rows orphaned under the payer's `wallet_id`, and both
///    wallets survive a real reload (`ensure_wallets_registered`), which is
///    exactly the failure mode ("Saved wallet data appears damaged and
///    cannot be loaded") the fix in #954 prevents.
/// 3. Re-submitting the SAME (now-spent) asset lock through the
///    `UseAssetLock` resume variant must never succeed a second time — the
///    only way today to reach a tracked `IdentityTopUpNotBound` lock is
///    through a `FundWithWallet` top-up that already consumed it, since
///    upstream keeps `consume_asset_lock` `pub(crate)`, so the lock lingers
///    in the resumable list with a non-`Consumed` status. This asserts that
///    lingering status does not translate into a double-credit: Platform
///    must still refuse the reuse.
/// 4. A same-wallet (owned-identity) top-up on `owner_wallet` still works —
///    a regression check that the new `resolve_top_up_route` branch left the
///    existing, common-case path alone.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn cross_wallet_topup_e2e() {
    let ctx = ctx().await;

    // --- Setup: two independent wallets, an identity owned by one of them ---
    tracing::info!("=== Setup: funded owner + payer wallets ===");
    let (owner_hash, owner_wallet) = ctx.create_funded_test_wallet(32_000_000).await;
    let (payer_hash, payer_wallet) = ctx.create_funded_test_wallet(3_000_000).await;

    let reg_info = build_identity_registration(&ctx.app_context, &owner_wallet, owner_hash).await;
    let reg_result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info)),
    )
    .await
    .expect("owner identity registration should succeed");

    let mut qualified_identity = match reg_result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, fee) => {
            tracing::info!(
                "registered owner identity {:?} (fee: {:?})",
                qi.identity.id(),
                fee
            );
            qi
        }
        other => panic!("expected RegisteredIdentity, got: {other:?}"),
    };
    let identity_id = qualified_identity.identity.id();
    let balance_after_registration = qualified_identity.identity.balance();

    assert_eq!(
        orphaned_identity_key_rows(&ctx._workdir),
        0,
        "baseline must be clean before any cross-wallet activity"
    );

    // --- Point 1 + 2: cross-wallet top-up, funded from a wallet that does ---
    // --- not own the identity ---
    tracing::info!("=== Step 1: cross-wallet top-up (payer funds owner's identity) ===");
    let top_up_amount = 500_000u64;
    let top_up_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentity(IdentityTopUpInfo {
            qualified_identity: qualified_identity.clone(),
            wallet: payer_wallet.clone(),
            identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                top_up_amount,
                0,
                0,
            ),
        })),
    )
    .await
    .expect("cross-wallet TopUpIdentity should succeed");

    let balance_after_cross_wallet_topup = match top_up_result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            assert_eq!(qi.identity.id(), identity_id, "wrong identity returned");
            tracing::info!(
                "cross-wallet top-up complete: balance {} -> {}, fee={:?}",
                balance_after_registration,
                qi.identity.balance(),
                fee_result
            );
            qualified_identity = qi;
            qualified_identity.identity.balance()
        }
        other => panic!("expected ToppedUpIdentity, got: {other:?}"),
    };
    assert!(
        balance_after_cross_wallet_topup > balance_after_registration,
        "identity balance must increase after a cross-wallet top-up (before={balance_after_registration}, after={balance_after_cross_wallet_topup})"
    );

    // Independent confirmation straight from Platform, not just the task's
    // own return value.
    let refreshed = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RefreshIdentity(qualified_identity.clone())),
    )
    .await
    .expect("RefreshIdentity should succeed");
    match refreshed {
        BackendTaskSuccessResult::RefreshedIdentity(qi) => {
            assert_eq!(qi.identity.id(), identity_id);
            assert!(
                qi.identity.balance() >= balance_after_cross_wallet_topup,
                "Platform-fetched balance ({}) should be >= the task-reported balance ({})",
                qi.identity.balance(),
                balance_after_cross_wallet_topup
            );
            tracing::info!(
                "RefreshIdentity confirms balance {} on Platform",
                qi.identity.balance()
            );
        }
        other => panic!("expected RefreshedIdentity, got: {other:?}"),
    }

    // The payer's own identity/persister state must be untouched.
    assert_eq!(
        orphaned_identity_key_rows(&ctx._workdir),
        0,
        "a cross-wallet top-up must not leave identity_keys rows the payer's \
         next wallet load cannot resolve"
    );

    // A reload must succeed for BOTH wallets — the exact failure mode
    // ("Saved wallet data appears damaged and cannot be loaded") the fix
    // prevents when the payer's identity_keys are corrupted.
    tracing::info!("=== Step 2: reload both wallets, confirm no corruption ===");
    ctx.app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .ensure_wallets_registered(&ctx.app_context)
        .await
        .expect("reload after a cross-wallet top-up must succeed for both wallets");

    // --- Point 3: the UseAssetLock resume variant must never double-credit ---
    tracing::info!("=== Step 3: attempt to resume the now-spent unbound lock ===");
    let tracked_locks = match run_task(
        &ctx.app_context,
        BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks {
            seed_hash: payer_hash,
        }),
    )
    .await
    .expect("ListTrackedAssetLocks should succeed")
    {
        BackendTaskSuccessResult::TrackedAssetLocks { locks, .. } => locks,
        other => panic!("expected TrackedAssetLocks, got: {other:?}"),
    };
    let unbound_lock = tracked_locks
        .iter()
        .find(|l| l.funding_type == AssetLockFundingType::IdentityTopUpNotBound)
        .expect(
            "the payer wallet must still track the unbound top-up lock it just spent \
             (upstream cannot mark it Consumed while consume_asset_lock stays pub(crate))",
        );
    tracing::info!(
        "found unbound top-up lock out_point={:?} status={:?}",
        unbound_lock.out_point,
        unbound_lock.status
    );
    // Documents the disclosed gap: the lock the fix cannot mark Consumed
    // still shows a non-terminal status locally.
    if unbound_lock.status == AssetLockStatus::Consumed {
        tracing::warn!(
            "unbound lock unexpectedly shows Consumed — either upstream now exposes \
             consume_asset_lock, or an earlier run already spent this exact lock"
        );
    }

    let resume_result = run_task(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentity(IdentityTopUpInfo {
            qualified_identity: qualified_identity.clone(),
            wallet: payer_wallet.clone(),
            identity_funding_method: TopUpIdentityFundingMethod::UseAssetLock {
                out_point: unbound_lock.out_point,
                identity_index: 0,
                top_up_index: 1,
            },
        })),
    )
    .await;

    match resume_result {
        Err(e) => {
            tracing::info!("resuming the spent lock was correctly refused: {:?}", e);
        }
        Ok(BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result)) => {
            panic!(
                "SAFETY BUG: resuming an already-spent unbound top-up lock succeeded a \
                 second time (balance now {}), fee={:?} — this is a double-credit / \
                 double-spend, not a UX gap. Do not attempt to fix this locally; report it.",
                qi.identity.balance(),
                fee_result
            );
        }
        Ok(other) => panic!("expected an error or ToppedUpIdentity, got: {other:?}"),
    }

    // Whatever the outcome, the payer's state must still be clean and both
    // wallets must still reload.
    assert_eq!(
        orphaned_identity_key_rows(&ctx._workdir),
        0,
        "attempting to resume a spent lock must not corrupt the payer's identity state"
    );
    ctx.app_context
        .wallet_backend()
        .expect("wallet backend wired")
        .ensure_wallets_registered(&ctx.app_context)
        .await
        .expect("reload after the resume attempt must still succeed for both wallets");

    // --- Point 4: same-wallet (owned) top-up regression ---
    tracing::info!("=== Step 4: owned-identity top-up regression on owner_wallet ===");
    let owned_top_up_amount = 500_000u64;
    let owned_top_up_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentity(IdentityTopUpInfo {
            qualified_identity: qualified_identity.clone(),
            wallet: owner_wallet.clone(),
            identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
                owned_top_up_amount,
                0,
                2,
            ),
        })),
    )
    .await
    .expect("owned-identity TopUpIdentity should still succeed");

    match owned_top_up_result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            assert_eq!(qi.identity.id(), identity_id, "wrong identity returned");
            assert!(
                qi.identity.balance() > balance_after_cross_wallet_topup,
                "owned top-up must further increase the balance (was {}, now {})",
                balance_after_cross_wallet_topup,
                qi.identity.balance()
            );
            tracing::info!(
                "owned top-up complete: balance now {}, fee={:?}",
                qi.identity.balance(),
                fee_result
            );
        }
        other => panic!("expected ToppedUpIdentity, got: {other:?}"),
    }

    tracing::info!("cross_wallet_topup_e2e PASSED");
}
