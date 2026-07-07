//! Backend E2E tests for CoreTask variants (TC-001 to TC-012).

use crate::framework::fixtures;
use crate::framework::harness::ctx;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::model::wallet::single_key::SingleKeyWallet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

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

    let balance = app_context
        .snapshot_balance(&ctx.framework_wallet_hash)
        .total;
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
//
// Single-key wallets are intentionally unsupported this release (PROJ-007 /
// single-key-mock.md, Decision #7): every single-key task arm returns the typed
// `SingleKeyWalletsUnsupported`. The test asserts that typed outcome rather than
// an unconditional success.
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
    let result = run_task(app_context, task).await;

    // Single-key wallets are unsupported in this version; the backend
    // returns the typed `SingleKeyWalletsUnsupported` error.
    let err = result.expect_err("RefreshSingleKeyWalletInfo must fail with a typed error");
    assert!(
        matches!(
            err,
            dash_evo_tool::backend_task::error::TaskError::SingleKeyWalletsUnsupported
        ),
        "Expected SingleKeyWalletsUnsupported, got: {:?}",
        err
    );
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

    match result {
        BackendTaskSuccessResult::Message(msg) => {
            tracing::info!("TC-004: asset lock broadcast message: {}", msg);
        }
        other => panic!(
            "TC-004: expected Message from CreateRegistrationAssetLock, got: {:?}",
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

    match result {
        BackendTaskSuccessResult::Message(msg) => {
            tracing::info!("TC-005: asset lock broadcast message: {}", msg);
        }
        other => panic!(
            "TC-005: expected Message from CreateTopUpAssetLock, got: {:?}",
            other
        ),
    }
}

// TC-006: RecoverAssetLocks — REMOVED (Decision #8: hard-removed; upstream
// AssetLockManager tracks asset locks continuously, no explicit recovery pass)

// TC-007: GetBestChainLock — REMOVED (Core RPC-specific, not available in SPV mode)
// TC-008: GetBestChainLocks — REMOVED (Core RPC-specific, not available in SPV mode)

// TC-009: SendSingleKeyWalletPayment
//
// Single-key wallets are intentionally unsupported this release (PROJ-007 /
// single-key-mock.md, Decision #7): every single-key task arm returns the typed
// `SingleKeyWalletsUnsupported`. The test verifies that
// `RefreshSingleKeyWalletInfo` returns `SingleKeyWalletsUnsupported` and stops;
// the single-key send flow is unreachable until single-key wallets are reinstated.
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

    let skw_arc = Arc::new(RwLock::new(skw));

    // Single-key wallets are unsupported this release (PROJ-007): the refresh
    // arm returns the typed `SingleKeyWalletsUnsupported` regardless of network
    // mode. We verify the typed error and stop; the send step is unreachable
    // until single-key wallets are reinstated.
    let refresh_result = run_task(
        app_context,
        BackendTask::CoreTask(CoreTask::RefreshSingleKeyWalletInfo(skw_arc.clone())),
    )
    .await;

    let err = refresh_result.expect_err("RefreshSingleKeyWalletInfo must fail with a typed error");
    assert!(
        matches!(
            err,
            dash_evo_tool::backend_task::error::TaskError::SingleKeyWalletsUnsupported
        ),
        "Expected SingleKeyWalletsUnsupported, got: {:?}",
        err
    );
    tracing::info!(
        "TC-009: single-key wallets are unsupported this release; \
         verified typed SingleKeyWalletsUnsupported error and skipping send step."
    );

    // ----------------------------------------------------------------------
    // Planned: full single-key wallet send flow, unreachable today under SPV.
    //
    // Once single-key wallets gain SPV parity (UTXO refresh + broadcast
    // without Core RPC) the block below should be un-commented so the test
    // exercises the real happy path: refresh UTXOs, derive a recipient from
    // the framework wallet, send a small payment back, and assert the
    // resulting `WalletPayment` txid + amount.
    //
    // Left in place (commented) so the intent and the shape of the future
    // assertions survive the SPV-only gap — saves re-deriving this when SPV
    // parity lands.
    // ----------------------------------------------------------------------
    //
    // refresh_result.expect("RefreshSingleKeyWalletInfo should succeed after funding");
    //
    // let balance = skw_arc.read().expect("skw lock").total_balance_duffs();
    // if balance == 0 {
    //     tracing::warn!(
    //         "TC-009: SKIPPED — single-key wallet has no balance after funding + refresh. \
    //          Core RPC did not return any UTXOs for the funded address."
    //     );
    //     return;
    // }
    //
    // // Derive a recipient address from the framework wallet
    // let recipient_address = {
    //     let wallets = app_context.wallets().read().expect("wallets lock");
    //     let fw = wallets
    //         .get(&ctx.framework_wallet_hash)
    //         .expect("framework wallet")
    //         .clone();
    //     let mut fw_guard = fw.write().expect("fw lock");
    //     fw_guard
    //         .receive_address(
    //             dash_sdk::dpp::dashcore::Network::Testnet,
    //             false,
    //             Some(app_context),
    //         )
    //         .expect("receive address")
    //         .to_string()
    // };
    //
    // let result = run_task(
    //     app_context,
    //     BackendTask::CoreTask(CoreTask::SendSingleKeyWalletPayment {
    //         wallet: skw_arc.clone(),
    //         request: WalletPaymentRequest {
    //             recipients: vec![PaymentRecipient {
    //                 address: recipient_address,
    //                 amount_duffs: 1_000,
    //             }],
    //             override_fee: None,
    //         },
    //     }),
    // )
    // .await
    // .expect("SendSingleKeyWalletPayment should succeed");
    //
    // match result {
    //     BackendTaskSuccessResult::WalletPayment {
    //         txid, total_amount, ..
    //     } => {
    //         assert_eq!(txid.len(), 64, "txid should be 64 hex chars");
    //         assert!(total_amount > 0, "total_amount should be > 0");
    //         tracing::info!(
    //             "TC-009: single-key payment txid={}, amount={}",
    //             txid,
    //             total_amount
    //         );
    //     }
    //     other => panic!("Expected WalletPayment, got: {:?}", other),
    // }
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

// TC-012: CreateRegistrationAssetLock — freshly-registered wallet, added
// *after* SPV is already connected and synced, as opposed to TC-004's
// framework wallet (registered before the SPV client's first sync pass —
// see `BackendTestContext::init` in `framework/harness.rs`). Isolates
// whether a wallet's coin-selection can pick an incorrectly-spendable UTXO
// regardless of when it was registered, or whether that is specific to
// wallets present from the client's initial (potentially long-lived,
// history-accumulating) sync. See
// docs/ai-design/2026-07-07-asset-lock-finality-retest/retest-findings.md
// for the investigation this isolates.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_tc012_create_registration_asset_lock_late_added_wallet() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // A brand-new wallet has no chain history predating its own funding
    // transaction below — there is no window in which a real spend of one
    // of its outputs could have happened without this test observing it.
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(2_000_000).await;

    // `create_funded_test_wallet` only waits for `DetWalletBalance::spendable()`
    // (confirmed + unconfirmed — a UI-display heuristic, see
    // `wallet_backend/snapshot.rs`'s FUND-SAFETY MANDATE doc comment). The
    // real asset-lock coin-selector requires strictly confirmed or IS-locked
    // funds, so a plain unconfirmed mempool deposit satisfies `spendable()`
    // immediately while still yielding "No UTXOs available for selection" in
    // the asset-lock builder — a harness/product balance-classification
    // mismatch, not the question this test isolates. Wait for `.confirmed`
    // specifically (IS-lock or block confirmation) before attempting the
    // asset lock, so a genuine coin-selection defect isn't confounded with
    // this test-harness timing gap. Budgeted for a full Dash block (~2.5min
    // average) since InstantSend lock arrival isn't guaranteed within any
    // fixed window in practice — observed anywhere from a few seconds to a
    // 180s window with zero IS-lock activity at all during this
    // investigation.
    let confirm_deadline = std::time::Instant::now() + Duration::from_secs(420);
    loop {
        let confirmed = app_context.snapshot_balance(&seed_hash).confirmed;
        if confirmed >= 2_000_000 {
            break;
        }
        assert!(
            std::time::Instant::now() < confirm_deadline,
            "TC-012: funding tx for the late-added wallet did not reach confirmed \
             status (IS-lock or block) within 420s — confirmed={confirmed}, target=2_000_000"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Use identity index 98 to avoid collision with TC-004 (index 99) and
    // the shared fixtures (index 0). Credits match TC-004 (100_000_000
    // credits == 100_000 duffs) for a direct comparison.
    let task = BackendTask::CoreTask(CoreTask::CreateRegistrationAssetLock(
        wallet_arc.clone(),
        100_000_000,
        98,
    ));

    let result = run_task(app_context, task)
        .await
        .expect("CreateRegistrationAssetLock should succeed for a freshly-added, funded wallet");

    match result {
        BackendTaskSuccessResult::Message(msg) => {
            tracing::info!("TC-012: asset lock broadcast message: {}", msg);
        }
        other => panic!(
            "TC-012: expected Message from CreateRegistrationAssetLock, got: {:?}",
            other
        ),
    }
}
