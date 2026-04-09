//! ShieldedTask backend E2E tests (TC-074 to TC-083).
//!
//! All tests are guarded by `E2E_SKIP_SHIELDED` — set the env var to skip
//! these compute-intensive ZK tests. The shielded lifecycle chain runs as a
//! single sequential test:
//! TC-074 (WarmUpProvingKey) -> TC-075 (InitializeShieldedWallet)
//!   -> TC-076 (SyncNotes) -> TC-077 (CheckNullifiers)
//!   -> TC-078 (ShieldFromAssetLock) -> TC-080 (ShieldedTransfer)
//!   -> TC-081 (UnshieldCredits) -> TC-082 (ShieldedWithdrawal)
//! TC-079 (ShieldCredits) is independent (self-funds a platform address).
//! TC-083 tests the error path for an uninitialized wallet.

use crate::framework::harness::ctx;
use crate::framework::shielded_helpers;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::shielded::ShieldedTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::dpp::dashcore::Network;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Lifecycle test — TC-074 through TC-082 as sequential steps
// ---------------------------------------------------------------------------

/// Shielded lifecycle: TC-074 → TC-075 → TC-076 → TC-077 → TC-078 →
///                     TC-080 → TC-081 → TC-082
///
/// Runs the full shielded dependency chain in a single test so that each
/// step can rely on state established by the previous one.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_074_shielded_lifecycle() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    step_warm_up_proving_key(app_context).await;
    step_init_wallet(app_context, seed_hash).await;
    step_sync_notes(app_context, seed_hash).await;
    step_check_nullifiers(app_context, seed_hash).await;

    if !step_shield_from_asset_lock(app_context, seed_hash).await {
        tracing::warn!(
            "tc_074_shielded_lifecycle: platform does not support shielded ops — stopping after TC-078"
        );
        return;
    }

    if !step_shielded_transfer(app_context, seed_hash).await {
        return;
    }
    if !step_unshield(app_context, seed_hash).await {
        return;
    }
    step_withdrawal(app_context, seed_hash).await;
}

// ---------------------------------------------------------------------------
// Step functions
// ---------------------------------------------------------------------------

/// Step 1 (TC-074): WarmUpProvingKey
///
/// Ensures the Halo 2 proving key is downloaded/built and cached.
/// May take 30-60s on first run.
async fn step_warm_up_proving_key(app_context: &Arc<AppContext>) {
    tracing::info!("=== Step 1: WarmUpProvingKey ===");

    let task = BackendTask::ShieldedTask(ShieldedTask::WarmUpProvingKey);
    let result = run_task(app_context, task)
        .await
        .expect("WarmUpProvingKey should succeed");

    assert!(
        matches!(result, BackendTaskSuccessResult::ProvingKeyReady),
        "Expected ProvingKeyReady, got: {:?}",
        result
    );
}

/// Step 2 (TC-075): InitializeShieldedWallet
///
/// Derives ZIP32 keys, loads commitment tree, and returns initial balance (likely 0).
async fn step_init_wallet(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) {
    tracing::info!("=== Step 2: InitializeShieldedWallet ===");

    let task = BackendTask::ShieldedTask(ShieldedTask::InitializeShieldedWallet { seed_hash });
    let result = run_task(app_context, task)
        .await
        .expect("InitializeShieldedWallet should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedInitialized {
            seed_hash: sh,
            balance,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            tracing::info!("Shielded wallet initialized (balance: {} credits)", balance);
        }
        other => panic!("Expected ShieldedInitialized, got: {:?}", other),
    }
}

/// Step 3 (TC-076): SyncNotes
///
/// Trial-decrypts platform notes and updates the commitment tree.
async fn step_sync_notes(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) {
    tracing::info!("=== Step 3: SyncNotes ===");

    let task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("Step 3: skipped — platform does not support shielded ops: {e}");
        }
        Err(e) => panic!("SyncNotes failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedNotesSynced {
            seed_hash: sh,
            new_notes,
            balance,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            tracing::info!(
                "SyncNotes: {} new note(s), balance: {} credits",
                new_notes,
                balance
            );
        }
        Ok(other) => panic!("Expected ShieldedNotesSynced, got: {:?}", other),
    }
}

