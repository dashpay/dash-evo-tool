//! Test: Register a DPNS name for an identity.

use crate::framework::harness::ctx;
use crate::framework::identity_helpers::build_identity_registration;
use crate::framework::task_runner::run_task;
use dash_evo_tool::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use rand::prelude::*;

/// Create identity, register a DPNS name, verify by searching.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_register_dpns_name() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Create funded test wallet (needs enough for identity + DPNS registration)
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(30_000_000).await;

    // Register identity on Platform
    let (reg_info, _master_key_bytes) =
        build_identity_registration(app_context, &wallet_arc, seed_hash);
    let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(reg_info));
    let result = run_task(app_context, task)
        .await
        .expect("Identity registration should succeed");

    let qualified_identity = match result {
        BackendTaskSuccessResult::RegisteredIdentity(qi, _) => qi,
        other => panic!("Expected RegisteredIdentity, got: {:?}", other),
    };

    // Generate a unique DPNS name >= 20 chars to avoid contest voting period.
    // Contested names (< 20 chars) go through masternode voting and don't
    // appear as regular domain documents immediately.
    let random_suffix: u64 = rand::rng().random();
    let dpns_name = format!("e2etest-{:016x}", random_suffix);
    tracing::info!("Registering DPNS name: {}", dpns_name);

    // TODO: DAPI propagation delay on identity registration
    // Expected: RegisterDpnsName succeeds immediately after RegisterIdentity
    // Actual: occasionally fails with "not found" because the identity hasn't
    //         propagated to the DAPI node that processes the DPNS registration
    let task = BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(RegisterDpnsNameInput {
        qualified_identity: qualified_identity.clone(),
        name_input: dpns_name.clone(),
    }));
    let result = run_task(app_context, task)
        .await
        .expect("DPNS registration should succeed");

    match result {
        BackendTaskSuccessResult::RegisteredDpnsName(fee_result) => {
            tracing::info!("DPNS name registered, fee: {:?}", fee_result);
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
            tracing::info!("DPNS name '{}' verified via search", dpns_name);
        }
        other => panic!("Expected LoadedIdentity from DPNS search, got: {:?}", other),
    }
}
