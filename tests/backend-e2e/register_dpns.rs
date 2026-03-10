//! Test: Register a DPNS name for an identity.

use crate::harness::ctx;
use crate::identity_helpers::build_identity_registration;
use crate::task_runner::run_task;
use crate::wait::wait_for_spendable_balance;
use dash_evo_tool::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use rand::Rng;
use std::time::Duration;

/// Create identity, register a DPNS name, verify by searching.
#[ignore]
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_register_dpns_name() {
    let ctx = ctx().await;
    let app_context = &ctx.app_context;

    // Create funded test wallet (needs enough for identity + DPNS registration)
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(2_000_000).await;

    // Wait for test wallet funds to become spendable before identity registration
    wait_for_spendable_balance(app_context, seed_hash, 1, Duration::from_secs(60))
        .await
        .expect("Test wallet funds should become spendable");

    // Register identity (with retry for transient chain sync issues)
    let mut last_error = String::new();
    let mut reg_result = None;
    for attempt in 1..=3 {
        let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
            build_identity_registration(app_context, &wallet_arc, seed_hash),
        ));
        match run_task(app_context, task).await {
            Ok(r) => {
                reg_result = Some(r);
                break;
            }
            Err(e) => {
                let err_str = e.to_string();
                if attempt < 3
                    && (err_str.contains("chain height")
                        || err_str.contains("Timeout waiting for asset lock"))
                {
                    println!(
                        "  Identity registration attempt {}/3 failed ({}), retrying in 30s...",
                        attempt, err_str
                    );
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    last_error = err_str;
                    continue;
                }
                panic!("Identity registration should succeed: {}", e);
            }
        }
    }
    let result = reg_result.unwrap_or_else(|| {
        panic!(
            "Identity registration failed after 3 attempts: {}",
            last_error
        )
    });

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
