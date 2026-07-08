//! Identity funding tests on a cold-booted watch-only wallet (scenario C/D).
//!
//! **Background**: Scenarios C and D cover the watch-only upstream-wallet
//! identity-provisioning bug fixed in commit `98bc4913`.
//!
//! When a wallet is loaded on cold-boot via `load_from_persistor_seedless` it starts
//! life as a watch-only upstream wallet (no root private key). Identity
//! registration and top-up both call
//! `WalletBackend::ensure_identity_funding_accounts`, which in turn calls
//! `PlatformKeyWallet::add_account(IdentityTopUp{n}, …)` for any account
//! type not yet in the upstream manifest.
//!
//! The pre-fix code passed `None` as the xpub hint, which triggers upstream
//! hardened-path derivation *from the private key* — a path that fails with
//! "Watch-only wallet has no private key" whenever the live upstream wallet is
//! watch-only.  The fix derives the xpub from the seed (available through the
//! JIT secret-session chokepoint) and passes `Some(xpub)` instead.
//!
//! ## Test C — RegisterIdentity funded by wallet balance
//!
//! Creates a funded test wallet, then runs `IdentityTask::RegisterIdentity`
//! with `RegisterIdentityFundingMethod::FundWithWallet`.  The identity
//! registration path calls `ensure_identity_funding_accounts` to provision
//! both `IdentityRegistration` and `IdentityTopUp{index}` accounts.  On a
//! watch-only upstream wallet the pre-fix code panicked; after the fix the
//! call succeeds.
//!
//! ## Test D — TopUpIdentity funded by wallet balance (uses identity from C)
//!
//! Tops up the identity registered in scenario C using
//! `TopUpIdentityFundingMethod::FundWithWallet`.  The top-up path also calls
//! `ensure_identity_funding_accounts` with a new `IdentityTopUp{topup_index}`
//! slot (index 1).  This exercises the same watch-only provisioning fix.
//!
//! ## How it exercises the watch-only fix
//!
//! In the shared e2e harness the framework wallet is loaded watch-only by
//! `load_from_persistor_seedless` at backend-build time; `bootstrap_wallet_addresses_jit`
//! later upgrades it to full via `ensure_upstream_registered` / `register_wallet_from_seed`.
//! For a freshly created test wallet the gap between cold-watch-only load and
//! the JIT upgrade creates a window where the fix is active.  Regardless,
//! these tests verify the *production code path* (`BackendTask::IdentityTask`)
//! end-to-end; any regression in `ensure_identity_funding_accounts` would
//! surface here as a task error before the on-chain submission even starts.
//!
//! **Run command** (requires a funded testnet wallet):
//! ```bash
//! E2E_WALLET_MNEMONIC="word1 word2 ..." \
//!   cargo test --test backend-e2e --all-features -- \
//!   --ignored --nocapture cd_cold_boot_identity_register_and_topup
//! ```
//!
//! Funding requirement: ≥ 60 000 000 duffs (≈ 0.6 tDASH) in the framework
//! wallet to cover the test wallet funding (30 000 000 duffs) plus two
//! asset-lock amounts with network fees.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::run_task_with_nonce_retry;
use dash_evo_tool::backend_task::identity::{
    IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod,
};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