/// Step 4 (TC-077): CheckNullifiers
///
/// Checks the nullifier set to detect spent notes.
async fn step_check_nullifiers(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) {
    tracing::info!("=== Step 4: CheckNullifiers ===");

    let task = BackendTask::ShieldedTask(ShieldedTask::CheckNullifiers { seed_hash });
    let result = run_task(app_context, task)
        .await
        .expect("CheckNullifiers should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedNullifiersChecked {
            seed_hash: sh,
            spent_count,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            tracing::info!("CheckNullifiers: {} spent note(s) detected", spent_count);
        }
        other => panic!("Expected ShieldedNullifiersChecked, got: {:?}", other),
    }
}

/// Step 5 (TC-078): ShieldFromAssetLock
///
/// Shields core DASH into the shielded pool via an asset lock (Type 18).
/// Returns `false` if the platform does not support shielded ops (caller should stop).
async fn step_shield_from_asset_lock(
    app_context: &Arc<AppContext>,
    seed_hash: WalletSeedHash,
) -> bool {
    tracing::info!("=== Step 5: ShieldFromAssetLock ===");

    let amount_duffs = 500_000; // 0.005 DASH
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
        seed_hash,
        amount_duffs,
        source_address: None,
    });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("Step 5: skipped — platform does not support shielded ops: {e}");
            return false;
        }
        Err(e) => panic!("ShieldFromAssetLock failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert!(amount > 0, "Shielded amount should be > 0, got: {}", amount);
            tracing::info!(
                "ShieldFromAssetLock: shielded {} credits from {} duffs",
                amount,
                amount_duffs
            );
        }
        Ok(other) => panic!("Expected ShieldedFromAssetLock, got: {:?}", other),
    }

    // Verify: SyncNotes should show increased balance
    let sync_task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash });
    let sync_result = run_task(app_context, sync_task).await;

    match sync_result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("Step 5: SyncNotes skipped — platform unsupported: {e}");
            return false;
        }
        Err(e) => panic!("SyncNotes after ShieldFromAssetLock failed: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedNotesSynced { balance, .. }) => {
            assert!(
                balance > 0,
                "Balance after shielding should be > 0, got: {}",
                balance
            );
            tracing::info!("Post-shield balance: {} credits", balance);
        }
        Ok(other) => panic!("Expected ShieldedNotesSynced, got: {:?}", other),
    }

    true
}

/// Step 6 (TC-080): ShieldedTransfer
///
/// Private transfer within the shielded pool (Type 16).
/// Returns `false` if the platform does not support shielded ops.
async fn step_shielded_transfer(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) -> bool {
    tracing::info!("=== Step 6: ShieldedTransfer ===");

    // Use the wallet's own default shielded address as recipient (self-transfer)
    let recipient_address_bytes = app_context
        .shielded_default_address(&seed_hash)
        .expect("shielded wallet should be initialized")
        .to_raw_address_bytes()
        .to_vec();

    let transfer_amount = 50_000;
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldedTransfer {
        seed_hash,
        amount: transfer_amount,
        recipient_address_bytes,
    });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("Step 6: skipped — platform does not support shielded ops: {e}");
            return false;
        }
        Err(e) => panic!("ShieldedTransfer failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedTransferComplete {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, transfer_amount, "transfer amount should match");
            tracing::info!("ShieldedTransfer: transferred {} credits", amount);
        }
        Ok(other) => panic!("Expected ShieldedTransferComplete, got: {:?}", other),
    }

    true
}

