//! Test: Create a new identity funded from a wallet.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::identity::IdentityTask;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;

/// Create a funded test wallet, register an identity on Platform, verify it was created.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_create_identity() {
    let ctx = ctx().await;

    // Asset lock (1M duffs) + tx fees. 2M duffs is sufficient.
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(2_000_000).await;

    // Register identity on Platform
    let (reg_info, _master_key_bytes) =
        build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash);
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info));
    let result = run_task(&ctx.app_context, task)
        .await
        .expect("Identity registration should succeed");

    match result {
        BackendTaskSuccessResult::RegisteredIdentity(qualified_identity, fee_result) => {
            tracing::info!("Identity created: {:?}", qualified_identity.identity.id());
            tracing::info!("Fee: {:?}", fee_result);
            assert!(
                qualified_identity.identity.balance() > 0,
                "Identity should have a balance"
            );
        }
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    }
}
