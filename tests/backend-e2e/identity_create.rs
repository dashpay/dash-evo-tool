//! Test: Create a new identity funded from a wallet.

use crate::harness::ctx;
use crate::identity_helpers::build_identity_registration;
use crate::task_runner::run_task;
use crate::wait::wait_for_spendable_balance;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use std::time::Duration;

/// Create a funded test wallet, register an identity on Platform, verify it was created.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_create_identity() {
    let ctx = ctx().await;

    // Create a funded test wallet (0.01 DASH = 1_000_000 duffs)
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(1_000_000).await;

    // Wait for test wallet funds to become spendable (confirmed/IS-locked)
    // before attempting identity registration which sends a transaction.
    wait_for_spendable_balance(&ctx.app_context, seed_hash, 1, Duration::from_secs(60))
        .await
        .expect("Test wallet funds should become spendable");

    // Build identity registration info
    let registration_info = build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);

    // Register identity on Platform
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(registration_info));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("Identity registration should succeed");

    match result {
        BackendTaskSuccessResult::RegisteredIdentity(qualified_identity, fee_result) => {
            println!("  Identity created: {:?}", qualified_identity.identity.id());
            println!("  Fee: {:?}", fee_result);
            assert!(
                qualified_identity.identity.balance() > 0,
                "Identity should have a balance"
            );
        }
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    }
}
