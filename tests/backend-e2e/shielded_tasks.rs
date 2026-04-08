//! ShieldedTask backend E2E tests (TC-074 to TC-083).
//!
//! All tests are guarded by `E2E_SKIP_SHIELDED` — set the env var to skip
//! these compute-intensive ZK tests. The shielded lifecycle chain is:
//! TC-074 (WarmUpProvingKey) -> TC-075 (InitializeShieldedWallet)
//!   -> TC-076 (SyncNotes), TC-077 (CheckNullifiers)
//!   -> TC-078 (ShieldFromAssetLock) -> TC-080 (ShieldedTransfer)
//!   -> TC-081 (UnshieldCredits), TC-082 (ShieldedWithdrawal)
//! TC-079 (ShieldCredits) is independent (self-funds a platform address).
//! TC-083 tests the error path for an uninitialized wallet.

use crate::framework::harness::ctx;
use crate::framework::shielded_helpers;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::shielded::ShieldedTask;
use dash_evo_tool::backend_task::wallet::WalletTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_sdk::dpp::dashcore::Network;

/// TC-074: WarmUpProvingKey
///
/// Ensures the Halo 2 proving key is downloaded/built and cached.
/// May take 30-60s on first run.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_074_warm_up_proving_key() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;

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

/// TC-075: InitializeShieldedWallet
///
/// Derives ZIP32 keys, loads commitment tree, and returns initial balance (likely 0).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_075_initialize_shielded_wallet() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

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

/// TC-076: SyncNotes
///
/// Trial-decrypts platform notes and updates the commitment tree.
/// Requires TC-075 (shielded wallet initialized).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_076_sync_notes() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;

    let task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash });
    let result = run_task(app_context, task)
        .await
        .expect("SyncNotes should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedNotesSynced {
            seed_hash: sh,
            new_notes,
            balance,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            tracing::info!(
                "SyncNotes: {} new note(s), balance: {} credits",
                new_notes,
                balance
            );
        }
        other => panic!("Expected ShieldedNotesSynced, got: {:?}", other),
    }
}

/// TC-077: CheckNullifiers
///
/// Checks the nullifier set to detect spent notes.
/// Requires TC-075 (shielded wallet initialized).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_077_check_nullifiers() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;

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

/// TC-078: ShieldFromAssetLock
///
/// Shields core DASH into the shielded pool via an asset lock (Type 18).
/// Requires: proving key (TC-074), initialized wallet (TC-075), funded framework wallet.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_078_shield_from_asset_lock() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;

    let amount_duffs = 500_000; // 0.005 DASH
    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
        seed_hash,
        amount_duffs,
        source_address: None,
    });
    let result = run_task(app_context, task)
        .await
        .expect("ShieldFromAssetLock should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedFromAssetLock {
            seed_hash: sh,
            amount,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert!(amount > 0, "Shielded amount should be > 0, got: {}", amount);
            tracing::info!(
                "ShieldFromAssetLock: shielded {} credits from {} duffs",
                amount,
                amount_duffs
            );
        }
        other => panic!("Expected ShieldedFromAssetLock, got: {:?}", other),
    }

    // Verify: SyncNotes should show increased balance
    let sync_task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash });
    let sync_result = run_task(app_context, sync_task)
        .await
        .expect("SyncNotes after ShieldFromAssetLock should succeed");

    match sync_result {
        BackendTaskSuccessResult::ShieldedNotesSynced { balance, .. } => {
            assert!(
                balance > 0,
                "Balance after shielding should be > 0, got: {}",
                balance
            );
            tracing::info!("Post-shield balance: {} credits", balance);
        }
        other => panic!("Expected ShieldedNotesSynced, got: {:?}", other),
    }
}

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
    let result = run_task(app_context, task)
        .await
        .expect("ShieldCredits should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedCreditsShielded {
            seed_hash: sh,
            amount,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, shield_amount, "shielded amount should match");
            tracing::info!("ShieldCredits: shielded {} credits", amount);
        }
        other => panic!("Expected ShieldedCreditsShielded, got: {:?}", other),
    }
}