/// Step 7 (TC-081): UnshieldCredits
///
/// Unshield credits from the shielded pool to a platform address (Type 17).
/// Returns `false` if the platform does not support shielded ops.
async fn step_unshield(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) -> bool {
    tracing::info!("=== Step 7: UnshieldCredits ===");

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

    let unshield_amount = 30_000;
    let task = BackendTask::ShieldedTask(ShieldedTask::UnshieldCredits {
        seed_hash,
        amount: unshield_amount,
        to_platform_address: platform_addr,
    });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("Step 7: skipped — platform does not support shielded ops: {e}");
            return false;
        }
        Err(e) => panic!("UnshieldCredits failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedCreditsUnshielded {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, unshield_amount, "unshielded amount should match");
            tracing::info!("UnshieldCredits: unshielded {} credits", amount);
        }
        Ok(other) => panic!("Expected ShieldedCreditsUnshielded, got: {:?}", other),
    }

    // Verify: platform address should show credits
    let balance_task =
        BackendTask::WalletTask(WalletTask::FetchPlatformAddressBalances { seed_hash });
    let balance_result = run_task(app_context, balance_task)
        .await
        .expect("FetchPlatformAddressBalances should succeed");

    match balance_result {
        BackendTaskSuccessResult::PlatformAddressBalances { balances, .. } => {
            if let Some((credits, _)) = balances.get(&platform_addr) {
                tracing::info!(
                    "Platform address balance after unshield: {} credits",
                    credits
                );
            }
        }
        other => panic!("Expected PlatformAddressBalances, got: {:?}", other),
    }

    true
}

/// Step 8 (TC-082): ShieldedWithdrawal
///
/// Withdraw from the shielded pool directly to a core L1 address (Type 19).
async fn step_withdrawal(app_context: &Arc<AppContext>, seed_hash: WalletSeedHash) {
    tracing::info!("=== Step 8: ShieldedWithdrawal ===");

    let core_addr = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let wallet_arc = wallets
            .get(&seed_hash)
            .expect("framework wallet must exist");
        let mut wallet = wallet_arc.write().expect("wallet lock");
        wallet
            .receive_address(Network::Testnet, false, Some(app_context))
            .expect("Failed to get receive address")
    };

    let withdrawal_amount = 20_000;
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldedWithdrawal {
        seed_hash,
        amount: withdrawal_amount,
        to_core_address: core_addr.clone(),
    });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("Step 8: skipped — platform does not support shielded ops: {e}");
        }
        Err(e) => panic!("ShieldedWithdrawal failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedWithdrawalComplete {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, withdrawal_amount, "withdrawal amount should match");
            tracing::info!(
                "ShieldedWithdrawal: withdrew {} credits to {}",
                amount,
                core_addr
            );
        }
        Ok(other) => panic!("Expected ShieldedWithdrawalComplete, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Independent tests — not part of the lifecycle chain
// ---------------------------------------------------------------------------

/// TC-079: ShieldCredits
///
/// Shields credits from a funded platform address into the shielded pool (Type 15).
/// Self-funds a platform address via `FundPlatformAddressFromWalletUtxos` first.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_079_shield_credits() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;

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

    // Shield a portion of the credits
    let shield_amount = available_credits / 2;
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldCredits {
        seed_hash,
        amount: shield_amount,
        from_address: platform_addr,
        nonce_override: None,
    });
    let result = run_task(app_context, task).await;

    match result {
        Err(e) if shielded_helpers::is_platform_shielded_unsupported(&e) => {
            tracing::warn!("TC-079: skipped — platform does not support shielded ops: {e}");
            return;
        }
        Err(e) => panic!("ShieldCredits failed unexpectedly: {e:?}"),
        Ok(BackendTaskSuccessResult::ShieldedCreditsShielded {
            seed_hash: sh,
            amount,
        }) => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, shield_amount, "shielded amount should match");
            tracing::info!("ShieldCredits: shielded {} credits", amount);
        }
        Ok(other) => panic!("Expected ShieldedCreditsShielded, got: {:?}", other),
    }
}

/// TC-083: ShieldedTask error - uninitialized wallet
///
/// Attempting SyncNotes on a wallet that has not been initialized should
/// return a typed error, not panic.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_083_error_uninitialized_wallet() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;

    // Use a fake seed hash that has never been initialized
    let fake_seed_hash: WalletSeedHash = [0xDE; 32];

    let task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes {
        seed_hash: fake_seed_hash,
    });
    let result = run_task(app_context, task).await;

    assert!(
        result.is_err(),
        "SyncNotes on uninitialized wallet should fail, got: {:?}",
        result
    );

    let err = result.unwrap_err();
    tracing::info!(
        "Uninitialized wallet error (expected): {} (debug: {:?})",
        err,
        err
    );
}
