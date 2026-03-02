//! Test: Register a DPNS name for an identity.

use crate::harness::CTX;
use crate::identity_helpers::build_identity_registration;
use crate::task_runner::run_task;
use dash_evo_tool::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use rand::Rng;

/// Create identity, register a DPNS name, verify by searching.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 12)]
async fn test_register_dpns_name() {
    let ctx = &*CTX;
    let app_context = &ctx.app_context;

    // Create funded test wallet (needs enough for identity + DPNS registration)
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(2_000_000).await;

    // Register identity
    let registration_info = build_identity_registration(app_context, &wallet_arc, seed_hash);
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(registration_info));
    let result = run_task(app_context, task)
        .await
        .expect("Identity registration should succeed");

    let qualified_identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };

    // Generate a unique DPNS name
    let random_suffix: u32 = rand::rng().random_range(100_000..999_999);
    let dpns_name = format!("e2etest{}", random_suffix);
    println!("  Registering DPNS name: {}", dpns_name);

    // Register DPNS name
    let task = BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
        qualified_identity: qualified_identity.clone(),
        name_input: dpns_name.clone(),
    }));

    let result = run_task(app_context, task)
        .await
        .expect("DPNS registration should succeed");

    match result {
        BackendTaskSuccessResult::RegisteredDpnsName(fee_result) => {
            println!("  DPNS name registered, fee: {:?}", fee_result);
        }
        other => panic!("Expected RegisteredDpnsName, got: {:?}", other),
    }

    // Search for the name to verify
    let task = BackendTask::IdentityTask(IdentityTask::SearchIdentityByDpnsName(
        dpns_name.clone(),
        None,
    ));

    let result = run_task(app_context, task)
        .await
        .expect("DPNS search should succeed");

    match result {
        BackendTaskSuccessResult::LoadedIdentity(found_identity) => {
            assert_eq!(
                found_identity.identity.id(),
                qualified_identity.identity.id(),
                "Found identity should match registered identity"
            );
            println!("  DPNS name '{}' verified via search", dpns_name);
        }
        other => panic!("Expected LoadedIdentity from DPNS search, got: {:?}", other),
    }
}