/// TC-080: ShieldedTransfer
///
/// Private transfer within the shielded pool (Type 16).
/// Requires shielded balance > 0 (from TC-078 ShieldFromAssetLock).
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_080_shielded_transfer() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;

    // Ensure we have shielded balance by shielding from asset lock first
    ensure_shielded_balance(app_context, seed_hash).await;

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
    let result = run_task(app_context, task)
        .await
        .expect("ShieldedTransfer should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedTransferComplete {
            seed_hash: sh,
            amount,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, transfer_amount, "transfer amount should match");
            tracing::info!("ShieldedTransfer: transferred {} credits", amount);
        }
        other => panic!("Expected ShieldedTransferComplete, got: {:?}", other),
    }
}

/// TC-081: UnshieldCredits
///
/// Unshield credits from the shielded pool to a platform address (Type 17).
/// Requires shielded balance > 0.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_081_unshield_credits() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;
    ensure_shielded_balance(app_context, seed_hash).await;

    // Get a platform address as destination
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
    let result = run_task(app_context, task)
        .await
        .expect("UnshieldCredits should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedCreditsUnshielded {
            seed_hash: sh,
            amount,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, unshield_amount, "unshielded amount should match");
            tracing::info!("UnshieldCredits: unshielded {} credits", amount);
        }
        other => panic!("Expected ShieldedCreditsUnshielded, got: {:?}", other),
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
}

/// TC-082: ShieldedWithdrawal
///
/// Withdraw from the shielded pool directly to a core L1 address (Type 19).
/// Requires shielded balance > 0.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn tc_082_shielded_withdrawal() {
    if shielded_helpers::skip_if_shielded_disabled() {
        return;
    }

    let test_ctx = ctx().await;
    let app_context = &test_ctx.app_context;
    let seed_hash = test_ctx.framework_wallet_hash;

    shielded_helpers::warm_up_and_init(app_context, seed_hash).await;
    ensure_shielded_balance(app_context, seed_hash).await;

    // Get a core L1 address from the framework wallet
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
    let result = run_task(app_context, task)
        .await
        .expect("ShieldedWithdrawal should succeed");

    match result {
        BackendTaskSuccessResult::ShieldedWithdrawalComplete {
            seed_hash: sh,
            amount,
        } => {
            assert_eq!(sh, seed_hash, "seed_hash should match");
            assert_eq!(amount, withdrawal_amount, "withdrawal amount should match");
            tracing::info!(
                "ShieldedWithdrawal: withdrew {} credits to {}",
                amount,
                core_addr
            );
        }
        other => panic!("Expected ShieldedWithdrawalComplete, got: {:?}", other),
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

/// Ensure the framework wallet has shielded balance by performing a
/// ShieldFromAssetLock if needed. Syncs notes afterward.
async fn ensure_shielded_balance(
    app_context: &std::sync::Arc<dash_evo_tool::context::AppContext>,
    seed_hash: WalletSeedHash,
) {
    // Sync notes first to see current balance
    let sync_task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash });
    let sync_result = run_task(app_context, sync_task)
        .await
        .expect("SyncNotes should succeed");

    let balance = match sync_result {
        BackendTaskSuccessResult::ShieldedNotesSynced { balance, .. } => balance,
        other => panic!("Expected ShieldedNotesSynced, got: {:?}", other),
    };

    if balance > 100_000 {
        tracing::info!(
            "ensure_shielded_balance: already have {} credits, skipping shield",
            balance
        );
        return;
    }

    tracing::info!(
        "ensure_shielded_balance: balance {} is low, shielding from asset lock...",
        balance
    );

    let task = BackendTask::ShieldedTask(ShieldedTask::ShieldFromAssetLock {
        seed_hash,
        amount_duffs: 500_000, // 0.005 DASH
        source_address: None,
    });
    run_task(app_context, task)
        .await
        .expect("ShieldFromAssetLock should succeed");

    // Sync notes to pick up the new shielded balance
    let sync_task = BackendTask::ShieldedTask(ShieldedTask::SyncNotes { seed_hash });
    let sync_result = run_task(app_context, sync_task)
        .await
        .expect("SyncNotes after shielding should succeed");

    match sync_result {
        BackendTaskSuccessResult::ShieldedNotesSynced { balance, .. } => {
            assert!(
                balance > 0,
                "ensure_shielded_balance: balance should be > 0 after shielding, got: {}",
                balance
            );
            tracing::info!(
                "ensure_shielded_balance: balance is now {} credits",
                balance
            );
        }
        other => panic!("Expected ShieldedNotesSynced, got: {:?}", other),
    }
}
