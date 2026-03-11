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
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
async fn test_create_identity() {
    let ctx = ctx().await;

    // Asset lock (1M duffs) + tx fees. 2M duffs is sufficient.
    let (seed_hash, wallet_arc) = ctx.create_funded_test_wallet(2_000_000).await;

    // Wait for test wallet funds to become spendable (confirmed/IS-locked)
    // before attempting identity registration which sends a transaction.
    wait_for_spendable_balance(&ctx.app_context, seed_hash, 1, Duration::from_secs(120))
        .await
        .expect("Test wallet funds should become spendable");

    // Register identity on Platform (with retry for transient chain sync issues)
    let mut last_error = String::new();
    let mut result = None;
    for attempt in 1..=3 {
        let task = BackendTask::IdentityTask(IdentityTask::RegisterIdentity(
            build_identity_registration(&ctx.app_context, &wallet_arc, seed_hash),
        ));
        match run_task(&ctx.app_context, task).await {
            Ok(r) => {
                result = Some(r);
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
    let result = result.unwrap_or_else(|| {
        panic!(
            "Identity registration failed after 3 attempts: {}",
            last_error
        )
    });

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