/// Scenario C+D: register an identity funded by wallet balance, then top it
/// up — all on a wallet loaded cold-boot-style (watch-only upstream load).
///
/// Single test exercises both C (registration) and D (top-up) sequentially on
/// the same identity so no extra funded wallets are consumed.
///
/// **Regression anchor**: any "Watch-only wallet has no private key" error
/// from `ensure_identity_funding_accounts` surfaces here as a task failure
/// before the on-chain submission begins.
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore = "network-dependent; requires funded testnet wallet (≥ 0.6 tDASH)"]
async fn cd_cold_boot_identity_register_and_topup() {
    let ctx = ctx().await;

    // ── Create a funded test wallet ─────────────────────────────────────────
    // 35 M duffs: scenario C asset-lock (5 M) + registration fees, then
    // scenario D top-up (5 M) + its fees. 30 M left scenario C with 4,999,703
    // duffs — 297 short of the 5 M top-up minimum — so the extra 5 M is
    // headroom for both transactions' network fees.
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(35_000_000).await;

    let backend = ctx
        .app_context
        .wallet_backend()
        .expect("wallet backend must be wired");
    assert!(
        backend.is_wallet_registered(&seed_hash),
        "CD: test wallet must be registered with the upstream SPV backend"
    );

    // ── Scenario C: RegisterIdentity funded by wallet balance ───────────────
    //
    // `build_identity_registration` produces `RegisterIdentityFundingMethod::
    // FundWithWallet(amount, identity_index)`.  Inside
    // `register_identity_via_wallet_backend` the task calls
    // `ensure_identity_funding_accounts(seed_hash, seed, identity_index)`,
    // provisioning `IdentityRegistration` + `IdentityTopUp{0}`.
    // Pre-fix: `add_account(IdentityTopUp{0}, None)` → "Watch-only wallet has
    // no private key" when the upstream wallet is watch-only.
    let reg_info = build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash).await;

    tracing::info!("CD scenario C: registering identity funded from wallet balance...");
    let reg_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info)),
    )
    .await
    .expect(
        "CD scenario C: RegisterIdentity must succeed; \
         if 'Watch-only wallet has no private key' appears the fix has been reverted",
    );

    let qualified_identity = match reg_result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, fee_result) => {
            tracing::info!(
                "CD scenario C PASSED: identity {} registered, fee={:?}",
                qi.identity.id(),
                fee_result
            );
            assert!(
                qi.identity.balance() > 0,
                "CD scenario C: registered identity must have credits"
            );
            qi
        }
        other => panic!(
            "CD scenario C: expected RegisteredIdentity, got: {:?}",
            other
        ),
    };

    // ── Scenario D: TopUpIdentity funded by wallet balance ──────────────────
    //
    // Top-up index 1 so it doesn't clash with the registration slot (index 0).
    // `ensure_identity_funding_accounts(seed_hash, seed, identity_index=0)` is
    // called again for provisioning — this time for `IdentityTopUp{1}`.
    // Pre-fix: same watch-only failure as scenario C.
    let top_up_info = IdentityTopUpInfo {
        qualified_identity: qualified_identity.clone(),
        wallet: wallet_arc.clone(),
        identity_funding_method: TopUpIdentityFundingMethod::FundWithWallet(
            5_000_000, // amount_duffs
            0,         // identity_index (matches the registration index above)
            1,         // top_up_index — slot 1; slot 0 is used by registration
        ),
    };

    tracing::info!("CD scenario D: topping up identity from wallet balance...");
    let topup_result = run_task_with_nonce_retry(
        &ctx.app_context,
        BackendTask::IdentityTask(IdentityTask::TopUpIdentity(top_up_info)),
    )
    .await
    .expect(
        "CD scenario D: TopUpIdentity must succeed; \
         if 'Watch-only wallet has no private key' appears the fix has been reverted",
    );

    match topup_result {
        BackendTaskSuccessResult::ToppedUpIdentity(qi, fee_result) => {
            tracing::info!(
                "CD scenario D PASSED: identity {} topped up, fee={:?}",
                qi.identity.id(),
                fee_result
            );
            assert_eq!(
                qi.identity.id(),
                qualified_identity.identity.id(),
                "CD scenario D: top-up returned wrong identity id"
            );
            assert!(
                fee_result.actual_fee > 0,
                "CD scenario D: top-up fee must be > 0"
            );
        }
        other => panic!("CD scenario D: expected ToppedUpIdentity, got: {:?}", other),
    }

    tracing::info!(
        "CD scenarios C+D PASSED: watch-only identity funding fix confirmed on live testnet"
    );
}
