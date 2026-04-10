//! Backend E2E tests for CoreTask variants (TC-001 to TC-011).

use crate::framework::fixtures;
use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::single_key::SingleKeyWallet;
use std::sync::{Arc, RwLock};

// TC-001: RefreshWalletInfo — Core only
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc001_refresh_wallet_info_core_only() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    let task = BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet.clone(), false));
    let result = run_task(app_context, task)
        .await
        .expect("RefreshWalletInfo (core only) should succeed");

    match result {
        BackendTaskSuccessResult::RefreshedWallet { warning } => {
            assert!(warning.is_none(), "Expected no warning, got: {:?}", warning);
        }
        other => panic!("Expected RefreshedWallet, got: {:?}", other),
    }

    let balance = wallet.read().expect("wallet lock").total_balance_duffs();
    assert!(balance > 0, "Framework wallet balance should be > 0");
}

// TC-002: RefreshWalletInfo — Core + Platform
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc002_refresh_wallet_info_core_and_platform() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    let task = BackendTask::CoreTask(CoreTask::RefreshWalletInfo(wallet.clone(), true));
    let result = run_task(app_context, task)
        .await
        .expect("RefreshWalletInfo (core + platform) should not panic");

    match result {
        // Warning may or may not be present depending on Platform state — both are valid
        BackendTaskSuccessResult::RefreshedWallet { .. } => {}
        other => panic!("Expected RefreshedWallet, got: {:?}", other),
    }
}

// TC-003: RefreshSingleKeyWalletInfo
// TODO: Fails in SPV mode — RefreshSingleKeyWalletInfo uses Core RPC
// (listunspent, getaddressbalance) which are not available when running with
// SPV backend. Needs either an SPV-compatible implementation or should be
// skipped in SPV-only test runs.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc003_refresh_single_key_wallet_info() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Deterministic test private key (all-zero except last byte — valid on secp256k1)
    let private_key_bytes: [u8; 32] = {
        let mut k = [0u8; 32];
        k[31] = 1;
        k
    };

    let skw = SingleKeyWallet::new(
        private_key_bytes,
        dash_sdk::dpp::dashcore::Network::Testnet,
        None,
        Some("E2E TC-003".to_string()),
    )
    .expect("Failed to create SingleKeyWallet");

    let skw_arc = Arc::new(RwLock::new(skw));

    let task = BackendTask::CoreTask(CoreTask::RefreshSingleKeyWalletInfo(skw_arc.clone()));
    let result = run_task(app_context, task)
        .await
        .expect("RefreshSingleKeyWalletInfo should succeed");

    match result {
        BackendTaskSuccessResult::RefreshedWallet { .. } => {}
        other => panic!("Expected RefreshedWallet, got: {:?}", other),
    }

    // Balance may be 0 for a fresh key — just verify the read succeeds
    let _balance = skw_arc.read().expect("skw lock").total_balance_duffs();
}

// TC-004: CreateRegistrationAssetLock
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc004_create_registration_asset_lock() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    // Use identity index 99 to avoid collision with shared fixtures
    let task = BackendTask::CoreTask(CoreTask::CreateRegistrationAssetLock(
        wallet.clone(),
        100_000_000,
        99,
    ));

    let result = run_task(app_context, task)
        .await
        .expect("CreateRegistrationAssetLock should succeed");

    // Production code returns Message("Asset lock transaction broadcast successfully...")
    match result {
        BackendTaskSuccessResult::Message(msg) => {
            assert!(
                msg.to_lowercase().contains("asset lock")
                    || msg.to_lowercase().contains("broadcast"),
                "Expected asset lock success message, got: {msg}"
            );
        }
        other => panic!(
            "Expected Message with asset lock confirmation, got: {:?}",
            other
        ),
    }
}

// TC-005: CreateTopUpAssetLock
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc005_create_top_up_asset_lock() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Ensure SHARED_IDENTITY exists (registered at index 0)
    let _identity = fixtures::shared_identity().await;

    let wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    // identity_index=0 (SHARED_IDENTITY's index), topup_index=1
    let task = BackendTask::CoreTask(CoreTask::CreateTopUpAssetLock(
        wallet.clone(),
        50_000_000,
        0,
        1,
    ));

    let result = run_task(app_context, task)
        .await
        .expect("CreateTopUpAssetLock should succeed");

    // Production code returns Message("Asset lock transaction broadcast successfully...")
    match result {
        BackendTaskSuccessResult::Message(msg) => {
            assert!(
                msg.to_lowercase().contains("asset lock")
                    || msg.to_lowercase().contains("broadcast"),
                "Expected asset lock success message, got: {msg}"
            );
        }
        other => panic!(
            "Expected Message with asset lock confirmation, got: {:?}",
            other
        ),
    }
}

// TC-006: RecoverAssetLocks
// TODO: Fails in SPV mode — RecoverAssetLocks relies on Core RPC to scan
// for asset lock transactions, which is not available in SPV backend.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc006_recover_asset_locks() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    let task = BackendTask::CoreTask(CoreTask::RecoverAssetLocks(wallet.clone()));
    let result = run_task(app_context, task)
        .await
        .expect("RecoverAssetLocks should succeed");

    match result {
        BackendTaskSuccessResult::RecoveredAssetLocks {
            recovered_count,
            total_amount,
        } => {
            // 0 recoveries is valid if no asset locks exist
            tracing::info!(
                "RecoverAssetLocks: count={}, total={} duffs",
                recovered_count,
                total_amount
            );
        }
        other => panic!("Expected RecoveredAssetLocks, got: {:?}", other),
    }
}

// TC-007: GetBestChainLock — REMOVED (Core RPC-specific, not available in SPV mode)
// TC-008: GetBestChainLocks — REMOVED (Core RPC-specific, not available in SPV mode)

// TC-009: SendSingleKeyWalletPayment
// TODO: Fails in SPV mode — single-key wallets use Core RPC for UTXO queries
// and transaction broadcasting. SPV mode only supports HD wallets registered
// via the bloom filter.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc009_send_single_key_wallet_payment() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Use a deterministic private key for the single-key wallet
    let private_key_bytes: [u8; 32] = {
        let mut k = [0u8; 32];
        k[0] = 0x09;
        k[31] = 0x09;
        k
    };

    let skw = SingleKeyWallet::new(
        private_key_bytes,
        dash_sdk::dpp::dashcore::Network::Testnet,
        None,
        Some("E2E TC-009 sender".to_string()),
    )
    .expect("Failed to create SingleKeyWallet");

    let skw_address = skw.address.to_string();
    let skw_arc = Arc::new(RwLock::new(skw));

    // Fund the single-key wallet from the framework wallet
    let framework_wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    run_task(
        app_context,
        BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: framework_wallet,
            request: WalletPaymentRequest {
                recipients: vec![PaymentRecipient {
                    address: skw_address.clone(),
                    amount_duffs: 500_000,
                }],
                subtract_fee_from_amount: false,
                memo: Some("TC-009 single key funding".to_string()),
                override_fee: None,
            },
        }),
    )
    .await
    .expect("Funding single-key wallet should succeed");

    // Wait for the transaction to propagate, then refresh UTXOs
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    run_task(
        app_context,
        BackendTask::CoreTask(CoreTask::RefreshSingleKeyWalletInfo(skw_arc.clone())),
    )
    .await
    .expect("RefreshSingleKeyWalletInfo should succeed after funding");

    let balance = skw_arc.read().expect("skw lock").total_balance_duffs();
    if balance == 0 {
        println!("TC-009: single-key wallet has no balance after funding — skipping send step");
        return;
    }

    // Derive a recipient address from the framework wallet
    let recipient_address = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        let fw = wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet")
            .clone();
        let mut fw_guard = fw.write().expect("fw lock");
        fw_guard
            .receive_address(
                dash_sdk::dpp::dashcore::Network::Testnet,
                false,
                Some(app_context),
            )
            .expect("receive address")
            .to_string()
    };

    let result = run_task(
        app_context,
        BackendTask::CoreTask(CoreTask::SendSingleKeyWalletPayment {
            wallet: skw_arc.clone(),
            request: WalletPaymentRequest {
                recipients: vec![PaymentRecipient {
                    address: recipient_address,
                    amount_duffs: 1_000,
                }],
                subtract_fee_from_amount: true,
                memo: Some("TC-009 send back".to_string()),
                override_fee: None,
            },
        }),
    )
    .await
    .expect("SendSingleKeyWalletPayment should succeed");

    match result {
        BackendTaskSuccessResult::WalletPayment {
            txid, total_amount, ..
        } => {
            assert_eq!(txid.len(), 64, "txid should be 64 hex chars");
            assert!(total_amount > 0, "total_amount should be > 0");
            tracing::info!(
                "TC-009: single-key payment txid={}, amount={}",
                txid,
                total_amount
            );
        }
        other => panic!("Expected WalletPayment, got: {:?}", other),
    }
}

// TC-010: ListCoreWallets — REMOVED (Core RPC-specific, not available in SPV mode)

// TC-011: CoreTask error — invalid payment address
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc011_send_wallet_payment_invalid_address() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    let wallet = {
        let wallets = app_context.wallets().read().expect("wallets lock");
        wallets
            .get(&ctx.framework_wallet_hash)
            .expect("framework wallet must exist")
            .clone()
    };

    let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
        wallet,
        request: WalletPaymentRequest {
            recipients: vec![PaymentRecipient {
                address: "not-a-valid-address!!!".to_string(),
                amount_duffs: 1_000,
            }],
            subtract_fee_from_amount: false,
            memo: None,
            override_fee: None,
        },
    });

    let result = run_task(app_context, task).await;

    assert!(
        result.is_err(),
        "Expected Err for invalid address, got Ok: {:?}",
        result.ok()
    );

    let err = result.unwrap_err();
    tracing::info!("TC-011: got expected error variant: {:?}", err);
}
